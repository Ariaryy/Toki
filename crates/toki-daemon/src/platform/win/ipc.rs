//! Named pipe server at `\\.\pipe\toki`, per DESIGN.md §8.
//!
//! Liveness — a connect succeeding at all — is already the answer to "is the daemon
//! running," which is the whole point of a pipe over, say, a lock file. What travels over
//! it is `crate::ipc`'s business.
//!
//! The pipe is the daemon's only way in, so it is guarded against anything else on the
//! machine. Only the user running the daemon may open it, remote clients are refused, and
//! the first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a process that
//! took the name first cannot pose as the server. Every read and write is overlapped with a
//! timeout: a client that connects and goes quiet costs two seconds, not the only instance
//! for good. The same wait also watches a stop event, which is how shutdown interrupts it.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use tracing::warn;
use windows::core::{w, HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE,
    HLOCAL, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent, SetEvent,
    WaitForMultipleObjects, INFINITE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use toki_store::Store;

use crate::paths;
use crate::platform::Raw;
use crate::resolver::ResolverMsg;
use crate::state::Shared;

/// How long one read or write may stall. The GUI writes its request the moment it
/// connects, so a client that is quiet this long is broken or squatting.
const IO_TIMEOUT_MS: u32 = 2_000;

/// The connection being served plus the one listening for the next client. Two is all a
/// serial server ever holds, and the cap keeps a misbehaving client from piling up more.
const MAX_INSTANCES: u32 = 2;

static STOP: AtomicBool = AtomicBool::new(false);

/// A manual-reset event, set once by [`stop`]. Held as a raw value because `HANDLE` is a
/// pointer and cannot sit in a static directly.
static STOP_EVENT: OnceLock<Option<usize>> = OnceLock::new();

fn stop_event() -> Option<HANDLE> {
    let raw = STOP_EVENT.get_or_init(|| {
        unsafe { CreateEventW(None, true, false, None) }
            .ok()
            .map(|h| h.0 as usize)
    });
    raw.map(|r| HANDLE(r as *mut _))
}

/// Ask the server to shut down. Safe to call before it has started.
pub fn stop() {
    STOP.store(true, Ordering::Relaxed);
    if let Some(event) = stop_event() {
        let _ = unsafe { SetEvent(event) };
    }
}

/// Run the server until [`stop`] is called. Intended as the body of a dedicated thread.
///
/// Opens its own `Store` connection, same reasoning as the resolver: SQLite's WAL mode
/// makes a second connection from the same process cheap, and it keeps Settings writes off
/// the collector's connection without needing a lock around it.
pub fn run(tx: Sender<Raw>, resolve_tx: Sender<ResolverMsg>, shared: Arc<Shared>) {
    let store = match Store::open(&paths::db_path()) {
        Ok(store) => store,
        Err(e) => {
            warn!(%e, "IPC server not started: cannot open the database");
            return;
        }
    };
    let (Some(stop), Some(security)) = (stop_event(), Security::current_user()) else {
        warn!("IPC server not started: cannot set up the pipe's security");
        return;
    };

    let mut listening: Option<Pipe> = None;
    let mut warned = false;
    while !STOP.load(Ordering::Relaxed) {
        // With no instance of ours alive, this creation is the first one, and asking for
        // that is what fails if another process already owns the name.
        let Some(pipe) = listening
            .take()
            .or_else(|| Pipe::create(&security, stop, true))
        else {
            if !warned {
                warn!("cannot create \\\\.\\pipe\\toki; another process may hold the name");
                warned = true;
            }
            // Whatever stopped pipe creation is unlikely to clear in the next instant;
            // back off, but stay interruptible.
            unsafe { WaitForMultipleObjects(&[stop], false, 5_000) };
            continue;
        };
        warned = false;

        match pipe.accept() {
            Accept::Connected => {}
            Accept::Stopped => return,
            Accept::Failed => continue,
        }
        // The next instance listens before this one is served, so a client arriving while
        // the server is busy waits on a pipe rather than finding none and reporting the
        // daemon down.
        listening = Pipe::create(&security, stop, false);
        let mut pipe = pipe;
        crate::ipc::serve(&mut pipe, &tx, &resolve_tx, &shared, &store);
    }
}

/// A security descriptor that admits only the user the daemon runs as.
struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl Security {
    fn current_user() -> Option<Self> {
        // Protected DACL, one entry: full access for this user's SID. Without it the
        // default DACL lets every account on the machine read the pipe. The explicit
        // medium label keeps the GUI able to write if the daemon was started elevated
        // (an object made by a high-integrity process can otherwise refuse medium
        // writers), and refuses low-integrity writers such as sandboxed browser tabs.
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;{})S:(ML;;NW;;;ME)", user_sid()?));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .ok()?;
        Some(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            },
        })
    }
}

