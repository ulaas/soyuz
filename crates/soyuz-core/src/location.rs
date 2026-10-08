//! What a pane can show, and the virtual-path helpers for archive browsing.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::entry::SearchId;

/// A place a pane can display.
///
/// String form (see [`Location::display`] / [`Location::parse`]):
/// `Dir` → the path itself, `Archive` → `"<archive>!/<inner>"`, `Trash` → `"trash:"`,
/// `Computer` → `"computer:"`, `Recent` → `"recent:"`, `Tags(t)` → `"tag:<t>"`,
/// `Search(id)` → `"search:<id>"`, `Remote(uri)` → the URI (`sftp://user@host/path`).
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Dir(PathBuf),
    /// `inner` is `/`-separated with no leading/trailing `/` (empty = archive root).
    Archive {
        archive: PathBuf,
        inner: String,
    },
    Trash,
    /// Not persisted: sessions store the `SavedSearch` instead.
    #[serde(skip)]
    Search(SearchId),
    /// Drives / volumes page.
    Computer,
    Recent,
    Tags(String),
    /// F2: a folder on a remote server, as a normalized URI string
    /// (`sftp://user@host:port/path`, `webdav(s)://`, `ftp(s)://`, `s3://bucket/…`;
    /// parsed and normalized by `soyuz-vfs`).
    Remote(String),
}

impl Location {
    /// Parse the string form produced by [`Location::display`]. A leading `~` expands to
    /// the home directory. Returns `None` for empty input.
    pub fn parse(s: &str) -> Option<Location> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let lower = s.to_ascii_lowercase();
        match lower.trim_end_matches('/') {
            "trash:" => return Some(Location::Trash),
            "computer:" => return Some(Location::Computer),
            "recent:" => return Some(Location::Recent),
            _ => {}
        }
        if let Some(tag) = strip_prefix_ci(s, "tag:") {
            return Some(Location::Tags(tag.to_string()));
        }
        if let Some(id) = strip_prefix_ci(s, "search:") {
            return id.trim().parse().ok().map(|n| Location::Search(SearchId(n)));
        }
        if looks_remote(s) {
            return Some(Location::Remote(s.to_string()));
        }
        let expanded = expand_tilde(s);
        if let Some((archive, inner)) = split_vpath(&expanded) {
            return Some(Location::Archive { archive, inner });
        }
        Some(Location::Dir(expanded))
    }

    /// Human-readable string form (round-trips through [`Location::parse`]).
    pub fn display(&self) -> String {
        match self {
            Location::Dir(p) => p.to_string_lossy().into_owned(),
            Location::Archive { archive, inner } => join_vpath(archive, inner).to_string_lossy().into_owned(),
            Location::Trash => "trash:".into(),
            Location::Search(id) => format!("search:{}", id.0),
            Location::Computer => "computer:".into(),
            Location::Recent => "recent:".into(),
            Location::Tags(t) => format!("tag:{t}"),
            Location::Remote(u) => u.clone(),
        }
    }

    /// The containing location: parent directory, parent folder inside an archive, the
    /// directory containing the archive for an archive root, and [`Location::Computer`]
    /// for a filesystem root. Virtual locations have no parent.
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Dir(p) => match p.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => Some(Location::Dir(parent.to_path_buf())),
                _ if p.has_root() => Some(Location::Computer),
                _ => None,
            },
            Location::Archive { archive, inner } => {
                if inner.is_empty() {
                    archive.parent().map(|p| Location::Dir(p.to_path_buf()))
                } else {
                    let parent = match inner.rfind('/') {
                        Some(i) => inner[..i].to_string(),
                        None => String::new(),
                    };
                    Some(Location::Archive { archive: archive.clone(), inner: parent })
                }
            }
            Location::Remote(u) => remote_parent(u).map(Location::Remote),
            Location::Trash | Location::Search(_) | Location::Computer | Location::Recent | Location::Tags(_) => None,
        }
    }

    /// The real directory behind this location, if it is a plain directory.
    pub fn as_fs_path(&self) -> Option<&Path> {
        match self {
            Location::Dir(p) => Some(p),
            _ => None,
        }
    }
}

