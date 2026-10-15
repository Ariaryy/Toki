//! The IPC protocol, independent of what carries it. DESIGN.md §8.
//!
//! One client at a time, length-prefixed JSON: a four-byte little-endian length, then that
//! many bytes of a request or response. The transport — a named pipe on Windows, a Unix
//! socket on Linux — lives in `platform::ipc` and hands each connection to [`serve`].
//!
//! Decisions post onto the same channels the tray already uses — [`Raw`] for pause and a
//! live config change, [`ResolverMsg`] for a resolver kick — so the collector remains the
//! only thing that assembles sessions and the resolver remains the only thing that reads
//! browser history. Settings writes (aliases, the ignore list, the idle threshold's
//! persisted copy) go through a `Store` connection the transport thread opens for itself,
//! the same pattern the resolver already uses for its own updates — simple values with no
//! bearing on the collector's in-memory state, so there is nothing to hand off.

use std::io::{Read, Write};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use toki_store::Store;
use tracing::{info, warn};

use crate::platform::Raw;
use crate::resolver::ResolverMsg;
use crate::state::Shared;

/// Guards against a garbage length prefix turning into a multi-gigabyte allocation. Every
/// real message here is a handful of fields.
const MAX_MESSAGE: usize = 64 * 1024;

/// The longest timed pause a client may ask for. The menus offer an hour at most; the bound
/// keeps `now + for_ms` from overflowing, which panics the IPC thread in a debug build and
/// wraps to an end in the past, an instant unpause, in a release one.
const MAX_PAUSE_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

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
    SetHistoryReading {
        on: bool,
    },
}

/// Serve requests on one connection until the client disconnects or sends something that
/// cannot be understood.
/// The IPC thread's own database connection, retried with backoff until it opens or the
/// daemon stops. Given up on after one failure, the GUI would read the daemon as not
/// running for as long as it tracked.
pub fn open_store(stopped: impl Fn() -> bool) -> Option<Store> {
    let mut wait = Duration::from_secs(1);
    let mut failing = false;
    while !stopped() {
        match Store::open(&toki_store::db_path()) {
            Ok(store) => {
                if failing {
                    info!("IPC server opened the database");
                }
                return Some(store);
            }
            Err(e) if !failing => {
                warn!(%e, "IPC server cannot open the database yet; retrying");
                failing = true;
            }
            Err(_) => {}
        }
        let until = Instant::now() + wait;
        while Instant::now() < until && !stopped() {
            std::thread::sleep(Duration::from_millis(250));
        }
        wait = (wait * 2).min(Duration::from_secs(60));
    }
    None
}

pub fn serve(
    pipe: &mut (impl Read + Write),
    tx: &Sender<Raw>,
    resolve_tx: &Sender<ResolverMsg>,
    shared: &Shared,
    store: &Store,
) {
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
        // Refused without a change; the reply is the state as it stands.
        Request::SetPause {
            paused: true,
            for_ms: Some(d),
        } if !(1..=MAX_PAUSE_MS).contains(&d) => pause_state(shared),
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
        Request::SetHistoryReading { on } => {
            let written = store.set_history_reading(on);
            // Switched back on: fill in what waited while it was off, now rather than at
            // the next trigger.
            if on && written.is_ok() {
                let _ = resolve_tx.send(ResolverMsg::RunNow);
            }
            ack(written)
        }
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

fn ack(result: toki_store::Result<()>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn pause_for(for_ms: i64) -> (Value, Option<Raw>) {
        let (tx, rx) = channel();
        let (resolve_tx, _resolve_rx) = channel();
        let reply = handle(
            Request::SetPause {
                paused: true,
                for_ms: Some(for_ms),
            },
            &tx,
            &resolve_tx,
            &Shared::default(),
            &Store::open_in_memory().unwrap(),
        );
        (reply, rx.try_recv().ok())
    }

    #[test]
    fn a_pause_length_out_of_range_changes_nothing() {
        for d in [i64::MAX, 0, -1] {
            let (reply, sent) = pause_for(d);
            assert!(sent.is_none());
            assert_eq!(reply["paused"], false);
        }
    }

    #[test]
    fn an_hour_long_pause_is_passed_on() {
        let (_, sent) = pause_for(60 * 60 * 1_000);
        assert!(matches!(sent, Some(Raw::Pause { until: Some(_), .. })));
    }
}
