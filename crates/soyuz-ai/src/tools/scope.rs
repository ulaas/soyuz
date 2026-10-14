//! Scope enforcement for AI tools: every path must lie inside an allowed root (checked
//! lexically *and* after resolving links of the nearest existing ancestor), and sensitive
//! locations (`.ssh`, `.env`, keychains, credential stores, browser profiles, wallets…)
//! are blocked unless the user explicitly allows them.

use std::path::{Component, Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use soyuz_core::config::AiScope;

/// Built-in sensitive patterns (matched case-insensitively against the full path with `/`
/// separators). A pattern ending in `/**` also blocks the folder itself.
pub const SENSITIVE: &[&str] = &[
    // keys and credentials
    "**/.ssh/**",
    "**/.gnupg/**",
    "**/.aws/**",
    "**/.azure/**",
    "**/.kube/**",
    "**/.docker/config.json",
    "**/.password-store/**",
    "**/.netrc",
    "**/_netrc",
    "**/.git-credentials",
    "**/.npmrc",
    "**/.pypirc",
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/*.p12",
    "**/*.pfx",
    "**/id_rsa*",
    "**/id_dsa*",
    "**/id_ecdsa*",
    "**/id_ed25519*",
    "**/*.kdbx",
    // OS keychains / credential stores
    "**/library/keychains/**",
    "**/.local/share/keyrings/**",
    "**/appdata/*/microsoft/credentials/**",
    "**/appdata/*/microsoft/protect/**",
    "**/appdata/*/microsoft/vault/**",
    // browser profiles
    "**/google/chrome/user data/**",
    "**/microsoft/edge/user data/**",
    "**/bravesoftware/**",
    "**/vivaldi/user data/**",
    "**/opera software/**",
    "**/mozilla/firefox/profiles/**",
    "**/.mozilla/**",
    "**/.config/google-chrome/**",
    "**/.config/chromium/**",
    "**/.config/bravesoftware/**",
    "**/library/application support/google/chrome/**",
    "**/library/application support/firefox/**",
    "**/library/safari/**",
    "**/library/cookies/**",
    // wallets
    "**/wallet.dat",
    "**/.bitcoin/**",
    "**/.electrum/**",
    "**/electrum/wallets/**",
    "**/.ethereum/keystore/**",
    "**/ethereum/keystore/**",
    "**/exodus/**",
    "**/.monero/**",
];

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    #[error("path must be absolute: {0}")]
    NotAbsolute(PathBuf),
    #[error("path escapes its root: {0}")]
    Invalid(PathBuf),
    #[error("outside the folders the assistant may use: {0}")]
    OutsideRoots(PathBuf),
    #[error("blocked sensitive location: {0}")]
    Sensitive(PathBuf),
    #[error("cannot change an allowed root folder itself: {0}")]
    RootItself(PathBuf),
}

/// Compiled scope.
#[derive(Clone, Debug)]
pub struct Scope {
    /// The roots resolved (symlinks, junctions and 8.3 short names expanded).
    roots: Vec<PathBuf>,
    /// The roots as configured, `.` / `..` resolved: a path spelled the same way (a short
    /// name such as `C:\Users\RUNNER~1`, a junction) is inside them before resolving.
    given: Vec<PathBuf>,
    blocked: GlobSet,
    patterns: Vec<String>,
}

impl Scope {
    /// Build from roots and extra blocked globs (`allow_sensitive` drops the built-ins).
    pub fn new(roots: Vec<PathBuf>, extra_blocked: &[String], allow_sensitive: bool) -> Scope {
        let mut patterns: Vec<String> = Vec::new();
        if !allow_sensitive {
            patterns.extend(SENSITIVE.iter().map(|s| s.to_string()));
        }
        patterns.extend(extra_blocked.iter().map(|s| s.replace('\\', "/").to_lowercase()));
        let mut b = GlobSetBuilder::new();
        for p in &patterns {
            let mut add = |pat: &str| match GlobBuilder::new(pat).literal_separator(true).build() {
                Ok(g) => {
                    b.add(g);
                }
                Err(e) => log::warn!("ai.scope: bad blocked glob “{pat}”: {e}"),
            };
            add(p);
            if let Some(dir) = p.strip_suffix("/**") {
                add(dir);
            }
        }
        let blocked = b.build().unwrap_or_else(|_| GlobSet::empty());
        let given = roots.iter().filter_map(|r| lexical(r)).collect();
        let roots = roots.into_iter().filter_map(|r| resolve(&r).ok()).collect();
        Scope { roots, given, blocked, patterns }
    }

