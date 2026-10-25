//! Entry points for the `fuzz/` targets (cargo feature `fuzzing`): every parser in this
//! crate that reads untrusted bytes. Each function must never panic for any input.

/// Windows shell link (`.lnk`) parser.
pub fn lnk(data: &[u8]) {
    if let Ok(s) = crate::launch::lnk::parse(data) {
        let _ = format!("{s:?}");
    }
}

/// PE resource parser for program icons, at a small and a large size; a returned icon
/// is a one-image `.ico` file.
pub fn exe_icon(data: &[u8]) {
    for px in [16, 256] {
        if let Some(ico) = crate::exe_icon::icon_from(&mut std::io::Cursor::new(data), px) {
            assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0], "one-image ICO header");
        }
    }
}

/// `$Recycle.Bin` `$I` record parser (v1 / v2); a parsed record re-encodes to a record
/// that parses to the same fields.
pub fn recycle_i(data: &[u8]) {
    if let Some(i) = crate::trash::windows::parse_i(data) {
        let again = crate::trash::windows::format_i(i.size, i.deleted, &i.path);
        let back = crate::trash::windows::parse_i(&again).expect("re-encoded $I record parses");
        assert_eq!((back.size, back.deleted, &back.path), (i.size, i.deleted, &i.path));
    }
}

/// freedesktop `.trashinfo` parser plus the percent-encoding round trip.
pub fn trashinfo(text: &str) {
    let _ = crate::trash::freedesktop::parse_trashinfo(text);
    let decoded = crate::trash::freedesktop::percent_decode(text);
    let enc = crate::trash::freedesktop::percent_encode(&decoded);
    assert_eq!(crate::trash::freedesktop::percent_decode(&enc), decoded, "percent round trip");
}

/// `.desktop` entries, `Exec=` tokenizing, `globs2` and `mimeapps.list`.
pub fn desktop(text: &str) {
    if let Some(e) = crate::launch::linux::parse_desktop(text) {
        let _ = crate::launch::linux::app_from_entry("fuzz.desktop", &e);
    }
    let _ = crate::launch::linux::exec_tokens(text);
    let globs = crate::launch::linux::parse_globs2(text);
    let _ = crate::launch::linux::mime_for_name(&globs, "file.tar.gz");
    let _ = crate::launch::linux::parse_mimeapps(text);
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::lnk(&[0x4c, 0, 0, 0, 1, 0x14, 2, 0]);
        super::lnk(&[]);
        let rec = crate::trash::windows::format_i(5, 7, &[b'C' as u16, b':' as u16]);
        super::recycle_i(&rec);
        super::recycle_i(&rec[..20]);
        super::trashinfo("[Trash Info]\nPath=/a%20b\nDeletionDate=2020-01-01T00:00:00\n%zz%");
        super::desktop("[Desktop Entry]\nName=x\nExec=\"a b\" %f\nMimeType=text/plain;\n");
    }
}
