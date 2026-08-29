//! A thin client for the daemon's named pipe (`\\.\pipe\toki`) — the GUI's only path to
//! writing anything, since its own database connection is read-only (DESIGN.md §7 keeps the
//! daemon the sole writer). Opening a pipe as a client is just opening a file path on
//! Windows, so this needs nothing beyond `std::fs`, unlike the daemon's server side
//! (`toki-daemon::platform::ipc`), which has to create the pipe instance itself via raw
//! `CreateNamedPipeW`.
//!
//! One function, not one per message: the wire protocol is already generic JSON on both
//! ends, and typing out a parallel Rust API for every daemon message here would just be a
//! second copy of the `Request` enum the daemon already defines.

use std::fs::OpenOptions;
use std::io::{Read, Write};

use serde_json::Value;

/// Send one request, wait for the reply. Blocking — the daemon answers after at most one
/// database write, so a synchronous round trip on its own thread (a Tauri command, off the
/// UI thread) is simplest.
pub fn call(request: &Value) -> std::io::Result<Value> {
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(r"\\.\pipe\toki")?;

    let body = serde_json::to_vec(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    pipe.write_all(&(body.len() as u32).to_le_bytes())?;
    pipe.write_all(&body)?;

    let mut len = [0u8; 4];
    pipe.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    let mut buf = vec![0u8; len];
    pipe.read_exact(&mut buf)?;
    serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
