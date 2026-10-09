//! WSL distributions registered for the current user, for terminal shell pickers.
//!
//! Windows: the subkeys of `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`, each
//! with a `DistributionName` value. Read-only advapi32 (no WSL API, no process spawned);
//! other platforms return nothing.

/// Names of the registered WSL distributions (`Ubuntu`, `Debian`, …), in registry order.
pub fn distros() -> Vec<String> {
    #[cfg(windows)]
    {
        use crate::sys::windows::registry::{Hive, get_string, subkeys};
        const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";
        let mut out: Vec<String> = subkeys(Hive::CurrentUser, LXSS)
            .iter()
            .filter_map(|k| get_string(Hive::CurrentUser, &format!(r"{LXSS}\{k}"), Some("DistributionName")))
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        out.dedup();
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn distros_do_not_panic() {
        // Machine-dependent; only checks that the lookup works without WSL installed.
        for d in super::distros() {
            assert!(!d.trim().is_empty());
        }
    }
}
