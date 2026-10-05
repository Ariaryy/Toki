//! Named pipe server at `\\.\pipe\toki`, per DESIGN.md §8.
//!
//! Liveness — a connect succeeding at all — is already the answer to "is the daemon
//! running," which is the whole point of a pipe over, say, a lock file. What travels over
//! it is `crate::ipc`'s business.

use std::fs::File;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

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

use crate::paths;
use crate::platform::Raw;
use crate::resolver::ResolverMsg;
use crate::state::Shared;

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
            // The `File` takes ownership and closes the handle on drop, so nothing here does.
            // Wrapped so framing is `read_exact`/`write_all` instead of hand-rolled
            // `ReadFile`/`WriteFile` loops.
            let mut pipe = unsafe { File::from_raw_handle(pipe.0 as RawHandle) };
            crate::ipc::serve(&mut pipe, &tx, &resolve_tx, &shared, &store);
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
