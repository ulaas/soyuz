//! Exclusion patterns for the crawler.
//!
//! A pattern without a separator matches a file or folder **name** (`node_modules`,
//! `*.tmp`); a pattern with `/` or `\` matches the end of a path (`AppData/Local/Temp`,
//! `AppData/Local/**/Cache`). On unix a pattern starting with `/` or `~/` is anchored
//! instead (`/proc`, `~/snap`: only that folder, not every path ending in it). Matching is
//! case-insensitive; excluded folders are not descended into.

use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

#[derive(Clone, Debug)]
pub struct Excluder {
    names: GlobSet,
    paths: GlobSet,
}

impl Default for Excluder {
    fn default() -> Self {
        Excluder { names: GlobSet::empty(), paths: GlobSet::empty() }
    }
}

/// The absolute glob of an anchored pattern (`/x`, `~`, `~/x`; unix only).
fn anchored(p: &str) -> Option<String> {
    if !cfg!(unix) {
        return None;
    }
    if p.starts_with('/') {
        return Some(p.to_string());
    }
    let rest = if p == "~" { "" } else { p.strip_prefix("~/")? };
    let home = soyuz_fs::home().to_string_lossy().into_owned();
    let home = globset::escape(home.trim_end_matches('/'));
    Some(if rest.is_empty() { home } else { format!("{home}/{rest}") })
}

impl Excluder {
    /// Compile patterns; invalid ones are reported and skipped.
    pub fn new(patterns: &[String]) -> (Excluder, Vec<String>) {
        let mut names = GlobSetBuilder::new();
        let mut paths = GlobSetBuilder::new();
        let mut errors = Vec::new();
        for raw in patterns {
            let p = raw.trim().replace('\\', "/");
            let p = p.trim_end_matches('/');
            if p.is_empty() {
                continue;
            }
            let (pattern, set) = if let Some(abs) = anchored(p) {
                (abs, &mut paths)
            } else if p.contains('/') {
                (format!("**/{}", p.trim_start_matches('/')), &mut paths)
            } else {
                (p.to_string(), &mut names)
            };
            match GlobBuilder::new(&pattern).case_insensitive(true).literal_separator(true).build() {
                Ok(g) => {
                    set.add(g);
                }
                Err(e) => errors.push(format!("exclusion “{raw}”: {e}")),
            }
        }
        let names = names.build().unwrap_or_else(|_| GlobSet::empty());
        let paths = paths.build().unwrap_or_else(|_| GlobSet::empty());
        (Excluder { names, paths }, errors)
    }

    /// Whether `path` (whose last component is `name`) is excluded.
    pub fn is_excluded(&self, path: &Path, name: &str) -> bool {
        if self.names.is_match(name) {
            return true;
        }
        if self.paths.is_empty() {
            return false;
        }
        let s = path.to_string_lossy().replace('\\', "/");
        self.paths.is_match(s.as_str())
    }

    /// Whether `path` or any of its ancestors below `root` is excluded.
    pub fn excluded_below(&self, root: &Path, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(root) else { return false };
        let mut cur = root.to_path_buf();
        for c in rel.components() {
            cur.push(c);
            let name = c.as_os_str().to_string_lossy();
            if self.is_excluded(&cur, &name) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(p: &[&str]) -> Excluder {
        let (e, errs) = Excluder::new(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(errs.is_empty(), "{errs:?}");
        e
    }

    #[test]
    fn names_and_paths() {
        let e = ex(&["node_modules", ".git", "*.tmp", "AppData/Local/Temp", r"AppData\Local\**\Cache"]);
        assert!(e.is_excluded(Path::new("/h/p/node_modules"), "node_modules"));
        assert!(e.is_excluded(Path::new("/h/p/NODE_MODULES"), "NODE_MODULES"));
        assert!(e.is_excluded(Path::new("/h/.git"), ".git"));
        assert!(!e.is_excluded(Path::new("/h/.github"), ".github"));
        assert!(e.is_excluded(Path::new("/h/x.TMP"), "x.TMP"));
        assert!(e.is_excluded(Path::new("/Users/me/AppData/Local/Temp"), "Temp"));
        assert!(e.is_excluded(Path::new(r"C:\Users\me\AppData\Local\Temp"), "Temp"));
        assert!(!e.is_excluded(Path::new("/Users/me/Temp"), "Temp"));
        assert!(e.is_excluded(Path::new("/u/AppData/Local/Google/Chrome/Cache"), "Cache"));
        assert!(!e.is_excluded(Path::new("/u/Projects/Cache"), "Cache"));
    }

    #[cfg(unix)]
    #[test]
    fn anchored_patterns() {
        let e = ex(&["/proc", "~/snap", ".var/app/*/cache", ".local/share/Trash"]);
        assert!(e.is_excluded(Path::new("/proc"), "proc"));
        assert!(!e.is_excluded(Path::new("/home/me/src/proc"), "proc"));
        assert!(e.excluded_below(Path::new("/"), Path::new("/proc/1/status")));
        let home = soyuz_fs::home();
        assert!(e.is_excluded(&home.join("snap"), "snap"));
        assert!(!e.is_excluded(&home.join("work/snap"), "snap"));
        assert!(e.is_excluded(&home.join(".var/app/org.gnome.Maps/cache"), "cache"));
        assert!(!e.is_excluded(&home.join(".var/app/org.gnome.Maps/data"), "data"));
        assert!(e.is_excluded(&home.join(".local/share/Trash"), "Trash"));
    }

    #[test]
    fn ancestors() {
        let e = ex(&["target"]);
        assert!(e.excluded_below(Path::new("/r"), Path::new("/r/p/target/debug/x.o")));
        assert!(!e.excluded_below(Path::new("/r"), Path::new("/r/p/src/x.rs")));
        assert!(
            !e.excluded_below(Path::new("/r/target"), Path::new("/r/target/a")),
            "the root itself is never excluded"
        );
    }

    #[test]
    fn bad_patterns_are_reported() {
        let (e, errs) = Excluder::new(&["[".to_string(), "ok".to_string(), "  ".to_string()]);
        assert_eq!(errs.len(), 1);
        assert!(e.is_excluded(Path::new("/ok"), "ok"));
    }
}
