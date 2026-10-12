//! The text editor: a virtualized code view over [`buffer::Buffer`] with incremental
//! syntax highlighting, line numbers, soft wrap, visible whitespace, find / replace with
//! match highlights, go to line and multi-level undo. Only the visible rows are laid
//! out, so multi-megabyte files stay fast. This is the document model (buffer, view
//! state, editing commands); `ui::editors::text` draws it.

pub mod buffer;
pub mod encoding;
pub mod highlight;
pub mod search;

use std::time::{Duration, Instant};

use soyuz_core::config::KeyName;

use buffer::{Buffer, EditKind, History, Pos, Sel, apply_all};
use encoding::{Decoded, Encoding, Eol};
use highlight::Highlighter;
use search::{Match, Query};

use super::{EditInput, KeyMods};

/// Lines longer than this are split into rows even without soft wrap (bounded layout).
pub const HARD_WRAP: usize = 10_000;
/// Per-frame time for syntax highlighting.
pub const HL_BUDGET: Duration = Duration::from_millis(6);

#[derive(Default)]
pub struct FindState {
    pub open: bool,
    pub replace: bool,
    pub query: Query,
    pub replace_text: String,
    /// `(query, buffer version)` the matches belong to.
    key: Option<(Query, u64)>,
    pub error: Option<String>,
    regex: Option<regex::Regex>,
    pub matches: Vec<Match>,
    pub current: Option<usize>,
    /// The frontend should focus (and select) the find field.
    pub focus_find: bool,
}

/// Visual rows per line (soft wrap / hard wrap) and their prefix sums.
#[derive(Default)]
pub struct Rows {
    pub cols: usize,
    pub tab: usize,
    widths: Vec<u32>,
    prefix: Vec<usize>,
    pub max_width: u32,
    dirty: bool,
}

impl Rows {
    pub fn rebuild(&mut self, buf: &Buffer, cols: usize, tab: usize) {
        if cols != self.cols || tab != self.tab || self.widths.len() != buf.line_count() {
            self.cols = cols;
            self.tab = tab;
            self.widths = buf.lines().iter().map(|l| disp_width(l, tab) as u32).collect();
            self.dirty = true;
        }
        if self.dirty {
            self.prefix.clear();
            self.prefix.reserve(self.widths.len() + 1);
            let mut acc = 0usize;
            let mut max = 0u32;
            for &w in &self.widths {
                self.prefix.push(acc);
                acc += (w as usize).div_ceil(cols).max(1);
                max = max.max(w);
            }
            self.prefix.push(acc);
            self.max_width = max;
            self.dirty = false;
        }
    }

    pub fn apply(&mut self, buf: &Buffer, changes: &[buffer::LineChange]) {
        if self.widths.is_empty() {
            return;
        }
        for c in changes {
            let end = (c.start + c.removed).min(self.widths.len());
            let new: Vec<u32> =
                (c.start..c.start + c.inserted).map(|l| disp_width(buf.line(l), self.tab) as u32).collect();
            self.widths.splice(c.start.min(end)..end, new);
        }
        self.dirty = true;
    }

    pub fn total(&self) -> usize {
        self.prefix.last().copied().unwrap_or(1)
    }

    pub fn rows_of(&self, line: usize) -> usize {
        self.prefix
            .get(line + 1)
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.prefix.get(line).copied().unwrap_or(0))
            .max(1)
    }

    pub fn row_of(&self, line: usize) -> usize {
        self.prefix.get(line).copied().unwrap_or(0)
    }

    /// `(line, sub-row)` of a visual row.
    pub fn line_of(&self, row: usize) -> (usize, usize) {
        let n = self.prefix.len().saturating_sub(1);
        if n == 0 {
            return (0, 0);
        }
        let line = self.prefix.partition_point(|&s| s <= row).saturating_sub(1).min(n - 1);
        (line, row - self.prefix[line])
    }
}

/// Display width of a line (tabs to the next stop).
pub fn disp_width(line: &str, tab: usize) -> usize {
    if !line.contains('\t') {
        return if line.is_ascii() { line.len() } else { line.chars().count() };
    }
    let mut w = 0;
    for c in line.chars() {
        w = if c == '\t' { (w / tab + 1) * tab } else { w + 1 };
    }
    w
}

/// Display cells of one line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Char,
    Space,
    TabStart,
    TabRest,
    Control,
}

pub struct LineMap {
    pub disp: Vec<char>,
    pub cells: Vec<Cell>,
    /// Byte offset of every source char, plus the line length.
    char_bytes: Vec<usize>,
    /// Display index of every source char, plus the display length.
    char_disp: Vec<usize>,
}

