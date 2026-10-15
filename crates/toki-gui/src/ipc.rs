//! A thin client for the daemon's named pipe (`\\.\pipe\toki-<user SID>`) on Windows, or its Unix
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
use std::time::Duration;

use serde_json::Value;

/// How long a call waits for the daemon. Its slowest reply is one database write; anything
/// near this means it is hung, and the caller should say so rather than wait forever.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Send one request, wait for the reply. Blocking: callers are `async` Tauri commands, which
/// Tauri runs on a worker thread, not the window's main thread.
#[cfg(unix)]
pub fn call(request: &Value) -> std::io::Result<Value> {
    round_trip(request)
}

/// A synchronous pipe handle has no timeout, so the round trip runs on its own thread and
/// this waits for it at most `TIMEOUT`. A thread left behind stays blocked until the daemon
/// answers or exits, which breaks the pipe.
#[cfg(windows)]
pub fn call(request: &Value) -> std::io::Result<Value> {
    let request = request.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(round_trip(&request));
    });
    rx.recv_timeout(TIMEOUT).unwrap_or_else(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "the daemon did not answer",
        ))
    })
}

fn round_trip(request: &Value) -> std::io::Result<Value> {
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
    let name = match user_sid() {
        Some(sid) => format!(r"\\.\pipe\toki-{sid}"),
        None => r"\\.\pipe\toki".to_string(),
    };
    let open = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
    };
    for _ in 0..50 {
        match open() {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            result => return result.and_then(verified),
        }
    }
    open().and_then(verified)
}

/// The pipe, if the process serving it is the `toki-daemon.exe` beside this GUI.
///
/// Pipe names are machine-wide and anyone may create one, so another account could take
/// this user's name before the daemon starts and answer in its place. It could not put a
/// file in this user's install folder, so the server's executable is what is checked.
#[cfg(windows)]
fn verified(pipe: std::fs::File) -> std::io::Result<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut pid = 0u32;
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }
        .map_err(std::io::Error::other)?;
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(std::io::Error::other)?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let named = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    named.map_err(std::io::Error::other)?;

    let server = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
    let expected = std::env::current_exe()?.with_file_name("toki-daemon.exe");
    // A missing `expected` means no daemon is installed beside this GUI: refused the same.
    if std::fs::canonicalize(&expected).ok() != Some(std::fs::canonicalize(&server)?) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "the pipe is served by {}, not Toki's daemon",
                server.display()
            ),
        ));
    }
    Ok(pipe)
}

#[cfg(unix)]
fn connect() -> std::io::Result<std::os::unix::net::UnixStream> {
    let socket = std::os::unix::net::UnixStream::connect(toki_store::ipc_socket_path())?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    socket.set_write_timeout(Some(TIMEOUT))?;
    Ok(socket)
}

/// The running process's user SID, which names the daemon's pipe. The same lookup as
/// `toki-daemon`'s `platform::win::user_sid`; the two crates share no Windows code.
#[cfg(windows)]
fn user_sid() -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    let mut len = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let got = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(token) };
    got.ok()?;

    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }.ok()?;
    let sid = unsafe { text.to_string() }.ok();
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    sid
}
