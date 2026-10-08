//! SMB share enumeration for `\\server` (Windows: `NetShareEnum`, netapi32).

use soyuz_core::{FsError, FsResult};

/// One share of a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareInfo {
    pub name: String,
    pub remark: String,
    /// A disk (folder) share, as opposed to printers / IPC.
    pub disk: bool,
    /// Administrative share (`C$`, `ADMIN$`, `IPC$`).
    pub special: bool,
}

/// Whether `s` is a bare server name `\\server` (no share part).
pub fn is_bare_server(s: &str) -> bool {
    let t = s.trim_end_matches(['\\', '/']);
    (t.starts_with(r"\\") || t.starts_with("//"))
        && t.len() > 2
        && !t[2..].contains(['\\', '/'])
        && !t[2..].starts_with('?')
        && !t[2..].starts_with('.')
}

/// Shares of `server` (`\\server` or `server`).
pub fn list_shares(server: &str) -> FsResult<Vec<ShareInfo>> {
    let name = format!(r"\\{}", server.trim_start_matches(['\\', '/']).trim_end_matches(['\\', '/']));
    #[cfg(windows)]
    {
        crate::sys::windows::shares::list(&name).map_err(|e| FsError::from_io("list_shares", Some(name.as_ref()), &e))
    }
    #[cfg(not(windows))]
    {
        Err(FsError::unsupported("list_shares", Some(std::path::Path::new(&name))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_server_names() {
        assert!(is_bare_server(r"\\nas"));
        assert!(is_bare_server(r"\\nas\"));
        assert!(is_bare_server("//nas"));
        assert!(!is_bare_server(r"\\nas\share"));
        assert!(!is_bare_server(r"\\?\C:\x"));
        assert!(!is_bare_server(r"\\.\pipe"));
        assert!(!is_bare_server(r"C:\x"));
        assert!(!is_bare_server(r"\\"));
    }

    #[cfg(windows)]
    #[test]
    fn local_machine_shares() {
        // The local machine normally has IPC$ (unless the Server service is stopped).
        if let Ok(v) = list_shares("localhost") {
            assert!(v.iter().all(|s| !s.name.is_empty()));
        }
    }
}
