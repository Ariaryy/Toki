//! One collector per machine.
//!
//! Two daemons would both open the database, both write sessions, and both mirror an open
//! session into `meta` — where the second one's crash recovery would close the first one's
//! live session. SQLite's locking would not catch it: every individual write is valid.

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// Held for the lifetime of the process. Windows releases it if we are killed.
pub struct Instance(HANDLE);

impl Instance {
    /// Claim the daemon slot, or report that another process already holds it.
    ///
    /// The name is in the `Global\` namespace so a second daemon started in another
    /// terminal session — a second logon, or a service — is still refused.
    pub fn acquire() -> Option<Self> {
        let handle = unsafe { CreateMutexW(None, true, w!("Global\\TokiDaemon")) }.ok()?;
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
