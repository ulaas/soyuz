//! Trusted SSH host keys (`known_hosts` in the Soyuz config dir).
//!
//! One line per host: `host:port algorithm SHA256:fingerprint` (`#` comments and blank
//! lines are kept out). The file is rewritten atomically (temp + rename).

use std::path::{Path, PathBuf};

use parking_lot::Mutex;

/// Result of checking a server key against the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    /// Never seen this host (first connection).
    Unknown,
    /// The host presented a different key than the one trusted before.
    Changed {
        previous: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    host: String,
    algo: String,
    fingerprint: String,
}

/// File-backed store (`path = None`: memory only, for tests).
pub struct KnownHosts {
    path: Option<PathBuf>,
    lines: Mutex<Vec<Line>>,
}

fn host_key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.to_ascii_lowercase())
}

impl KnownHosts {
    pub fn in_memory() -> Self {
        KnownHosts { path: None, lines: Mutex::new(Vec::new()) }
    }

    /// Load `path` (a missing file is an empty store).
    pub fn open(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        KnownHosts { path: Some(path.to_path_buf()), lines: Mutex::new(parse(&text)) }
    }

    pub fn check(&self, host: &str, port: u16, fingerprint: &str) -> HostKeyStatus {
        let key = host_key(host, port);
        let lines = self.lines.lock();
        let mine: Vec<&Line> = lines.iter().filter(|l| l.host == key).collect();
        if mine.is_empty() {
            HostKeyStatus::Unknown
        } else if mine.iter().any(|l| l.fingerprint == fingerprint) {
            HostKeyStatus::Trusted
        } else {
            HostKeyStatus::Changed { previous: mine[0].fingerprint.clone() }
        }
    }

    /// Trust `fingerprint` for the host (replacing earlier keys) and save.
    pub fn trust(&self, host: &str, port: u16, algo: &str, fingerprint: &str) -> std::io::Result<()> {
        let key = host_key(host, port);
        let text = {
            let mut lines = self.lines.lock();
            lines.retain(|l| l.host != key);
            lines.push(Line { host: key, algo: algo.to_string(), fingerprint: fingerprint.to_string() });
            render(&lines)
        };
        match &self.path {
            Some(p) => write_atomic(p, &text),
            None => Ok(()),
        }
    }

    /// Forget a host (all its keys) and save.
    pub fn forget(&self, host: &str, port: u16) -> std::io::Result<()> {
        let key = host_key(host, port);
        let text = {
            let mut lines = self.lines.lock();
            lines.retain(|l| l.host != key);
            render(&lines)
        };
        match &self.path {
            Some(p) => write_atomic(p, &text),
            None => Ok(()),
        }
    }

    /// `(host:port, algorithm, fingerprint)` rows, for the settings page.
    pub fn entries(&self) -> Vec<(String, String, String)> {
        self.lines.lock().iter().map(|l| (l.host.clone(), l.algo.clone(), l.fingerprint.clone())).collect()
    }
}

fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let host = it.next()?.to_ascii_lowercase();
            let algo = it.next()?.to_string();
            let fingerprint = it.next()?.to_string();
            Some(Line { host, algo, fingerprint })
        })
        .collect()
}

fn render(lines: &[Line]) -> String {
    let mut s = String::from("# Soyuz trusted SSH host keys: host:port algorithm fingerprint\n");
    for l in lines {
        s.push_str(&format!("{} {} {}\n", l.host, l.algo, l.fingerprint));
    }
    s
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_check_change_forget_persist() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("known_hosts");
        let kh = KnownHosts::open(&p);
        assert_eq!(kh.check("Host", 22, "SHA256:aaa"), HostKeyStatus::Unknown);
        kh.trust("Host", 22, "ssh-ed25519", "SHA256:aaa").unwrap();
        assert_eq!(kh.check("host", 22, "SHA256:aaa"), HostKeyStatus::Trusted);
        assert_eq!(kh.check("host", 2222, "SHA256:aaa"), HostKeyStatus::Unknown, "ports are separate hosts");
        assert_eq!(kh.check("host", 22, "SHA256:bbb"), HostKeyStatus::Changed { previous: "SHA256:aaa".into() });

        // Reload from disk.
        let again = KnownHosts::open(&p);
        assert_eq!(again.check("host", 22, "SHA256:aaa"), HostKeyStatus::Trusted);
        assert_eq!(again.entries().len(), 1);

        // Re-trusting replaces the old key.
        again.trust("host", 22, "ssh-ed25519", "SHA256:bbb").unwrap();
        assert_eq!(again.check("host", 22, "SHA256:aaa"), HostKeyStatus::Changed { previous: "SHA256:bbb".into() });
        again.forget("host", 22).unwrap();
        assert_eq!(KnownHosts::open(&p).check("host", 22, "SHA256:bbb"), HostKeyStatus::Unknown);
    }

    #[test]
    fn parse_ignores_comments_and_garbage() {
        let l = parse("# c\n\nh:22 ssh-rsa SHA256:x\nbroken\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].host, "h:22");
    }
}
