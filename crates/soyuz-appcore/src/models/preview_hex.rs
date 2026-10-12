//! The preview's hex view as data: 16 bytes per row (offset · hex in two groups of 8 ·
//! ASCII) over 64 KiB pages that load on demand from the preview worker, so a 200 MB
//! file opens instantly because only the visible pages are ever read. Both frontends
//! draw rows from a [`HexDoc`] (the GUI in `ui::preview::hex`, the terminal UI in
//! `view::preview`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::services::preview::{HEX_PAGE, HexInit, PreviewClient};

pub const BYTES_PER_ROW: u64 = 16;

/// Number of rows needed for `len` bytes (at least one).
pub fn row_count(len: u64) -> u64 {
    len.div_ceil(BYTES_PER_ROW).max(1)
}

/// Width of the offset column in hex digits (8, or more for files > 4 GiB).
pub fn offset_digits(len: u64) -> usize {
    let mut d = 8;
    while d < 16 && len > 1u64 << (d * 4) {
        d += 2;
    }
    d
}

/// Hex part of a row: `"48 65 6C 6C 6F 20 57 6F  72 6C 64 0A"`, padded to full width.
pub fn hex_part(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(16 * 3 + 1);
    for i in 0..BYTES_PER_ROW as usize {
        if i == 8 {
            s.push(' ');
        }
        match bytes.get(i) {
            Some(b) => s.push_str(&format!("{b:02X}")),
            None => s.push_str("  "),
        }
        if i + 1 < BYTES_PER_ROW as usize {
            s.push(' ');
        }
    }
    s
}

/// Printable ASCII with `·` for everything else.
pub fn ascii_part(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '·' }).collect()
}

/// One full row as text (used by tests and "copy row").
pub fn format_row(offset: u64, bytes: &[u8], digits: usize) -> String {
    format!("{offset:0digits$X}  {}  {}", hex_part(bytes), ascii_part(bytes))
}

/// What a row shows while its page is still loading.
pub const PENDING_ROW: &str = "·· ·· ·· ·· ·· ·· ·· ··  ·· ·· ·· ·· ·· ·· ·· ··";

/// Client-side state of an open hex document.
pub struct HexDoc {
    pub doc: u64,
    pub len: u64,
    path: Option<PathBuf>,
    pages: HashMap<u64, Arc<[u8]>>,
    requested: HashSet<u64>,
}

impl HexDoc {
    pub fn new(init: &HexInit) -> Self {
        HexDoc {
            doc: init.doc,
            len: init.len,
            path: init.path.clone(),
            pages: init.pages.clone(),
            requested: init.pages.keys().copied().collect(),
        }
    }

    pub fn on_page(&mut self, doc: u64, page: u64, bytes: Option<Arc<[u8]>>) {
        if doc == self.doc {
            self.pages.insert(page, bytes.unwrap_or_else(|| Arc::from(Vec::new())));
        }
    }

    /// Bytes of row `row`, requesting its page from `client` when missing.
    pub fn row(&mut self, row: u64, client: &PreviewClient) -> Option<Vec<u8>> {
        let start = row * BYTES_PER_ROW;
        let page = start / HEX_PAGE;
        let Some(data) = self.pages.get(&page) else {
            if let Some(p) = &self.path
                && self.requested.insert(page)
            {
                client.page(self.doc, p, page);
            }
            return None;
        };
        let off = (start - page * HEX_PAGE) as usize;
        let end = (off + BYTES_PER_ROW as usize).min(data.len());
        Some(data.get(off..end).map(<[u8]>::to_vec).unwrap_or_default())
    }

    /// Drop pages far from the rows `first_row..last_row` to bound memory on huge files.
    pub fn trim_rows(&mut self, first_row: u64, last_row: u64) {
        self.trim(first_row * BYTES_PER_ROW / HEX_PAGE, last_row * BYTES_PER_ROW / HEX_PAGE);
    }

    /// Drop pages far from `keep_from..=keep_to` to bound memory on huge files.
    pub fn trim(&mut self, keep_from: u64, keep_to: u64) {
        if self.path.is_none() || self.pages.len() < 64 {
            return;
        }
        self.pages.retain(|&p, _| p + 8 >= keep_from && p <= keep_to + 8);
        let pages = &self.pages;
        self.requested.retain(|p| pages.contains_key(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_digits() {
        assert_eq!(row_count(0), 1);
        assert_eq!(row_count(16), 1);
        assert_eq!(row_count(17), 2);
        assert_eq!(offset_digits(1000), 8);
        assert_eq!(offset_digits(0xFFFF_FFFF), 8);
        assert_eq!(offset_digits(0x1_0000_0001), 10);
    }

    #[test]
    fn row_formatting() {
        let r = format_row(0x20, b"Hello, World!\n\x00\xFF", 8);
        assert_eq!(r, "00000020  48 65 6C 6C 6F 2C 20 57  6F 72 6C 64 21 0A 00 FF  Hello, World!···");
        // Short rows keep the ASCII column aligned.
        let short = format_row(0, b"ab", 8);
        assert_eq!(short.chars().count(), format_row(0, &[0u8; 16], 8).chars().count() - 14);
        assert!(hex_part(b"ab").starts_with("61 62 "));
        assert_eq!(hex_part(&[]).len(), hex_part(&[0; 16]).len());
        assert_eq!(PENDING_ROW.chars().count(), hex_part(&[0; 16]).chars().count());
    }

    #[test]
    fn doc_pages_resolve_rows() {
        let init = HexInit {
            doc: 1,
            len: 40,
            path: None,
            pages: HashMap::from([(0u64, Arc::<[u8]>::from((0u8..40).collect::<Vec<_>>()))]),
        };
        let mut d = HexDoc::new(&init);
        let c = PreviewClient::new(crate::host::no_wake(), crate::services::preview_gfx::no_upload());
        assert_eq!(d.row(0, &c).unwrap(), (0u8..16).collect::<Vec<_>>());
        assert_eq!(d.row(2, &c).unwrap(), (32u8..40).collect::<Vec<_>>());
        d.on_page(2, 0, None);
        assert_eq!(d.row(1, &c).unwrap().len(), 16, "pages of other docs are ignored");
    }
}
