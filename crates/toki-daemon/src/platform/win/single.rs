//! One collector per user.
//!
//! Two daemons would both open the database, both write sessions, and both mirror an open
//! session into `meta` — where the second one's crash recovery would close the first one's
//! live session. SQLite's locking would not catch it: every individual write is valid.

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// Held for the lifetime of the process. Windows releases it if we are killed.
pub struct Instance(HANDLE);

impl Instance {
    /// Claim the daemon slot, or report that another process already holds it.
    ///
    /// Named for the user, because the database is per user: another account on the
    /// machine runs its own daemon. In the `Global\` namespace so the same user is still
    /// refused a second daemon from another session, such as a remote desktop logon.
    pub fn acquire() -> Option<Self> {
        let name = match super::user_sid() {
            Some(sid) => format!(r"Global\TokiDaemon-{sid}"),
            None => r"Global\TokiDaemon".to_string(),
        };
        let handle = unsafe { CreateMutexW(None, true, &HSTRING::from(name)) }.ok()?;
        // The handle is returned even when the mutex already existed, so the error code is
        // the only signal.
        if unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = unsafe { CloseHandle(handle) };
            return None;
        }
        Some(Self(handle))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
