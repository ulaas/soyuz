//! The hex editor: a virtualized 16-bytes-per-row view over any file size (64 KiB pages
//! read on a worker thread), overwrite editing in the hex or text column, a data
//! inspector, go to offset and find bytes / text. Saving writes the patched bytes to a
//! temp file and renames it over the original. This is the document model (pages,
//! patches, cursor, bars); `ui::editors::hex` draws it.

pub mod io;
pub mod model;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Receiver;
use soyuz_core::config::KeyName;

use io::{PAGE, Pager, SearchReply};
use model::Patches;

use super::EditInput;
use crate::services::mailbox::Wake;

/// Bytes per row.
pub const ROW: u64 = 16;
/// Pages kept in memory.
const MAX_PAGES: usize = 256;

pub struct FindBar {
    pub text: String,
    /// The needle is hex bytes (else text).
    pub hex: bool,
    pub status: Option<String>,
    /// The last search found a match (the status is shown in green).
    pub found: bool,
    /// The frontend should focus the field.
    pub focus: bool,
}

pub struct HexDoc {
    pub path: PathBuf,
    pub len: u64,
    generation: u64,
    pager: Pager,
    wake: Wake,
    pages: HashMap<u64, Arc<[u8]>>,
    requested: HashSet<u64>,
    /// A page read failed.
    pub error: Option<String>,
    pub patches: Patches,
    pub cursor: u64,
    /// 1 after the high nibble of the cursor byte was typed.
    pub nibble: u8,
    /// The cursor is in the text column (else the hex column).
    pub text_col: bool,
    /// Top row (fractional).
    pub scroll_row: f64,
    pub scroll_to_cursor: bool,
    pub read_only: bool,
    /// Opened read-only because the file is too large for the text editor.
    pub too_large_for_text: bool,
    /// The inspector reads multi-byte values big-endian.
    pub big_endian: bool,
    /// The go-to field's text while that bar is open.
    pub goto: Option<String>,
    /// The frontend should focus the go-to field.
    pub goto_focus: bool,
    pub find: Option<FindBar>,
    search: Option<(Receiver<SearchReply>, Arc<AtomicBool>, bool)>,
    /// The frontend should give the byte view keyboard focus.
    pub focus_request: bool,
    /// Rows a Page Up / Down moves (set by the view from its height).
    pub page_rows: u64,
}