    /// From the `[ai.scope]` config (empty roots = the home folder).
    pub fn from_config(cfg: &AiScope) -> Scope {
        let roots = if cfg.allowed_roots.is_empty() {
            vec![soyuz_fs::home()]
        } else {
            cfg.allowed_roots.iter().map(|r| soyuz_fs::expand_env(&r.to_string_lossy()).into()).collect()
        };
        Scope::new(roots, &cfg.blocked, cfg.allow_sensitive)
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn blocked_patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Whether `path` (absolute, normalized) matches a blocked pattern.
    pub fn is_blocked(&self, path: &Path) -> bool {
        self.blocked.is_match(match_key(path))
    }

    /// Check a path for reading; returns the normalized path.
    pub fn check_read(&self, path: &Path) -> Result<PathBuf, ScopeError> {
        self.check(path, false)
    }

    /// Check a path that will be created or changed (also refuses the roots themselves).
    pub fn check_write(&self, path: &Path) -> Result<PathBuf, ScopeError> {
        self.check(path, true)
    }

    fn check(&self, path: &Path, write: bool) -> Result<PathBuf, ScopeError> {
        if !path.is_absolute() {
            return Err(ScopeError::NotAbsolute(path.to_path_buf()));
        }
        let lexical = lexical(path).ok_or_else(|| ScopeError::Invalid(path.to_path_buf()))?;
        let real = resolve(&lexical).map_err(|_| ScopeError::Invalid(path.to_path_buf()))?;
        // The path as given may match the roots as given or resolved; the resolved path
        // must be inside a resolved root (no escape through a link).
        let all: Vec<&PathBuf> = self.roots.iter().chain(&self.given).collect();
        for (p, roots) in [(&lexical, all), (&real, self.roots.iter().collect())] {
            if self.is_blocked(p) {
                return Err(ScopeError::Sensitive(path.to_path_buf()));
            }
            if !roots.iter().any(|r| within(p, r)) {
                return Err(ScopeError::OutsideRoots(path.to_path_buf()));
            }
            if write && roots.iter().any(|r| same(p, r)) {
                return Err(ScopeError::RootItself(path.to_path_buf()));
            }
        }
        Ok(lexical)
    }
}

/// Lowercased, `/`-separated form used for glob matching.
fn match_key(path: &Path) -> String {
    soyuz_fs::strip_verbatim(path).to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Resolve `.` / `..` lexically; `None` if `..` climbs above the root.
pub fn lexical(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                out.pop();
                depth -= 1;
            }
            Component::Normal(n) => {
                out.push(n);
                depth += 1;
            }
        }
    }
    Some(out)
}

/// Canonicalize the nearest existing ancestor (resolving symlinks / junctions) and append
/// the not-yet-existing tail.
fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    let mut tail = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        match std::fs::canonicalize(&cur) {
            Ok(c) => {
                let mut out = soyuz_fs::strip_verbatim(&c);
                for t in tail.iter().rev() {
                    out.push(t);
                }
                return Ok(out);
            }
            Err(_) => {
                let Some(name) = cur.file_name().map(|n| n.to_os_string()) else {
                    return Ok(path.to_path_buf());
                };
                tail.push(name);
                if !cur.pop() {
                    return Ok(path.to_path_buf());
                }
            }
        }
    }
}

fn comps(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| {
            let s = c.as_os_str().to_string_lossy().to_string();
            if cfg!(windows) { s.to_lowercase() } else { s }
        })
        .collect()
}

