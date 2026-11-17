//! Memory budget, so the browser stays usable on a 4 GB laptop: when the browser grows past
//! its budget, or the computer itself runs low, background tabs are put to sleep (oldest
//! first) until it's back under. A sleeping tab keeps its place and history and reloads
//! when shown.

/// Megabytes from a /proc/meminfo line such as `MemAvailable:  123456 kB`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn meminfo(text: &str, key: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with(key))?;
    let kb: u64 = line[key.len()..].trim().trim_start_matches(':').trim().split_whitespace().next()?.parse().ok()?;
    Some(kb / 1024)
}

/// The browser's resident memory in MB.
pub fn rss_mb() -> u64 {
    crate::platform::rss_mb()
}

/// How much memory the browser may use: 40% of the computer's memory, at most 2 GB
/// (IRONSWIFT_MEMORY_BUDGET_MB overrides).
pub fn budget_mb(total_mb: u64) -> u64 {
    if let Some(mb) = std::env::var("IRONSWIFT_MEMORY_BUDGET_MB").ok().and_then(|v| v.parse().ok()) {
        return mb;
    }
    (total_mb * 2 / 5).clamp(512, 2048)
}

/// Should background tabs be freed? The browser is over budget, or the computer has less
/// than 300 MB (or 8%) left.
pub fn under_pressure() -> bool {
    let Some((total, avail)) = crate::platform::memory_mb() else {
        return false;
    };
    pressure(rss_mb(), total, avail)
}

fn pressure(rss: u64, total: u64, avail: u64) -> bool {
    rss > budget_mb(total) || avail < 300.max(total / 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_and_pressure() {
        let info = "MemTotal:        3884000 kB\nMemFree:  100 kB\nMemAvailable:    1200000 kB\n";
        assert_eq!(meminfo(info, "MemTotal"), Some(3792));
        assert_eq!(meminfo(info, "MemAvailable"), Some(1171));
        // A 4 GB laptop: the browser gets about 1.5 GB.
        assert_eq!(budget_mb(3792), 1516);
        assert!(!pressure(800, 3792, 1171));
        assert!(pressure(1600, 3792, 1171), "over budget");
        assert!(pressure(800, 3792, 250), "computer nearly out");
        assert_eq!(budget_mb(32000), 2048);
    }
}
