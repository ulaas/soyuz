//! The text editor: a virtualized code view over [`buffer::Buffer`] with incremental
//! syntax highlighting, line numbers, soft wrap, visible whitespace, find / replace with
//! match highlights, go to line and multi-level undo. Only the visible rows are laid
//! out, so multi-megabyte files stay fast.

pub mod buffer;
pub mod encoding;
pub mod highlight;
pub mod search;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Event, EventFilter, FontId, Galley, Id, Key, Modifiers, Rect, Sense,
    Stroke, Ui, pos2, vec2,
};
use std::sync::Arc;

use buffer::{Buffer, EditKind, History, Pos, Sel, apply_all};
use encoding::{Decoded, Encoding, Eol};
use highlight::Highlighter;
use search::{Match, Query};

use super::widgets::{self, SCROLLBAR};
use crate::theme::{Palette, icons::ph};

/// Lines longer than this are split into rows even without soft wrap (bounded layout).
const HARD_WRAP: usize = 10_000;
/// Per-frame time for syntax highlighting.
const HL_BUDGET: Duration = Duration::from_millis(6);
const TOOLBAR_H: f32 = 34.0;
const BAR_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;

/// What the surrounding panel should do after a frame.
#[derive(Default)]
pub struct TextOutput {
    pub save: bool,
    pub toggle_wrap: bool,
    pub toggle_whitespace: bool,
}

/// Settings and theme for one frame.
pub struct TextEnv<'a> {
    pub cfg: &'a soyuz_core::config::Editor,
    pub p: &'a Palette,
    /// Focusable widget ids of the editor (the app routes keys to the editor when one of
    /// them has focus).
    pub focus: &'a mut Vec<Id>,
    pub id: Id,
}

#[derive(Default)]
struct FindState {
    open: bool,
    replace: bool,
    query: Query,
    replace_text: String,
    /// `(query, buffer version)` the matches belong to.
    key: Option<(Query, u64)>,
    error: Option<String>,
    regex: Option<regex::Regex>,
    matches: Vec<Match>,
    current: Option<usize>,
    focus_find: bool,
}

/// Visual rows per line (soft wrap / hard wrap) and their prefix sums.
#[derive(Default)]
struct Rows {
    cols: usize,
    tab: usize,
    widths: Vec<u32>,
    prefix: Vec<usize>,
    max_width: u32,
    dirty: bool,
}

