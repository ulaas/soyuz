//! Natural sorting, unique-name generation and file-name validation.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::locale::{Locale, collation_weight, lower_char_in};

/// Default pattern for [`unique_name`].
pub const DEFAULT_UNIQUE_PATTERN: &str = "{stem} ({n}){ext}";

/// Natural ("human") string comparison: digit runs compare numerically
/// (`file2` < `file10`), everything else char by char, optionally case-insensitive.
/// Ties under case folding are broken by the raw comparison so the order is total.
///
/// Letters compare by the current [`locale`](crate::locale)'s collation: in Turkish,
/// `I`/`ı` and `İ`/`i` are case pairs and `ç ğ ı ö ş ü` sort right after their base
/// letters.
pub fn natural_cmp(a: &str, b: &str, case_insensitive: bool) -> Ordering {
    natural_cmp_in(a, b, case_insensitive, crate::locale::current())
}

/// [`natural_cmp`] in an explicit locale.
pub fn natural_cmp_in(a: &str, b: &str, case_insensitive: bool, l: Locale) -> Ordering {
    let ord = natural_cmp_inner(a, b, case_insensitive, l);
    if ord == Ordering::Equal { a.cmp(b) } else { ord }
}

/// Compare two (non-digit) chars in locale `l`.
#[inline]
fn char_cmp(ca: char, cb: char, ci: bool, l: Locale) -> Ordering {
    if ca == cb {
        return Ordering::Equal;
    }
    match l {
        Locale::EnUs if !ci => ca.cmp(&cb),
        // Unchanged from before locales existed, except that 'İ' folds to a plain 'i'.
        Locale::EnUs if ca != 'İ' && cb != 'İ' => ca.to_lowercase().cmp(cb.to_lowercase()),
        _ => {
            let (fa, fb) = if ci { (lower_char_in(ca, l), lower_char_in(cb, l)) } else { (ca, cb) };
            collation_weight(fa, l).cmp(&collation_weight(fb, l))
        }
    }
}

fn natural_cmp_inner(a: &str, b: &str, ci: bool, l: Locale) -> Ordering {
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
                let ord = char_cmp(ca, cb, ci, l);
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

/// Whether names on a file system of this type (as `/proc/self/mountinfo` or `statfs`
/// tools spell it: `vfat`, `exfat`, `ntfs3`, `fuseblk`, `cifs` …) must follow the Windows
/// rules ([`validate_name_with`]`(name, true)`): FAT, exFAT, NTFS (kernel `ntfs3` or
/// `ntfs-3g` through FUSE), SMB shares and WSL's `drvfs`. Case-insensitive; a FUSE
/// subtype (`fuse.ntfs-3g`, `fuseblk.exfat`) is judged by the subtype. Plain `fuseblk`
/// (no subtype; in practice ntfs-3g or exfat-fuse) counts as Windows-like.
pub fn fs_needs_windows_names(fs_type: &str) -> bool {
    let t = fs_type.trim().to_ascii_lowercase();
    let t = match t.split_once('.') {
        Some(("fuse" | "fuseblk", sub)) if !sub.is_empty() => sub.to_string(),
        _ => t,
    };
    matches!(
        t.as_str(),
        "vfat"
            | "fat"
            | "fat12"
            | "fat16"
            | "fat32"
            | "msdos"
            | "umsdos"
            | "exfat"
            | "exfat-fuse"
            | "ntfs"
            | "ntfs3"
            | "ntfs-3g"
            | "lowntfs-3g"
            | "fuseblk"
            | "cifs"
            | "smb3"
            | "smbfs"
            | "smb"
            | "drvfs"
    )
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

// ---- hidden direction / format characters (display only) --------------------------

/// Whether `c` is an invisible control that can disguise how a name reads: C0/C1
/// controls, bidi embeddings / overrides / isolates / marks (U+202A–U+202E,
/// U+2066–U+2069, U+200E/U+200F, U+061C), zero-width and invisible format characters
/// (U+200B–U+200D, U+2060–U+2064, U+FEFF, U+180E, U+00AD, U+034F, U+115F/U+1160,
/// U+3164, U+FFA0), interlinear annotation marks and Unicode tag characters.
///
/// This is context-free; [`name_parts`] additionally lets ZWJ / ZWNJ through between two
/// non-ASCII characters (emoji sequences, Persian and Indic scripts use them).
pub fn is_hidden_control(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{1F}' | '\u{7F}'..='\u{9F}'
        | '\u{AD}' | '\u{34F}' | '\u{61C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{206A}'..='\u{206F}'
        | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}'
        | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0000}'..='\u{E007F}')
}

/// One piece of a name split by [`name_parts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamePart<'a> {
    /// Ordinary text, shown as-is.
    Text(&'a str),
    /// A hidden control character, to be shown as a visible token.
    Control(char),
}

/// Split `name` into ordinary text and hidden controls (see [`is_hidden_control`]).
/// ZWJ / ZWNJ between two non-ASCII characters are kept as text.
pub fn name_parts(name: &str) -> Vec<NamePart<'_>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<char> = None;
    let mut it = name.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let next = it.peek().map(|&(_, n)| n);
        let joiner_ok = matches!(c, '\u{200C}' | '\u{200D}')
            && prev.is_some_and(|p| !p.is_ascii() && !is_hidden_control(p))
            && next.is_some_and(|n| !n.is_ascii() && !is_hidden_control(n));
        if is_hidden_control(c) && !joiner_ok {
            if start < i {
                out.push(NamePart::Text(&name[start..i]));
            }
            out.push(NamePart::Control(c));
            start = i + c.len_utf8();
        }
        prev = Some(c);
    }
    if start < name.len() {
        out.push(NamePart::Text(&name[start..]));
    }
    out
}

