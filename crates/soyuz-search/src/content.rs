//! Content matching (regex / literal, binary detection, size cap) (M6).
//!
//! Files are read whole (they are capped by `SearchQuery::content_max_size`, 8 MiB by
//! default) into a per-thread buffer. A NUL byte in the first 8 KiB marks a file as
//! binary and skips it. Case-sensitive literals use `memchr::memmem`; everything else
//! goes through `regex::bytes` (which has its own memchr/teddy prefilters). Each matching
//! line is reported once (first match column), at most [`MAX_MATCHES_PER_FILE`] per file.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use memchr::memmem;
use soyuz_core::PatternError;

use crate::ContentMatch;
use crate::query::ContentPatternSpec;

/// Bytes inspected for the NUL-byte binary heuristic.
pub const BINARY_SNIFF: usize = 8 * 1024;
/// Matching lines reported per file.
pub const MAX_MATCHES_PER_FILE: usize = 100;
/// Longest `ContentMatch::text` (bytes, cut on a char boundary).
pub const MAX_LINE_TEXT: usize = 512;

/// Compiled content pattern (`Send + Sync`).
#[derive(Debug, Clone)]
pub enum ContentMatcher {
    Literal(Box<memmem::Finder<'static>>),
    Regex(regex::bytes::Regex),
}

impl ContentMatcher {
    pub fn new(spec: &ContentPatternSpec) -> Result<Self, PatternError> {
        if !spec.regex && spec.case_sensitive {
            return Ok(ContentMatcher::Literal(Box::new(memmem::Finder::new(spec.text.as_bytes()).into_owned())));
        }
        let pat = if spec.regex { spec.text.clone() } else { regex::escape(&spec.text) };
        regex::bytes::RegexBuilder::new(&pat)
            .case_insensitive(!spec.case_sensitive)
            .multi_line(true)
            .crlf(true)
            .build()
            .map(ContentMatcher::Regex)
            .map_err(|e| PatternError::Regex(e.to_string()))
    }

    /// Next match at or after `from`: `(start, end)`.
    fn find_at(&self, hay: &[u8], from: usize) -> Option<(usize, usize)> {
        match self {
            ContentMatcher::Literal(f) => f.find(&hay[from..]).map(|i| (from + i, from + i + f.needle().len())),
            ContentMatcher::Regex(r) => r.find_at(hay, from).map(|m| (m.start(), m.end())),
        }
    }

    /// All matching lines of `hay` (one entry per line, first match column).
    pub fn search(&self, hay: &[u8]) -> Vec<ContentMatch> {
        let mut out = Vec::new();
        let mut from = 0;
        // `line` is the 1-based number of the line starting at offset `line_start`.
        let mut line = 1u64;
        let mut line_start = 0usize;
        while out.len() < MAX_MATCHES_PER_FILE {
            let Some((start, _end)) = self.find_at(hay, from) else { break };
            let gap = &hay[line_start..start];
            line += memchr::memchr_iter(b'\n', gap).count() as u64;
            let ls = memchr::memrchr(b'\n', gap).map_or(line_start, |i| line_start + i + 1);
            let le = memchr::memchr(b'\n', &hay[start..]).map_or(hay.len(), |i| start + i);
            out.push(ContentMatch { line, col: start - ls, text: line_text(&hay[ls..le]) });
            if le >= hay.len() {
                break;
            }
            // One report per line: continue at the next line.
            line += 1;
            line_start = le + 1;
            from = line_start;
        }
        out
    }
}

/// NUL byte in the first [`BINARY_SNIFF`] bytes.
pub fn is_binary(buf: &[u8]) -> bool {
    memchr::memchr(0, &buf[..buf.len().min(BINARY_SNIFF)]).is_some()
}

fn line_text(raw: &[u8]) -> String {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let raw = &raw[..raw.len().min(MAX_LINE_TEXT * 2)];
    let mut s = String::from_utf8_lossy(raw).into_owned();
    if s.len() > MAX_LINE_TEXT {
        let mut cut = MAX_LINE_TEXT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// Read `path` (at most `max` bytes, the caller already checked the size) into `buf` and
/// search it. `Ok(None)` = binary file.
pub(crate) fn search_file(
    m: &ContentMatcher,
    path: &Path,
    max: u64,
    buf: &mut Vec<u8>,
) -> std::io::Result<Option<Vec<ContentMatch>>> {
    buf.clear();
    let f = File::open(path)?;
    f.take(max).read_to_end(buf)?;
    if is_binary(buf) {
        return Ok(None);
    }
    Ok(Some(m.search(buf)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(regex: bool, text: &str, cs: bool) -> ContentMatcher {
        ContentMatcher::new(&ContentPatternSpec { regex, text: text.into(), case_sensitive: cs }).unwrap()
    }

    #[test]
    fn literal_lines_and_columns() {
        let hay = b"alpha\r\nbeta needle\ngamma\nneedle needle\n";
        let m = spec(false, "needle", true).search(hay);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].line, m[0].col, m[0].text.as_str()), (2, 5, "beta needle"));
        assert_eq!((m[1].line, m[1].col, m[1].text.as_str()), (4, 0, "needle needle"));
    }

    #[test]
    fn case_insensitive_and_regex() {
        let hay = b"Foo\nbar FOO\nbaz";
        assert_eq!(spec(false, "foo", false).search(hay).len(), 2);
        assert_eq!(spec(false, "foo", true).search(hay).len(), 0);
        let m = spec(true, r"^ba\w$", true).search(hay);
        assert_eq!(m.iter().map(|m| m.line).collect::<Vec<_>>(), [3]);
        let m = spec(true, r"o$", false).search(b"foo\r\nbar\r\nzoo");
        assert_eq!(m.iter().map(|m| m.line).collect::<Vec<_>>(), [1, 3]);
    }

    #[test]
    fn empty_matches_terminate() {
        let m = spec(true, r"x*", true).search(b"a\nb\n");
        assert_eq!(m.len(), 3);
        assert_eq!(m[2].line, 3);
    }

    #[test]
    fn binary_and_truncation() {
        assert!(is_binary(b"ab\0cd"));
        assert!(!is_binary(b"plain text"));
        let long = "é".repeat(600);
        let t = line_text(long.as_bytes());
        assert!(t.len() <= MAX_LINE_TEXT + '…'.len_utf8());
        assert!(t.ends_with('…'));
    }
}
