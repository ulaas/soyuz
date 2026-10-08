//! The text editor's document model: a line vector (no line endings; the file's ending is
//! kept separately), byte-column positions, one primitive edit (`replace`) and the
//! multi-level undo history built on it. Pure logic, unit tested.

use std::time::{Duration, Instant};

/// A position: line index and byte offset into that line (always on a char boundary).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(line: usize, col: usize) -> Self {
        Pos { line, col }
    }
}

/// A selection; `head` is the caret. Empty when `anchor == head`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Sel {
    pub anchor: Pos,
    pub head: Pos,
}

impl Sel {
    pub fn caret(p: Pos) -> Self {
        Sel { anchor: p, head: p }
    }

    pub fn new(anchor: Pos, head: Pos) -> Self {
        Sel { anchor, head }
    }

    /// Ordered `(start, end)`.
    pub fn range(&self) -> (Pos, Pos) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Lines `[start, start + removed)` were replaced by `[start, start + inserted)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineChange {
    pub start: usize,
    pub removed: usize,
    pub inserted: usize,
}

#[derive(Clone, Debug)]
pub struct Buffer {
    lines: Vec<String>,
    version: u64,
    changes: Vec<LineChange>,
}

impl Buffer {
    /// From `\n`-separated text (a trailing newline gives a final empty line).
    pub fn new(text: &str) -> Self {
        Buffer { lines: text.split('\n').map(str::to_string).collect(), version: 1, changes: Vec::new() }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Bumped by every edit.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Line ranges changed since the last call (for layout / highlight caches).
    pub fn take_changes(&mut self) -> Vec<LineChange> {
        std::mem::take(&mut self.changes)
    }

    pub fn end(&self) -> Pos {
        let last = self.lines.len() - 1;
        Pos::new(last, self.lines[last].len())
    }

    /// Clamp into the document and onto a char boundary.
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        let s = &self.lines[line];
        let mut col = p.col.min(s.len());
        while !s.is_char_boundary(col) {
            col -= 1;
        }
        Pos::new(line, col)
    }

    /// Text between two positions (`a <= b`), lines joined with `\n`.
    pub fn text_range(&self, a: Pos, b: Pos) -> String {
        let (a, b) = (self.clamp(a), self.clamp(b));
        if a.line == b.line {
            return self.lines[a.line][a.col..b.col.max(a.col)].to_string();
        }
        let mut out = String::from(&self.lines[a.line][a.col..]);
        for l in &self.lines[a.line + 1..b.line] {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        out.push_str(&self.lines[b.line][..b.col]);
        out
    }

    /// The whole document joined with `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replace `[a, b)` with `text`; returns the position after the inserted text.
    pub fn replace(&mut self, a: Pos, b: Pos, text: &str) -> Pos {
        let (a, b) = (self.clamp(a), self.clamp(b));
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        self.version += 1;
        if a.line == b.line && !text.contains('\n') {
            self.lines[a.line].replace_range(a.col..b.col, text);
            self.changes.push(LineChange { start: a.line, removed: 1, inserted: 1 });
            return Pos::new(a.line, a.col + text.len());
        }
        let suffix = self.lines[b.line][b.col..].to_string();
        let mut new: Vec<String> = Vec::new();
        let mut first = self.lines[a.line][..a.col].to_string();
        let mut parts = text.split('\n');
        first.push_str(parts.next().unwrap_or(""));
        new.push(first);
        new.extend(parts.map(str::to_string));
        let last = new.len() - 1;
        let end = Pos::new(a.line + last, new[last].len());
        new[last].push_str(&suffix);
        let inserted = new.len();
        self.lines.splice(a.line..=b.line, new);
        self.changes.push(LineChange { start: a.line, removed: b.line - a.line + 1, inserted });
        end
    }

    /// Position after `text` inserted at `at`.
    pub fn end_after(at: Pos, text: &str) -> Pos {
        match text.rfind('\n') {
            None => Pos::new(at.line, at.col + text.len()),
            Some(i) => Pos::new(at.line + text.matches('\n').count(), text.len() - i - 1),
        }
    }

