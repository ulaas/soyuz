//! `file://` URIs as the freedesktop specs use them (desktop-entry `%u`, `FileManager1`,
//! the thumbnail cache, GTK bookmarks): the path's raw bytes, percent-encoded, so
//! non-UTF-8 names survive the round trip.

use std::borrow::Cow;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The raw bytes of a path (lossy UTF-8 off unix, where paths are not byte strings).
pub(crate) fn path_bytes(p: &Path) -> Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Cow::Borrowed(p.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    {
        match p.to_string_lossy() {
            Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
            Cow::Owned(s) => Cow::Owned(s.into_bytes()),
        }
    }
}

/// A path from raw bytes (lossy off unix).
pub(crate) fn path_from_bytes(b: Vec<u8>) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(OsString::from_vec(b))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(OsString::from(String::from_utf8_lossy(&b).into_owned()))
    }
}

/// Percent-encode everything except RFC 3986 unreserved characters and `/` (what GLib's
/// `g_filename_to_uri` keeps is a superset; encoding more is always valid).
pub(crate) fn percent_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
    out
}

/// Decode `%XX` escapes (malformed escapes are kept literally); shared with the trash
/// `.trashinfo` parser.
pub(crate) fn percent_decode(s: &str) -> Vec<u8> {
    crate::trash::percent_decode_pub(s)
}

/// `file:///abs/path` for an absolute path (relative paths are resolved against the
/// current directory first).
pub(crate) fn file_uri(path: &Path) -> String {
    let abs: Cow<'_, Path> = if path.is_absolute() {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(std::env::current_dir().unwrap_or_default().join(path))
    };
    let bytes = path_bytes(&abs);
    let mut s = String::from("file://");
    if !bytes.starts_with(b"/") {
        s.push('/');
    }
    s.push_str(&percent_encode(&bytes));
    s
}

/// The local path of a `file:` URI (`file:///p`, `file://localhost/p`, `file:/p`);
/// `None` for other schemes and remote hosts. A `?query` / `#fragment` is dropped.
pub(crate) fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = strip_file_scheme(uri)?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let path = if let Some(after) = rest.strip_prefix("//") {
        let (host, p) = after.split_at(after.find('/')?);
        if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
            return None;
        }
        p
    } else {
        rest
    };
    path.starts_with('/').then(|| path_from_bytes(percent_decode(path)))
}

/// `uri` after a case-insensitive `file:` scheme.
fn strip_file_scheme(uri: &str) -> Option<&str> {
    uri.get(..5).filter(|s| s.eq_ignore_ascii_case("file:")).map(|_| &uri[5..])
}

/// The Windows path of a `file:` URI: `file:///C:/x` → `C:\x` (also `C|`),
/// `file://host/share/x` and `file:////host/share/x` → `\\host\share\x`;
/// `file://localhost/C:/x` is local. `None` for other schemes, a rooted path without a
/// drive and a host without a share. Pure string logic, so it is tested on every OS.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn uri_to_windows_path(uri: &str) -> Option<String> {
    let rest = strip_file_scheme(uri)?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (host, path) = match rest.strip_prefix("//") {
        Some(after) => after.split_at(after.find('/').unwrap_or(after.len())),
        None => ("", rest),
    };
    let decoded = String::from_utf8_lossy(&percent_decode(path)).into_owned();
    let host = if host.eq_ignore_ascii_case("localhost") { "" } else { host };
    let unc = |host: &str, path: &str| {
        let path = path.trim_start_matches('/');
        let share_ok = !host.is_empty() && path.split('/').next().is_some_and(|s| !s.is_empty());
        share_ok.then(|| format!(r"\\{host}\{}", path.replace('/', "\\")))
    };
    if !host.is_empty() {
        let host = String::from_utf8_lossy(&percent_decode(host)).into_owned();
        return unc(&host, &decoded);
    }
    // `file:////server/share/x` (an empty authority, then a UNC path).
    if let Some(after) = decoded.strip_prefix("//") {
        let (h, p) = after.split_at(after.find('/').unwrap_or(after.len()));
        return unc(h, p);
    }
    let p = decoded.strip_prefix('/').unwrap_or(&decoded);
    let b = p.as_bytes();
    let drive = b.len() >= 2
        && b[0].is_ascii_alphabetic()
        && matches!(b[1], b':' | b'|')
        && (b.len() == 2 || matches!(b[2], b'/' | b'\\'));
    if !drive {
        return None;
    }
    let tail = p[2..].replace('/', "\\");
    Some(format!("{}:{}", b[0] as char, if tail.is_empty() { "\\" } else { &tail }))
}

