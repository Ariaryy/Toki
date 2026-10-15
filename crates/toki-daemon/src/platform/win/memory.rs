//! This process's memory, for `toki-daemon bench`.

use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};

/// Working set and private bytes right now.
pub fn usage() -> Option<(u64, u64)> {
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) }.ok()?;
    Some((c.WorkingSetSize as u64, c.PagefileUsage as u64))
}

/// Page out everything not in use, so what a measured step touches shows up as growth
/// rather than hiding under pages left resident by the setup before it.
pub fn trim() {
    let _ = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}
