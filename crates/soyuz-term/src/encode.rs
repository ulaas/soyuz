//! Toolkit-free input encoding: paste framing and focus reports.

/// Bytes for a paste: newlines become CR; bracketed mode wraps the text and strips ESC so
/// the paste cannot terminate the bracket early.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend(normalized.replace('\x1b', "").into_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

/// Focus in/out report (`FOCUS_IN_OUT` mode).
pub fn focus_report(focused: bool) -> &'static [u8] {
    if focused { b"\x1b[I" } else { b"\x1b[O" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_framing() {
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
        assert_eq!(focus_report(true), b"\x1b[I");
    }
}