/// The scheme of a URL (`https`, `mailto`, …), lowercased; `None` for things that do not
/// look like a URL (including Windows drive paths such as `C:\x`).
pub(crate) fn scheme(url: &str) -> Option<String> {
    let (s, _) = url.split_once(':')?;
    let ok = s.len() >= 2
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then(|| s.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_file_uris() {
        let w = |u: &str| uri_to_windows_path(u);
        assert_eq!(w("file:///C:/Users/a%20b/x.txt").as_deref(), Some(r"C:\Users\a b\x.txt"));
        assert_eq!(w("FILE:///c:/x").as_deref(), Some(r"c:\x"));
        assert_eq!(w("File:///D|/x/y").as_deref(), Some(r"D:\x\y"));
        assert_eq!(w("file:///C:").as_deref(), Some(r"C:\"));
        assert_eq!(w("file:///C:/").as_deref(), Some(r"C:\"));
        assert_eq!(w("file://localhost/C:/x").as_deref(), Some(r"C:\x"));
        assert_eq!(w("file:C:/x").as_deref(), Some(r"C:\x"));
        assert_eq!(w("file://server/share/dir/f.txt").as_deref(), Some(r"\\server\share\dir\f.txt"));
        assert_eq!(w("file:////server/share/f").as_deref(), Some(r"\\server\share\f"));
        assert_eq!(w("file:///C:/x%23y?q#frag").as_deref(), Some(r"C:\x#y"));
        assert_eq!(w("file:///C:/%C3%BC").as_deref(), Some(r"C:\ü"));
        assert_eq!(w("file://server"), None, "no share");
        assert_eq!(w("file:///usr/bin"), None, "no drive");
        assert_eq!(w("https://x/C:/y"), None);
        assert_eq!(w("fil"), None);
    }

    #[cfg(unix)]
    #[test]
    fn file_uris_round_trip() {
        assert_eq!(file_uri(Path::new("/home/u/My Docs/a#b.txt")), "file:///home/u/My%20Docs/a%23b.txt");
        assert_eq!(file_uri(Path::new("/tmp/ü")), "file:///tmp/%C3%BC");
        assert_eq!(uri_to_path("file:///home/u/My%20Docs"), Some(PathBuf::from("/home/u/My Docs")));
        assert_eq!(uri_to_path("file://localhost/x"), Some(PathBuf::from("/x")));
        assert_eq!(uri_to_path("file:/x%2"), Some(PathBuf::from("/x%2")));
        assert_eq!(uri_to_path("file://host/x"), None);
        assert_eq!(uri_to_path("https://x/y"), None);
        // Non-UTF-8 bytes survive.
        use std::os::unix::ffi::OsStrExt;
        let p = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff name"));
        let u = file_uri(p);
        assert_eq!(u, "file:///tmp/%FF%20name");
        assert_eq!(uri_to_path(&u).as_deref(), Some(p));
    }

    #[test]
    fn schemes() {
        assert_eq!(scheme("https://x").as_deref(), Some("https"));
        assert_eq!(scheme("MailTo:a@b").as_deref(), Some("mailto"));
        assert_eq!(scheme(r"C:\x"), None);
        assert_eq!(scheme("/abs/path"), None);
        assert_eq!(scheme("x-scheme+a.b:1").as_deref(), Some("x-scheme+a.b"));
    }
}
