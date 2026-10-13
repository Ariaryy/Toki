//! A thin client for the daemon's named pipe (`\\.\pipe\toki`) on Windows, or its Unix
//! socket (`toki_store::ipc_socket_path`) on Linux — the GUI's only path to writing anything,
//! since its own database connection is read-only (DESIGN.md §7 keeps the daemon the sole
//! writer). Opening a pipe as a client is just opening a file path on Windows, and a socket
//! is one `connect` on Linux, so this needs nothing beyond `std`, unlike the daemon's server
//! side (`toki-daemon::platform::ipc`).
//!
//! One function, not one per message: the wire protocol is already generic JSON on both
//! ends, and typing out a parallel Rust API for every daemon message here would just be a
//! second copy of the `Request` enum the daemon already defines.

use std::io::{Read, Write};

use serde_json::Value;

/// Send one request, wait for the reply. Blocking — the daemon answers after at most one
/// database write, so a synchronous round trip on its own thread (a Tauri command, off the
/// UI thread) is simplest.
pub fn call(request: &Value) -> std::io::Result<Value> {
    let mut pipe = connect()?;

    let body = serde_json::to_vec(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    pipe.write_all(&(body.len() as u32).to_le_bytes())?;
    pipe.write_all(&body)?;

    let mut len = [0u8; 4];
    pipe.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_REPLY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "reply too large",
        ));
    }
    let mut buf = vec![0u8; len];
    pipe.read_exact(&mut buf)?;
    serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Every real reply is a handful of fields. Trusting a larger length would let whatever
/// answers on the pipe make the GUI allocate up to 4 GB.
const MAX_REPLY: usize = 64 * 1024;

/// Both server instances taken: one being served, the other by a call made at the same
/// moment. The retry outlasts the daemon's 2s stall timeout, so even a client that went
/// quiet on one instance has been dropped before this gives up.
#[cfg(windows)]
const ERROR_PIPE_BUSY: i32 = 231;

#[cfg(windows)]
fn connect() -> std::io::Result<std::fs::File> {
    let open = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(r"\\.\pipe\toki")
    };
    for _ in 0..50 {
        match open() {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            result => return result,
        }
    }
    open()
}

#[cfg(unix)]
fn connect() -> std::io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(toki_store::ipc_socket_path())
}
