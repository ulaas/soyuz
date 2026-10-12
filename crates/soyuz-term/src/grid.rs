//! Toolkit-free view of the terminal screen: [`GridSnapshot`] (the visible cells with
//! their resolved colours, the cursor, the selection and the scroll position) for front
//! ends that draw the grid themselves (the terminal UI), plus link detection
//! ([`TermLink`]: OSC 8 hyperlinks and plain-text URLs) shared with the egui widget.
//!
//! No alacritty type appears in the public API: cells carry [`TermColor`]s resolved by
//! [`crate::color`] (the same rules the egui widget paints with) and their own flags.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, point_to_viewport};
use alacritty_terminal::vte::ansi::CursorShape as VteCursorShape;

use crate::backend::{Listener, is_spacer};
use crate::color::{Resolver, TermColor, TermPalette};
use crate::selection::row_span;
use crate::url;

/// The cursor shape a program asked for (DECSCUSR), or hidden (DECTCEM off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
    /// An outline (the unfocused look some programs request).
    HollowBlock,
    Hidden,
}

impl CursorShape {
    fn from_vte(s: VteCursorShape) -> Self {
        match s {
            VteCursorShape::Block => CursorShape::Block,
            VteCursorShape::Beam => CursorShape::Beam,
            VteCursorShape::Underline => CursorShape::Underline,
            VteCursorShape::HollowBlock => CursorShape::HollowBlock,
            VteCursorShape::Hidden => CursorShape::Hidden,
        }
    }
}

/// How [`crate::Terminal::snapshot`] resolves colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridStyle {
    /// Bold text in ANSI 0-7 uses the bright variant (the egui widget's default).
    pub bright_bold: bool,
}

impl Default for GridStyle {
    fn default() -> Self {
        GridStyle { bright_bold: true }
    }
}

/// One cell of the visible grid, colours resolved (bold brightening, dim, inverse and
/// hidden already applied; [`TermColor`] channels as given by the palette).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridCell {
    /// The character (`' '` for empty cells; hidden text is reported as `' '`).
    pub c: char,
    /// Combining characters drawn on top of `c` (usually empty).
    pub zerowidth: String,
    /// A double-width character: it covers this cell and the spacer to its right.
    pub wide: bool,
    /// The right half of a wide character (or the filler before a wrapped wide one):
    /// nothing to draw.
    pub spacer: bool,
    pub fg: TermColor,
    pub bg: TermColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// The OSC 8 hyperlink this cell belongs to.
    pub hyperlink: Option<String>,
}

/// The cursor position in viewport cells and its shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GridCursor {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
}

/// The visible part of a terminal (`rows × cols` cells, row-major).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<GridCell>,
    /// `None` when the cursor is outside the viewport (scrolled back). A hidden cursor is
    /// reported with [`CursorShape::Hidden`].
    pub cursor: Option<GridCursor>,
    /// Lines scrolled back into the history (0 = at the bottom).
    pub display_offset: usize,
    /// Lines of scrollback available.
    pub history: usize,
    /// Selected cells: `(row, first col, last col)`, inclusive, one entry per row.
    pub selection: Vec<(usize, usize, usize)>,
    /// The background of cells without an explicit colour (a program's OSC 11 colour,
    /// else the palette's).
    pub default_bg: TermColor,
    /// The cursor colour (a program's OSC 12 colour, else the palette's).
    pub cursor_color: TermColor,
}

impl GridSnapshot {
    /// The cells of viewport row `r` (empty when out of range).
    pub fn row(&self, r: usize) -> &[GridCell] {
        if r >= self.rows {
            return &[];
        }
        &self.cells[r * self.cols..(r + 1) * self.cols]
    }

    /// The cell at viewport `(row, col)`.
    pub fn cell(&self, row: usize, col: usize) -> Option<&GridCell> {
        (col < self.cols).then(|| self.row(row).get(col)).flatten()
    }

