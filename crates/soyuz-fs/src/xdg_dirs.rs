//! XDG user directories (`user-dirs.dirs`) and GTK bookmarks. Linux fills these in; other
//! OSes return nothing.
//!
//! * `$XDG_CONFIG_HOME/user-dirs.dirs` (written by `xdg-user-dirs-update`): shell-style
//!   `XDG_DESKTOP_DIR="$HOME/Desktop"` lines; only `"$HOME/…"` and absolute values are
//!   valid. A directory equal to `$HOME` means "disabled".
//! * `$XDG_CONFIG_HOME/gtk-3.0/bookmarks` (the GTK / Nautilus sidebar bookmarks, legacy
//!   `~/.gtk-bookmarks` as a fallback): one `URI [label]` per line.

use std::path::{Path, PathBuf};

/// The well-known user folders (`None` when not configured or disabled).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserDirs {
    pub desktop: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
    pub music: Option<PathBuf>,
    pub pictures: Option<PathBuf>,
    pub videos: Option<PathBuf>,
    pub templates: Option<PathBuf>,
    pub public: Option<PathBuf>,
}

/// One GTK bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    /// The URI as written (`file:///…`, `sftp://host/…`, `smb://…`).
    pub uri: String,
    /// The local path for `file://` URIs (percent-decoded).
    pub path: Option<PathBuf>,
    /// The label after the URI, if any.
    pub label: Option<String>,
}

impl Bookmark {
    /// The label, else the last path segment of the URI (percent-decoded).
    pub fn display_name(&self) -> String {
        if let Some(l) = &self.label {
            return l.clone();
        }
        if let Some(n) = self.path.as_deref().and_then(Path::file_name) {
            return n.to_string_lossy().into_owned();
        }
        let tail = self.uri.trim_end_matches('/').rsplit('/').next().unwrap_or(&self.uri);
        String::from_utf8_lossy(&crate::launch::linux::uri::percent_decode(tail)).into_owned()
    }
}

/// Parse a shell-quoted value: `"…"` with `\"`, `\\`, `\$`, `` \` `` escapes, or a
/// bare word.
fn unquote(v: &str) -> String {
    let v = v.trim();
    let Some(inner) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return v.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c == '\\'
            && let Some(n) = it.next()
        {
            if !matches!(n, '"' | '\\' | '$' | '`') {
                out.push('\\');
            }
            out.push(n);
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse `user-dirs.dirs` text for `home`.
pub fn parse_user_dirs(text: &str, home: &Path) -> UserDirs {
    let mut d = UserDirs::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = unquote(v);
        let path = if v == "$HOME" || v == "$HOME/" {
            None
        } else if let Some(rest) = v.strip_prefix("$HOME/") {
            Some(home.join(rest.trim_end_matches('/')))
        } else if v.starts_with('/') {
            Some(PathBuf::from(v.trim_end_matches('/'))).filter(|p| p != home)
        } else {
            None
        };
        let slot = match k.trim() {
            "XDG_DESKTOP_DIR" => &mut d.desktop,
            "XDG_DOCUMENTS_DIR" => &mut d.documents,
            "XDG_DOWNLOAD_DIR" => &mut d.downloads,
            "XDG_MUSIC_DIR" => &mut d.music,
            "XDG_PICTURES_DIR" => &mut d.pictures,
            "XDG_VIDEOS_DIR" => &mut d.videos,
            "XDG_TEMPLATES_DIR" => &mut d.templates,
            "XDG_PUBLICSHARE_DIR" => &mut d.public,
            _ => continue,
        };
        *slot = path;
    }
    d
}

/// Parse a GTK `bookmarks` file.
pub fn parse_bookmarks(text: &str) -> Vec<Bookmark> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (uri, label) = match l.split_once(' ') {
                Some((u, rest)) => (u, Some(rest.trim().to_string()).filter(|s| !s.is_empty())),
                None => (l, None),
            };
            uri.contains(':').then(|| Bookmark {
                uri: uri.to_string(),
                path: crate::launch::linux::uri::uri_to_path(uri),
                label,
            })
        })
        .collect()
}

/// The user folders of the current user (Linux / BSD; empty elsewhere).
pub fn user_dirs() -> UserDirs {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let file = crate::launch::linux::host_config_home().join("user-dirs.dirs");
        std::fs::read_to_string(file).map(|t| parse_user_dirs(&t, &crate::path::home())).unwrap_or_default()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        UserDirs::default()
    }
}

/// The GTK sidebar bookmarks (Linux / BSD; empty elsewhere).
pub fn gtk_bookmarks() -> Vec<Bookmark> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let primary = crate::launch::linux::host_config_home().join("gtk-3.0/bookmarks");
        let legacy = crate::path::home().join(".gtk-bookmarks");
        std::fs::read_to_string(primary)
            .or_else(|_| std::fs::read_to_string(legacy))
            .map(|t| parse_bookmarks(&t))
            .unwrap_or_default()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_file() {
        let home = Path::new("/home/u");
        let text = r#"
# This file is written by xdg-user-dirs-update
XDG_DESKTOP_DIR="$HOME/Desktop"
XDG_DOWNLOAD_DIR="$HOME/İndirilenler"
XDG_TEMPLATES_DIR="$HOME/"
XDG_PUBLICSHARE_DIR="/srv/public/"
XDG_DOCUMENTS_DIR="$HOME/My \"Docs\""
XDG_MUSIC_DIR="relative/ignored"
XDG_PICTURES_DIR=$HOME/Pictures
XDG_VIDEOS_DIR="/home/u"
"#;
        let d = parse_user_dirs(text, home);
        assert_eq!(d.desktop, Some(home.join("Desktop")));
        assert_eq!(d.downloads, Some(home.join("İndirilenler")));
        assert_eq!(d.templates, None, "$HOME means disabled");
        assert_eq!(d.public, Some(PathBuf::from("/srv/public")));
        assert_eq!(d.documents, Some(home.join("My \"Docs\"")));
        assert_eq!(d.music, None);
        assert_eq!(d.pictures, Some(home.join("Pictures")));
        assert_eq!(d.videos, None);
    }

    #[cfg(unix)]
    #[test]
    fn bookmarks_file() {
        let b = parse_bookmarks(
            "file:///home/u/My%20Projects Projects\nfile:///mnt/data\nsftp://host/srv/www Web server\n\n# x\nnot-a-uri\n",
        );
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].path, Some(PathBuf::from("/home/u/My Projects")));
        assert_eq!(b[0].label.as_deref(), Some("Projects"));
        assert_eq!(b[1].display_name(), "data");
        assert_eq!(b[2].path, None);
        assert_eq!(b[2].display_name(), "Web server");
        let unlabeled = parse_bookmarks("smb://nas/My%20Share/")[0].display_name();
        assert_eq!(unlabeled, "My Share");
    }
}
