//! Named pipe server at `\\.\pipe\toki`, per DESIGN.md §8.
//!
//! One client at a time, length-prefixed JSON: a four-byte little-endian length, then that
//! many bytes of a request or response. Liveness — a connect succeeding at all — is already
//! the answer to "is the daemon running," which is the whole point of a pipe over, say, a
//! lock file.
//!
//! Decisions post onto the same channels the tray already uses — [`Raw`] for pause and a
//! live config change, [`ResolverMsg`] for a resolver kick — so the collector remains the
//! only thing that assembles sessions and the resolver remains the only thing that reads
//! browser history. Settings writes (aliases, the ignore list, the idle threshold's
//! persisted copy) go through a `Store` connection this thread opens for itself, the same
//! pattern the resolver already uses for its own updates — simple values with no bearing
//! on the collector's in-memory state, so there is nothing to hand off.

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use windows::core::w;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use toki_store::Store;

use super::Raw;
use crate::paths;
use crate::resolver::ResolverMsg;
use crate::state::Shared;

/// Guards against a garbage length prefix turning into a multi-gigabyte allocation. Every
/// real message here is a handful of fields.
const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    GetState,
    /// `for_ms` is a duration from now, matching the tray's own pause menu, and is ignored
    /// on unpause.
    SetPause {
        paused: bool,
        for_ms: Option<i64>,
    },
    ResolveNow,
    SetAppAlias {
        exe_path: String,
        alias: Option<String>,
    },
    SetDomainAlias {
        domain: String,
        alias: Option<String>,
    },
    SetIgnored {
        exe_path: String,
        ignored: bool,
    },
    SetIdleThresholdMs {
        ms: i64,
    },
}

static STOP: AtomicBool = AtomicBool::new(false);

/// Ask the server to shut down. Safe to call before it has started.
///
/// A pending `ConnectNamedPipe` only returns when a client connects, so shutdown dials the
/// pipe itself to unblock it — the same idea as `pump::stop` posting a message to unblock
/// `GetMessage`.
pub fn stop() {
    STOP.store(true, Ordering::Relaxed);
    if let Ok(h) = unsafe {
        CreateFileW(
            w!("\\\\.\\pipe\\toki"),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    } {
        let _ = unsafe { CloseHandle(h) };
    }
}

/// Run the server until [`stop`] is called. Intended as the body of a dedicated thread.
///
/// Opens its own `Store` connection, same reasoning as the resolver: SQLite's WAL mode
/// makes a second connection from the same process cheap, and it keeps Settings writes off
/// the collector's connection without needing a lock around it.
pub fn run(tx: Sender<Raw>, resolve_tx: Sender<ResolverMsg>, shared: Arc<Shared>) {
    let Ok(store) = Store::open(&paths::db_path()) else {
        return;
    };
    loop {
        if STOP.load(Ordering::Relaxed) {
            return;
        }
        let Some(pipe) = create_pipe() else {
            // Whatever stopped pipe creation from working is unlikely to clear in the next
            // instant either; back off rather than spin a thread on it.
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        let connected = unsafe { ConnectNamedPipe(pipe, None) }.is_ok()
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if STOP.load(Ordering::Relaxed) {
            let _ = unsafe { CloseHandle(pipe) };
            return;
        }
        if connected {
            // Consumes and closes the handle, so nothing here does.
            serve(pipe, &tx, &resolve_tx, &shared, &store);
        } else {
            let _ = unsafe { CloseHandle(pipe) };
        }
    }
}

fn create_pipe() -> Option<HANDLE> {
    let handle = unsafe {
        CreateNamedPipeW(
            w!("\\\\.\\pipe\\toki"),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            None,
        )
    };
    (!handle.is_invalid()).then_some(handle)
}

/// Serve requests on one connection until the client disconnects or sends something that
/// cannot be understood. Wrapped in a `File` so framing is `read_exact`/`write_all` instead
/// of hand-rolled `ReadFile`/`WriteFile` loops.
fn serve(
    pipe: HANDLE,
    tx: &Sender<Raw>,
    resolve_tx: &Sender<ResolverMsg>,
    shared: &Shared,
    store: &Store,
) {
    let mut pipe = unsafe { File::from_raw_handle(pipe.0 as RawHandle) };
    loop {
        let mut len = [0u8; 4];
        if pipe.read_exact(&mut len).is_err() {
            return;
        }
        let len = u32::from_le_bytes(len) as usize;
        if len > MAX_MESSAGE {
            return;
        }
        let mut body = vec![0u8; len];
        if pipe.read_exact(&mut body).is_err() {
            return;
        }
        let Ok(request) = serde_json::from_slice::<Request>(&body) else {
            return;
        };

        let response = handle(request, tx, resolve_tx, shared, store);
        let Ok(body) = serde_json::to_vec(&response) else {
            return;
        };
        let sent = pipe
            .write_all(&(body.len() as u32).to_le_bytes())
            .and_then(|_| pipe.write_all(&body));
        if sent.is_err() {
            return;
        }
    }
}

fn handle(
    req: Request,
    tx: &Sender<Raw>,
    resolve_tx: &Sender<ResolverMsg>,
    shared: &Shared,
    store: &Store,
) -> Value {
    let at = Utc::now().timestamp_millis();
    match req {
        Request::GetState => pause_state(shared),
        Request::SetPause {
            paused: true,
            for_ms,
        } => {
            let until = for_ms.map(|d| at + d);
            let _ = tx.send(Raw::Pause { at, until });
            json!({ "paused": true, "until_ms": until })
        }
        Request::SetPause { paused: false, .. } => {
            let _ = tx.send(Raw::Unpause { at });
            json!({ "paused": false, "until_ms": null })
        }
        Request::ResolveNow => {
            let _ = resolve_tx.send(ResolverMsg::RunNow);
            pause_state(shared)
        }
        Request::SetAppAlias { exe_path, alias } => {
            ack(store.set_app_alias(&exe_path, alias.as_deref()))
        }
        Request::SetDomainAlias { domain, alias } => {
            ack(store.set_domain_alias(&domain, alias.as_deref()))
        }
        Request::SetIgnored { exe_path, ignored } => ack(store.set_ignored(&exe_path, ignored)),
        Request::SetIdleThresholdMs { ms } => match store.set_idle_threshold_ms(ms) {
            Ok(()) => {
                // Persisted for the next restart; this reaches the *running* tracker too,
                // since a store write alone has no way to touch the collector's own state.
                let _ = tx.send(Raw::SetConfig {
                    idle_threshold_ms: ms,
                });
                json!({ "ok": true })
            }
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        },
    }
}

fn pause_state(shared: &Shared) -> Value {
    let s = shared.get();
    json!({ "paused": s.paused, "until_ms": s.until })
}

fn ack(result: rusqlite::Result<()>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}