fn within(p: &Path, root: &Path) -> bool {
    let (a, b) = (comps(p), comps(root));
    a.len() >= b.len() && a[..b.len()] == b[..]
}

fn same(a: &Path, b: &Path) -> bool {
    comps(a) == comps(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_and_sensitive_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(root.join(".ssh")).unwrap();
        let s = Scope::new(vec![root.clone()], &["**/private/**".into()], false);

        assert!(s.check_read(&root.join("sub")).is_ok());
        assert!(s.check_write(&root.join("sub/new.txt")).is_ok(), "non-existent tail is fine");
        assert_eq!(s.check_read(Path::new("relative")), Err(ScopeError::NotAbsolute("relative".into())));
        assert!(matches!(s.check_read(dir.path()), Err(ScopeError::OutsideRoots(_))));
        assert!(matches!(s.check_read(&root.join("sub/../../escape")), Err(ScopeError::OutsideRoots(_))));
        assert!(matches!(s.check_read(&root.join(".ssh")), Err(ScopeError::Sensitive(_))));
        assert!(matches!(s.check_read(&root.join(".ssh/id_ed25519")), Err(ScopeError::Sensitive(_))));
        assert!(matches!(s.check_read(&root.join("app/.env")), Err(ScopeError::Sensitive(_))));
        assert!(matches!(s.check_read(&root.join("app/.env.local")), Err(ScopeError::Sensitive(_))));
        assert!(matches!(s.check_read(&root.join("x/PRIVATE/y")), Err(ScopeError::Sensitive(_))));
        assert!(matches!(
            s.check_read(&root.join("AppData/Local/Google/Chrome/User Data/Default")),
            Err(ScopeError::Sensitive(_))
        ));
        assert!(matches!(s.check_read(&root.join("btc/wallet.dat")), Err(ScopeError::Sensitive(_))));
        assert!(matches!(s.check_write(&root), Err(ScopeError::RootItself(_))));
        assert!(s.check_read(&root).is_ok());
        assert!(s.check_read(&root.join("notes/environment.md")).is_ok());

        let open = Scope::new(vec![root.clone()], &[], true);
        assert!(open.check_read(&root.join(".ssh")).is_ok(), "explicit override");
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_insensitive_roots() {
        let dir = tempfile::tempdir().unwrap();
        let s = Scope::new(vec![dir.path().to_path_buf()], &[], false);
        let upper = PathBuf::from(dir.path().to_string_lossy().to_uppercase()).join("x");
        assert!(s.check_read(&upper).is_ok());
    }

    /// A root reached through a junction (like an 8.3 short name such as
    /// `C:\Users\RUNNER~1` on CI) resolves to another spelling; paths written the
    /// way the root was given are still inside it, and the link cannot be used to escape.
    #[cfg(windows)]
    #[test]
    fn roots_given_through_a_junction() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("sub")).unwrap();
        std::fs::create_dir_all(dir.path().join("other")).unwrap();
        let link = dir.path().join("link");
        soyuz_fs::junction(&real, &link).unwrap();
        let s = Scope::new(vec![link.clone()], &[], false);
        assert!(s.check_read(&link.join("sub")).is_ok());
        assert!(s.check_write(&link.join("sub/new.txt")).is_ok());
        // Resolved fully: the temp folder itself may be spelled with a short name.
        let resolved = soyuz_fs::strip_verbatim(&std::fs::canonicalize(&real).unwrap());
        assert!(s.check_read(&resolved.join("sub")).is_ok(), "the resolved spelling too");
        assert!(matches!(s.check_write(&link), Err(ScopeError::RootItself(_))));
        assert!(matches!(s.check_read(&dir.path().join("other")), Err(ScopeError::OutsideRoots(_))));
        // A junction inside the root that points out of it is still refused.
        soyuz_fs::junction(&dir.path().join("other"), &real.join("out")).unwrap();
        assert!(matches!(s.check_read(&link.join("out")), Err(ScopeError::OutsideRoots(_))));
    }
}
