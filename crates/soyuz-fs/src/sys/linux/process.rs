//! This process's memory counters from `/proc/self/status` (a procfs read, no syscall
//! beyond open / read; it never blocks on a disk).

/// `(RssAnon, VmRSS)` in bytes: resident anonymous memory (heap, stacks; what this process
/// alone holds) and the whole resident set (plus mapped files and shared memory).
pub(crate) fn memory() -> Option<(u64, u64)> {
    parse_status(&std::fs::read_to_string("/proc/self/status").ok()?)
}

fn parse_status(s: &str) -> Option<(u64, u64)> {
    let kb = |key: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix(key))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map(|k| k * 1024)
    };
    let rss = kb("VmRSS:")?;
    Some((kb("RssAnon:").unwrap_or(rss), rss))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_status() {
        let s = "Name:\tsoyuz\nVmRSS:\t  204800 kB\nRssAnon:\t  102400 kB\nRssFile:\t 90000 kB\n";
        assert_eq!(super::parse_status(s), Some((100 << 20, 200 << 20)));
        assert_eq!(super::parse_status("VmRSS:\t1 kB\n"), Some((1024, 1024)));
        assert_eq!(super::parse_status("Name: x\n"), None);
        assert!(super::memory().is_some());
    }
}
