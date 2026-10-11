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
    let rest = uri.strip_prefix("file:").or_else(|| uri.strip_prefix("FILE:"))?;
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
