//! Everything that knows Windows exists. The Linux side is `super::linux`.
//!
//! No module here decides what gets recorded. They report facts — which window is in front,
//! when the keyboard was last touched, whether sound is playing, whether the machine went
//! to sleep — and [`toki_core`] draws the conclusions. That line is what makes the rules
//! testable without a desktop.

pub mod audio;
pub mod autostart;
pub mod idle;
pub mod ipc;
pub mod memory;
pub mod pump;
pub mod single;
pub mod tray;
pub mod window;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The running process's user SID in `S-1-5-…` form.
pub fn user_sid() -> Option<String> {
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

/// Ctrl+C and console close, so a daemon run from a terminal closes its session cleanly
/// instead of leaving it for crash recovery.
pub mod console {
    use windows::core::BOOL;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{
        FreeConsole, GetConsoleProcessList, SetConsoleCtrlHandler, SetStdHandle, STD_ERROR_HANDLE,
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    unsafe extern "system" fn handler(_kind: u32) -> BOOL {
        // Every control event means the same thing to us. Returning TRUE says it is
        // handled; Windows still terminates the process after a short grace period on
        // close and logoff, which is why shutdown must be prompt rather than thorough.
        super::pump::stop();
        true.into()
    }

    pub fn trap() {
        let _ = unsafe { SetConsoleCtrlHandler(Some(handler), true) };
    }

    /// Let go of a console that exists only because we were started, and report whether one
    /// is still attached.
    ///
    /// Started from the Run key, Windows hands a console-subsystem process a console window
    /// of its own — which for a daemon means a black window sitting on the taskbar for the
    /// entire session. Started from a terminal, the shell is attached to the same console
    /// and the count is at least two, so output stays where it was asked for. Counting is
    /// exact where a subsystem flag would have to guess.
    pub fn detach_if_ours() -> bool {
        match attached() {
            0 => false,
            1 => {
                let _ = unsafe { FreeConsole() };
                // FreeConsole leaves the standard handles pointing at the console it just
                // released, and a child spawned with inherited stdio then fails outright:
                // os error 50 on every tray click to open the GUI. Null is what a GUI
                // process starts with, and what `std::process` passes on as "no handle".
                for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                    let _ = unsafe { SetStdHandle(which, HANDLE::default()) };
                }
                false
            }
            _ => true,
        }
    }

    /// Whether this process is alone on its console — started by a double-click or the Run
    /// key rather than typed into a terminal. Nobody is there to read a usage message.
    pub fn is_ours() -> bool {
        attached() == 1
    }

    /// Processes sharing this console. Returns the number attached, or the buffer size
    /// needed, which is also more than one and so answers the only question being asked.
    fn attached() -> u32 {
        let mut pids = [0u32; 4];
        unsafe { GetConsoleProcessList(&mut pids) }
    }
}