    /// Whether viewport `(row, col)` is selected.
    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        self.selection.iter().any(|&(r, a, b)| r == row && (a..=b).contains(&col))
    }

    /// Text of viewport row `r` (spacers skipped, trailing blanks trimmed).
    pub fn row_text(&self, r: usize) -> String {
        let mut s = String::new();
        for cell in self.row(r).iter().filter(|c| !c.spacer) {
            s.push(cell.c);
            s.push_str(&cell.zerowidth);
        }
        s.truncate(s.trim_end().len());
        s
    }

    /// The visible text, rows joined with `\n` (like [`crate::Terminal::screen_text`]).
    pub fn text(&self) -> String {
        (0..self.rows).map(|r| self.row_text(r) + "\n").collect()
    }
}

/// A link in the grid: the URL and the viewport spans `(row, first col, last col)` it
/// covers (a plain-text URL can continue on soft-wrapped rows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TermLink {
    pub url: String,
    pub spans: Vec<(usize, usize, usize)>,
}

/// Copy of the visible terminal state with alacritty cells (taken under the term lock);
/// the egui widget paints from it, [`GridSnapshot`] is built from it.
pub(crate) struct RawSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    pub cursor: Option<(usize, usize)>,
    pub cursor_shape: VteCursorShape,
    pub selection: Option<SelectionRange>,
    pub display_offset: usize,
    pub history: usize,
    pub colors: Colors,
}

impl RawSnapshot {
    pub(crate) fn take(term: &Term<Listener>) -> RawSnapshot {
        let cols = term.columns();
        let rows = term.screen_lines();
        let content = term.renderable_content();
        let off = content.display_offset;
        let mut cells = vec![Cell::default(); cols * rows];
        for ind in content.display_iter {
            if let Some(vp) = point_to_viewport(off, ind.point)
                && vp.line < rows
                && vp.column.0 < cols
            {
                cells[vp.line * cols + vp.column.0] = ind.cell.clone();
            }
        }
        let cursor = point_to_viewport(off, content.cursor.point)
            .filter(|p| p.line < rows && p.column.0 < cols)
            .map(|p| (p.line, p.column.0));
        RawSnapshot {
            cols,
            rows,
            cells,
            cursor,
            cursor_shape: content.cursor.shape,
            selection: content.selection,
            display_offset: off,
            history: term.grid().history_size(),
            colors: *content.colors,
        }
    }

    pub(crate) fn row(&self, r: usize) -> &[Cell] {
        &self.cells[r * self.cols..(r + 1) * self.cols]
    }

    /// Inclusive column span of the selection on viewport row `r`.
    pub(crate) fn selected_span(&self, r: usize) -> Option<(usize, usize)> {
        let range = self.selection.as_ref()?;
        let line = Line(r as i32 - self.display_offset as i32);
        row_span(range, line, self.cols.checked_sub(1)?)
    }

    /// The toolkit-free snapshot, colours resolved against `palette`.
    pub(crate) fn to_grid(&self, palette: &TermPalette, style: GridStyle) -> GridSnapshot {
        let resolver = Resolver::new(palette, &self.colors, style.bright_bold);
        let cells = self
            .cells
            .iter()
            .map(|cell| {
                let st = resolver.style(cell);
                let hidden = cell.flags.contains(Flags::HIDDEN);
                GridCell {
                    c: if cell.c == '\0' || hidden { ' ' } else { cell.c },
                    zerowidth: if hidden {
                        String::new()
                    } else {
                        cell.zerowidth().unwrap_or_default().iter().collect()
                    },
                    wide: cell.flags.contains(Flags::WIDE_CHAR),
                    spacer: is_spacer(cell),
                    fg: st.fg,
                    bg: st.bg,
                    bold: st.bold,
                    italic: st.italic,
                    underline: st.underline,
                    strike: st.strike,
                    hyperlink: cell.hyperlink().map(|h| h.uri().to_owned()),
                }
            })
            .collect();
        let shape = CursorShape::from_vte(self.cursor_shape);
        GridSnapshot {
            cols: self.cols,
            rows: self.rows,
            cells,
            cursor: self.cursor.map(|(row, col)| GridCursor { row, col, shape }),
            display_offset: self.display_offset,
            history: self.history,
            selection: (0..self.rows).filter_map(|r| self.selected_span(r).map(|(a, b)| (r, a, b))).collect(),
            default_bg: resolver.default_bg(),
            cursor_color: resolver.cursor(),
        }
    }
}

