//! A multi-line text editor for the AI composer and the plan JSON dialog
//! (`TuiDialog::PlanJson`): the multi-line sibling of [`crate::widgets::LineEdit`].
//!
//! [`TextArea`] keeps the cursor, the wanted column of vertical moves and the scroll
//! position; the text is a borrowed `&mut String` owned elsewhere (the core's
//! `panel.composer`, the dialog's state). Lines end with `\n`; pasted `\r\n` / `\r` become
//! `\n`, a tab becomes four spaces and other control characters are dropped.
//!
//! Layout: with `wrap = Some(width)` long lines wrap at word boundaries (the composer);
//! with `None` every line is one row and the view scrolls sideways (JSON). Rows are byte
//! ranges of the text ([`rows`]).
//!
//! Keys ([`TextArea::apply`]): typing and paste insert, Backspace / Delete remove a
//! grapheme (with Ctrl: a word), Left / Right move by a grapheme (Ctrl: a word) across
//! lines, Up / Down move between rows keeping the column, Home / End go to the start /
//! end of the line (with Ctrl: of the text). Enter, Esc, Tab, Shift+Tab, PageUp and
//! PageDown end the run with an [`EditOutcome`] for the caller, and so do Up on the first
//! row and Down on the last one. Alt chords and other Ctrl chords are left alone: the
//! caller decides what Enter means (send, or a new line with [`TextArea::newline`]).

use soyuz_appcore::keys::input::InputEvent;
use soyuz_core::config::{KeyChord, KeyName, Mods};
use unicode_segmentation::UnicodeSegmentation;

use crate::view::text;
use crate::widgets::EditOutcome;

/// Spaces a typed or pasted tab becomes.
const TAB: &str = "    ";

/// One row of the laid-out text: `text[start..end]` (without the line's `\n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    /// The last row of its line (a cursor at `end` stays on this row).
    pub last: bool,
}

/// The rows of `text`: one per line, more where a line is wider than `wrap` cells
/// (broken after a space when there is one in the row, else between graphemes).
pub fn rows(text: &str, wrap: Option<usize>) -> Vec<Row> {
    let mut out = Vec::new();
    let mut ls = 0;
    for line in text.split('\n') {
        let le = ls + line.len();
        match wrap {
            Some(w) if w > 0 && text::width(line) > w => wrap_line(line, ls, w, &mut out),
            _ => out.push(Row { start: ls, end: le, last: true }),
        }
        ls = le + 1;
    }
    out
}

fn wrap_line(line: &str, base: usize, w: usize, out: &mut Vec<Row>) {
    let mut start = 0;
    let mut x = 0;
    // The end of the last space in the row and the width up to it.
    let mut space: Option<(usize, usize)> = None;
    for (i, g) in line.grapheme_indices(true) {
        let gw = text::width(g);
        if x + gw > w && i > start {
            let cut = match space {
                Some((s, sx)) if s > start && s <= i => {
                    x -= sx;
                    s
                }
                _ => {
                    x = 0;
                    i
                }
            };
            out.push(Row { start: base + start, end: base + cut, last: false });
            start = cut;
            space = None;
        }
        x += gw;
        if g == " " {
            space = Some((i + g.len(), x));
        }
    }
    out.push(Row { start: base + start, end: base + line.len(), last: true });
}

/// The row of byte `pos`.
fn row_of(rows: &[Row], pos: usize) -> usize {
    rows.iter()
        .position(|r| r.start <= pos && (pos < r.end || (pos == r.end && r.last)))
        .unwrap_or(rows.len().saturating_sub(1))
}