impl HexDoc {
    pub fn new(path: PathBuf, len: u64, wake: Wake) -> Self {
        HexDoc {
            pager: Pager::new(path.clone(), wake.clone()),
            path,
            len,
            generation: 1,
            wake,
            pages: HashMap::new(),
            requested: HashSet::new(),
            error: None,
            patches: Patches::default(),
            cursor: 0,
            nibble: 0,
            text_col: false,
            scroll_row: 0.0,
            scroll_to_cursor: false,
            read_only: false,
            too_large_for_text: false,
            big_endian: false,
            goto: None,
            goto_focus: false,
            find: None,
            search: None,
            focus_request: true,
            page_rows: 16,
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.patches.is_empty()
    }

    /// The file changed on disk (or was saved): drop cached pages.
    pub fn reload(&mut self, len: u64) {
        self.len = len;
        self.generation += 1;
        self.pages.clear();
        self.requested.clear();
        self.error = None;
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }

    pub fn undo(&mut self) {
        if let Some(off) = self.patches.undo() {
            self.cursor = off;
            self.scroll_to_cursor = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(off) = self.patches.redo() {
            self.cursor = off;
            self.scroll_to_cursor = true;
        }
    }

    pub fn open_goto(&mut self) {
        self.goto = Some(String::new());
        self.goto_focus = true;
        self.find = None;
    }

    pub fn open_find(&mut self) {
        let hex = self.find.as_ref().is_none_or(|f| f.hex);
        let text = self.find.take().map(|f| f.text).unwrap_or_default();
        self.find = Some(FindBar { text, hex, status: None, found: false, focus: true });
        self.goto = None;
    }

    pub fn close_bars(&mut self) -> bool {
        let any = self.find.is_some() || self.goto.is_some();
        self.find = None;
        self.goto = None;
        if let Some((_, cancel, _)) = self.search.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if any {
            self.focus_request = true;
        }
        any
    }

    pub fn find_step(&mut self, forward: bool) {
        let Some(f) = self.find.as_mut() else {
            self.open_find();
            return;
        };
        match model::parse_needle(&f.text, f.hex) {
            Ok(needle) => {
                if let Some((_, cancel, _)) = self.search.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                let cancel = Arc::new(AtomicBool::new(false));
                let rx = io::spawn_search(
                    self.path.clone(),
                    self.len,
                    self.patches.map().clone(),
                    needle,
                    self.cursor,
                    forward,
                    cancel.clone(),
                    self.wake.clone(),
                );
                f.status = Some(tr!("editors-hex-searching").into());
                f.found = false;
                self.search = Some((rx, cancel, forward));
            }
            Err(e) => {
                f.status = Some(e);
                f.found = false;
            }
        }
    }

    pub fn go_to(&mut self, off: u64) {
        self.cursor = off.min(self.len.saturating_sub(1));
        self.nibble = 0;
        self.scroll_to_cursor = true;
    }

    /// `SOYUZ_SCRIPT editor:hex=<offset>=<byte>`.
    pub fn devscript_set(&mut self, off: u64, value: u8) {
        self.go_to(off);
        self.set_byte(off, value);
    }

    /// The (patched) byte at `off`, if its page is loaded.
    pub fn byte(&self, off: u64) -> Option<u8> {
        if off >= self.len {
            return None;
        }
        if let Some(b) = self.patches.get(off) {
            return Some(b);
        }
        self.original(off)
    }

    fn original(&self, off: u64) -> Option<u8> {
        self.pages.get(&(off / PAGE)).and_then(|p| p.get((off % PAGE) as usize).copied())
    }

    /// Apply page reads and search results (call every frame before drawing).
    pub fn pump(&mut self) {
        while let Ok(r) = self.pager.rx.try_recv() {
            if r.generation != self.generation {
                continue;
            }
            match r.bytes {
                Ok(b) => {
                    self.pages.insert(r.page, b);
                }
                Err(e) => self.error = Some(e),
            }
        }
        if let Some((rx, _, _)) = &self.search
            && let Ok(res) = rx.try_recv()
        {
            self.search = None;
            let found = matches!(res, Ok(Some(_)));
            let status = match res {
                Ok(Some(off)) => {
                    self.go_to(off);
                    self.text_col = self.find.as_ref().is_some_and(|f| !f.hex);
                    tr!("editors-hex-found", offset = format!("{off:X}"))
                }
                Ok(None) => tr!("editors-hex-not-found").into(),
                Err(e) => e,
            };
            if let Some(f) = self.find.as_mut() {
                f.status = Some(status);
                f.found = found;
            }
        }
    }

    /// Request the pages of bytes `first..=last` (the visible rows).
    pub fn ensure_pages(&mut self, first: u64, last: u64) {
        for page in first / PAGE..=last / PAGE {
            if self.requested.insert(page) {
                self.pager.request(self.generation, page);
            }
        }
        if self.pages.len() > MAX_PAGES {
            let keep = first / PAGE;
            let mut far: Vec<u64> = self.pages.keys().copied().collect();
            far.sort_by_key(|p| std::cmp::Reverse(p.abs_diff(keep)));
            for p in far.into_iter().take(self.pages.len() - MAX_PAGES / 2) {
                self.pages.remove(&p);
                self.requested.remove(&p);
            }
        }
    }

    fn set_byte(&mut self, off: u64, value: u8) {
        if self.read_only {
            return;
        }
        if let Some(orig) = self.original(off) {
            self.patches.set(off, orig, value);
        }
    }

    /// One input event while the byte view has keyboard focus; returns text the
    /// frontend puts on the clipboard (copy).
    pub fn input(&mut self, ev: EditInput) -> Option<String> {
        let rows = self.page_rows.max(1);
        match ev {
            EditInput::Text(t) => {
                for c in t.chars() {
                    if self.cursor >= self.len {
                        break;
                    }
                    if self.text_col {
                        let mut b = [0u8; 4];
                        for &byte in c.encode_utf8(&mut b).as_bytes() {
                            if self.cursor < self.len {
                                self.set_byte(self.cursor, byte);
                                self.cursor = (self.cursor + 1).min(self.len - 1);
                            }
                        }
                    } else if let Some(d) = c.to_digit(16) {
                        let Some(cur) = self.byte(self.cursor) else { continue };
                        let d = d as u8;
                        if self.nibble == 0 {
                            self.set_byte(self.cursor, (cur & 0x0F) | (d << 4));
                            self.nibble = 1;
                        } else {
                            self.set_byte(self.cursor, (cur & 0xF0) | d);
                            self.nibble = 0;
                            self.cursor = (self.cursor + 1).min(self.len.saturating_sub(1));
                        }
                    }
                    self.scroll_to_cursor = true;
                }
            }
            EditInput::Copy => {
                if let Some(b) = self.byte(self.cursor) {
                    return Some(format!("{b:02X}"));
                }
            }
            EditInput::Key(Some(key), m) => {
                let last = self.len.saturating_sub(1);
                let c = self.cursor;
                let moved = match key {
                    KeyName::ArrowLeft => Some(c.saturating_sub(1)),
                    KeyName::ArrowRight => Some((c + 1).min(last)),
                    KeyName::ArrowUp => Some(c.saturating_sub(ROW)),
                    KeyName::ArrowDown => Some(if c + ROW <= last { c + ROW } else { c }),
                    KeyName::PageUp => Some(c.saturating_sub(ROW * rows)),
                    KeyName::PageDown => Some((c + ROW * rows).min(last)),
                    KeyName::Home if m.command => Some(0),
                    KeyName::End if m.command => Some(last),
                    KeyName::Home => Some(c - c % ROW),
                    KeyName::End => Some((c - c % ROW + ROW - 1).min(last)),
                    KeyName::Tab => {
                        self.text_col = !self.text_col;
                        self.nibble = 0;
                        None
                    }
                    KeyName::Z if m.command && m.shift => {
                        self.redo();
                        None
                    }
                    KeyName::Z if m.command => {
                        self.undo();
                        None
                    }
                    KeyName::Y if m.command => {
                        self.redo();
                        None
                    }
                    _ => None,
                };
                if let Some(to) = moved {
                    self.cursor = to;
                    self.nibble = 0;
                    self.scroll_to_cursor = true;
                }
            }
            EditInput::Key(None, _) | EditInput::ImeCommit(_) | EditInput::Cut | EditInput::Paste(_) => {}
        }
        None
    }

    // ---- view support (the frontend draws; these keep the logic toolkit-free) ----

    /// The "Find" toolbar button.
    pub fn toggle_find(&mut self) {
        if self.find.is_some() {
            self.find = None;
        } else {
            self.open_find();
        }
    }

    /// The "Go to offset" toolbar button.
    pub fn toggle_goto(&mut self) {
        if self.goto.is_some() {
            self.goto = None;
        } else {
            self.open_goto();
        }
    }

    /// Enter in the go-to field: jump, or clear a field that is not an offset.
    pub fn submit_goto(&mut self) {
        let Some(text) = self.goto.as_mut() else { return };
        match model::parse_offset(text, self.cursor) {
            Some(off) => {
                self.goto = None;
                self.go_to(off);
                self.focus_request = true;
            }
            None => text.clear(),
        }
    }

    /// The HEX / TXT chip of the find bar.
    pub fn toggle_find_mode(&mut self) {
        if let Some(f) = self.find.as_mut() {
            f.hex = !f.hex;
            f.status = None;
            f.found = false;
        }
    }

    /// A click (or drag) on byte `col` of row `row`, in the text or the hex column.
    pub fn click(&mut self, row: u64, col: u64, text: bool) {
        self.cursor = (row * ROW + col).min(self.len.saturating_sub(1));
        self.nibble = 0;
        self.text_col = text;
    }

    /// Rows in the file (at least one).
    pub fn total_rows(&self) -> u64 {
        self.len.div_ceil(ROW).max(1)
    }

    /// Scroll so the cursor row is visible when a move asked for it (`visible` rows fit).
    pub fn scroll_cursor_into_view(&mut self, visible: f64) {
        let cur_row = (self.cursor / ROW) as f64;
        if std::mem::take(&mut self.scroll_to_cursor) {
            if cur_row < self.scroll_row {
                self.scroll_row = cur_row;
            } else if cur_row > self.scroll_row + visible - 2.0 {
                self.scroll_row = cur_row - visible + 2.0;
            }
        }
    }

    /// The inspector rows for the bytes at the cursor (`(label, value)`).
    pub fn inspect(&self) -> Vec<(&'static str, String)> {
        let bytes: Vec<u8> = (self.cursor..(self.cursor + 8).min(self.len)).map_while(|o| self.byte(o)).collect();
        model::inspect(&bytes, self.big_endian)
            .into_iter()
            .map(|(label, v)| (if label == "binary" { tr!("editors-hex-binary") } else { label }, v))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editors::KeyMods;

    fn loaded(bytes: &[u8]) -> (tempfile::TempDir, HexDoc) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, bytes).unwrap();
        let mut doc = HexDoc::new(path, bytes.len() as u64, Arc::new(|| {}));
        doc.ensure_pages(0, bytes.len() as u64 - 1);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.byte(0).is_none() && std::time::Instant::now() < until {
            doc.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        (dir, doc)
    }

    #[test]
    fn input_overwrites_moves_and_copies() {
        let (_dir, mut doc) = loaded(&[0u8; 40]);
        assert_eq!(doc.byte(0), Some(0));
        assert_eq!(doc.input(EditInput::Text("aB".into())), None);
        assert_eq!(doc.byte(0), Some(0xAB), "two hex digits make one byte");
        assert_eq!(doc.cursor, 1);
        doc.input(EditInput::Key(Some(KeyName::ArrowDown), KeyMods::default()));
        assert_eq!(doc.cursor, 1 + ROW);
        let cmd = KeyMods { command: true, ..Default::default() };
        doc.input(EditInput::Key(Some(KeyName::Home), cmd));
        assert_eq!(doc.input(EditInput::Copy), Some("AB".into()));
        doc.input(EditInput::Key(Some(KeyName::Z), cmd));
        assert_eq!(doc.byte(0), Some(0xA0), "undo steps by nibble");
        doc.input(EditInput::Key(Some(KeyName::Z), cmd));
        assert_eq!(doc.byte(0), Some(0));
        doc.input(EditInput::Key(Some(KeyName::Tab), KeyMods::default()));
        assert!(doc.text_col);
        doc.input(EditInput::Text("z".into()));
        assert_eq!(doc.byte(0), Some(b'z'), "the text column types characters");
    }

    #[test]
    fn goto_bar_accepts_offsets_only() {
        let (_dir, mut doc) = loaded(&[1u8; 64]);
        doc.toggle_goto();
        doc.goto = Some("0x20".into());
        doc.submit_goto();
        assert_eq!((doc.cursor, doc.goto.is_none()), (0x20, true));
        doc.toggle_goto();
        doc.goto = Some("nope".into());
        doc.submit_goto();
        assert_eq!(doc.goto.as_deref(), Some(""), "a bad offset clears the field");
        doc.click(2, 3, true);
        assert_eq!((doc.cursor, doc.text_col), (2 * ROW + 3, true));
    }
}