/// Text of a viewport row with a char-index → column map (spacers skipped).
pub(crate) fn row_text(cells: &[Cell]) -> (String, Vec<usize>) {
    let mut s = String::with_capacity(cells.len());
    let mut map = Vec::with_capacity(cells.len());
    for (col, cell) in cells.iter().enumerate() {
        if is_spacer(cell) {
            continue;
        }
        s.push(if cell.c == '\0' { ' ' } else { cell.c });
        map.push(col);
    }
    (s, map)
}

/// The link at viewport `(row, col)`: an OSC 8 hyperlink first, else a plain-text URL
/// (joining soft-wrapped rows).
pub(crate) fn link_at(snap: &RawSnapshot, row: usize, col: usize) -> Option<TermLink> {
    if row >= snap.rows || col >= snap.cols {
        return None;
    }
    let cells = snap.row(row);
    // OSC 8 hyperlinks first.
    if let Some(h) = cells[col].hyperlink() {
        let same = |c: &Cell| c.hyperlink().is_some_and(|x| x.id() == h.id());
        let mut a = col;
        while a > 0 && same(&cells[a - 1]) {
            a -= 1;
        }
        let mut b = col;
        while b + 1 < cells.len() && same(&cells[b + 1]) {
            b += 1;
        }
        return Some(TermLink { url: h.uri().to_owned(), spans: vec![(row, a, b)] });
    }
    // Plain-text URLs, joining soft-wrapped rows.
    let wraps = |r: usize| snap.row(r).last().is_some_and(|c| c.flags.contains(Flags::WRAPLINE));
    let mut first = row;
    while first > 0 && wraps(first - 1) {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < snap.rows && wraps(last) {
        last += 1;
    }
    let mut text = String::new();
    let mut map: Vec<(usize, usize)> = Vec::new();
    for r in first..=last {
        let (s, m) = row_text(snap.row(r));
        text.push_str(&s);
        map.extend(m.into_iter().map(|c| (r, c)));
    }
    let target = map.iter().position(|&(r, c)| r == row && c == col)?;
    let m = url::find_urls(&text).into_iter().find(|m| m.start <= target && target < m.end)?;
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for &(r, c) in &map[m.start..m.end] {
        match spans.last_mut() {
            Some(s) if s.0 == r => s.2 = c,
            _ => spans.push((r, c, c)),
        }
    }
    Some(TermLink { url: m.url, spans })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_helpers() {
        let cell = |c: char| GridCell { c, ..GridCell::default() };
        let mut cells: Vec<GridCell> = "ab  ".chars().map(cell).collect();
        cells.extend("x".chars().map(cell));
        cells.push(GridCell { c: '漢', wide: true, ..GridCell::default() });
        cells.push(GridCell { c: ' ', spacer: true, ..GridCell::default() });
        cells.push(cell(' '));
        let g = GridSnapshot { cols: 4, rows: 2, cells, selection: vec![(1, 1, 2)], ..GridSnapshot::default() };
        assert_eq!(g.row_text(0), "ab");
        assert_eq!(g.row_text(1), "x漢");
        assert_eq!(g.text(), "ab\nx漢\n");
        assert!(g.is_selected(1, 2) && !g.is_selected(1, 3) && !g.is_selected(0, 1));
        assert_eq!(g.cell(1, 1).map(|c| c.c), Some('漢'));
        assert_eq!(g.cell(0, 4), None);
        assert!(g.row(2).is_empty());
        assert!(GridStyle::default().bright_bold);
    }
}