/// Whether `s` looks like `scheme://…` (a remote location; drive letters don't count).
pub fn looks_remote(s: &str) -> bool {
    match s.find("://") {
        Some(i) if i >= 2 => s[..i].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')),
        _ => false,
    }
}

/// Parent of a normalized remote URI (`None` at the server root). The query string
/// (connection parameters) is kept.
pub fn remote_parent(uri: &str) -> Option<String> {
    let (base, query) = match uri.find('?') {
        Some(q) => (&uri[..q], &uri[q..]),
        None => (uri, ""),
    };
    let start = base.find("://")? + 3;
    let path_at = base[start..].find('/').map(|i| start + i)?;
    let path = base[path_at..].trim_end_matches('/');
    if path.is_empty() {
        return None;
    }
    let cut = path.rfind('/').unwrap_or(0);
    let parent = if cut == 0 { "/" } else { &path[..cut] };
    Some(format!("{}{parent}{query}", &base[..path_at]))
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    // `get` (not slicing): the prefix length may fall inside a multibyte character.
    s.get(..prefix.len()).filter(|h| h.eq_ignore_ascii_case(prefix)).map(|_| &s[prefix.len()..])
}

fn expand_tilde(s: &str) -> PathBuf {
    if (s == "~" || s.starts_with("~/") || s.starts_with("~\\"))
        && let Some(dirs) = directories::BaseDirs::new()
    {
        let home = dirs.home_dir().to_path_buf();
        let rest = s[1..].trim_start_matches(['/', '\\']);
        return if rest.is_empty() { home } else { home.join(rest) };
    }
    PathBuf::from(s)
}

