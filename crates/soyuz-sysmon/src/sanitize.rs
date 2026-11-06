//! OS strings are untrusted: a process can name itself anything, including terminal escapes
//! and bidirectional overrides that wreck the terminal UI or spoof the table. Every text that
//! reaches a frame or a list goes through here once.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use soyuz_fs::sysmon::{Connection, ProcDetails, ProcRaw, ServiceInfo, StartupItem};

fn banned(c: char) -> bool {
    c.is_control() // C0, DEL, C1 (including the 8-bit escape introducers)
        || matches!(c,
            '\u{061C}' | '\u{200E}' | '\u{200F}' // directional marks
            | '\u{2028}' | '\u{2029}' // line and paragraph separators
            | '\u{202A}'..='\u{202E}' // embeddings and overrides
            | '\u{2066}'..='\u{2069}') // isolates
}

/// Removes control characters and bidirectional controls. Tabs and line breaks become one
/// space, so words stay apart. Returns the input untouched when it is already clean.
pub fn sanitize_text(s: &str) -> Cow<'_, str> {
    if !s.chars().any(banned) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1B}' {
            // The whole ANSI sequence goes, not only its introducer.
            strip_escape(&mut chars);
        } else if matches!(c, '\t' | '\n' | '\r') {
            out.push(' ');
        } else if !banned(c) {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

fn strip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.peek() {
        // CSI: parameters up to a final byte in @..~
        Some('[') => {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
        // OSC: up to BEL or ST
        Some(']') => {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '\u{7}' || c == '\u{9C}' {
                    break;
                }
                if c == '\u{1B}' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    break;
                }
            }
        }
        // A two-byte sequence.
        Some(c) if ('@'..='~').contains(c) => {
            chars.next();
        }
        _ => {}
    }
}

/// [`sanitize_text`] on a shared string; a clean one stays the same allocation.
pub fn sanitize_arc(s: &Arc<str>) -> Arc<str> {
    match sanitize_text(s) {
        Cow::Borrowed(_) => s.clone(),
        Cow::Owned(clean) => Arc::from(clean),
    }
}

/// Executable paths are shown, not opened by this crate: a dirty one is replaced by its clean
/// text. Clean paths, including ones that are not UTF-8, stay untouched.
pub fn sanitize_path(p: &Arc<Path>) -> Arc<Path> {
    let text = p.to_string_lossy();
    match sanitize_text(&text) {
        Cow::Borrowed(_) => p.clone(),
        Cow::Owned(clean) => Arc::from(Path::new(&clean)),
    }
}

/// Cleans every text field of one process record.
pub fn sanitize_raw(raw: &mut ProcRaw) {
    raw.name = sanitize_arc(&raw.name);
    raw.exe = raw.exe.as_ref().map(sanitize_path);
    for field in
        [&mut raw.cmdline, &mut raw.user, &mut raw.session, &mut raw.arch, &mut raw.cgroup, &mut raw.description]
    {
        *field = field.as_ref().map(sanitize_arc);
    }
}

fn in_place(s: &mut String) {
    if let Cow::Owned(clean) = sanitize_text(s) {
        *s = clean;
    }
}

/// Cleans a service list in place.
pub fn sanitize_services(list: &mut [ServiceInfo]) {
    for s in list {
        in_place(&mut s.name);
        in_place(&mut s.display_name);
        in_place(&mut s.description);
        in_place(&mut s.start_mode);
    }
}

/// Cleans a startup item list in place.
pub fn sanitize_startup(list: &mut [StartupItem]) {
    for s in list {
        in_place(&mut s.name);
        in_place(&mut s.command);
        in_place(&mut s.source);
    }
}

/// Cleans a socket list in place.
pub fn sanitize_connections(list: &mut [Connection]) {
    for c in list {
        in_place(&mut c.local_addr);
        in_place(&mut c.remote_addr);
        in_place(&mut c.state);
    }
}

/// Cleans the texts of on-demand process details in place.
pub fn sanitize_details(d: &mut ProcDetails) {
    if let Some(c) = &mut d.cmdline {
        in_place(c);
    }
    for t in &mut d.threads {
        in_place(&mut t.name);
    }
    for m in &mut d.modules {
        in_place(&mut m.path);
    }
    for f in &mut d.open_files {
        in_place(&mut f.path);
    }
    for (k, v) in &mut d.environment {
        in_place(k);
        in_place(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_is_borrowed() {
        assert!(matches!(sanitize_text("firefox.exe --new-tab"), Cow::Borrowed(_)));
        assert!(matches!(sanitize_text("Türkçe 日本語"), Cow::Borrowed(_)));
    }

    #[test]
    fn strips_escapes_and_controls() {
        assert_eq!(sanitize_text("\u{1B}[2Jevil"), "evil");
        assert_eq!(sanitize_text("a\u{1B}]0;title\u{7}b"), "ab");
        assert_eq!(sanitize_text("a\u{1B}]0;title\u{1B}\\b"), "ab");
        assert_eq!(sanitize_text("a\u{0}b\u{7F}c\u{9B}d"), "abcd");
        assert_eq!(sanitize_text("one\ttwo\nthree"), "one two three");
    }

    #[test]
    fn strips_bidi_overrides() {
        assert_eq!(sanitize_text("cod\u{202E}exe.bat"), "codexe.bat");
        assert_eq!(sanitize_text("\u{2066}x\u{2069}"), "x");
    }

    #[test]
    fn clean_arc_keeps_its_allocation() {
        let a: Arc<str> = Arc::from("clean");
        assert!(Arc::ptr_eq(&a, &sanitize_arc(&a)));
        let dirty: Arc<str> = Arc::from("d\u{1B}[0mirty");
        assert_eq!(&*sanitize_arc(&dirty), "dirty");
    }

    #[test]
    fn dirty_path_is_replaced_clean_path_kept() {
        let clean: Arc<Path> = Arc::from(Path::new("/usr/bin/x"));
        assert!(Arc::ptr_eq(&clean, &sanitize_path(&clean)));
        let dirty: Arc<Path> = Arc::from(Path::new("/tmp/a\u{202E}b"));
        assert_eq!(&*sanitize_path(&dirty), Path::new("/tmp/ab"));
    }
}