/// The byte of row `r` whose display column is closest to `col` (not after it); never the
/// end of a row that is not the last of its line (that is the next row's start).
fn at_col(text: &str, r: Row, col: usize) -> usize {
    let mut x = 0;
    let mut best = r.start;
    for (i, g) in text[r.start..r.end].grapheme_indices(true) {
        let gw = text::width(g);
        if x + gw > col {
            return r.start + i;
        }
        x += gw;
        best = r.start + i + g.len();
    }
    if !r.last && best == r.end {
        // Before the last grapheme of a wrapped row.
        text[r.start..r.end].grapheme_indices(true).next_back().map_or(r.start, |(i, _)| r.start + i)
    } else {
        best
    }
}

/// Keys that type a character (their `Text` event follows the chord).
fn types(key: KeyName) -> bool {
    use KeyName as N;
    let letters = key.as_str().len() == 1 && key.as_str().chars().all(|c| c.is_ascii_alphanumeric());
    letters
        || matches!(
            key,
            N::Space
                | N::Backtick
                | N::Minus
                | N::Plus
                | N::Equals
                | N::Comma
                | N::Period
                | N::Slash
                | N::Backslash
                | N::Semicolon
                | N::Quote
                | N::OpenBracket
                | N::CloseBracket
        )
}

/// Word characters for Ctrl+Left / Right / Backspace / Delete.
fn is_word(g: &str) -> bool {
    g.chars().next().is_some_and(char::is_alphanumeric)
}

