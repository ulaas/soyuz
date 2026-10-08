//! Path helpers: long-path wide strings, normalization, env expansion, overlap checks.

use std::path::{Component, Path, PathBuf};

/// Windows: absolute + `\\?\C:\...` / `\\?\UNC\...`, NUL-terminated UTF-16 (verbatim and
/// `\\.\` device paths pass through). Other OSes: the lossy path as NUL-terminated UTF-16
/// (unused by the unix implementation; present so callers and tests are cross-platform).
pub fn to_wide_long(path: &Path) -> Vec<u16> {
    let s = long_form(path);
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn long_form(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.starts_with(r"\\?\") || raw.starts_with(r"\\.\") {
        return raw.into_owned();
    }
    let abs = normalize(path);
    let s = abs.to_string_lossy().replace('/', "\\");
    if let Some(unc) = s.strip_prefix(r"\\") { format!(r"\\?\UNC\{unc}") } else { format!(r"\\?\{s}") }
}

#[cfg(not(windows))]
fn long_form(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Remove a `\\?\` / `\\?\UNC\` prefix for display.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

/// `/`, `C:\`, `\\server\share\`.
pub fn is_root(path: &Path) -> bool {
    path.has_root() && path.parent().is_none()
}

/// `std::path::absolute` + component cleanup (`.` removed, `..` pops, never above the
/// root). No symlink resolution.
pub fn normalize(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !is_root(&out) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Expand `%VAR%`, `$VAR`, `${VAR}` and a leading `~`. Unknown variables are kept as-is.
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let rest = if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        out.push_str(&home().to_string_lossy());
        &s[1..]
    } else {
        s
    };
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|&x| x == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if !name.is_empty()
                    && is_var_name(&name)
                    && let Ok(v) = std::env::var(&name)
                {
                    out.push_str(&v);
                    i += end + 2;
                    continue;
                }
            }
        } else if c == '$' {
            let (name, consumed) = if chars.get(i + 1) == Some(&'{') {
                match chars[i + 2..].iter().position(|&x| x == '}') {
                    Some(end) => (chars[i + 2..i + 2 + end].iter().collect::<String>(), end + 3),
                    None => (String::new(), 0),
                }
            } else {
                let n: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_').collect();
                let len = n.chars().count();
                (n, len + 1)
            };
            if !name.is_empty()
                && is_var_name(&name)
                && let Ok(v) = std::env::var(&name)
            {
                out.push_str(&v);
                i += consumed;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_var_name(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '(' || c == ')')
}

/// The user's home directory (`/` / `C:\` fallback).
pub fn home() -> PathBuf {
    #[allow(deprecated)]
    std::env::home_dir().unwrap_or_else(|| if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") })
}

/// `a == b` or one is an ancestor of the other (after [`normalize`]; case-insensitive on
/// Windows). Used by the jobs scheduler and the MoveIntoSelf check.
pub fn path_overlaps(a: &Path, b: &Path) -> bool {
    let ca = comparable(&normalize(a));
    let cb = comparable(&normalize(b));
    let starts = |x: &[String], y: &[String]| x.len() >= y.len() && x[..y.len()] == *y;
    starts(&ca, &cb) || starts(&cb, &ca)
}

fn comparable(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, Component::RootDir))
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if cfg!(windows) { s.to_lowercase() } else { s.into_owned() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_overlap() {
        let base = std::env::temp_dir();
        let n = normalize(&base.join("a").join(".").join("b").join("..").join("c"));
        assert_eq!(n, normalize(&base.join("a").join("c")));
        assert!(path_overlaps(&base.join("a"), &base.join("a").join("b")));
        assert!(path_overlaps(&base.join("a").join("b"), &base.join("a")));
        assert!(path_overlaps(&base.join("a"), &base.join("a")));
        assert!(!path_overlaps(&base.join("a"), &base.join("ab")));
        assert!(!path_overlaps(&base.join("a").join("x"), &base.join("a").join("y")));
    }

    #[test]
    fn roots() {
        if cfg!(windows) {
            assert!(is_root(Path::new("C:\\")));
            assert!(!is_root(Path::new("C:\\x")));
        } else {
            assert!(is_root(Path::new("/")));
            assert!(!is_root(Path::new("/x")));
        }
        assert!(!is_root(Path::new("relative")));
    }

    #[test]
    fn verbatim() {
        assert_eq!(strip_verbatim(Path::new(r"\\?\C:\x")), PathBuf::from(r"C:\x"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\UNC\srv\share\x")), PathBuf::from(r"\\srv\share\x"));
        assert_eq!(strip_verbatim(Path::new("/plain")), PathBuf::from("/plain"));
    }

    #[cfg(windows)]
    #[test]
    fn wide_long_windows() {
        let w = |s: &str| String::from_utf16(&to_wide_long(Path::new(s))[..]).unwrap();
        assert_eq!(w(r"C:\a\b"), "\\\\?\\C:\\a\\b\0");
        assert_eq!(w(r"C:/a/./b"), "\\\\?\\C:\\a\\b\0");
        assert_eq!(w(r"\\srv\share\x"), "\\\\?\\UNC\\srv\\share\\x\0");
        assert_eq!(w(r"\\?\C:\already"), "\\\\?\\C:\\already\0");
        assert_eq!(w(r"\\.\PhysicalDrive0"), "\\\\.\\PhysicalDrive0\0");
        assert!(w("relative").starts_with("\\\\?\\"));
        assert_eq!(*to_wide_long(Path::new("C:\\x")).last().unwrap(), 0);
    }

    #[test]
    fn env_expansion() {
        // `set_var` is unsafe in edition 2024; use a variable that always exists.
        let path_var = std::env::var("PATH").unwrap();
        assert_eq!(expand_env("$PATH"), path_var);
        assert_eq!(expand_env("${PATH}x"), format!("{path_var}x"));
        assert_eq!(expand_env("%PATH%"), path_var);
        assert_eq!(expand_env("$SOYUZ_SURELY_UNSET_VAR"), "$SOYUZ_SURELY_UNSET_VAR");
        assert_eq!(expand_env("100%"), "100%");
        assert_eq!(expand_env("~"), home().to_string_lossy());
    }
}