impl Drop for Security {
    fn drop(&mut self) {
        unsafe { LocalFree(Some(HLOCAL(self.descriptor.0))) };
    }
}

/// The running process's user SID in `S-1-5-…` form, for the SDDL string.
fn user_sid() -> Option<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.ok()?;
    let mut len = 0u32;
    // Sizing call: fails by design and reports the length it needs.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
    // `u64`s rather than bytes so the buffer is aligned for the `TOKEN_USER` read below.
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

enum Accept {
    Connected,
    Stopped,
    Failed,
}

/// One overlapped pipe instance, with the event its operations complete on.
struct Pipe {
    handle: HANDLE,
    event: HANDLE,
    stop: HANDLE,
}

impl Pipe {
    fn create(security: &Security, stop: HANDLE, first: bool) -> Option<Self> {
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let handle = unsafe {
            CreateNamedPipeW(
                w!("\\\\.\\pipe\\toki"),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                MAX_INSTANCES,
                4096,
                4096,
                0,
                Some(&security.attributes),
            )
        };
        if handle.is_invalid() {
            return None;
        }
        match unsafe { CreateEventW(None, true, false, None) } {
            Ok(event) => Some(Self {
                handle,
                event,
                stop,
            }),
            Err(_) => {
                let _ = unsafe { CloseHandle(handle) };
                None
            }
        }
    }

    /// Wait for a client. Not timed: an idle daemon waits here most of its life.
    fn accept(&self) -> Accept {
        let mut ov = self.overlapped();
        let started = unsafe { ConnectNamedPipe(self.handle, Some(&mut ov)) };
        // A client that connected between creation and this call is already here.
        if let Err(e) = &started {
            if e.code() == ERROR_PIPE_CONNECTED.to_hresult() {
                return Accept::Connected;
            }
        }
        match self.finish(&ov, started, INFINITE) {
            Ok(_) => Accept::Connected,
            Err(_) if STOP.load(Ordering::Relaxed) => Accept::Stopped,
            Err(_) => Accept::Failed,
        }
    }

    fn overlapped(&self) -> OVERLAPPED {
        let _ = unsafe { ResetEvent(self.event) };
        OVERLAPPED {
            hEvent: self.event,
            ..Default::default()
        }
    }

    /// Complete an overlapped call, giving up after `timeout_ms` or when the server stops.
    fn finish(
        &self,
        ov: &OVERLAPPED,
        started: windows::core::Result<()>,
        timeout_ms: u32,
    ) -> io::Result<u32> {
        if let Err(e) = started {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(to_io(e));
            }
            let woke =
                unsafe { WaitForMultipleObjects(&[self.event, self.stop], false, timeout_ms) };
            if woke != WAIT_OBJECT_0 {
                // The kernel still owns `ov` and the buffer until the cancel lands, so wait
                // for it before either goes out of scope.
                let _ = unsafe { CancelIoEx(self.handle, Some(ov)) };
                let mut n = 0;
                let _ = unsafe { GetOverlappedResult(self.handle, ov, &mut n, true) };
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "pipe stalled, or the server is stopping",
                ));
            }
        }
        let mut n = 0;
        unsafe { GetOverlappedResult(self.handle, ov, &mut n, false) }.map_err(to_io)?;
        Ok(n)
    }
}

impl io::Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut ov = self.overlapped();
        let started = unsafe { ReadFile(self.handle, Some(buf), None, Some(&mut ov)) };
        match self.finish(&ov, started, IO_TIMEOUT_MS) {
            // The client hung up: end of stream, which `read_exact` turns into its own error.
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE.0 as i32) => Ok(0),
            r => r.map(|n| n as usize),
        }
    }
}

impl io::Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut ov = self.overlapped();
        let started = unsafe { WriteFile(self.handle, Some(buf), None, Some(&mut ov)) };
        self.finish(&ov, started, IO_TIMEOUT_MS).map(|n| n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
        let _ = unsafe { CloseHandle(self.event) };
    }
}

/// A Win32 failure as an `io::Error` carrying the original error code.
///
/// Only a `FACILITY_WIN32` HRESULT carries a Win32 code in its low word; anything else is
/// passed through whole rather than masked into an unrelated code.
fn to_io(e: windows::core::Error) -> io::Error {
    const FACILITY_WIN32: i32 = 7;
    let hr = e.code().0;
    if (hr >> 16) & 0x1FFF == FACILITY_WIN32 {
        io::Error::from_raw_os_error(hr & 0xFFFF)
    } else {
        io::Error::other(e)
    }
}