    /// One character to the left (wrapping to the previous line end).
    pub fn prev_pos(&self, p: Pos) -> Pos {
        let p = self.clamp(p);
        if p.col == 0 {
            if p.line == 0 { p } else { Pos::new(p.line - 1, self.lines[p.line - 1].len()) }
        } else {
            let s = &self.lines[p.line];
            let c = s[..p.col].chars().next_back().map_or(1, char::len_utf8);
            Pos::new(p.line, p.col - c)
        }
    }

    /// One character to the right (wrapping to the next line start).
    pub fn next_pos(&self, p: Pos) -> Pos {
        let p = self.clamp(p);
        let s = &self.lines[p.line];
        if p.col >= s.len() {
            if p.line + 1 >= self.lines.len() { p } else { Pos::new(p.line + 1, 0) }
        } else {
            let c = s[p.col..].chars().next().map_or(1, char::len_utf8);
            Pos::new(p.line, p.col + c)
        }
    }

    /// Start of the previous word (Ctrl+Left).
    pub fn word_left(&self, p: Pos) -> Pos {
        let p = self.clamp(p);
        if p.col == 0 {
            return self.prev_pos(p);
        }
        let s = &self.lines[p.line];
        let chars: Vec<(usize, char)> = s[..p.col].char_indices().collect();
        let mut i = chars.len();
        while i > 0 && chars[i - 1].1.is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let cls = class(chars[i - 1].1);
            while i > 0 && class(chars[i - 1].1) == cls && !chars[i - 1].1.is_whitespace() {
                i -= 1;
            }
        }
        Pos::new(p.line, chars.get(i).map_or(0, |c| c.0))
    }

    /// End of the next word (Ctrl+Right).
    pub fn word_right(&self, p: Pos) -> Pos {
        let p = self.clamp(p);
        let s = &self.lines[p.line];
        if p.col >= s.len() {
            return self.next_pos(p);
        }
        let mut it = s[p.col..].char_indices().peekable();
        while let Some(&(_, c)) = it.peek() {
            if !c.is_whitespace() {
                break;
            }
            it.next();
        }
        if let Some(&(_, c)) = it.peek() {
            let cls = class(c);
            while let Some(&(_, c)) = it.peek() {
                if class(c) != cls || c.is_whitespace() {
                    break;
                }
                it.next();
            }
        }
        Pos::new(p.line, it.peek().map_or(s.len(), |(i, _)| p.col + i))
    }

    /// The word (or run of punctuation / whitespace) around `p` (double-click).
    pub fn word_at(&self, p: Pos) -> (Pos, Pos) {
        let p = self.clamp(p);
        let s = &self.lines[p.line];
        let chars: Vec<(usize, char)> = s.char_indices().collect();
        let idx = chars.iter().position(|c| c.0 >= p.col).unwrap_or(chars.len());
        let probe = if idx < chars.len() {
            idx
        } else if idx > 0 {
            idx - 1
        } else {
            return (p, p);
        };
        let cls = class(chars[probe].1);
        let mut a = probe;
        while a > 0 && class(chars[a - 1].1) == cls {
            a -= 1;
        }
        let mut b = probe;
        while b < chars.len() && class(chars[b].1) == cls {
            b += 1;
        }
        (Pos::new(p.line, chars[a].0), Pos::new(p.line, chars.get(b).map_or(s.len(), |c| c.0)))
    }

    /// Leading whitespace of a line.
    pub fn indent_of(&self, line: usize) -> &str {
        let s = self.line(line);
        &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
    }

    /// The document indents with tabs (more tab- than space-indented lines in a sample).
    pub fn uses_tabs(&self) -> bool {
        let (mut tabs, mut spaces) = (0, 0);
        for l in self.lines.iter().take(2000) {
            if l.starts_with('\t') {
                tabs += 1;
            } else if l.starts_with("  ") {
                spaces += 1;
            }
        }
        tabs > spaces
    }
}