impl Rows {
    fn rebuild(&mut self, buf: &Buffer, cols: usize, tab: usize) {
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

    fn apply(&mut self, buf: &Buffer, changes: &[buffer::LineChange]) {
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

    fn total(&self) -> usize {
        self.prefix.last().copied().unwrap_or(1)
    }

    fn rows_of(&self, line: usize) -> usize {
        self.prefix
            .get(line + 1)
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.prefix.get(line).copied().unwrap_or(0))
            .max(1)
    }

    fn row_of(&self, line: usize) -> usize {
        self.prefix.get(line).copied().unwrap_or(0)
    }

    /// `(line, sub-row)` of a visual row.
    fn line_of(&self, row: usize) -> (usize, usize) {
        let n = self.prefix.len().saturating_sub(1);
        if n == 0 {
            return (0, 0);
        }
        let line = self.prefix.partition_point(|&s| s <= row).saturating_sub(1).min(n - 1);
        (line, row - self.prefix[line])
    }
}

/// Display width of a line (tabs to the next stop).
fn disp_width(line: &str, tab: usize) -> usize {
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
enum Cell {
    Char,
    Space,
    TabStart,
    TabRest,
    Control,
}

struct LineMap {
    disp: Vec<char>,
    cells: Vec<Cell>,
    /// Byte offset of every source char, plus the line length.
    char_bytes: Vec<usize>,
    /// Display index of every source char, plus the display length.
    char_disp: Vec<usize>,
}

impl LineMap {
    fn new(line: &str, tab: usize) -> Self {
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

    fn disp_of(&self, col: usize) -> usize {
        let ci = self.char_bytes.partition_point(|&b| b < col);
        self.char_disp[ci.min(self.char_disp.len() - 1)]
    }

    fn byte_of(&self, d: usize) -> usize {
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
    saved_eol: Eol,
    pub mixed_eol: bool,
    pub lossy: bool,
    pub read_only: bool,
    /// Opened read-only on purpose (larger than the limit etc.).
    hl: Highlighter,
    rows: Rows,
    find: FindState,
    goto: Option<String>,
    goto_focus: bool,
    /// Top visual row (fractional).
    scroll_row: f64,
    scroll_x: f32,
    scroll_to_caret: bool,
    center_caret: bool,
    /// Wanted display column for vertical motion.
    want_col: Option<usize>,
    pub focus_request: bool,
    uses_tabs: bool,
    dragging: bool,
    page_rows: usize,
    notice_dismissed: bool,
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

    fn select_match(&mut self, i: usize) {
        if let Some(m) = self.find.matches.get(i) {
            self.sel = Sel::new(m.start_pos(), m.end_pos());
            self.find.current = Some(i);
            self.scroll_to_caret = true;
            self.center_caret = true;
        }
    }

    fn refresh_matches(&mut self) {
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

    fn replace_current(&mut self) {
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

    fn replace_all(&mut self) -> usize {
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

    /// Keyboard handling while the text area has focus.
    fn handle_events(&mut self, ui: &Ui, tab: usize) {
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            match ev {
                Event::Text(t) => {
                    if !t.chars().all(char::is_control) {
                        self.replace_sel(&t, EditKind::Typing);
                    }
                }
                Event::Ime(egui::ImeEvent::Commit(t)) if !t.is_empty() => self.replace_sel(&t, EditKind::Other),
                Event::Copy => {
                    if !self.sel.is_empty() {
                        ui.ctx().copy_text(self.selected_text());
                    }
                }
                Event::Cut => {
                    if !self.sel.is_empty() && !self.read_only {
                        ui.ctx().copy_text(self.selected_text());
                        self.replace_sel("", EditKind::Other);
                    }
                }
                Event::Paste(s) => {
                    let s = encoding::normalize_eol(&s);
                    self.replace_sel(&s, EditKind::Other);
                }
                Event::Key { key, pressed: true, modifiers, .. } => self.key(key, modifiers, tab),
                _ => {}
            }
        }
    }

    fn key(&mut self, key: Key, m: Modifiers, tab: usize) {
        let ext = m.shift;
        let cmd = m.command;
        if !matches!(key, Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown) {
            self.want_col = None;
        }
        let head = self.sel.head;
        match key {
            Key::ArrowLeft => {
                let p = if !ext && !self.sel.is_empty() && !cmd {
                    self.sel.range().0
                } else if cmd {
                    self.buf.word_left(head)
                } else {
                    self.buf.prev_pos(head)
                };
                self.move_to(p, ext);
            }
            Key::ArrowRight => {
                let p = if !ext && !self.sel.is_empty() && !cmd {
                    self.sel.range().1
                } else if cmd {
                    self.buf.word_right(head)
                } else {
                    self.buf.next_pos(head)
                };
                self.move_to(p, ext);
            }
            Key::ArrowUp if !m.alt => self.vertical(-1, ext),
            Key::ArrowDown if !m.alt => self.vertical(1, ext),
            Key::PageUp => self.vertical(-(self.page_rows as isize), ext),
            Key::PageDown => self.vertical(self.page_rows as isize, ext),
            Key::Home if cmd => self.move_to(Pos::default(), ext),
            Key::End if cmd => self.move_to(self.buf.end(), ext),
            Key::Home => {
                let first = self.buf.indent_of(head.line).len();
                let col = if head.col == first { 0 } else { first };
                self.move_to(Pos::new(head.line, col), ext);
            }
            Key::End => self.move_to(Pos::new(head.line, self.buf.line(head.line).len()), ext),
            Key::A if cmd => {
                self.sel = Sel::new(Pos::default(), self.buf.end());
            }
            Key::Z if cmd && m.shift => self.redo(),
            Key::Z if cmd => self.undo(),
            Key::Y if cmd => self.redo(),
            Key::Backspace => {
                if !self.sel.is_empty() {
                    self.replace_sel("", EditKind::Other);
                } else {
                    let a = if cmd { self.buf.word_left(head) } else { self.buf.prev_pos(head) };
                    self.delete_range(a, head, if cmd { EditKind::Other } else { EditKind::Deleting });
                }
            }
            Key::Delete => {
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
            Key::Enter if !cmd => {
                let (a, _) = self.sel.range();
                let indent = self.buf.indent_of(a.line);
                let indent = indent[..indent.len().min(a.col)].to_string();
                self.replace_sel(&format!("\n{indent}"), EditKind::Other);
            }
            Key::Tab if !cmd => {
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
            Key::D if cmd && !self.read_only => {
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

    // ---- drawing ----

    pub fn show(&mut self, ui: &mut Ui, rect: Rect, env: &mut TextEnv<'_>) -> TextOutput {
        let mut out = TextOutput::default();
        let p = env.p;
        let cfg = env.cfg;
        let tab = (cfg.tab_width as usize).clamp(1, 16);
        // Toolbar.
        let tb = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
        self.toolbar(ui, tb, env, &mut out);
        let mut top = tb.bottom();
        if self.find.open {
            let h = if self.find.replace { BAR_H * 2.0 - 4.0 } else { BAR_H };
            let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), h));
            self.find_bar(ui, r, env);
            top += h;
        }
        if self.goto.is_some() {
            let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), BAR_H));
            self.goto_bar(ui, r, env);
            top += BAR_H;
        }
        let notice = if self.lossy && self.read_only {
            Some((
                ph::WARNING,
                p.warning,
                "Not valid UTF-8: invalid bytes are shown as \u{FFFD}. Saving would write them that way.",
                &["Edit anyway"][..],
            ))
        } else if self.mixed_eol && !self.notice_dismissed {
            Some((ph::INFO, p.info, "Mixed line endings: saving uses the most common one.", &["OK"][..]))
        } else {
            None
        };
        if let Some((glyph, color, text, buttons)) = notice {
            let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
            if widgets::banner(ui, r, p, glyph, color, text, buttons).is_some() {
                if self.lossy && self.read_only {
                    self.read_only = false;
                } else {
                    self.notice_dismissed = true;
                }
            }
            top += 36.0;
        }
        let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STATUS_H), rect.max);
        let body = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
        self.body(ui, body, env, tab);
        self.status(ui, status, p, tab);
        out
    }

    fn toolbar(&mut self, ui: &mut Ui, r: Rect, env: &mut TextEnv<'_>, out: &mut TextOutput) {
        let p = env.p;
        let id = env.id;
        let x = std::cell::Cell::new(r.left() + 6.0);
        let btn = |ui: &mut Ui, glyph: &str, tip: &str, enabled: bool, on: bool| -> bool {
            let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
            x.set(x.get() + 28.0);
            if on {
                ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
            }
            widgets::icon_at(ui, b, id.with(("tb", tip)), glyph, tip, enabled, p)
        };
        let dirty = self.is_dirty();
        out.save |= btn(ui, ph::FLOPPY_DISK, "Save (Ctrl+S)", dirty && !self.read_only, false);
        if btn(ui, ph::ARROW_COUNTER_CLOCKWISE, "Undo (Ctrl+Z)", self.history.can_undo(), false) {
            self.undo();
        }
        if btn(ui, ph::ARROW_CLOCKWISE, "Redo (Ctrl+Y)", self.history.can_redo(), false) {
            self.redo();
        }
        x.set(x.get() + 8.0);
        let find_open = self.find.open;
        let replace_open = find_open && self.find.replace;
        let b = |ui: &mut Ui, glyph: &str, tip: &str, on: bool| -> bool {
            let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
            x.set(x.get() + 28.0);
            if on {
                ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
            }
            widgets::icon_at(ui, b, id.with(("tb2", tip)), glyph, tip, true, p)
        };
        if b(ui, ph::MAGNIFYING_GLASS, "Find (Ctrl+F)", find_open && !replace_open) {
            if find_open && !replace_open {
                self.find.open = false;
            } else {
                self.find.replace = false;
                self.open_find(false);
                self.find.replace = false;
            }
        }
        if b(ui, ph::SWAP, "Replace (Ctrl+H)", replace_open) {
            if replace_open {
                self.find.open = false;
            } else {
                self.open_find(true);
                self.find.replace = true;
            }
        }
        if b(ui, ph::ARROW_LINE_DOWN, "Go to line (Ctrl+G)", self.goto.is_some()) {
            if self.goto.is_some() {
                self.goto = None;
            } else {
                self.open_goto();
            }
        }
        x.set(x.get() + 8.0);
        out.toggle_wrap |= b(ui, ph::TEXT_ALIGN_LEFT, "Wrap long lines", env.cfg.wrap);
        out.toggle_whitespace |= b(ui, ph::PARAGRAPH, "Show whitespace", env.cfg.show_whitespace);
        if self.read_only {
            ui.painter().text(
                pos2(r.right() - 10.0, r.center().y),
                Align2::RIGHT_CENTER,
                "Read-only",
                FontId::proportional(11.5),
                p.warning,
            );
        }
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }

    fn find_bar(&mut self, ui: &mut Ui, r: Rect, env: &mut TextEnv<'_>) {
        let p = env.p;
        let id = env.id;
        ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
        let row1 = Rect::from_min_size(r.min, vec2(r.width(), BAR_H));
        let field_w = (r.width() - 300.0).clamp(120.0, 420.0);
        let fr = Rect::from_min_size(pos2(row1.left() + 8.0, row1.top() + 5.0), vec2(field_w, 24.0));
        let fid = id.with("find.field");
        let before = self.find.query.text.clone();
        let resp = widgets::field(ui, fr, fid, &mut self.find.query.text, "Find", true);
        env.focus.push(fid);
        if std::mem::take(&mut self.find.focus_find) {
            resp.request_focus();
            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), fid) {
                let n = self.find.query.text.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::two(CCursor::new(0), CCursor::new(n))));
                st.store(ui.ctx(), fid);
            }
        }
        if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
            let back = ui.input(|i| i.modifiers.shift);
            self.find_step(!back);
            resp.request_focus();
        }
        let mut x = fr.right() + 6.0;
        let mut chip = |ui: &mut Ui, label: &str, tip: &str, v: &mut bool| {
            let cr = Rect::from_min_size(pos2(x, row1.top() + 6.0), vec2(26.0, 22.0));
            x += 28.0;
            if widgets::chip(ui, cr, id.with(("chip", label)), label, tip, *v, p) {
                *v = !*v;
            }
        };
        chip(ui, "Aa", "Match case", &mut self.find.query.case);
        chip(ui, "W", "Whole word", &mut self.find.query.whole_word);
        chip(ui, ".*", "Regular expression", &mut self.find.query.regex);
        if before != self.find.query.text {
            // Incremental: jump to the first match from the selection start.
            self.refresh_matches();
            let from = self.sel.range().0;
            if let Some(i) = search::next_from(&self.find.matches, from) {
                self.select_match(i);
            }
        }
        self.refresh_matches();
        let count = match (&self.find.error, self.find.matches.len()) {
            (Some(e), _) => e.clone(),
            (None, _) if self.find.query.text.is_empty() => String::new(),
            (None, 0) => "No results".into(),
            (None, n) => {
                let (a, b) = self.sel.range();
                let cur = self.find.matches.iter().position(|m| m.start_pos() == a && m.end_pos() == b);
                let more = if n >= search::MAX_MATCHES { "+" } else { "" };
                match cur {
                    Some(i) => format!("{} of {n}{more}", i + 1),
                    None => format!("{n}{more} results"),
                }
            }
        };
        let color = if self.find.error.is_some() { p.danger } else { p.text_secondary };
        ui.painter().text(
            pos2(x + 4.0, row1.center().y),
            Align2::LEFT_CENTER,
            &count,
            FontId::proportional(11.5),
            color,
        );
        let right = row1.right() - 8.0;
        let ir = |i: f32| Rect::from_min_size(pos2(right - 26.0 * (i + 1.0), row1.top() + 5.0), vec2(24.0, 24.0));
        if widgets::icon_at(ui, ir(0.0), id.with("find.close"), ph::X, "Close (Esc)", true, p) {
            self.find.open = false;
            self.focus_request = true;
        }
        if widgets::icon_at(ui, ir(1.0), id.with("find.next"), ph::ARROW_DOWN, "Next (Enter / F3)", true, p) {
            self.find_step(true);
        }
        if widgets::icon_at(ui, ir(2.0), id.with("find.prev"), ph::ARROW_UP, "Previous (Shift+Enter)", true, p) {
            self.find_step(false);
        }
        if self.find.replace {
            let row2 = Rect::from_min_size(pos2(r.left(), row1.bottom() - 4.0), vec2(r.width(), BAR_H));
            let rr = Rect::from_min_size(pos2(row2.left() + 8.0, row2.top() + 5.0), vec2(field_w, 24.0));
            let rid = id.with("replace.field");
            let hint = if self.find.query.regex { "Replace ($1 for groups)" } else { "Replace" };
            let resp = widgets::field(ui, rr, rid, &mut self.find.replace_text, hint, true);
            env.focus.push(rid);
            if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
                self.replace_current();
                resp.request_focus();
            }
            let br = |x: f32, w: f32| Rect::from_min_size(pos2(x, row2.top() + 5.0), vec2(w, 24.0));
            let b1 = br(rr.right() + 6.0, 70.0);
            let b2 = br(b1.right() + 6.0, 86.0);
            for (b, label, all) in [(b1, "Replace", false), (b2, "Replace all", true)] {
                let resp = ui.interact(b, id.with(("rep", label)), Sense::click());
                crate::ui::a11y::button(&resp, label);
                let fill = if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 };
                ui.painter().rect_filled(b, CornerRadius::same(4), fill);
                ui.painter().rect_stroke(
                    b,
                    CornerRadius::same(4),
                    Stroke::new(1.0, p.border_subtle),
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(b.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), p.text_primary);
                if resp.clicked() && !self.read_only {
                    if all {
                        self.replace_all();
                    } else {
                        self.replace_current();
                    }
                }
            }
        }
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }

    fn goto_bar(&mut self, ui: &mut Ui, r: Rect, env: &mut TextEnv<'_>) {
        let p = env.p;
        let id = env.id.with("goto.field");
        ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
        let n = self.buf.line_count();
        let fr = Rect::from_min_size(pos2(r.left() + 8.0, r.top() + 5.0), vec2(180.0, 24.0));
        let Some(text) = self.goto.as_mut() else { return };
        let resp = widgets::field(ui, fr, id, text, "Line[:column]", true);
        env.focus.push(id);
        if std::mem::take(&mut self.goto_focus) {
            resp.request_focus();
        }
        ui.painter().text(
            pos2(fr.right() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            format!("1 – {n}  ·  Enter to jump"),
            FontId::proportional(11.5),
            p.text_secondary,
        );
        if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
            let t = text.trim().to_string();
            let mut parts = t.splitn(2, [':', ',']);
            let line = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
            let col = parts.next().and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(1);
            if let Some(l) = line {
                self.goto = None;
                self.goto_line(l, col);
            }
        }
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }

    fn status(&mut self, ui: &mut Ui, r: Rect, p: &Palette, tab: usize) {
        ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
        let head = self.sel.head;
        let col = self.buf.line(head.line)[..head.col.min(self.buf.line(head.line).len())].chars().count() + 1;
        let mut left = format!("Ln {}, Col {col}", head.line + 1);
        if !self.sel.is_empty() {
            let (a, b) = self.sel.range();
            let n = self.buf.text_range(a, b).chars().count();
            left.push_str(&format!("  ·  {n} selected"));
        }
        left.push_str(&format!("  ·  {} lines", self.buf.line_count()));
        ui.painter().text(
            pos2(r.left() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            left,
            FontId::proportional(11.5),
            p.text_secondary,
        );
        let lang = self.language().unwrap_or("Plain text");
        let indent = if self.uses_tabs { "Tabs".to_string() } else { format!("Spaces: {tab}") };
        let right = format!("{lang}  ·  {indent}  ·  {}", self.encoding.label());
        let eol_rect =
            Rect::from_min_max(pos2(r.right() - 50.0, r.top() + 2.0), pos2(r.right() - 6.0, r.bottom() - 2.0));
        ui.painter().text(
            pos2(eol_rect.left() - 8.0, r.center().y),
            Align2::RIGHT_CENTER,
            right,
            FontId::proportional(11.5),
            p.text_secondary,
        );
        let resp = ui.interact(eol_rect, ui.id().with("eol"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(eol_rect, CornerRadius::same(4), p.bg_surface2);
        }
        ui.painter().text(
            eol_rect.center(),
            Align2::CENTER_CENTER,
            self.eol.label(),
            FontId::proportional(11.5),
            if self.eol != self.saved_eol { p.accent } else { p.text_secondary },
        );
        if resp.on_hover_text("Line endings used when saving (click to switch)").clicked() && !self.read_only {
            self.eol = if self.eol == Eol::CrLf { Eol::Lf } else { Eol::CrLf };
        }
    }

    fn body(&mut self, ui: &mut Ui, body: Rect, env: &mut TextEnv<'_>, tab: usize) {
        let p = env.p;
        let cfg = env.cfg;
        let font = crate::theme::fonts::code(ui.ctx(), cfg.font_size.clamp(6.0, 48.0));
        let (char_w, row_h) =
            ui.ctx().fonts_mut(|f| (f.glyph_width(&font, 'M').max(1.0), (f.row_height(&font) * 1.3).round().max(8.0)));
        let digits = (self.buf.line_count().max(1).ilog10() as usize + 1).max(3);
        let gutter_w = (digits as f32 + 2.0) * char_w;
        let gutter = Rect::from_min_max(body.min, pos2(body.left() + gutter_w, body.bottom()));
        let vbar = Rect::from_min_max(pos2(body.right() - SCROLLBAR, body.top()), body.max);
        let text_left = gutter.right() + 8.0;
        let mut text = Rect::from_min_max(pos2(text_left, body.top()), pos2(vbar.left(), body.bottom()));
        let cols = if cfg.wrap { (((text.width() - char_w) / char_w).floor() as usize).max(8) } else { HARD_WRAP };
        // Layout caches.
        let changes = self.buf.take_changes();
        if let Some(first) = changes.iter().map(|c| c.start).min() {
            self.hl.invalidate_from(first);
            self.rows.apply(&self.buf, &changes);
        }
        self.rows.rebuild(&self.buf, cols, tab);
        let content_w = (self.rows.max_width.min(cols as u32) as f32 + 2.0) * char_w;
        let need_h = !cfg.wrap && content_w > text.width();
        let hbar = Rect::from_min_max(pos2(text.left(), body.bottom() - SCROLLBAR), pos2(vbar.left(), body.bottom()));
        if need_h {
            text.max.y = hbar.top();
        }
        let visible_rows = (text.height() / row_h).max(1.0) as f64;
        self.page_rows = (visible_rows as usize).saturating_sub(1).max(1);
        let total = self.rows.total() as f64;
        let max_scroll = (total - visible_rows + 1.0).max(0.0);

        // Interaction.
        let id = env.id.with("text");
        env.focus.push(id);
        let resp = ui.interact(text, id, Sense::click_and_drag());
        // Focus handed over this frame (e.g. Enter in the go-to bar): this frame's keys
        // belonged to the previous owner.
        let just_focused = std::mem::take(&mut self.focus_request) && !resp.has_focus();
        if just_focused {
            resp.request_focus();
        }
        let focused = resp.has_focus();
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
            let d = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta));
            if d != egui::Vec2::ZERO {
                self.scroll_row -= (d.y / row_h) as f64;
                if !cfg.wrap {
                    self.scroll_x -= d.x;
                }
            }
        }
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false },
                )
            });
            if !just_focused {
                self.handle_events(ui, tab);
            }
            // Edits from the keys above: refresh row counts before drawing.
            let changes = self.buf.take_changes();
            if let Some(first) = changes.iter().map(|c| c.start).min() {
                self.hl.invalidate_from(first);
                self.rows.apply(&self.buf, &changes);
                self.rows.rebuild(&self.buf, cols, tab);
            }
        }
        let total = self.rows.total() as f64;
        let max_scroll = max_scroll.max((total - visible_rows + 1.0).max(0.0));
        // Caret row for scrolling.
        let head = self.buf.clamp(self.sel.head);
        let head_map = LineMap::new(self.buf.line(head.line), tab);
        let head_disp = head_map.disp_of(head.col);
        let head_sub = (head_disp / cols).min(self.rows.rows_of(head.line) - 1);
        let head_row = (self.rows.row_of(head.line) + head_sub) as f64;
        if std::mem::take(&mut self.scroll_to_caret) {
            if std::mem::take(&mut self.center_caret) {
                if head_row < self.scroll_row || head_row >= self.scroll_row + visible_rows - 1.0 {
                    self.scroll_row = head_row - (visible_rows / 2.0).floor();
                }
            } else if head_row < self.scroll_row {
                self.scroll_row = head_row;
            } else if head_row > self.scroll_row + visible_rows - 2.0 {
                self.scroll_row = head_row - visible_rows + 2.0;
            }
            if !cfg.wrap {
                let x = (head_disp - head_sub * cols) as f32 * char_w;
                if x < self.scroll_x {
                    self.scroll_x = (x - 4.0 * char_w).max(0.0);
                } else if x > self.scroll_x + text.width() - 2.0 * char_w {
                    self.scroll_x = x - text.width() + 4.0 * char_w;
                }
            }
        }
        // Scrollbars.
        widgets::vscroll(
            ui,
            vbar,
            env.id.with("vbar"),
            total + visible_rows - 1.0,
            visible_rows,
            &mut self.scroll_row,
            p,
        );
        self.scroll_row = self.scroll_row.clamp(0.0, max_scroll.max(0.0));
        if cfg.wrap {
            self.scroll_x = 0.0;
        } else {
            let mut sx = self.scroll_x as f64;
            if need_h {
                widgets::hscroll(ui, hbar, env.id.with("hbar"), content_w as f64, text.width() as f64, &mut sx, p);
            }
            self.scroll_x = (sx as f32).clamp(0.0, (content_w - text.width()).max(0.0));
        }

        // Pointer → position.
        let first_row = self.scroll_row.floor() as usize;
        let frac = (self.scroll_row - first_row as f64) as f32;
        let origin_y = text.top() - frac * row_h;
        let origin_x = text.left() - self.scroll_x;
        let n_rows = (visible_rows.ceil() as usize + 2).min(self.rows.total().saturating_sub(first_row));
        let painter = ui.painter_at(body);
        let mut maps: HashMap<usize, LineMap> = HashMap::new();
        let dark = p.dark;
        if !self.hl.prepare(
            self.buf.lines(),
            self.rows.line_of(first_row).0,
            self.rows.line_of(first_row + n_rows.saturating_sub(1)).0,
            dark,
            HL_BUDGET,
        ) {
            ui.ctx().request_repaint();
        }
        let row_galley =
            |doc: &TextDoc, maps: &mut HashMap<usize, LineMap>, row: usize| -> (Arc<Galley>, usize, usize) {
                let (line, sub) = doc.rows.line_of(row);
                let map = maps.entry(line).or_insert_with(|| LineMap::new(doc.buf.line(line), tab));
                let start = sub * cols;
                let end = ((sub + 1) * cols).min(map.disp.len());
                let job = doc.row_job(line, map, start, end, &font, p, cfg.show_whitespace);
                (painter.layout_job(job), line, start)
            };
        let pointer_pos = |doc: &TextDoc, maps: &mut HashMap<usize, LineMap>, pt: egui::Pos2| -> Pos {
            let row = ((pt.y - origin_y) / row_h).floor();
            let row = (first_row as f32 + row).clamp(0.0, (doc.rows.total() - 1) as f32) as usize;
            let (g, line, start) = row_galley(doc, maps, row);
            let c = g.cursor_from_pos(vec2(pt.x - origin_x, row_h / 2.0));
            let map = &maps[&line];
            Pos::new(line, map.byte_of(start + c.index.0))
        };
        if let Some(pt) = resp.interact_pointer_pos() {
            let shift = ui.input(|i| i.modifiers.shift);
            if resp.triple_clicked() {
                let l = pointer_pos(self, &mut maps, pt).line;
                let end = if l + 1 < self.buf.line_count() {
                    Pos::new(l + 1, 0)
                } else {
                    Pos::new(l, self.buf.line(l).len())
                };
                self.sel = Sel::new(Pos::new(l, 0), end);
                self.dragging = false;
            } else if resp.double_clicked() {
                let at = pointer_pos(self, &mut maps, pt);
                let (a, b) = self.buf.word_at(at);
                self.sel = Sel::new(a, b);
                self.dragging = false;
            } else if resp.drag_started() || (resp.is_pointer_button_down_on() && !self.dragging && !resp.dragged()) {
                if ui.input(|i| i.pointer.primary_pressed()) {
                    let at = pointer_pos(self, &mut maps, pt);
                    self.sel = if shift { Sel::new(self.sel.anchor, at) } else { Sel::caret(at) };
                    self.dragging = true;
                    self.want_col = None;
                }
            } else if resp.dragged() && self.dragging {
                let at = pointer_pos(self, &mut maps, pt);
                self.sel.head = at;
                if pt.y < text.top() || pt.y > text.bottom() {
                    self.scroll_to_caret = true;
                    ui.ctx().request_repaint();
                }
            }
        }
        if !resp.is_pointer_button_down_on() && !resp.dragged() {
            self.dragging = false;
        }

        // Paint.
        painter.rect_filled(body, CornerRadius::ZERO, p.bg_sunken.gamma_multiply(if p.dark { 0.55 } else { 0.35 }));
        painter.rect_filled(gutter, CornerRadius::ZERO, p.bg_surface1);
        let (sa, sb) = self.sel.range();
        let head = self.sel.head;
        let match_color = p.warning.gamma_multiply(0.28);
        let current_match = p.warning.gamma_multiply(0.55);
        let sel_color = p.accent_subtle.gamma_multiply(1.6);
        let first_line = self.rows.line_of(first_row).0;
        let last_line = self.rows.line_of(first_row + n_rows.saturating_sub(1)).0;
        let vis_matches: &[Match] = if self.find.open && !self.find.matches.is_empty() {
            let a = self.find.matches.partition_point(|m| m.line < first_line);
            let b = self.find.matches.partition_point(|m| m.line <= last_line);
            &self.find.matches[a..b]
        } else {
            &[]
        };
        let clip = ui.painter_at(text);
        let num_font = crate::theme::fonts::code(ui.ctx(), (cfg.font_size - 1.0).clamp(6.0, 48.0));
        for i in 0..n_rows {
            let row = first_row + i;
            let y = origin_y + i as f32 * row_h;
            let (g, line, start) = row_galley(self, &mut maps, row);
            let map = &maps[&line];
            let row_end = (start + cols).min(map.disp.len());
            let last_row_of_line = start + cols >= map.disp.len();
            let x_of = |d: usize| -> f32 {
                let d = d.clamp(start, row_end);
                origin_x + g.pos_from_cursor(CCursor::new(d - start)).min.x
            };
            let row_rect = Rect::from_min_size(pos2(text.left(), y), vec2(text.width(), row_h));
            if line == head.line && self.sel.is_empty() && focused {
                clip.rect_filled(row_rect, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.55));
            }
            // Find matches.
            for m in vis_matches.iter().filter(|m| m.line == line) {
                let (da, db) = (map.disp_of(m.start), map.disp_of(m.end));
                if db <= start || da >= row_end.max(start + 1) {
                    continue;
                }
                let r =
                    Rect::from_min_max(pos2(x_of(da), y + 1.0), pos2(x_of(db).max(x_of(da) + 2.0), y + row_h - 1.0));
                let cur = sa == m.start_pos() && sb == m.end_pos();
                clip.rect_filled(r, CornerRadius::same(2), if cur { current_match } else { match_color });
            }
            // Selection.
            if !self.sel.is_empty() && line >= sa.line && line <= sb.line {
                let da = if line == sa.line { map.disp_of(sa.col) } else { 0 };
                let (db, eol) = if line == sb.line { (map.disp_of(sb.col), false) } else { (map.disp.len(), true) };
                if db >= start && da <= row_end && !(da == db && !eol) {
                    let x0 = x_of(da.max(start));
                    let mut x1 = x_of(db.min(row_end));
                    if eol && last_row_of_line {
                        x1 += char_w * 0.6;
                    }
                    clip.rect_filled(
                        Rect::from_min_max(pos2(x0, y), pos2(x1.max(x0 + 1.0), y + row_h)),
                        CornerRadius::same(2),
                        sel_color,
                    );
                }
            }
            clip.galley(pos2(origin_x, y + (row_h - g.size().y) / 2.0), g.clone(), p.text_primary);
            // Caret.
            if line == head.line {
                let d = map.disp_of(head.col);
                let on_row = d >= start && (d < start + cols || last_row_of_line) && d <= row_end;
                if on_row && focused {
                    let x = x_of(d);
                    let cr = Rect::from_min_max(pos2(x - 0.5, y + 2.0), pos2(x + 1.5, y + row_h - 2.0));
                    clip.rect_filled(cr, CornerRadius::ZERO, p.accent);
                    ui.ctx().output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            purpose: egui::IMEPurpose::Normal,
                            rect: text,
                            cursor_rect: cr,
                            should_interrupt_composition: false,
                        })
                    });
                }
            }
            // Line number.
            if start == 0 {
                let color = if line == head.line { p.text_primary } else { p.text_tertiary };
                painter.text(
                    pos2(gutter.right() - char_w * 0.8, y + row_h / 2.0),
                    Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    num_font.clone(),
                    color,
                );
            }
        }
        // Gutter / text divider.
        painter.vline(gutter.right(), body.y_range(), Stroke::new(1.0, p.border_subtle));
        if self.buf.line_count() == 1 && self.buf.line(0).is_empty() && !focused {
            clip.text(
                pos2(text.left(), text.top() + row_h / 2.0),
                Align2::LEFT_CENTER,
                "Empty file",
                FontId::proportional(12.0),
                p.text_tertiary,
            );
        }
    }

    /// Layout job for display cells `[start, end)` of a line.
    #[allow(clippy::too_many_arguments)]
    fn row_job(
        &self,
        line: usize,
        map: &LineMap,
        start: usize,
        end: usize,
        font: &FontId,
        p: &Palette,
        show_ws: bool,
    ) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_rows = 1;
        job.break_on_newline = false;
        let spans = self.hl.get(line);
        // Display-cell span boundaries.
        let mut dspans: Vec<(usize, usize, Color32, bool)> = Vec::new();
        if let Some(sp) = spans {
            for s in sp.iter() {
                let a = map.disp_of(s.start as usize);
                let b = map.disp_of(s.end as usize);
                if b > start && a < end {
                    let mut color = s.color;
                    if !p.dark && color == Color32::from_rgb(0x32, 0x32, 0x32) {
                        color = p.text_primary;
                    }
                    dspans.push((a, b, color, s.italic));
                }
            }
        }
        let ws_color = p.text_tertiary.gamma_multiply(0.7);
        let mut k = 0usize;
        let mut run = String::new();
        let mut run_fmt: Option<(Color32, bool)> = None;
        let flush = |job: &mut LayoutJob, run: &mut String, fmt: Option<(Color32, bool)>| {
            if let Some((color, italics)) = fmt
                && !run.is_empty()
            {
                job.append(run, 0.0, TextFormat { font_id: font.clone(), color, italics, ..Default::default() });
                run.clear();
            }
        };
        for d in start..end {
            while k < dspans.len() && dspans[k].1 <= d {
                k += 1;
            }
            let (mut color, italic) =
                if k < dspans.len() && dspans[k].0 <= d { (dspans[k].2, dspans[k].3) } else { (p.text_primary, false) };
            let mut ch = map.disp[d];
            match map.cells[d] {
                Cell::Space if show_ws => {
                    ch = '·';
                    color = ws_color;
                }
                Cell::TabStart if show_ws => {
                    ch = '→';
                    color = ws_color;
                }
                Cell::Control => color = p.warning,
                _ => {}
            }
            let fmt = Some((color, italic));
            if fmt != run_fmt {
                flush(&mut job, &mut run, run_fmt);
                run_fmt = fmt;
            }
            run.push(ch);
        }
        flush(&mut job, &mut run, run_fmt);
        if job.text.is_empty() {
            job.append("", 0.0, TextFormat { font_id: font.clone(), color: p.text_primary, ..Default::default() });
        }
        job
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
    fn indent_and_outdent_lines() {
        let mut doc = TextDoc::new("x.rs", encoding::decode(b"a\n  b\nc"));
        doc.sel = Sel::new(Pos::new(0, 0), Pos::new(2, 1));
        doc.indent_lines(false, 4);
        assert_eq!(doc.buf.text(), "    a\n      b\n    c");
        doc.indent_lines(true, 4);
        assert_eq!(doc.buf.text(), "a\n  b\nc");
    }
}
