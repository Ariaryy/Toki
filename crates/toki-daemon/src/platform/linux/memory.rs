//! This process's memory, for `toki-daemon bench`.

/// Resident set and private (anonymous) resident bytes right now.
pub fn usage() -> Option<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = |key: &str| -> Option<u64> {
        let line = status.lines().find(|l| l.starts_with(key))?;
        line.split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
            .map(|k| k * 1024)
    };
    Some((kb("VmRSS:")?, kb("RssAnon:")?))
}

/// Linux has no way to evict a process's own resident pages, so this does nothing; the
/// peak is still sampled, it just starts higher.
pub fn trim() {}