/// Character class for word motion: word chars, whitespace, other.
fn class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        0
    } else if c.is_whitespace() {
        1
    } else {
        2
    }
}

// ---- undo -------------------------------------------------------------------------

/// One replacement: `removed` (starting at `at`) became `inserted`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub at: Pos,
    pub removed: String,
    pub inserted: String,
}

/// How consecutive edits group into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typing characters (grouped per word).
    Typing,
    /// Backspace / Delete of single characters (grouped).
    Deleting,
    /// Anything else (paste, replace, indent…): one step each.
    Other,
}

#[derive(Clone, Debug)]
struct Record {
    id: u64,
    /// In application order.
    edits: Vec<Edit>,
    before: Sel,
    after: Sel,
    kind: EditKind,
    at: Instant,
}

/// Typing / deleting pauses longer than this start a new undo step.
const GROUP_GAP: Duration = Duration::from_millis(1500);
/// Oldest steps are dropped beyond this.
const MAX_STEPS: usize = 10_000;

/// Multi-level undo / redo with a save point (for the dirty indicator).
#[derive(Debug)]
pub struct History {
    undo: Vec<Record>,
    redo: Vec<Record>,
    next_id: u64,
    /// Id of the top undo step when the file was last loaded / saved (0 = none).
    saved: u64,
}

impl Default for History {
    fn default() -> Self {
        History { undo: Vec::new(), redo: Vec::new(), next_id: 1, saved: 0 }
    }
}

impl History {
    fn top_id(&self) -> u64 {
        self.undo.last().map_or(0, |r| r.id)
    }

    /// The document differs from the last loaded / saved state.
    pub fn is_dirty(&self) -> bool {
        self.top_id() != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.top_id();
    }

    /// Id of the current state for a save in flight: later typing never merges into it.
    pub fn seal(&mut self) -> u64 {
        if let Some(r) = self.undo.last_mut() {
            r.kind = EditKind::Other;
        }
        self.top_id()
    }

    /// The state with this id (from [`Self::seal`]) is now on disk.
    pub fn set_saved(&mut self, id: u64) {
        self.saved = id;
    }

