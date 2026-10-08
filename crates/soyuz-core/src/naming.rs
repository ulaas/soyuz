//! Natural sorting, unique-name generation and file-name validation.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// Default pattern for [`unique_name`].
pub const DEFAULT_UNIQUE_PATTERN: &str = "{stem} ({n}){ext}";

/// Natural ("human") string comparison: digit runs compare numerically
/// (`file2` < `file10`), everything else char by char, optionally case-insensitive.
/// Ties under case folding are broken by the raw comparison so the order is total.
pub fn natural_cmp(a: &str, b: &str, case_insensitive: bool) -> Ordering {
    let ord = natural_cmp_inner(a, b, case_insensitive);
    if ord == Ordering::Equal { a.cmp(b) } else { ord }
}

fn natural_cmp_inner(a: &str, b: &str, ci: bool) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let da = take_digits(&mut ai);
                let db = take_digits(&mut bi);
                let ta = da.trim_start_matches('0');
                let tb = db.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
                // Same value: fewer leading zeros first ("1" < "01").
                let ord = da.len().cmp(&db.len());
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                ai.next();
                bi.next();
                let ord = if ci {
                    let la = ca.to_lowercase();
                    let lb = cb.to_lowercase();
                    la.cmp(lb)
                } else {
                    ca.cmp(&cb)
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn take_digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(&c) = it.peek() {
        if c.is_ascii_digit() {
            s.push(c);
            it.next();
        } else {
            break;
        }
    }
    s
}

/// Pick a free name in `dir` for `name`: returns `dir/name` if free, otherwise expands
/// `pattern` (`{stem}`, `{n}`, `{ext}` where `{ext}` includes the dot; empty pattern =
/// [`DEFAULT_UNIQUE_PATTERN`]) with n = 2, 3, ... until `exists` returns false.
/// With the default pattern an existing `" (k)"` suffix is continued (`a (2).txt` → `a (3).txt`).
pub fn unique_name(dir: &Path, name: &str, pattern: &str, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let pattern = if pattern.is_empty() { DEFAULT_UNIQUE_PATTERN } else { pattern };
    let (mut stem, ext) = match crate::entry::name_extension(name) {
        Some(e) => (&name[..name.len() - e.len() - 1], &name[name.len() - e.len() - 1..]),
        None => (name, ""),
    };
    let mut n: u64 = 2;
    if pattern == DEFAULT_UNIQUE_PATTERN
        && let Some((base, k)) = split_counter_suffix(stem)
    {
        stem = base;
        n = k + 1;
    }
    loop {
        let candidate = pattern.replace("{stem}", stem).replace("{n}", &n.to_string()).replace("{ext}", ext);
        let p = dir.join(&candidate);
        if !exists(&p) {
            return p;
        }
        n += 1;
    }
}

/// `"name (3)"` → `Some(("name", 3))`.
fn split_counter_suffix(stem: &str) -> Option<(&str, u64)> {
    let rest = stem.strip_suffix(')')?;
    let open = rest.rfind(" (")?;
    let num = &rest[open + 2..];
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((&rest[..open], num.parse().ok()?))
}

/// Why a file name is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("name is empty")]
    Empty,
    #[error("\"{0}\" is a reserved name")]
    Reserved(String),
    #[error("character {0:?} is not allowed")]
    InvalidChar(char),
    #[error("name must not end with a dot or space")]
    TrailingDotOrSpace,
    #[error("name is too long ({0} units, max 255)")]
    TooLong(usize),
}

/// Validate a single path component with the rules of the current OS.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    validate_name_with(name, cfg!(windows))
}

/// Validate a single path component; `windows_rules` applies the Windows reserved
/// names / characters / trailing-dot rules (used on Windows and with
/// `general.strict_windows_names = true`).
pub fn validate_name_with(name: &str, windows_rules: bool) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name == "." || name == ".." {
        return Err(NameError::Reserved(name.to_string()));
    }
    let len = if windows_rules { name.encode_utf16().count() } else { name.len() };
    if len > 255 {
        return Err(NameError::TooLong(len));
    }
    for c in name.chars() {
        if c == '/' || c == '\0' {
            return Err(NameError::InvalidChar(c));
        }
        if windows_rules && (matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || (c as u32) < 32) {
            return Err(NameError::InvalidChar(c));
        }
    }
    if windows_rules {
        if name.ends_with('.') || name.ends_with(' ') {
            return Err(NameError::TrailingDotOrSpace);
        }
        let base = name.split('.').next().unwrap_or(name).trim_end();
        if is_windows_reserved(base) {
            return Err(NameError::Reserved(base.to_string()));
        }
    }
    Ok(())
}