/// `s` as the text area keeps it: `\n` line ends, tabs as spaces, no control characters.
pub fn clean(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push('\n'),
            '\t' => out.push_str(TAB),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Cursor and scroll of a multi-line field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextArea {
    /// Byte offset in the text (on a grapheme boundary).
    cursor: usize,
    /// The column Up / Down keep (set by the first vertical move).
    want: Option<usize>,
    /// First row shown.
    top: usize,
    /// First display column shown (no wrapping).
    left: usize,
}

impl TextArea {
    pub fn new() -> TextArea {
        TextArea::default()
    }

    /// A field with the cursor at the end of `text`.
    pub fn at_end(text: &str) -> TextArea {
        TextArea { cursor: text.len(), ..TextArea::default() }
    }

    /// The cursor as a byte offset.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Put the cursor at byte `pos` (clamped, moved back to a grapheme boundary).
    pub fn set_cursor(&mut self, text: &str, pos: usize) {
        let pos = pos.min(text.len());
        self.cursor = if pos == text.len() {
            pos
        } else {
            text.grapheme_indices(true).map(|(i, _)| i).take_while(|&i| i <= pos).last().unwrap_or(0)
        };
        self.want = None;
    }

    fn clamp(&mut self, text: &str) {
        if self.cursor > text.len() || !text.is_char_boundary(self.cursor) {
            self.set_cursor(text, self.cursor);
        }
    }

    fn prev(&self, text: &str) -> usize {
        text[..self.cursor].grapheme_indices(true).next_back().map_or(0, |(i, _)| i)
    }

    fn next(&self, text: &str) -> usize {
        text[self.cursor..].graphemes(true).next().map_or(self.cursor, |g| self.cursor + g.len())
    }

    fn prev_word(&self, text: &str) -> usize {
        let mut gs = text[..self.cursor].grapheme_indices(true).rev().peekable();
        while gs.next_if(|(_, g)| !is_word(g)).is_some() {}
        let mut at = gs.peek().map_or(0, |(i, _)| *i);
        while let Some((i, _)) = gs.next_if(|(_, g)| is_word(g)) {
            at = i;
        }
        at
    }

    fn next_word(&self, text: &str) -> usize {
        let mut gs = text[self.cursor..].grapheme_indices(true).peekable();
        while gs.next_if(|(_, g)| !is_word(g)).is_some() {}
        while gs.next_if(|(_, g)| is_word(g)).is_some() {}
        gs.peek().map_or(text.len(), |(i, _)| self.cursor + i)
    }

    /// Insert `s` at the cursor (cleaned: see [`clean`]).
    pub fn insert(&mut self, text: &mut String, s: &str) {
        self.clamp(text);
        let s = clean(s);
        text.insert_str(self.cursor, &s);
        self.cursor += s.len();
        self.want = None;
    }

    /// Start a new line at the cursor.
    pub fn newline(&mut self, text: &mut String) {
        self.insert(text, "\n");
    }

    /// The cursor's row and display column.
    pub fn position(&self, text: &str, wrap: Option<usize>) -> (usize, usize) {
        let rows = rows(text, wrap);
        let c = self.cursor.min(text.len());
        let r = row_of(&rows, c);
        let row = rows[r];
        (r, text::width(&text[row.start..c.clamp(row.start, row.end)]))
    }

    /// Move `n` rows down (negative: up), keeping the column. Returns whether it moved.
    pub fn move_rows(&mut self, text: &str, wrap: Option<usize>, n: i64) -> bool {
        self.clamp(text);
        let rows = rows(text, wrap);
        let (r, col) = self.position(text, wrap);
        let want = *self.want.get_or_insert(col);
        let target = (r as i64 + n).clamp(0, rows.len() as i64 - 1) as usize;
        if target == r {
            return false;
        }
        self.cursor = at_col(text, rows[target], want);
        true
    }

    fn line_start(&self, text: &str) -> usize {
        text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, text: &str) -> usize {
        text[self.cursor..].find('\n').map_or(text.len(), |i| self.cursor + i)
    }

    fn chord(&mut self, text: &mut String, c: KeyChord, wrap: Option<usize>) -> EditOutcome {
        use EditOutcome as O;
        let ctrl = c.mods.intersects(Mods::CTRL | Mods::CMD);
        let shift = c.mods.contains(Mods::SHIFT);
        if c.mods.contains(Mods::ALT) {
            return O::Ignored;
        }
        let vertical = matches!(c.key, KeyName::ArrowUp | KeyName::ArrowDown);
        match c.key {
            KeyName::Backspace => {
                let from = if ctrl { self.prev_word(text) } else { self.prev(text) };
                text.replace_range(from..self.cursor, "");
                self.cursor = from;
            }
            KeyName::Delete => {
                let to = if ctrl { self.next_word(text) } else { self.next(text) };
                text.replace_range(self.cursor..to, "");
            }
            KeyName::ArrowLeft => self.cursor = if ctrl { self.prev_word(text) } else { self.prev(text) },
            KeyName::ArrowRight => self.cursor = if ctrl { self.next_word(text) } else { self.next(text) },
            KeyName::Home => self.cursor = if ctrl { 0 } else { self.line_start(text) },
            KeyName::End => self.cursor = if ctrl { text.len() } else { self.line_end(text) },
            KeyName::ArrowUp if !ctrl => {
                if !self.move_rows(text, wrap, -1) {
                    return O::Up;
                }
            }
            KeyName::ArrowDown if !ctrl => {
                if !self.move_rows(text, wrap, 1) {
                    return O::Down;
                }
            }
            _ if ctrl => return O::Ignored,
            KeyName::Enter => return O::Submit,
            KeyName::Escape => return O::Cancel,
            KeyName::Tab if shift => return O::ShiftTab,
            KeyName::Tab => return O::Tab,
            KeyName::PageUp => return O::PageUp,
            KeyName::PageDown => return O::PageDown,
            k if types(k) => return O::Consumed,
            _ => return O::Ignored,
        }
        if !vertical {
            self.want = None;
        }
        O::Consumed
    }

    /// Apply one event to `text` (`wrap`: the width rows wrap at, for Up / Down).
    pub fn apply(&mut self, text: &mut String, ev: &InputEvent, wrap: Option<usize>) -> EditOutcome {
        self.clamp(text);
        match ev {
            InputEvent::Chord(c) => self.chord(text, *c, wrap),
            InputEvent::Text(s) => {
                self.insert(text, s);
                EditOutcome::Consumed
            }
            InputEvent::Other => EditOutcome::Ignored,
        }
    }

    /// Scroll so the cursor shows in a `width` × `height` view (`wrap`: see [`rows`]).
    pub fn scroll_to_cursor(&mut self, text: &str, wrap: Option<usize>, width: usize, height: usize) {
        self.clamp(text);
        let n = rows(text, wrap).len();
        let (r, col) = self.position(text, wrap);
        let h = height.max(1);
        self.top = self.top.min(n.saturating_sub(h));
        if r < self.top {
            self.top = r;
        } else if r >= self.top + h {
            self.top = r + 1 - h;
        }
        if wrap.is_some() || width == 0 {
            self.left = 0;
        } else if col < self.left {
            self.left = col;
        } else if col >= self.left + width {
            self.left = col + 1 - width;
        }
    }

    /// What a `width` × `height` view shows (after [`TextArea::scroll_to_cursor`]): the
    /// visible rows and the cursor's (row, column) in them.
    pub fn view(&self, text: &str, wrap: Option<usize>, width: usize, height: usize) -> (Vec<String>, (u16, u16)) {
        let mut me = self.clone();
        me.scroll_to_cursor(text, wrap, width, height);
        let all = rows(text, wrap);
        let lines = all
            .iter()
            .skip(me.top)
            .take(height)
            .map(|r| {
                let mut out = String::new();
                let mut x = 0;
                for g in text[r.start..r.end].graphemes(true) {
                    let gw = text::width(g);
                    if x >= me.left && x + gw <= me.left + width {
                        out.push_str(g);
                    } else if x + gw > me.left + width {
                        break;
                    }
                    x += gw;
                }
                out
            })
            .collect();
        let (r, col) = me.position(text, wrap);
        (lines, (r.saturating_sub(me.top) as u16, col.saturating_sub(me.left) as u16))
    }

    /// A click on visible row `row`, display column `col` of the view.
    pub fn click(&mut self, text: &str, wrap: Option<usize>, row: usize, col: usize) {
        self.clamp(text);
        let all = rows(text, wrap);
        let r = (self.top + row).min(all.len().saturating_sub(1));
        self.cursor = at_col(text, all[r], col + self.left);
        self.want = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn txt(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
    }

    fn run(text: &str, cursor: usize, wrap: Option<usize>, events: &[InputEvent]) -> (String, TextArea, EditOutcome) {
        let mut t = text.to_string();
        let mut a = TextArea::new();
        a.set_cursor(&t, cursor);
        let mut out = EditOutcome::Ignored;
        for e in events {
            out = a.apply(&mut t, e, wrap);
        }
        (t, a, out)
    }

    #[test]
    fn paste_keeps_newlines_and_cleans_the_rest() {
        let (t, a, _) = run("", 0, None, &[txt("a\r\nb\rc\td\u{7}")]);
        assert_eq!(t, "a\nb\nc    d");
        assert_eq!(a.cursor(), t.len());
        let mut t = "ab".to_string();
        let mut a = TextArea::at_end(&t);
        a.set_cursor(&t, 1);
        a.newline(&mut t);
        assert_eq!(t, "a\nb");
        assert_eq!(a.cursor(), 2);
    }

    #[test]
    fn moves_across_lines() {
        let s = "first\nab\nthird line";
        // Left at the start of a line goes to the end of the line before.
        let (_, a, _) = run(s, 6, None, &[ch("Left")]);
        assert_eq!(a.cursor(), 5);
        // Down keeps the column (clamped on a short line, restored after it).
        let (_, a, _) = run(s, 4, None, &[ch("Down")]);
        assert_eq!(a.cursor(), 8, "end of `ab`");
        let (_, a, _) = run(s, 4, None, &[ch("Down"), ch("Down")]);
        assert_eq!(a.cursor(), 9 + 4, "column 4 of `third line`");
        // Up on the first row and Down on the last end the run.
        assert_eq!(run(s, 2, None, &[ch("Up")]).2, EditOutcome::Up);
        assert_eq!(run(s, s.len(), None, &[ch("Down")]).2, EditOutcome::Down);
        // Home / End: the line; Ctrl: the text.
        let (_, a, _) = run(s, 7, None, &[ch("Home")]);
        assert_eq!(a.cursor(), 6);
        let (_, a, _) = run(s, 6, None, &[ch("End")]);
        assert_eq!(a.cursor(), 8);
        let (_, a, _) = run(s, 7, None, &[ch("Ctrl+End")]);
        assert_eq!(a.cursor(), s.len());
        let (_, a, _) = run(s, 7, None, &[ch("Ctrl+Home")]);
        assert_eq!(a.cursor(), 0);
        // Backspace joins lines.
        let (t, _, _) = run(s, 6, None, &[ch("Backspace")]);
        assert_eq!(t, "firstab\nthird line");
    }

    #[test]
    fn caller_keys_end_the_run() {
        for (k, want) in [
            ("Enter", EditOutcome::Submit),
            ("Esc", EditOutcome::Cancel),
            ("Tab", EditOutcome::Tab),
            ("Shift+Tab", EditOutcome::ShiftTab),
            ("PageUp", EditOutcome::PageUp),
            ("PageDown", EditOutcome::PageDown),
        ] {
            assert_eq!(run("x", 1, None, &[ch(k)]).2, want, "{k}");
        }
        for k in ["Alt+Enter", "Ctrl+J", "Ctrl+S", "Ctrl+E", "Alt+S", "F5"] {
            let (t, _, out) = run("x", 1, None, &[ch(k)]);
            assert_eq!((t.as_str(), out), ("x", EditOutcome::Ignored), "{k}");
        }
    }

    #[test]
    fn wrapping_rows_and_vertical_moves() {
        let s = "hello big world";
        let r = rows(s, Some(8));
        let parts: Vec<&str> = r.iter().map(|r| &s[r.start..r.end]).collect();
        assert_eq!(parts, ["hello ", "big ", "world"]);
        assert!(!r[0].last && r[2].last);
        // Without a space the line breaks between graphemes.
        let r = rows("abcdefghij", Some(4));
        assert_eq!(r.len(), 3);
        // A cursor at a wrap point belongs to the next row.
        let mut a = TextArea::new();
        a.set_cursor(s, 6);
        assert_eq!(a.position(s, Some(8)), (1, 0));
        // Up from `world` goes to `big`, up again to `hello`.
        a.set_cursor(s, s.len());
        assert!(a.move_rows(s, Some(8), -1));
        assert_eq!(a.position(s, Some(8)).0, 1);
        assert!(a.move_rows(s, Some(8), -1));
        assert_eq!(a.position(s, Some(8)).0, 0);
        // Wide characters wrap whole.
        let r = rows("日本語テキスト", Some(5));
        assert!(r.iter().all(|r| text::width(&"日本語テキスト"[r.start..r.end]) <= 5));
    }

    #[test]
    fn view_scrolls_both_ways() {
        let s = "0\n1\n2\n3\n4\n5";
        let mut a = TextArea::at_end(s);
        a.scroll_to_cursor(s, None, 10, 3);
        let (lines, cur) = a.view(s, None, 10, 3);
        assert_eq!(lines, ["3", "4", "5"]);
        assert_eq!(cur, (2, 1));
        // Clicks land in the visible rows.
        a.click(s, None, 0, 0);
        assert_eq!(a.cursor(), 6);
        // Sideways without wrapping.
        let s = "0123456789";
        let mut a = TextArea::at_end(s);
        a.scroll_to_cursor(s, None, 4, 1);
        let (lines, cur) = a.view(s, None, 4, 1);
        assert_eq!((lines[0].as_str(), cur), ("789", (0, 3)));
        // Empty text and zero sizes never panic.
        let a = TextArea::new();
        assert_eq!(a.view("", Some(0), 0, 0).1, (0, 0));
        assert_eq!(rows("", Some(3)).len(), 1);
    }
}
