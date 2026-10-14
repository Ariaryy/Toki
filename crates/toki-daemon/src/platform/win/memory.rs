//! This process's memory, for `toki-daemon bench`.

use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::GetCurrentProcess;

/// Working set now and its peak since the process started, in bytes.
pub fn working_set() -> Option<(u64, u64)> {
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) }.ok()?;
    Some((c.WorkingSetSize as u64, c.PeakWorkingSetSize as u64))
}