/// Whether `name` contains hidden direction / format characters (see [`name_parts`]).
pub fn has_hidden_controls(name: &str) -> bool {
    name_parts(name).iter().any(|p| matches!(p, NamePart::Control(_)))
}

/// Visible token for a hidden control, e.g. `⟨U+202E⟩`.
pub fn control_token(c: char) -> String {
    format!("\u{27E8}U+{:04X}\u{27E9}", c as u32)
}

/// `name` with every hidden control replaced by a visible `⟨U+XXXX⟩` token, so a
/// right-to-left override can't make `gpj.exe` read as `exe.jpg`. Display only: never
/// use the result as a file name. Borrows when there is nothing to reveal.
pub fn reveal_controls(name: &str) -> std::borrow::Cow<'_, str> {
    let parts = name_parts(name);
    if !parts.iter().any(|p| matches!(p, NamePart::Control(_))) {
        return std::borrow::Cow::Borrowed(name);
    }
    let mut s = String::with_capacity(name.len() + 8 * parts.len());
    for p in parts {
        match p {
            NamePart::Text(t) => s.push_str(t),
            NamePart::Control(c) => s.push_str(&control_token(c)),
        }
    }
    std::borrow::Cow::Owned(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn turkish_natural_sort() {
        let tr = Locale::TrTr;
        let mut v = vec!["zeytin", "çay", "Irmak", "istanbul", "ılık", "İzmir", "cam", "Şeker", "su", "öykü", "ozan"];
        v.sort_by(|a, b| natural_cmp_in(a, b, true, tr));
        assert_eq!(v, ["cam", "çay", "ılık", "Irmak", "istanbul", "İzmir", "ozan", "öykü", "su", "Şeker", "zeytin"]);
        // I/ı and İ/i are case pairs in Turkish: these differ only by case.
        assert_eq!(natural_cmp_inner("IŞIK", "ışık", true, tr), Ordering::Equal);
        assert_eq!(natural_cmp_inner("İNCE", "ince", true, tr), Ordering::Equal);
        assert_ne!(natural_cmp_inner("IŞIK", "işik", true, tr), Ordering::Equal);
        // Digits still compare numerically.
        let mut v = vec!["Şarkı 10", "şarkı 2", "Şarkı 1"];
        v.sort_by(|a, b| natural_cmp_in(a, b, true, tr));
        assert_eq!(v, ["Şarkı 1", "şarkı 2", "Şarkı 10"]);
        // English: 'İ' folds to a plain 'i'; code point order otherwise.
        assert_eq!(natural_cmp_inner("İzmir", "izmir", true, Locale::EnUs), Ordering::Equal);
        let mut v = vec!["zebra", "çay", "apple"];
        v.sort_by(|a, b| natural_cmp_in(a, b, true, Locale::EnUs));
        assert_eq!(v, ["apple", "zebra", "çay"]);
    }

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

    #[test]
    fn reveal_controls_tokens() {
        use std::borrow::Cow;
        assert!(matches!(reveal_controls("plain name.txt"), Cow::Borrowed(_)));
        assert!(!has_hidden_controls("Ünïcødé 名前.txt"));
        // The classic RLO spoof: "photo\u{202E}gpj.exe" renders as "photoexe.jpg".
        assert_eq!(reveal_controls("photo\u{202E}gpj.exe"), "photo\u{27E8}U+202E\u{27E9}gpj.exe");
        assert!(has_hidden_controls("photo\u{202E}gpj.exe"));
        for c in ['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'] {
            assert!(has_hidden_controls(&format!("a{c}b")), "{c:?}");
        }
        assert_eq!(reveal_controls("pay\u{200B}pal"), "pay\u{27E8}U+200B\u{27E9}pal");
        assert_eq!(reveal_controls("\u{FEFF}x"), "\u{27E8}U+FEFF\u{27E9}x");
        assert_eq!(reveal_controls("a\nb"), "a\u{27E8}U+000A\u{27E9}b");
        assert_eq!(reveal_controls("\u{E0041}"), "\u{27E8}U+E0041\u{27E9}");
        assert_eq!(control_token('\u{200F}'), "\u{27E8}U+200F\u{27E9}");
    }

    #[test]
    fn windows_name_rules_by_fs_type() {
        for t in [
            "vfat",
            "VFAT",
            "msdos",
            "exfat",
            "ntfs",
            "ntfs3",
            "fuseblk",
            "fuse.ntfs-3g",
            "fuseblk.exfat",
            "cifs",
            "smb3",
            " smbfs ",
            "drvfs",
        ] {
            assert!(fs_needs_windows_names(t), "{t}");
        }
        for t in ["ext4", "btrfs", "xfs", "tmpfs", "nfs4", "fuse.sshfs", "fuse", "overlay", "zfs", "", "9p"] {
            assert!(!fs_needs_windows_names(t), "{t}");
        }
    }

    #[test]
    fn joiners_in_scripts_and_emoji_are_not_flagged() {
        // Family emoji (ZWJ sequence) and Persian ZWNJ are legitimate.
        assert!(!has_hidden_controls("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}.png"));
        assert!(!has_hidden_controls("\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}"));
        // ...but a joiner touching ASCII, or at an edge, is.
        assert!(has_hidden_controls("ab\u{200D}cd"));
        assert!(has_hidden_controls("\u{200C}\u{0645}"));
        assert_eq!(
            name_parts("a\u{202E}b"),
            vec![NamePart::Text("a"), NamePart::Control('\u{202E}'), NamePart::Text("b")]
        );
        assert_eq!(name_parts(""), Vec::<NamePart<'_>>::new());
        assert_eq!(name_parts("\u{202E}"), vec![NamePart::Control('\u{202E}')]);
    }
}
