//! Text file decoding / encoding for the editor: BOM detection (UTF-8, UTF-16 LE/BE),
//! a lossy UTF-8 fallback that is reported, and line-ending detection so a file is
//! written back with the endings (and final newline) it had.

/// How the file's bytes map to text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Utf8 => "UTF-8",
            Encoding::Utf8Bom => "UTF-8 BOM",
            Encoding::Utf16Le => "UTF-16 LE",
            Encoding::Utf16Be => "UTF-16 BE",
        }
    }
}

/// Line ending written between lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    CrLf,
    Cr,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Cr => "\r",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::CrLf => "CRLF",
            Eol::Cr => "CR",
        }
    }

    /// The platform default for new files.
    pub fn native() -> Eol {
        if cfg!(windows) { Eol::CrLf } else { Eol::Lf }
    }
}

/// A decoded file: `text` uses `\n` only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    pub encoding: Encoding,
    pub eol: Eol,
    /// The file mixed line endings; it is saved with the dominant one (`eol`).
    pub mixed_eol: bool,
    /// Invalid byte sequences were replaced with U+FFFD; saving would write those.
    pub lossy: bool,
}

/// Decode a whole file.
pub fn decode(bytes: &[u8]) -> Decoded {
    let (raw, encoding, lossy) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let (s, lossy) = utf8(rest);
        (s, Encoding::Utf8Bom, lossy)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let (s, lossy) = utf16(rest, false);
        (s, Encoding::Utf16Le, lossy)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (s, lossy) = utf16(rest, true);
        (s, Encoding::Utf16Be, lossy)
    } else {
        let (s, lossy) = utf8(bytes);
        (s, Encoding::Utf8, lossy)
    };
    let (eol, mixed_eol) = detect_eol(&raw);
    Decoded { text: normalize_eol(&raw), encoding, eol, mixed_eol, lossy }
}

fn utf8(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), false),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
    }
}

fn utf16(bytes: &[u8], be: bool) -> (String, bool) {
    let (chunks, rest) = bytes.as_chunks::<2>();
    let units: Vec<u16> =
        chunks.iter().map(|c| if be { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) }).collect();
    match String::from_utf16(&units) {
        Ok(s) => (s, !rest.is_empty()),
        Err(_) => (String::from_utf16_lossy(&units), true),
    }
}

/// The dominant line ending (`Eol::native()` for text without any) and whether the
/// text mixes several kinds.
pub fn detect_eol(text: &str) -> (Eol, bool) {
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [lf, crlf, cr].iter().filter(|&&n| n > 0).count();
    let eol = if lf + crlf + cr == 0 {
        Eol::native()
    } else if crlf >= lf && crlf >= cr {
        Eol::CrLf
    } else if lf >= cr {
        Eol::Lf
    } else {
        Eol::Cr
    };
    (eol, kinds > 1)
}

/// Replace `\r\n` and lone `\r` with `\n`.
pub fn normalize_eol(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Encode `\n`-separated lines with `eol` in `encoding` (BOMs included).
pub fn encode_lines<'a>(lines: impl Iterator<Item = &'a str>, encoding: Encoding, eol: Eol) -> Vec<u8> {
    let mut text = String::new();
    for (i, l) in lines.enumerate() {
        if i > 0 {
            text.push_str(eol.as_str());
        }
        text.push_str(l);
    }
    encode(&text, encoding)
}

/// Encode text (line endings already applied).
pub fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(text.as_bytes());
            v
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let be = encoding == Encoding::Utf16Be;
            let mut v = if be { vec![0xFE, 0xFF] } else { vec![0xFF, 0xFE] };
            for u in text.encode_utf16() {
                v.extend_from_slice(&if be { u.to_be_bytes() } else { u.to_le_bytes() });
            }
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(bytes: &[u8]) -> Vec<u8> {
        let d = decode(bytes);
        encode_lines(d.text.split('\n'), d.encoding, d.eol)
    }

    #[test]
    fn boms_are_detected_and_kept() {
        let d = decode(b"\xEF\xBB\xBFhi\n");
        assert_eq!((d.text.as_str(), d.encoding, d.lossy), ("hi\n", Encoding::Utf8Bom, false));
        let le = [0xFF, 0xFE, b'h', 0, b'\r', 0, b'\n', 0, b'i', 0];
        let d = decode(&le);
        assert_eq!((d.text.as_str(), d.encoding, d.eol), ("h\ni", Encoding::Utf16Le, Eol::CrLf));
        assert_eq!(roundtrip(&le), le);
        let be = [0xFE, 0xFF, 0, b'h', 0, b'i'];
        assert_eq!(decode(&be).encoding, Encoding::Utf16Be);
        assert_eq!(roundtrip(&be), be);
        assert_eq!(roundtrip(b"\xEF\xBB\xBFx\r\ny\r\n"), b"\xEF\xBB\xBFx\r\ny\r\n");
    }

    #[test]
    fn invalid_utf8_is_lossy_and_flagged() {
        let d = decode(&[b'a', 0xFF, b'b']);
        assert!(d.lossy);
        assert_eq!(d.encoding, Encoding::Utf8);
        assert_eq!(d.text, "a\u{FFFD}b");
        assert!(!decode("ünï".as_bytes()).lossy);
        // A dangling half of a UTF-16 unit is reported too.
        assert!(decode(&[0xFF, 0xFE, b'a', 0, b'b']).lossy);
    }

    #[test]
    fn line_endings_are_preserved() {
        for src in [&b"a\nb\n"[..], b"a\r\nb\r\n", b"a\rb", b"no newline", b"", b"\r\n"] {
            assert_eq!(roundtrip(src), src, "{:?}", String::from_utf8_lossy(src));
        }
        let (eol, mixed) = detect_eol("a\r\nb\nc\r\n");
        assert_eq!((eol, mixed), (Eol::CrLf, true));
        let d = decode(b"a\r\nb\nc\r\n");
        assert!(d.mixed_eol);
        assert_eq!(d.text, "a\nb\nc\n");
        // Mixed files are normalized to the dominant ending on save.
        assert_eq!(roundtrip(b"a\r\nb\nc\r\n"), b"a\r\nb\r\nc\r\n");
        assert_eq!(detect_eol("x").0, Eol::native());
    }
}
