//! Terminal cells of one text line: grapheme clusters measured with `unicode-width`
//! (CJK and emoji take two cells), tabs expanded to the next stop, control characters
//! shown as `¤`. The text editor's view and its keys (vertical motion, clicks) both use
//! [`LineLayout`], so the caret always sits where the character is drawn.
//!
//! The core's own text model counts one cell per character (the GUI measures glyphs
//! itself); the terminal UI therefore lays rows out here and keeps its own scroll
//! position (`crate::input::editor::DocView`).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// What a cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A visible grapheme cluster.
    Text,
    /// A space (drawn as `·` with visible whitespace).
    Space,
    /// A tab, expanded to the next tab stop (drawn as `→` with visible whitespace).
    Tab,
    /// A control or zero-width character, drawn as `¤`.
    Control,
}

/// One grapheme cluster of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Byte offset in the line.
    pub byte: usize,
    /// Bytes of the cluster.
    pub len: usize,
    /// First display column.
    pub col: usize,
    /// Cells it takes (at least one).
    pub width: usize,
    pub kind: Kind,
}

/// The cells of one line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineLayout {
    pub glyphs: Vec<Glyph>,
    /// Display width of the whole line.
    pub width: usize,
    /// Byte length of the line.
    pub len: usize,
}

/// The symbol drawn for control / zero-width characters.
pub const CONTROL: &str = "¤";

impl LineLayout {
    /// Lay out `line` with tab stops every `tab` columns.
    pub fn new(line: &str, tab: usize) -> LineLayout {
        let tab = tab.max(1);
        let mut glyphs = Vec::with_capacity(line.len());
        let mut col = 0;
        for (byte, g) in line.grapheme_indices(true) {
            let (kind, width) = if g == "\t" {
                (Kind::Tab, tab - col % tab)
            } else if g == " " {
                (Kind::Space, 1)
            } else if g.chars().any(char::is_control) {
                (Kind::Control, 1)
            } else {
                match g.width() {
                    0 => (Kind::Control, 1),
                    w => (Kind::Text, w),
                }
            };
            glyphs.push(Glyph { byte, len: g.len(), col, width, kind });
            col += width;
        }
        LineLayout { glyphs, width: col, len: line.len() }
    }

    /// Index of the glyph that contains byte `b` (`glyphs.len()` at the end of the line).
    pub fn glyph_at_byte(&self, b: usize) -> usize {
        self.glyphs.partition_point(|g| g.byte + g.len <= b)
    }

    /// The display column of byte offset `b` (the column of the cluster it falls in).
    pub fn col_of(&self, b: usize) -> usize {
        self.glyphs.get(self.glyph_at_byte(b)).map_or(self.width, |g| g.col)
    }

    /// The byte offset for display column `col`: the cluster drawn there (inside a tab:
    /// the nearer end), the line end past the last one.
    pub fn byte_at(&self, col: usize) -> usize {
        let i = self.glyphs.partition_point(|g| g.col + g.width <= col);
        match self.glyphs.get(i) {
            None => self.len,
            Some(g) if g.kind == Kind::Tab && col - g.col >= g.width.div_ceil(2) => g.byte + g.len,
            Some(g) => g.byte,
        }
    }

    /// The visual rows of the line `cols` cells wide (soft wrap): the index of each row's
    /// first glyph. Always at least one row; a row never splits a cluster.
    pub fn rows(&self, cols: usize) -> Vec<usize> {
        let cols = cols.max(1);
        let mut out = vec![0];
        let mut start = 0;
        for (i, g) in self.glyphs.iter().enumerate() {
            if i > *out.last().unwrap_or(&0) && g.col + g.width > start + cols {
                out.push(i);
                start = g.col;
            }
        }
        out
    }

    /// The display column where row `r` of `rows` starts.
    pub fn row_col(&self, rows: &[usize], r: usize) -> usize {
        rows.get(r).and_then(|&i| self.glyphs.get(i)).map_or(if r == 0 { 0 } else { self.width }, |g| g.col)
    }

    /// The row (of `rows`) that shows byte offset `b`.
    pub fn row_of(&self, rows: &[usize], b: usize) -> usize {
        let gi = self.glyph_at_byte(b);
        rows.partition_point(|&s| s <= gi).saturating_sub(1)
    }

    /// The byte offset in row `r` at column `x` of that row, staying inside the row.
    pub fn byte_in_row(&self, rows: &[usize], r: usize, x: usize) -> usize {
        let start = self.row_col(rows, r);
        let target = start + x;
        if let Some(&next) = rows.get(r + 1) {
            // Not past the row's end: the last cluster of the row.
            let next_col = self.glyphs.get(next).map_or(self.width, |g| g.col);
            if target >= next_col {
                return self.glyphs.get(next.saturating_sub(1)).map_or(0, |g| g.byte);
            }
        }
        self.byte_at(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_characters_and_tabs() {
        let l = LineLayout::new("a日b\tc", 4);
        let cols: Vec<(usize, usize)> = l.glyphs.iter().map(|g| (g.col, g.width)).collect();
        assert_eq!(cols, vec![(0, 1), (1, 2), (3, 1), (4, 4), (8, 1)]);
        assert_eq!(l.width, 9);
        assert_eq!(l.col_of(1), 1, "before 日");
        assert_eq!(l.col_of(4), 3, "after 日 (3 bytes)");
        assert_eq!(l.col_of(2), 1, "inside 日: its column");
        assert_eq!(l.col_of(l.len), 9);
        assert_eq!(l.byte_at(2), 1, "the right half of 日 is 日");
        assert_eq!(l.byte_at(5), 5, "inside the tab, nearer its start");
        assert_eq!(l.byte_at(7), 6, "inside the tab, nearer its end");
        assert_eq!(l.byte_at(99), l.len);
    }

    #[test]
    fn emoji_combining_marks_and_controls() {
        let l = LineLayout::new("e\u{301}👍\u{1b}x", 4);
        let kinds: Vec<(Kind, usize)> = l.glyphs.iter().map(|g| (g.kind, g.width)).collect();
        assert_eq!(kinds, vec![(Kind::Text, 1), (Kind::Text, 2), (Kind::Control, 1), (Kind::Text, 1)]);
        assert_eq!(l.width, 5);
        assert_eq!(l.col_of(1), 0, "the combining mark belongs to its letter");
    }

    #[test]
    fn soft_wrap_never_splits_a_cluster() {
        let l = LineLayout::new("ab日本語", 4);
        // Columns: a0 b1 日2-3 本4-5 語6-7, 5 wide: 本 starts the second row.
        let rows = l.rows(5);
        assert_eq!(rows, vec![0, 3]);
        assert_eq!(l.row_col(&rows, 1), 4);
        assert_eq!(l.row_of(&rows, 0), 0);
        assert_eq!(l.row_of(&rows, l.glyphs[3].byte), 1);
        assert_eq!(l.byte_in_row(&rows, 0, 9), l.glyphs[2].byte, "clamped to the row's last cluster");
        assert_eq!(l.byte_in_row(&rows, 1, 9), l.len);
        assert_eq!(LineLayout::new("", 4).rows(5), vec![0]);
        assert_eq!(LineLayout::new("日", 4).rows(1), vec![0], "a cluster wider than the row stays");
    }
}
