//! Memory use of this process (About dialog, automation status).

/// Memory counters of the calling process, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessMemory {
    /// Memory only this process holds: private bytes (commit charge) on Windows,
    /// resident anonymous memory (`RssAnon`) on Linux.
    pub private: u64,
    /// Resident set: the working set on Windows, `VmRSS` on Linux (includes mapped files
    /// such as the executable and fonts, and memory shared with other processes).
    pub resident: u64,
}

/// This process's memory counters; `None` where the OS has no cheap query (macOS) or the
/// query failed. One non-blocking call (`K32GetProcessMemoryInfo`, a procfs read), cheap
/// enough to run on the UI thread every few seconds.
pub fn memory() -> Option<ProcessMemory> {
    #[cfg(windows)]
    let raw = crate::sys::windows::process::memory();
    #[cfg(target_os = "linux")]
    let raw = crate::sys::linux::process::memory();
    #[cfg(not(any(windows, target_os = "linux")))]
    let raw: Option<(u64, u64)> = None;
    raw.map(|(private, resident)| ProcessMemory { private, resident })
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn reports_this_process() {
        let m = super::memory().expect("memory counters");
        assert!(m.private > 0 && m.resident > 0, "{m:?}");
    }
}
