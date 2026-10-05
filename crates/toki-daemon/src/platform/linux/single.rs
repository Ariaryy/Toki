//! One collector per user.
//!
//! Two daemons would both write sessions and both mirror an open session into `meta`, where
//! the second one's crash recovery would close the first one's live session. An advisory
//! `flock` is the Unix counterpart of the Windows mutex: the kernel drops it when the
//! process dies, however it dies, so a crash never leaves the slot taken.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

/// Held for the lifetime of the process.
pub struct Instance(#[allow(dead_code)] File);

impl Instance {
    /// Claim the daemon slot, or report that another process already holds it.
    pub fn acquire() -> Option<Self> {
        let dir = toki_store::data_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("daemon.lock"))
            .ok()?;
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        (locked == 0).then_some(Self(file))
    }
}