fn is_windows_reserved(base: &str) -> bool {
    let upper = base.to_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    if prefix != "COM" && prefix != "LPT" {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    matches!(rest.as_slice(), [c] if matches!(c, '1'..='9' | '\u{00B9}' | '\u{00B2}' | '\u{00B3}'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn natural_order_table() {
        let mut v = vec![
            "file10.txt",
            "file2.txt",
            "File1.txt",
            "file01.txt",
            "a",
            "B",
            "file1.txt",
            "z9",
            "z10",
            "img_0010",
            "img_9",
        ];
        v.sort_by(|a, b| natural_cmp(a, b, true));
        assert_eq!(
            v,
            vec![
                "a",
                "B",
                "File1.txt",
                "file1.txt",
                "file01.txt",
                "file2.txt",
                "file10.txt",
                "img_9",
                "img_0010",
                "z9",
                "z10"
            ]
        );
    }

    #[test]
    fn natural_case_sensitive() {
        assert_eq!(natural_cmp("B", "a", false), Ordering::Less);
        assert_eq!(natural_cmp("B", "a", true), Ordering::Greater);
        assert_eq!(natural_cmp("abc", "abc", true), Ordering::Equal);
        assert_eq!(natural_cmp("x99999999999999999999999", "x100000000000000000000000", true), Ordering::Less);
    }

    #[test]
    fn unique_name_collisions() {
        let dir = Path::new("/d");
        let taken: HashSet<PathBuf> =
            ["/d/a.txt", "/d/a (2).txt", "/d/b", "/d/c (4).txt"].iter().map(PathBuf::from).collect();
        let exists = |p: &Path| taken.contains(p);
        assert_eq!(unique_name(dir, "new.txt", "", &exists), PathBuf::from("/d/new.txt"));
        assert_eq!(unique_name(dir, "a.txt", "", &exists), PathBuf::from("/d/a (3).txt"));
        assert_eq!(unique_name(dir, "b", "", &exists), PathBuf::from("/d/b (2)"));
        let taken2: HashSet<PathBuf> = ["/d/c (4).txt"].iter().map(PathBuf::from).collect();
        let exists2 = |p: &Path| taken2.contains(p);
        assert_eq!(unique_name(dir, "c (4).txt", "", &exists2), PathBuf::from("/d/c (5).txt"));
        assert_eq!(unique_name(dir, "a.txt", "{stem} - Copy {n}{ext}", &exists), PathBuf::from("/d/a - Copy 2.txt"));
        // Dot-files keep their full name as stem.
        let taken3: HashSet<PathBuf> = ["/d/.env"].iter().map(PathBuf::from).collect();
        assert_eq!(unique_name(dir, ".env", "", &|p: &Path| taken3.contains(p)), PathBuf::from("/d/.env (2)"));
    }

    #[test]
    fn validate_rules() {
        assert_eq!(validate_name_with("", false), Err(NameError::Empty));
        assert_eq!(validate_name_with("..", false), Err(NameError::Reserved("..".into())));
        assert_eq!(validate_name_with("a/b", false), Err(NameError::InvalidChar('/')));
        assert_eq!(validate_name_with("a:b", false), Ok(()));
        assert_eq!(validate_name_with("a:b", true), Err(NameError::InvalidChar(':')));
        assert_eq!(validate_name_with("con", true), Err(NameError::Reserved("con".into())));
        assert_eq!(validate_name_with("CON.txt", true), Err(NameError::Reserved("CON".into())));
        assert_eq!(validate_name_with("COM1", true), Err(NameError::Reserved("COM1".into())));
        assert_eq!(validate_name_with("COM\u{00B9}", true), Err(NameError::Reserved("COM\u{00B9}".into())));
        assert_eq!(validate_name_with("COM10", true), Ok(()));
        assert_eq!(validate_name_with("console", true), Ok(()));
        assert_eq!(validate_name_with("trail.", true), Err(NameError::TrailingDotOrSpace));
        assert_eq!(validate_name_with("trail ", true), Err(NameError::TrailingDotOrSpace));
        assert_eq!(validate_name_with("trail.", false), Ok(()));
        let long = "x".repeat(256);
        assert_eq!(validate_name_with(&long, false), Err(NameError::TooLong(256)));
        assert!(validate_name("normal name.txt").is_ok());
    }
}
