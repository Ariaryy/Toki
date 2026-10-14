//! This process's memory, for `toki-daemon bench`.

/// Resident set now and its peak since the process started, in bytes.
pub fn working_set() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = |key: &str| -> Option<u64> {
        let line = status.lines().find(|l| l.starts_with(key))?;
        line.split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
            .map(|k| k * 1024)
    };
    Some((kb("VmRSS:")?, kb("VmHWM:")?))
}
