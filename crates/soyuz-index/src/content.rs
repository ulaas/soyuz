//! Text extraction for content indexing: small text files only (UTF-8, or UTF-16 with a
//! BOM); anything with NUL bytes in its first 8 KiB counts as binary and is skipped.

use std::io::Read;
use std::path::Path;

/// Default extensions whose contents are indexed (`[search.index] content_extensions`).
pub const DEFAULT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rst", "org", "log", "csv", "tsv", "json", "jsonl", "toml", "yaml", "yml", "ini", "cfg",
    "conf", "xml", "html", "htm", "css", "scss", "js", "mjs", "ts", "tsx", "jsx", "rs", "py", "go", "java", "kt", "c",
    "h", "cc", "cpp", "hpp", "cs", "rb", "php", "swift", "sh", "ps1", "bat", "cmd", "sql", "lua", "tex", "srt",
];

/// Read `path` (at most `max` bytes) as text; `None` for binary or unreadable files.
pub fn read_text(path: &Path, max: u64) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).ok()?;
    decode(&buf)
}

/// Decode a file body: UTF-16 LE/BE with BOM, else UTF-8 (lossy). Binary → `None`.
pub fn decode(buf: &[u8]) -> Option<String> {
    if let Some(rest) = buf.strip_prefix(&[0xFF, 0xFE]) {
        let u: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return Some(String::from_utf16_lossy(&u));
    }
    if let Some(rest) = buf.strip_prefix(&[0xFE, 0xFF]) {
        let u: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return Some(String::from_utf16_lossy(&u));
    }
    let head = &buf[..buf.len().min(8192)];
    if head.contains(&0) {
        return None;
    }
    let body = buf.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(buf);
    Some(String::from_utf8_lossy(body).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_text_and_skips_binary() {
        assert_eq!(decode(b"hello").as_deref(), Some("hello"));
        assert_eq!(decode(b"\xEF\xBB\xBFbom").as_deref(), Some("bom"));
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, b'i', 0]).as_deref(), Some("hi"));
        assert_eq!(decode(&[0xFE, 0xFF, 0, b'h', 0, b'i']).as_deref(), Some("hi"));
        assert_eq!(decode(b"\x7FELF\0\0\0"), None);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, "0123456789").unwrap();
        assert_eq!(read_text(&p, 4).as_deref(), Some("0123"), "capped");
        assert_eq!(read_text(&dir.path().join("missing"), 4), None);
    }
}
