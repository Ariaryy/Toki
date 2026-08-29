//! A thin client for the daemon's named pipe, used only to ask for an immediate resolver
//! pass on launch — DESIGN.md §6 names this explicitly ("every 5 min, and on GUI launch"),
//! and until now nothing ever sent it.
//!
//! Opening a named pipe as a client is just opening a file path on Windows, so this needs
//! nothing beyond `std::fs` — unlike the daemon's server side (`toki-daemon::platform::ipc`),
//! which has to create the pipe instance itself via raw `CreateNamedPipeW`.

use std::fs::OpenOptions;
use std::io::{Read, Write};

use serde::Deserialize;

/// Only enough of the daemon's response to prove the round trip parsed; `resolve_now`
/// doesn't need the pause state it carries.
#[derive(Deserialize)]
struct Response {
    #[allow(dead_code)]
    paused: bool,
    #[allow(dead_code)]
    until_ms: Option<i64>,
}

/// Best-effort: a daemon that isn't running, or a pipe instance that's briefly busy, is not
/// an error worth surfacing. A missed kick just means the resolver's own 5-minute timer
/// catches the same rows shortly after.
pub fn resolve_now() -> std::io::Result<()> {
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(r"\\.\pipe\toki")?;

    let body = br#"{"type":"resolve_now"}"#;
    pipe.write_all(&(body.len() as u32).to_le_bytes())?;
    pipe.write_all(body)?;

    let mut len = [0u8; 4];
    pipe.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    let mut buf = vec![0u8; len];
    pipe.read_exact(&mut buf)?;
    let _: Response = serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(())
}