impl LineMap {
    pub fn new(line: &str, tab: usize) -> Self {
        let mut m = LineMap {
            disp: Vec::with_capacity(line.len()),
            cells: Vec::with_capacity(line.len()),
            char_bytes: Vec::with_capacity(line.len() + 1),
            char_disp: Vec::with_capacity(line.len() + 1),
        };
        for (b, c) in line.char_indices() {
            m.char_bytes.push(b);
            m.char_disp.push(m.disp.len());
            match c {
                '\t' => {
                    let n = tab - m.disp.len() % tab;
                    for i in 0..n {
                        m.disp.push(' ');
                        m.cells.push(if i == 0 { Cell::TabStart } else { Cell::TabRest });
                    }
                }
                ' ' => {
                    m.disp.push(' ');
                    m.cells.push(Cell::Space);
                }
                c if c.is_control() => {
                    m.disp.push('¤');
                    m.cells.push(Cell::Control);
                }
                c => {
                    m.disp.push(c);
                    m.cells.push(Cell::Char);
                }
            }
        }
        m.char_bytes.push(line.len());
        m.char_disp.push(m.disp.len());
        m
    }

    pub fn disp_of(&self, col: usize) -> usize {
        let ci = self.char_bytes.partition_point(|&b| b < col);
        self.char_disp[ci.min(self.char_disp.len() - 1)]
    }

    pub fn byte_of(&self, d: usize) -> usize {
        let n = self.char_disp.len() - 1;
        let ci = self.char_disp.partition_point(|&x| x <= d).saturating_sub(1).min(n);
        if ci >= n {
            return self.char_bytes[n];
        }
        let (s, e) = (self.char_disp[ci], self.char_disp[ci + 1]);
        if e - s > 1 && d - s > (e - s) / 2 { self.char_bytes[ci + 1] } else { self.char_bytes[ci] }
    }
}

pub struct TextDoc {
    pub buf: Buffer,
    pub history: History,
    pub sel: Sel,
    pub encoding: Encoding,
    pub eol: Eol,
    pub saved_eol: Eol,
    pub mixed_eol: bool,
    pub lossy: bool,
    pub read_only: bool,
    /// Opened read-only on purpose (larger than the limit etc.).
    pub hl: Highlighter,
    pub rows: Rows,
    pub find: FindState,
    /// The go-to-line field's text while that bar is open.
    pub goto: Option<String>,
    /// The frontend should focus the go-to field.
    pub goto_focus: bool,
    /// Top visual row (fractional).
    pub scroll_row: f64,
    /// Horizontal scroll in pixels (no soft wrap).
    pub scroll_x: f32,
    pub scroll_to_caret: bool,
    pub center_caret: bool,
    /// Wanted display column for vertical motion.
    pub want_col: Option<usize>,
    /// The frontend should give the text area keyboard focus.
    pub focus_request: bool,
    pub uses_tabs: bool,
    /// A pointer selection drag is in progress.
    pub dragging: bool,
    /// Rows a Page Up / Down moves (set by the view from its height).
    pub page_rows: usize,
    pub notice_dismissed: bool,
}

impl TextDoc {
    pub fn new(name: &str, d: Decoded) -> Self {
        let buf = Buffer::new(&d.text);
        let uses_tabs = buf.uses_tabs();
        TextDoc {
            buf,
            history: History::default(),
            sel: Sel::default(),
            encoding: d.encoding,
            eol: d.eol,
            saved_eol: d.eol,
            mixed_eol: d.mixed_eol,
            lossy: d.lossy,
            read_only: d.lossy,
            hl: Highlighter::new(name),
            rows: Rows::default(),
            find: FindState::default(),
            goto: None,
            goto_focus: false,
            scroll_row: 0.0,
            scroll_x: 0.0,
            scroll_to_caret: false,
            center_caret: false,
            want_col: None,
            focus_request: true,
            uses_tabs,
            dragging: false,
            page_rows: 20,
            notice_dismissed: false,
        }
    }

    /// Take the contents of a fresh load from disk (external change), keeping the view.
    pub fn reload_from(&mut self, fresh: TextDoc) {
        let caret = self.sel.head;
        self.buf = fresh.buf;
        self.history.clear();
        self.encoding = fresh.encoding;
        self.eol = fresh.eol;
        self.saved_eol = fresh.eol;
        self.mixed_eol = fresh.mixed_eol;
        self.lossy = fresh.lossy;
        self.read_only = fresh.read_only;
        self.hl.reset();
        self.rows = Rows::default();
        self.find.key = None;
        self.sel = Sel::caret(self.buf.clamp(caret));
        self.uses_tabs = fresh.uses_tabs;
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty() || self.eol != self.saved_eol
    }

