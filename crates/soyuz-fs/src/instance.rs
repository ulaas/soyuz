//! Process facts and window handover for the single-instance launch handoff (`soyuz`
//! started while a window runs hands its request to that window): the session and
//! elevation that scope the instance endpoint, and letting the running window take the
//! foreground. Kernel / user32 calls on Windows; everything is a no-op elsewhere.

/// Remote Desktop / fast-user-switching session of this process (Windows; `None` elsewhere).
pub fn session_id() -> Option<u32> {
    #[cfg(windows)]
    {
        crate::sys::windows::instance::session_id()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// The process runs elevated: an administrator token on Windows, euid 0 on unix.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::instance::is_elevated()
    }
    #[cfg(unix)]
    {
        rustix::process::geteuid().is_root()
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

/// Let process `pid` take the foreground (Windows `AllowSetForegroundWindow`; the
/// launcher grants it to the window it hands over to). `false` elsewhere or when refused.
pub fn allow_set_foreground(pid: u32) -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::instance::allow_set_foreground(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// Restore window `hwnd` if minimized and bring it to the foreground without synthetic
/// input (Windows; `false` elsewhere).
pub fn foreground(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::instance::foreground(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_facts() {
        if cfg!(windows) {
            assert!(super::session_id().is_some(), "every process has a session");
        }
        // Elevation is whatever the test runner has; it must not fail.
        let _ = super::is_elevated();
        assert!(!super::foreground(0));
    }
}