    /// Forget everything (reload); the current state is the saved one.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.saved = 0;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Record edits that were just applied.
    pub fn push(&mut self, edits: Vec<Edit>, before: Sel, after: Sel, kind: EditKind, now: Instant) {
        if edits.is_empty() {
            return;
        }
        self.redo.clear();
        let saved = self.saved;
        if let Some(last) = self.undo.last_mut()
            && last.id != saved
            && last.kind == kind
            && kind != EditKind::Other
            && edits.len() == 1
            && last.edits.len() == 1
            && now.duration_since(last.at) < GROUP_GAP
            && merge(&mut last.edits[0], &edits[0], kind)
        {
            last.after = after;
            last.at = now;
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.undo.push(Record { id, edits, before, after, kind, at: now });
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
    }

    /// Undo one step; returns the selection to restore.
    pub fn undo(&mut self, buf: &mut Buffer) -> Option<Sel> {
        let rec = self.undo.pop()?;
        for e in rec.edits.iter().rev() {
            buf.replace(e.at, Buffer::end_after(e.at, &e.inserted), &e.removed);
        }
        let sel = rec.before;
        self.redo.push(rec);
        Some(sel)
    }

    /// Redo one step; returns the selection to restore.
    pub fn redo(&mut self, buf: &mut Buffer) -> Option<Sel> {
        let mut rec = self.redo.pop()?;
        for e in &rec.edits {
            buf.replace(e.at, Buffer::end_after(e.at, &e.removed), &e.inserted);
        }
        let sel = rec.after;
        // A redone step never merges with later typing.
        rec.kind = EditKind::Other;
        self.undo.push(rec);
        Some(sel)
    }
}

/// Try to extend `last` with `new` (single-character typing / deleting).
fn merge(last: &mut Edit, new: &Edit, kind: EditKind) -> bool {
    match kind {
        EditKind::Typing => {
            if !new.removed.is_empty() || !last.removed.is_empty() || new.inserted.contains('\n') {
                return false;
            }
            // Word boundary: a space after a word starts a new step.
            let new_ws = new.inserted.chars().all(char::is_whitespace);
            let last_ws = last.inserted.chars().last().is_some_and(char::is_whitespace);
            if new_ws && !last_ws {
                return false;
            }
            if Buffer::end_after(last.at, &last.inserted) != new.at {
                return false;
            }
            last.inserted.push_str(&new.inserted);
            true
        }
        EditKind::Deleting => {
            if !new.inserted.is_empty() || !last.inserted.is_empty() {
                return false;
            }
            if Buffer::end_after(new.at, &new.removed) == last.at {
                // Backspace: grows to the left.
                last.removed = format!("{}{}", new.removed, last.removed);
                last.at = new.at;
                true
            } else if new.at == last.at {
                // Delete: grows to the right.
                last.removed.push_str(&new.removed);
                true
            } else {
                false
            }
        }
        EditKind::Other => false,
    }
}

/// Apply replacements given in *descending* position order (so earlier ones stay
/// valid) and return the edits in application order.
pub fn apply_all(buf: &mut Buffer, ranges: Vec<(Pos, Pos, String)>) -> Vec<Edit> {
    let mut edits = Vec::with_capacity(ranges.len());
    for (a, b, text) in ranges {
        let (a, b) = (buf.clamp(a), buf.clamp(b));
        let removed = buf.text_range(a, b);
        if removed == text {
            continue;
        }
        buf.replace(a, b, &text);
        edits.push(Edit { at: a, removed, inserted: text });
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: usize, col: usize) -> Pos {
        Pos::new(line, col)
    }

    #[test]
    fn replace_and_ranges() {
        let mut b = Buffer::new("hello\nworld\n");
        assert_eq!(b.line_count(), 3);
        let end = b.replace(p(0, 5), p(1, 0), ", ");
        assert_eq!(end, p(0, 7));
        assert_eq!(b.text(), "hello, world\n");
        let end = b.replace(p(0, 5), p(0, 5), "\nbig\n");
        assert_eq!(end, p(2, 0));
        assert_eq!(b.text(), "hello\nbig\n, world\n");
        assert_eq!(b.text_range(p(0, 3), p(2, 1)), "lo\nbig\n,");
        assert_eq!(Buffer::end_after(p(4, 2), "ab\ncde"), p(5, 3));
        assert_eq!(Buffer::end_after(p(4, 2), "ab"), p(4, 4));
        let ch = b.take_changes();
        assert_eq!(ch[0], LineChange { start: 0, removed: 2, inserted: 1 });
        assert_eq!(ch[1], LineChange { start: 0, removed: 1, inserted: 3 });
        assert!(b.take_changes().is_empty());
    }

    #[test]
    fn motion_respects_utf8_and_words() {
        let b = Buffer::new("aé€ x\nfoo_bar.baz  qux");
        assert_eq!(b.next_pos(p(0, 1)), p(0, 3));
        assert_eq!(b.prev_pos(p(0, 6)), p(0, 3));
        assert_eq!(b.next_pos(p(0, 8)), p(1, 0));
        assert_eq!(b.prev_pos(p(1, 0)), p(0, 8));
        assert_eq!(b.clamp(p(0, 2)), p(0, 1));
        assert_eq!(b.word_right(p(1, 0)), p(1, 7));
        assert_eq!(b.word_right(p(1, 7)), p(1, 8));
        assert_eq!(b.word_left(p(1, 16)), p(1, 13));
        assert_eq!(b.word_left(p(1, 7)), p(1, 0));
        assert_eq!(b.word_at(p(1, 2)), (p(1, 0), p(1, 7)));
        assert_eq!(b.indent_of(1), "");
    }

    #[test]
    fn undo_redo_multi_level_and_dirty() {
        let mut b = Buffer::new("abc");
        let mut h = History::default();
        let t = Instant::now();
        assert!(!h.is_dirty());
        // Type "xy" (grouped), then paste "Z" (own step).
        for (i, ch) in ["x", "y"].iter().enumerate() {
            let at = p(0, 3 + i);
            let e = apply_all(&mut b, vec![(at, at, ch.to_string())]);
            h.push(e, Sel::caret(at), Sel::caret(p(0, 4 + i)), EditKind::Typing, t);
        }
        let e = apply_all(&mut b, vec![(p(0, 0), p(0, 1), "Z".into())]);
        h.push(e, Sel::caret(p(0, 0)), Sel::caret(p(0, 1)), EditKind::Other, t);
        assert_eq!(b.text(), "Zbcxy");
        assert!(h.is_dirty());
        assert_eq!(h.undo(&mut b), Some(Sel::caret(p(0, 0))));
        assert_eq!(b.text(), "abcxy");
        assert_eq!(h.undo(&mut b), Some(Sel::caret(p(0, 3))));
        assert_eq!(b.text(), "abc");
        assert!(!h.is_dirty(), "back at the loaded state");
        assert_eq!(h.undo(&mut b), None);
        h.redo(&mut b);
        h.redo(&mut b);
        assert_eq!(b.text(), "Zbcxy");
        h.mark_saved();
        assert!(!h.is_dirty());
        h.undo(&mut b);
        assert!(h.is_dirty());
        // A new edit clears the redo stack.
        let e = apply_all(&mut b, vec![(p(0, 0), p(0, 0), "!".into())]);
        h.push(e, Sel::default(), Sel::default(), EditKind::Other, t);
        assert!(!h.can_redo());
        assert!(h.is_dirty());
    }

    #[test]
    fn typing_groups_by_word_and_never_merges_into_saved_step() {
        let mut b = Buffer::new("");
        let mut h = History::default();
        let t = Instant::now();
        let mut col = 0;
        for ch in "hi yo".chars() {
            let at = p(0, col);
            let e = apply_all(&mut b, vec![(at, at, ch.to_string())]);
            col += 1;
            h.push(e, Sel::caret(at), Sel::caret(p(0, col)), EditKind::Typing, t);
            if ch == 'i' {
                h.mark_saved();
            }
        }
        assert_eq!(b.text(), "hi yo");
        h.undo(&mut b);
        assert_eq!(b.text(), "hi", "\" yo\" is one step after the save point");
        assert!(!h.is_dirty());
        h.undo(&mut b);
        assert_eq!(b.text(), "");
        // Backspaces group too.
        let mut b = Buffer::new("abcd");
        let mut h = History::default();
        for col in (1..=4).rev() {
            let e = apply_all(&mut b, vec![(p(0, col - 1), p(0, col), String::new())]);
            h.push(e, Sel::caret(p(0, col)), Sel::caret(p(0, col - 1)), EditKind::Deleting, t);
        }
        assert_eq!(b.text(), "");
        h.undo(&mut b);
        assert_eq!(b.text(), "abcd");
    }

    #[test]
    fn multi_edit_step_undoes_in_reverse() {
        // Replace-all style: bottom-up edits that change line counts.
        let mut b = Buffer::new("a\nb\na\nb");
        let mut h = History::default();
        let e = apply_all(&mut b, vec![(p(2, 0), p(2, 1), "x\ny".into()), (p(0, 0), p(0, 1), "x\ny".into())]);
        h.push(e, Sel::default(), Sel::default(), EditKind::Other, Instant::now());
        assert_eq!(b.text(), "x\ny\nb\nx\ny\nb");
        h.undo(&mut b);
        assert_eq!(b.text(), "a\nb\na\nb");
        h.redo(&mut b);
        assert_eq!(b.text(), "x\ny\nb\nx\ny\nb");
    }
}