    pub fn language(&self) -> Option<&str> {
        self.hl.language()
    }

    /// Bytes to write plus the token to pass to [`Self::saved`] when the write succeeded.
    pub fn save_snapshot(&mut self) -> (Vec<u8>, (u64, Eol)) {
        let id = self.history.seal();
        let bytes = encoding::encode_lines(self.buf.lines().iter().map(String::as_str), self.encoding, self.eol);
        (bytes, (id, self.eol))
    }

    pub fn saved(&mut self, token: (u64, Eol)) {
        self.history.set_saved(token.0);
        self.saved_eol = token.1;
        self.mixed_eol = false;
        self.lossy = false;
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.history.undo(&mut self.buf) {
            self.sel = clamp_sel(&self.buf, s);
            self.scroll_to_caret = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.history.redo(&mut self.buf) {
            self.sel = clamp_sel(&self.buf, s);
            self.scroll_to_caret = true;
        }
    }

    pub fn open_find(&mut self, replace: bool) {
        self.find.open = true;
        self.find.replace = replace || (self.find.replace && self.find.open);
        self.find.focus_find = true;
        self.goto = None;
        let (a, b) = self.sel.range();
        if !self.sel.is_empty() && a.line == b.line {
            self.find.query.text = self.buf.text_range(a, b);
        }
    }

    pub fn open_goto(&mut self) {
        self.goto = Some(String::new());
        self.goto_focus = true;
        self.find.open = false;
    }

    /// Esc: close the find / go-to bar; returns `false` when nothing was open.
    pub fn close_bars(&mut self) -> bool {
        let any = self.find.open || self.goto.is_some();
        self.find.open = false;
        self.goto = None;
        if any {
            self.focus_request = true;
        }
        any
    }

    pub fn find_step(&mut self, forward: bool) {
        self.refresh_matches();
        let after = if forward { self.sel.range().1 } else { self.sel.range().0 };
        let i = if forward {
            search::next_from(&self.find.matches, after)
        } else {
            search::prev_from(&self.find.matches, after)
        };
        if let Some(i) = i {
            self.select_match(i);
        }
    }

    pub fn select_match(&mut self, i: usize) {
        if let Some(m) = self.find.matches.get(i) {
            self.sel = Sel::new(m.start_pos(), m.end_pos());
            self.find.current = Some(i);
            self.scroll_to_caret = true;
            self.center_caret = true;
        }
    }

    pub fn refresh_matches(&mut self) {
        let key = (self.find.query.clone(), self.buf.version());
        if self.find.key.as_ref() == Some(&key) {
            return;
        }
        let q_changed = self.find.key.as_ref().is_none_or(|k| k.0 != key.0);
        self.find.key = Some(key);
        if q_changed {
            match search::compile(&self.find.query) {
                Ok(re) => {
                    self.find.regex = re;
                    self.find.error = None;
                }
                Err(e) => {
                    self.find.regex = None;
                    self.find.error = Some(e);
                }
            }
        }
        self.find.matches = match &self.find.regex {
            Some(re) => search::find_all(self.buf.lines(), re, search::MAX_MATCHES),
            None => Vec::new(),
        };
        self.find.current = self.find.matches.iter().position(|m| m.start_pos() == self.sel.range().0);
    }

    pub fn replace_current(&mut self) {
        if self.read_only {
            return;
        }
        self.refresh_matches();
        let (a, b) = self.sel.range();
        let cur = self.find.matches.iter().position(|m| m.start_pos() == a && m.end_pos() == b);
        let (Some(i), Some(re)) = (cur, self.find.regex.clone()) else {
            self.find_step(true);
            return;
        };
        let m = self.find.matches[i];
        let text = search::replacement(&re, self.buf.line(m.line), &m, &self.find.replace_text, self.find.query.regex);
        let before = self.sel;
        let edits = apply_all(&mut self.buf, vec![(a, b, text.clone())]);
        let end = Buffer::end_after(a, &text);
        self.sel = Sel::caret(end);
        self.history.push(edits, before, self.sel, EditKind::Other, Instant::now());
        self.find_step(true);
    }

    pub fn replace_all(&mut self) -> usize {
        if self.read_only {
            return 0;
        }
        self.refresh_matches();
        let Some(re) = self.find.regex.clone() else { return 0 };
        let ranges = search::replace_all_ranges(self.buf.lines(), &re, &self.find.replace_text, self.find.query.regex);
        let n = ranges.len();
        let before = self.sel;
        let edits = apply_all(&mut self.buf, ranges);
        self.sel = clamp_sel(&self.buf, self.sel);
        self.history.push(edits, before, self.sel, EditKind::Other, Instant::now());
        n
    }

    pub fn goto_line(&mut self, line: usize, col: usize) {
        let l = line.saturating_sub(1).min(self.buf.line_count() - 1);
        let s = self.buf.line(l);
        let c = s.char_indices().nth(col.saturating_sub(1)).map_or(s.len(), |(b, _)| b);
        self.sel = Sel::caret(Pos::new(l, c));
        self.scroll_to_caret = true;
        self.center_caret = true;
        self.focus_request = true;
    }

    /// `SOYUZ_SCRIPT editor:type=…`: insert at the caret.
    pub fn devscript_type(&mut self, text: &str) {
        self.replace_sel(text, EditKind::Other);
    }

    /// `SOYUZ_SCRIPT editor:find=…`: open the find bar and select the next match.
    pub fn devscript_find(&mut self, text: &str) {
        self.find.open = true;
        self.find.query.text = text.to_string();
        self.find_step(true);
    }

    // ---- editing ----

    fn replace_sel(&mut self, text: &str, kind: EditKind) {
        if self.read_only {
            return;
        }
        let before = self.sel;
        let (a, b) = self.sel.range();
        let edits = apply_all(&mut self.buf, vec![(a, b, text.to_string())]);
        self.sel = Sel::caret(Buffer::end_after(a, text));
        self.history.push(edits, before, self.sel, kind, Instant::now());
        self.want_col = None;
        self.scroll_to_caret = true;
    }

    fn delete_range(&mut self, a: Pos, b: Pos, kind: EditKind) {
        if self.read_only || a == b {
            return;
        }
        let before = self.sel;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let edits = apply_all(&mut self.buf, vec![(a, b, String::new())]);
        self.sel = Sel::caret(a);
        self.history.push(edits, before, self.sel, kind, Instant::now());
        self.want_col = None;
        self.scroll_to_caret = true;
    }

    fn indent_unit(&self, tab: usize) -> String {
        if self.uses_tabs { "\t".into() } else { " ".repeat(tab) }
    }

    fn indent_lines(&mut self, outdent: bool, tab: usize) {
        if self.read_only {
            return;
        }
        let (a, b) = self.sel.range();
        let last = if b.col == 0 && b.line > a.line { b.line - 1 } else { b.line };
        let unit = self.indent_unit(tab);
        let mut ranges = Vec::new();
        for l in (a.line..=last).rev() {
            if outdent {
                let s = self.buf.line(l);
                let n = if s.starts_with('\t') { 1 } else { s.chars().take(tab).take_while(|&c| c == ' ').count() };
                if n > 0 {
                    ranges.push((Pos::new(l, 0), Pos::new(l, n), String::new()));
                }
            } else if !self.buf.line(l).is_empty() {
                ranges.push((Pos::new(l, 0), Pos::new(l, 0), unit.clone()));
            }
        }
        let before = self.sel;
        let edits = apply_all(&mut self.buf, ranges);
        self.sel = Sel::new(Pos::new(a.line, 0), Pos::new(last, self.buf.line(last).len()));
        self.history.push(edits, before, self.sel, EditKind::Other, Instant::now());
    }

    fn selected_text(&self) -> String {
        let (a, b) = self.sel.range();
        self.buf.text_range(a, b).replace('\n', self.eol.as_str())
    }

    fn move_to(&mut self, p: Pos, extend: bool) {
        let p = self.buf.clamp(p);
        self.sel = if extend { Sel::new(self.sel.anchor, p) } else { Sel::caret(p) };
        self.scroll_to_caret = true;
    }

    /// Up / down by visual rows.
    fn vertical(&mut self, delta: isize, extend: bool) {
        let head = self.sel.head;
        let map = LineMap::new(self.buf.line(head.line), self.rows.tab.max(1));
        let cols = self.rows.cols.max(1);
        let d = map.disp_of(head.col);
        let sub = (d / cols).min(self.rows.rows_of(head.line) - 1);
        let want = *self.want_col.get_or_insert(d - sub * cols);
        let row = (self.rows.row_of(head.line) + sub) as isize + delta;
        let row = row.clamp(0, self.rows.total() as isize - 1) as usize;
        let (line, sub) = self.rows.line_of(row);
        let m = LineMap::new(self.buf.line(line), self.rows.tab.max(1));
        let target = (sub * cols + want).min(m.disp.len()).min((sub + 1) * cols);
        let col = if target == (sub + 1) * cols && sub + 1 < self.rows.rows_of(line) {
            m.byte_of(target.saturating_sub(1))
        } else {
            m.byte_of(target)
        };
        let keep = self.want_col;
        self.move_to(Pos::new(line, col), extend);
        self.want_col = keep;
    }

    /// One input event while the text area has keyboard focus; returns text the
    /// frontend puts on the clipboard (copy / cut).
    pub fn input(&mut self, ev: EditInput, tab: usize) -> Option<String> {
        match ev {
            EditInput::Text(t) => {
                if !t.chars().all(char::is_control) {
                    self.replace_sel(&t, EditKind::Typing);
                }
            }
            EditInput::ImeCommit(t) => {
                if !t.is_empty() {
                    self.replace_sel(&t, EditKind::Other);
                }
            }
            EditInput::Copy => {
                if !self.sel.is_empty() {
                    return Some(self.selected_text());
                }
            }
            EditInput::Cut => {
                if !self.sel.is_empty() && !self.read_only {
                    let text = self.selected_text();
                    self.replace_sel("", EditKind::Other);
                    return Some(text);
                }
            }
            EditInput::Paste(s) => {
                let s = encoding::normalize_eol(&s);
                self.replace_sel(&s, EditKind::Other);
            }
            EditInput::Key(key, m) => self.key(key, m, tab),
        }
        None
    }

    /// A key press (`None`: a key without a neutral name).
    pub fn key(&mut self, key: Option<KeyName>, m: KeyMods, tab: usize) {
        let ext = m.shift;
        let cmd = m.command;
        if !matches!(key, Some(KeyName::ArrowUp | KeyName::ArrowDown | KeyName::PageUp | KeyName::PageDown)) {
            self.want_col = None;
        }
        let Some(key) = key else { return };
        let head = self.sel.head;
        match key {
            KeyName::ArrowLeft => {
                let p = if !ext && !self.sel.is_empty() && !cmd {
                    self.sel.range().0
                } else if cmd {
                    self.buf.word_left(head)
                } else {
                    self.buf.prev_pos(head)
                };
                self.move_to(p, ext);
            }
            KeyName::ArrowRight => {
                let p = if !ext && !self.sel.is_empty() && !cmd {
                    self.sel.range().1
                } else if cmd {
                    self.buf.word_right(head)
                } else {
                    self.buf.next_pos(head)
                };
                self.move_to(p, ext);
            }
            KeyName::ArrowUp if !m.alt => self.vertical(-1, ext),
            KeyName::ArrowDown if !m.alt => self.vertical(1, ext),
            KeyName::PageUp => self.vertical(-(self.page_rows as isize), ext),
            KeyName::PageDown => self.vertical(self.page_rows as isize, ext),
            KeyName::Home if cmd => self.move_to(Pos::default(), ext),
            KeyName::End if cmd => self.move_to(self.buf.end(), ext),
            KeyName::Home => {
                let first = self.buf.indent_of(head.line).len();
                let col = if head.col == first { 0 } else { first };
                self.move_to(Pos::new(head.line, col), ext);
            }
            KeyName::End => self.move_to(Pos::new(head.line, self.buf.line(head.line).len()), ext),
            KeyName::A if cmd => {
                self.sel = Sel::new(Pos::default(), self.buf.end());
            }
            KeyName::Z if cmd && m.shift => self.redo(),
            KeyName::Z if cmd => self.undo(),
            KeyName::Y if cmd => self.redo(),
            KeyName::Backspace => {
                if !self.sel.is_empty() {
                    self.replace_sel("", EditKind::Other);
                } else {
                    let a = if cmd { self.buf.word_left(head) } else { self.buf.prev_pos(head) };
                    self.delete_range(a, head, if cmd { EditKind::Other } else { EditKind::Deleting });
                }
            }
            KeyName::Delete => {
                if !self.sel.is_empty() {
                    self.replace_sel("", EditKind::Other);
                } else {
                    let b = if cmd { self.buf.word_right(head) } else { self.buf.next_pos(head) };
                    if !self.read_only && b != head {
                        let before = self.sel;
                        let edits = apply_all(&mut self.buf, vec![(head, b, String::new())]);
                        self.history.push(edits, before, self.sel, EditKind::Deleting, Instant::now());
                        self.scroll_to_caret = true;
                    }
                }
            }
            KeyName::Enter if !cmd => {
                let (a, _) = self.sel.range();
                let indent = self.buf.indent_of(a.line);
                let indent = indent[..indent.len().min(a.col)].to_string();
                self.replace_sel(&format!("\n{indent}"), EditKind::Other);
            }
            KeyName::Tab if !cmd => {
                let (a, b) = self.sel.range();
                if m.shift || a.line != b.line {
                    self.indent_lines(m.shift, tab);
                } else {
                    let unit = if self.uses_tabs {
                        "\t".to_string()
                    } else {
                        let map = LineMap::new(self.buf.line(a.line), tab);
                        let d = map.disp_of(a.col);
                        " ".repeat(tab - d % tab)
                    };
                    self.replace_sel(&unit, EditKind::Other);
                }
            }
            KeyName::D if cmd && !self.read_only => {
                // Duplicate the line (or selection).
                let (a, b) = self.sel.range();
                if self.sel.is_empty() {
                    let line = self.buf.line(a.line).to_string();
                    let at = Pos::new(a.line, line.len());
                    let before = self.sel;
                    let edits = apply_all(&mut self.buf, vec![(at, at, format!("\n{line}"))]);
                    self.sel = Sel::caret(Pos::new(a.line + 1, a.col));
                    self.history.push(edits, before, self.sel, EditKind::Other, Instant::now());
                } else {
                    let text = self.buf.text_range(a, b);
                    let before = self.sel;
                    let edits = apply_all(&mut self.buf, vec![(b, b, text.clone())]);
                    self.sel = Sel::new(b, Buffer::end_after(b, &text));
                    self.history.push(edits, before, self.sel, EditKind::Other, Instant::now());
                }
                self.scroll_to_caret = true;
            }
            _ => {}
        }
    }

    // ---- view support (the frontend draws; these keep the logic toolkit-free) ----

    /// Fold buffer edits into the highlighter and the row cache; `true` when anything
    /// changed (the caller then rebuilds the rows with [`Rows::rebuild`]).
    pub fn sync_changes(&mut self) -> bool {
        let changes = self.buf.take_changes();
        let Some(first) = changes.iter().map(|c| c.start).min() else { return false };
        self.hl.invalidate_from(first);
        self.rows.apply(&self.buf, &changes);
        true
    }

    /// Visual row of the caret and its display column within that row.
    pub fn caret_row(&self, cols: usize, tab: usize) -> (usize, usize) {
        let head = self.buf.clamp(self.sel.head);
        let map = LineMap::new(self.buf.line(head.line), tab);
        let disp = map.disp_of(head.col);
        let sub = (disp / cols).min(self.rows.rows_of(head.line) - 1);
        (self.rows.row_of(head.line) + sub, disp - sub * cols)
    }

    /// Scroll so the caret is visible when a move / edit asked for it: `visible_rows`
    /// rows fit; without soft wrap, the text area is `text_w` wide in cells of `char_w`.
    pub fn scroll_caret_into_view(
        &mut self,
        cols: usize,
        tab: usize,
        visible_rows: f64,
        wrap: bool,
        char_w: f32,
        text_w: f32,
    ) {
        if !std::mem::take(&mut self.scroll_to_caret) {
            return;
        }
        let (row, col) = self.caret_row(cols, tab);
        let head_row = row as f64;
        if std::mem::take(&mut self.center_caret) {
            if head_row < self.scroll_row || head_row >= self.scroll_row + visible_rows - 1.0 {
                self.scroll_row = head_row - (visible_rows / 2.0).floor();
            }
        } else if head_row < self.scroll_row {
            self.scroll_row = head_row;
        } else if head_row > self.scroll_row + visible_rows - 2.0 {
            self.scroll_row = head_row - visible_rows + 2.0;
        }
        if !wrap {
            let x = col as f32 * char_w;
            if x < self.scroll_x {
                self.scroll_x = (x - 4.0 * char_w).max(0.0);
            } else if x > self.scroll_x + text_w - 2.0 * char_w {
                self.scroll_x = x - text_w + 4.0 * char_w;
            }
        }
    }

    /// The "Find" toolbar button: toggle the plain find bar.
    pub fn toggle_find(&mut self) {
        if self.find.open && !self.find.replace {
            self.find.open = false;
        } else {
            self.find.replace = false;
            self.open_find(false);
            self.find.replace = false;
        }
    }

    /// The "Replace" toolbar button: toggle the find + replace bar.
    pub fn toggle_replace(&mut self) {
        if self.find.open && self.find.replace {
            self.find.open = false;
        } else {
            self.open_find(true);
            self.find.replace = true;
        }
    }

    /// The "Go to line" toolbar button.
    pub fn toggle_goto(&mut self) {
        if self.goto.is_some() {
            self.goto = None;
        } else {
            self.open_goto();
        }
    }

    /// The find field's text changed: jump to the first match from the selection start.
    pub fn find_incremental(&mut self) {
        self.refresh_matches();
        let from = self.sel.range().0;
        if let Some(i) = search::next_from(&self.find.matches, from) {
            self.select_match(i);
        }
    }

    /// The match counter of the find bar (empty without a query) and whether it is an
    /// error message.
    pub fn find_status(&mut self) -> (String, bool) {
        self.refresh_matches();
        let text = match (&self.find.error, self.find.matches.len()) {
            (Some(e), _) => e.clone(),
            (None, _) if self.find.query.text.is_empty() => String::new(),
            (None, 0) => tr!("editors-find-no-results").into(),
            (None, n) => {
                let (a, b) = self.sel.range();
                let cur = self.find.matches.iter().position(|m| m.start_pos() == a && m.end_pos() == b);
                let more = if n >= search::MAX_MATCHES { "+" } else { "" };
                match cur {
                    Some(i) => tr!("editors-find-position", current = i + 1, total = n, more = more),
                    None => tr!("editors-find-results", count = n, more = more),
                }
            }
        };
        (text, self.find.error.is_some())
    }

    /// Enter in the go-to field: `line[:col]` (also `line,col`). The bar stays open when
    /// the text is not a line number.
    pub fn submit_goto(&mut self) {
        let Some(text) = &self.goto else { return };
        let t = text.trim().to_string();
        let mut parts = t.splitn(2, [':', ',']);
        let line = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
        let col = parts.next().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1);
        if let Some(l) = line {
            self.goto = None;
            self.goto_line(l, col);
        }
    }

