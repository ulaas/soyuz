//! Breadcrumb segmentation and path-completion helpers (pure; the widget lives in
//! `ui/breadcrumb.rs`).

use std::path::{Component, Path, PathBuf};

use soyuz_core::Location;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrumbKind {
    /// Filesystem root / drive.
    Root,
    /// The user's home directory (collapsed prefix).
    Home,
    Folder,
    /// The archive file itself.
    Archive,
    /// Trash, Computer, Recent, Tags, Search.
    Virtual,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Crumb {
    pub label: String,
    pub location: Location,
    pub kind: CrumbKind,
}

/// Split a location into clickable segments. With `home` set, a path inside the home
/// directory starts with a single "Home" crumb instead of the root / Users / name chain.
pub fn crumbs(loc: &Location, home: Option<&Path>) -> Vec<Crumb> {
    match loc {
        Location::Dir(p) => path_crumbs(p, home),
        Location::Archive { archive, inner } => {
            let mut v = path_crumbs(archive.parent().unwrap_or(archive), home);
            v.push(Crumb {
                label: soyuz_core::entry::display_name(archive),
                location: Location::Archive { archive: archive.clone(), inner: String::new() },
                kind: CrumbKind::Archive,
            });
            let mut acc = String::new();
            for seg in inner.split('/').filter(|s| !s.is_empty()) {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(seg);
                v.push(Crumb {
                    label: seg.to_string(),
                    location: Location::Archive { archive: archive.clone(), inner: acc.clone() },
                    kind: CrumbKind::Folder,
                });
            }
            v
        }
        other => vec![Crumb { label: location_title(other), location: other.clone(), kind: CrumbKind::Virtual }],
    }
}

fn path_crumbs(p: &Path, home: Option<&Path>) -> Vec<Crumb> {
    let p = soyuz_fs::strip_verbatim(p);
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    let mut rest: &Path = &p;
    if let Some(h) = home.map(soyuz_fs::strip_verbatim)
        && !h.as_os_str().is_empty()
        && let Ok(rel) = p.strip_prefix(&h)
    {
        out.push(Crumb { label: "Home".into(), location: Location::Dir(h.clone()), kind: CrumbKind::Home });
        acc = h.clone();
        rest = rel;
        for c in rest.components() {
            if let Component::Normal(s) = c {
                acc.push(s);
                out.push(Crumb {
                    label: s.to_string_lossy().into_owned(),
                    location: Location::Dir(acc.clone()),
                    kind: CrumbKind::Folder,
                });
            }
        }
        return out;
    }
    let mut prefix_label: Option<String> = None;
    for c in rest.components() {
        match c {
            Component::Prefix(pre) => {
                acc.push(pre.as_os_str());
                prefix_label = Some(pre.as_os_str().to_string_lossy().trim_start_matches(r"\\").to_string());
            }
            Component::RootDir => {
                acc.push(c.as_os_str());
                let label = prefix_label.take().unwrap_or_else(|| "/".to_string());
                out.push(Crumb { label, location: Location::Dir(acc.clone()), kind: CrumbKind::Root });
            }
            Component::Normal(s) => {
                if let Some(label) = prefix_label.take() {
                    out.push(Crumb { label, location: Location::Dir(acc.clone()), kind: CrumbKind::Root });
                }
                acc.push(s);
                out.push(Crumb {
                    label: s.to_string_lossy().into_owned(),
                    location: Location::Dir(acc.clone()),
                    kind: CrumbKind::Folder,
                });
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }
    if let Some(label) = prefix_label {
        out.push(Crumb { label, location: Location::Dir(acc), kind: CrumbKind::Root });
    }
    out
}

/// Title for a location (tab titles, window title, single-crumb virtual places).
pub fn location_title(loc: &Location) -> String {
    match loc {
        Location::Dir(p) => {
            let p = soyuz_fs::strip_verbatim(p);
            match p.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => {
                    let s = p.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
                    if s.is_empty() { "/".into() } else { s }
                }
            }
        }
        Location::Archive { archive, inner } => match inner.rsplit('/').next() {
            Some(last) if !last.is_empty() => last.to_string(),
            _ => soyuz_core::entry::display_name(archive),
        },
        Location::Trash => "Trash".into(),
        Location::Computer => "This PC".into(),
        Location::Recent => "Recent".into(),
        Location::Timeline => "Timeline".into(),
        Location::Tags(t) => format!("Tag: {}", crate::services::tags::display_name(t)),
        // Smart folders show their name (F5).
        Location::Search(id) => {
            crate::services::search::spec(*id).and_then(|s| s.smart).unwrap_or_else(|| "Search".into())
        }
    }
}

/// Split typed path text into the directory to list and the partial name being typed:
/// `"C:\Users\ul"` → `("C:\Users\", "ul")`, `"/usr/"` → `("/usr/", "")`.
pub fn split_for_completion(text: &str) -> Option<(PathBuf, String)> {
    let idx = text.rfind(['/', '\\'])?;
    let (dir, rest) = text.split_at(idx + 1);
    let dir = if dir.is_empty() { "/" } else { dir };
    let expanded = match Location::parse(dir) {
        Some(Location::Dir(p)) => p,
        _ => PathBuf::from(dir),
    };
    Some((expanded, rest.to_string()))
}

/// Names from `candidates` that start with `prefix` (case-insensitive), sorted naturally
/// and capped at `limit`.
pub fn completions<'a>(candidates: impl Iterator<Item = &'a str>, prefix: &str, limit: usize) -> Vec<String> {
    let lp = prefix.to_lowercase();
    let mut v: Vec<String> =
        candidates.filter(|n| n.to_lowercase().starts_with(&lp) && *n != prefix).map(str::to_string).collect();
    v.sort_by(|a, b| soyuz_core::natural_cmp(a, b, true));
    v.truncate(limit);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[Crumb]) -> Vec<&str> {
        v.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    #[cfg(unix)]
    fn unix_paths() {
        let c = crumbs(&Location::Dir("/usr/local/bin".into()), None);
        assert_eq!(labels(&c), ["/", "usr", "local", "bin"]);
        assert_eq!(c[0].kind, CrumbKind::Root);
        assert_eq!(c[2].location, Location::Dir("/usr/local".into()));
        let c = crumbs(&Location::Dir("/".into()), None);
        assert_eq!(labels(&c), ["/"]);
    }

    #[test]
    #[cfg(windows)]
    fn windows_paths() {
        let c = crumbs(&Location::Dir(r"C:\Windows\System32".into()), None);
        assert_eq!(labels(&c), ["C:", "Windows", "System32"]);
        assert_eq!(c[0].location, Location::Dir(r"C:\".into()));
        assert_eq!(c[2].location, Location::Dir(r"C:\Windows\System32".into()));
        let c = crumbs(&Location::Dir(r"\\?\D:\data".into()), None);
        assert_eq!(labels(&c), ["D:", "data"]);
        let c = crumbs(&Location::Dir(r"\\server\share\x".into()), None);
        assert_eq!(labels(&c), [r"server\share", "x"]);
        let c = crumbs(&Location::Dir(r"C:\".into()), None);
        assert_eq!(labels(&c), ["C:"]);
    }

    #[test]
    fn home_is_collapsed() {
        let home = std::env::temp_dir().join("soyuz-home");
        let p = home.join("src").join("proj");
        let c = crumbs(&Location::Dir(p.clone()), Some(&home));
        assert_eq!(labels(&c), ["Home", "src", "proj"]);
        assert_eq!(c[0].kind, CrumbKind::Home);
        assert_eq!(c[2].location, Location::Dir(p));
        let c = crumbs(&Location::Dir(home.clone()), Some(&home));
        assert_eq!(labels(&c), ["Home"]);
    }

    #[test]
    fn archive_and_virtual() {
        let a = std::env::temp_dir().join("a.zip");
        let c = crumbs(&Location::Archive { archive: a.clone(), inner: "x/y".into() }, None);
        let l = labels(&c);
        assert_eq!(&l[l.len() - 3..], ["a.zip", "x", "y"]);
        assert_eq!(c.last().unwrap().location, Location::Archive { archive: a, inner: "x/y".into() });
        let t = crumbs(&Location::Trash, None);
        assert_eq!(labels(&t), ["Trash"]);
        assert_eq!(t[0].kind, CrumbKind::Virtual);
    }

    #[test]
    fn titles() {
        assert_eq!(location_title(&Location::Dir(std::env::temp_dir().join("abc"))), "abc");
        assert_eq!(location_title(&Location::Computer), "This PC");
        #[cfg(windows)]
        assert_eq!(location_title(&Location::Dir(r"C:\".into())), "C:");
    }

    #[test]
    fn completion_helpers() {
        let (dir, rest) = split_for_completion("/usr/lo").unwrap();
        assert_eq!(dir, PathBuf::from("/usr/"));
        assert_eq!(rest, "lo");
        let (_, rest) = split_for_completion(r"C:\Users\").unwrap();
        assert_eq!(rest, "");
        assert!(split_for_completion("nothing").is_none());
        let names = ["local", "lib", "Lib64", "share", "lo"];
        assert_eq!(completions(names.iter().copied(), "l", 10), ["lib", "Lib64", "lo", "local"]);
        assert_eq!(completions(names.iter().copied(), "lo", 10), ["local"], "exact match is not offered");
        assert_eq!(completions(names.iter().copied(), "l", 2).len(), 2);
    }
}