/// Normalize an archive-inner path: `\` → `/`, no empty / `.` segments, no leading or
/// trailing `/`. `..` segments pop the previous segment (never escaping the root).
pub fn normalize_inner(inner: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in inner.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Build the virtual path `"<archive>!/<inner>"` used by archive browsing.
pub fn join_vpath(archive: &Path, inner: &str) -> PathBuf {
    let mut s: OsString = archive.as_os_str().to_owned();
    s.push("!/");
    s.push(normalize_inner(inner));
    PathBuf::from(s)
}

/// Split a virtual path produced by [`join_vpath`] into `(archive, inner)`.
///
/// The separator is the first `!/` or `!\` whose preceding path component has an
/// extension (so folders with `!` in their name are not mistaken for archives); a
/// trailing `!` (`"a.zip!"`) also denotes the archive root.
pub fn split_vpath(p: &Path) -> Option<(PathBuf, String)> {
    let s = p.to_string_lossy();
    let bytes = s.as_bytes();
    let mut candidates = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'!' {
            let next = bytes.get(i + 1).copied();
            if matches!(next, None | Some(b'/') | Some(b'\\')) {
                candidates.push(i);
            }
        }
    }
    let looks_like_archive = |i: usize| {
        let prefix = &s[..i];
        let last = prefix.rsplit(['/', '\\']).next().unwrap_or("");
        crate::entry::name_extension(last).is_some()
    };
    let idx = candidates.iter().copied().find(|&i| i > 0 && looks_like_archive(i))?;
    let archive = PathBuf::from(&s[..idx]);
    if archive.components().all(|c| matches!(c, Component::RootDir | Component::Prefix(_))) {
        return None;
    }
    let inner = if idx + 1 >= s.len() { "" } else { &s[idx + 2..] };
    Some((archive, normalize_inner(inner)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpath_round_trip() {
        let a = Path::new("C:/data/a.zip");
        let v = join_vpath(a, "x/y");
        assert_eq!(v, PathBuf::from("C:/data/a.zip!/x/y"));
        assert_eq!(split_vpath(&v), Some((a.to_path_buf(), "x/y".to_string())));

        let root = join_vpath(a, "");
        assert_eq!(split_vpath(&root), Some((a.to_path_buf(), String::new())));
        assert_eq!(split_vpath(Path::new("/tmp/b.tar.gz!")), Some((PathBuf::from("/tmp/b.tar.gz"), String::new())));
        // Messy inner paths are normalized.
        assert_eq!(join_vpath(a, "/x//y/"), PathBuf::from("C:/data/a.zip!/x/y"));
        assert_eq!(
            split_vpath(Path::new(r"C:\data\a.zip!\x\y")),
            Some((PathBuf::from(r"C:\data\a.zip"), "x/y".to_string()))
        );
    }

    #[test]
    fn vpath_bang_in_folder_name() {
        assert_eq!(split_vpath(Path::new("/tmp/wow!/plain")), None);
        assert_eq!(
            split_vpath(Path::new("/tmp/wow!/x.zip!/a")),
            Some((PathBuf::from("/tmp/wow!/x.zip"), "a".to_string()))
        );
        assert_eq!(split_vpath(Path::new("/tmp/normal/dir")), None);
    }

    #[test]
    fn normalize_inner_rules() {
        assert_eq!(normalize_inner("a/./b/../c"), "a/c");
        assert_eq!(normalize_inner("../../a"), "a");
        assert_eq!(normalize_inner(r"\a\b\"), "a/b");
    }

    #[test]
    fn location_parse_display() {
        for loc in [
            Location::Trash,
            Location::Computer,
            Location::Recent,
            Location::Tags("red".into()),
            Location::Search(SearchId(42)),
            Location::Dir(PathBuf::from("/usr/share")),
            Location::Archive { archive: PathBuf::from("/tmp/a.zip"), inner: "x/y".into() },
            Location::Archive { archive: PathBuf::from("/tmp/a.zip"), inner: String::new() },
            Location::Remote("sftp://u@h/home/u".into()),
        ] {
            let s = loc.display();
            assert_eq!(Location::parse(&s), Some(loc.clone()), "round trip of {s}");
        }
        assert_eq!(Location::parse("  "), None);
        assert_eq!(Location::parse("TRASH:"), Some(Location::Trash));
    }

    #[test]
    fn remote_parents() {
        assert_eq!(remote_parent("sftp://u@h:22/a/b"), Some("sftp://u@h:22/a".into()));
        assert_eq!(remote_parent("sftp://u@h/a"), Some("sftp://u@h/".into()));
        assert_eq!(remote_parent("sftp://u@h/"), None);
        assert_eq!(remote_parent("s3://b/x/y?endpoint=http://m:9000"), Some("s3://b/x?endpoint=http://m:9000".into()));
        assert_eq!(Location::parse("webdav://h/x").unwrap().parent(), Some(Location::Remote("webdav://h/".into())));
        assert!(!looks_remote("C://x") && !looks_remote("/x") && looks_remote("ftp://h"));
    }

    #[test]
    fn location_tilde() {
        let home = directories::BaseDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(Location::parse("~"), Some(Location::Dir(home.clone())));
        assert_eq!(Location::parse("~/x"), Some(Location::Dir(home.join("x"))));
    }

    #[test]
    fn location_parent() {
        let l = Location::Archive { archive: PathBuf::from("/t/a.zip"), inner: "x/y".into() };
        let p1 = l.parent().unwrap();
        assert_eq!(p1, Location::Archive { archive: PathBuf::from("/t/a.zip"), inner: "x".into() });
        let p2 = p1.parent().unwrap();
        assert_eq!(p2, Location::Archive { archive: PathBuf::from("/t/a.zip"), inner: String::new() });
        assert_eq!(p2.parent(), Some(Location::Dir(PathBuf::from("/t"))));

        assert_eq!(Location::Dir(PathBuf::from("/a/b")).parent(), Some(Location::Dir(PathBuf::from("/a"))));
        assert_eq!(Location::Dir(PathBuf::from("/")).parent(), Some(Location::Computer));
        assert_eq!(Location::Computer.parent(), None);
        assert_eq!(Location::Trash.as_fs_path(), None);
    }
}

#[cfg(test)]
mod multibyte_tests {
    use super::*;

    #[test]
    fn non_ascii_paths_do_not_panic() {
        for p in ["D:/Ürünler", "D:/Ürün/", "ÇğŞ", "日本語/x", "sé", "tag:ü", "search:ö", "Ürün"] {
            let _ = Location::parse(p);
        }
        assert_eq!(strip_prefix_ci("TAG:red", "tag:"), Some("red"));
        assert_eq!(strip_prefix_ci("D:/Ürün", "tag:"), None);
    }
}