    /// The banner above the text: `Some(true)` lossy decode (read-only, "Edit anyway"),
    /// `Some(false)` mixed line endings ("OK").
    pub fn notice(&self) -> Option<bool> {
        if self.lossy && self.read_only {
            Some(true)
        } else if self.mixed_eol && !self.notice_dismissed {
            Some(false)
        } else {
            None
        }
    }

    /// The banner's button.
    pub fn dismiss_notice(&mut self) {
        if self.lossy && self.read_only {
            self.read_only = false;
        } else {
            self.notice_dismissed = true;
        }
    }

    /// Click on the line-ending label of the status bar.
    pub fn toggle_eol(&mut self) {
        if !self.read_only {
            self.eol = if self.eol == Eol::CrLf { Eol::Lf } else { Eol::CrLf };
        }
    }

    /// 1-based caret column in characters (status bar).
    pub fn caret_col(&self) -> usize {
        let head = self.sel.head;
        let line = self.buf.line(head.line);
        line[..head.col.min(line.len())].chars().count() + 1
    }

    /// Characters in the selection.
    pub fn selected_chars(&self) -> usize {
        let (a, b) = self.sel.range();
        self.buf.text_range(a, b).chars().count()
    }

    /// Triple click: select line `l` with its line break.
    pub fn select_line(&mut self, l: usize) {
        let end = if l + 1 < self.buf.line_count() { Pos::new(l + 1, 0) } else { Pos::new(l, self.buf.line(l).len()) };
        self.sel = Sel::new(Pos::new(l, 0), end);
        self.dragging = false;
    }

    /// Double click: select the word at `at`.
    pub fn select_word(&mut self, at: Pos) {
        let (a, b) = self.buf.word_at(at);
        self.sel = Sel::new(a, b);
        self.dragging = false;
    }

    /// Primary press at `at` (Shift extends the selection): starts a drag selection.
    pub fn press(&mut self, at: Pos, extend: bool) {
        self.sel = if extend { Sel::new(self.sel.anchor, at) } else { Sel::caret(at) };
        self.dragging = true;
        self.want_col = None;
    }
}

fn clamp_sel(buf: &Buffer, s: Sel) -> Sel {
    Sel::new(buf.clamp(s.anchor), buf.clamp(s.head))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_map_tabs_and_snapping() {
        let m = LineMap::new("a\tbé", 4);
        assert_eq!(m.disp.len(), 6);
        assert_eq!(m.disp_of(0), 0);
        assert_eq!(m.disp_of(1), 1);
        assert_eq!(m.disp_of(2), 4);
        assert_eq!(m.disp_of(3), 5);
        assert_eq!(m.disp_of(5), 6);
        assert_eq!(m.byte_of(2), 1, "inside the tab, nearer its start");
        assert_eq!(m.byte_of(3), 2, "inside the tab, nearer its end");
        assert_eq!(m.byte_of(9), 5);
        assert_eq!(disp_width("a\tb", 4), 5);
        assert_eq!(disp_width("éé", 4), 2);
    }

    #[test]
    fn rows_wrap_and_track_edits() {
        let mut b = Buffer::new("aaaaaaaaaa\nb\n");
        let mut r = Rows::default();
        r.rebuild(&b, 4, 4);
        assert_eq!(r.total(), 3 + 1 + 1);
        assert_eq!(r.line_of(2), (0, 2));
        assert_eq!(r.line_of(3), (1, 0));
        assert_eq!(r.row_of(2), 4);
        b.take_changes();
        b.replace(Pos::new(1, 1), Pos::new(1, 1), "\nccccc");
        let ch = b.take_changes();
        r.apply(&b, &ch);
        r.rebuild(&b, 4, 4);
        assert_eq!(b.line_count(), 4);
        assert_eq!(r.total(), 3 + 1 + 2 + 1);
        assert_eq!(r.line_of(5), (2, 1));
    }

    #[test]
    fn save_snapshot_preserves_endings_and_encoding() {
        let d = encoding::decode(b"\xEF\xBB\xBFa\r\nb\r\n");
        let mut doc = TextDoc::new("x.txt", d);
        doc.sel = Sel::caret(Pos::new(0, 1));
        doc.replace_sel("!", EditKind::Typing);
        assert!(doc.is_dirty());
        let (bytes, token) = doc.save_snapshot();
        assert_eq!(bytes, b"\xEF\xBB\xBFa!\r\nb\r\n");
        // Typing after the snapshot does not merge into the saved step.
        doc.replace_sel("?", EditKind::Typing);
        doc.saved(token);
        assert!(doc.is_dirty());
        doc.undo();
        assert!(!doc.is_dirty());
        doc.eol = Eol::Lf;
        assert!(doc.is_dirty(), "switching line endings is a change");
    }

    #[test]
    fn find_replace_through_the_doc() {
        let mut doc = TextDoc::new("x.txt", encoding::decode(b"one two one\nONE"));
        doc.find.query = Query { text: "one".into(), regex: false, case: false, whole_word: true };
        doc.find_step(true);
        assert_eq!(doc.sel.range(), (Pos::new(0, 0), Pos::new(0, 3)));
        doc.find_step(true);
        assert_eq!(doc.sel.range().0, Pos::new(0, 8));
        doc.find.replace_text = "1".into();
        assert_eq!(doc.replace_all(), 3);
        assert_eq!(doc.buf.text(), "1 two 1\n1");
        doc.undo();
        assert_eq!(doc.buf.text(), "one two one\nONE");
        doc.goto_line(2, 2);
        assert_eq!(doc.sel.head, Pos::new(1, 1));
    }

    #[test]
    fn input_events_edit_and_reach_the_clipboard() {
        use crate::editors::{EditInput, KeyMods};
        let mut doc = TextDoc::new("x.txt", encoding::decode(b"one\ntwo"));
        doc.sel = Sel::caret(Pos::new(0, 3));
        assert_eq!(doc.input(EditInput::Text("!".into()), 4), None);
        assert_eq!(doc.buf.text(), "one!\ntwo");
        doc.input(EditInput::ImeCommit(String::new()), 4);
        let shift = KeyMods { shift: true, ..Default::default() };
        doc.input(EditInput::Key(Some(KeyName::Home), shift), 4);
        assert_eq!(doc.input(EditInput::Copy, 4), Some("one!".into()));
        assert_eq!(doc.input(EditInput::Cut, 4), Some("one!".into()));
        assert_eq!(doc.buf.text(), "\ntwo");
        doc.input(EditInput::Paste("a\r\nb".into()), 4);
        assert_eq!(doc.buf.text(), "a\nb\ntwo", "pasted line endings are normalized");
        let cmd = KeyMods { command: true, ..Default::default() };
        doc.input(EditInput::Key(Some(KeyName::Z), cmd), 4);
        assert_eq!(doc.buf.text(), "\ntwo", "undo");
        doc.want_col = Some(3);
        doc.input(EditInput::Key(None, KeyMods::default()), 4);
        assert_eq!(doc.want_col, None, "any other key ends vertical motion");
    }

    #[test]
    fn goto_field_parses_line_and_column() {
        let mut doc = TextDoc::new("x.txt", encoding::decode(b"a\nbcd\ne"));
        doc.toggle_goto();
        doc.goto = Some("x".into());
        doc.submit_goto();
        assert!(doc.goto.is_some(), "not a line: the bar stays");
        doc.goto = Some("2:3".into());
        doc.submit_goto();
        assert_eq!((doc.goto.is_none(), doc.sel.head), (true, Pos::new(1, 2)));
        assert_eq!(doc.caret_col(), 3);
    }

    #[test]
    fn indent_and_outdent_lines() {
        let mut doc = TextDoc::new("x.rs", encoding::decode(b"a\n  b\nc"));
        doc.sel = Sel::new(Pos::new(0, 0), Pos::new(2, 1));
        doc.indent_lines(false, 4);
        assert_eq!(doc.buf.text(), "    a\n      b\n    c");
        doc.indent_lines(true, 4);
        assert_eq!(doc.buf.text(), "a\n  b\nc");
    }
}
