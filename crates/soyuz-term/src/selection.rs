//! Mouse selection helpers: click counting (single/double/triple) → alacritty
//! `SelectionType`, and selection-range → per-row column spans for painting.

use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::{SelectionRange, SelectionType};

/// Max seconds between clicks of a multi-click.
const MULTI_CLICK_SECS: f64 = 0.4;

#[derive(Debug, Default)]
pub(crate) struct ClickTracker {
    last_time: f64,
    last_cell: Option<(i32, usize)>,
    count: u8,
}

impl ClickTracker {
    /// Register a press at `time` (seconds) on `cell`; returns 1, 2 or 3.
    pub(crate) fn register(&mut self, time: f64, cell: (i32, usize)) -> u8 {
        let continues = self.last_cell == Some(cell) && time - self.last_time <= MULTI_CLICK_SECS;
        self.count = if continues { self.count % 3 + 1 } else { 1 };
        self.last_time = time;
        self.last_cell = Some(cell);
        self.count
    }
}

pub(crate) fn selection_type(clicks: u8) -> SelectionType {
    match clicks {
        2 => SelectionType::Semantic,
        3 => SelectionType::Lines,
        _ => SelectionType::Simple,
    }
}

/// Inclusive column span of `range` on terminal line `line` (`None` when not covered).
pub(crate) fn row_span(range: &SelectionRange, line: Line, last_col: usize) -> Option<(usize, usize)> {
    let (start, end): (Point, Point) = (range.start, range.end);
    if line < start.line || line > end.line {
        return None;
    }
    if range.is_block {
        return Some((start.column.0.min(end.column.0), start.column.0.max(end.column.0)));
    }
    let from = if line == start.line { start.column } else { Column(0) };
    let to = if line == end.line { end.column } else { Column(last_col) };
    (from <= to).then_some((from.0, to.0.min(last_col)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_counting_cycles() {
        let mut t = ClickTracker::default();
        assert_eq!(t.register(0.0, (0, 1)), 1);
        assert_eq!(t.register(0.2, (0, 1)), 2);
        assert_eq!(t.register(0.3, (0, 1)), 3);
        assert_eq!(t.register(0.4, (0, 1)), 1);
        assert_eq!(t.register(2.0, (0, 1)), 1);
        assert_eq!(t.register(2.1, (0, 2)), 1);
        assert_eq!(selection_type(2), SelectionType::Semantic);
        assert_eq!(selection_type(3), SelectionType::Lines);
    }

    #[test]
    fn spans() {
        let r = SelectionRange::new(Point::new(Line(0), Column(3)), Point::new(Line(2), Column(1)), false);
        assert_eq!(row_span(&r, Line(-1), 79), None);
        assert_eq!(row_span(&r, Line(0), 79), Some((3, 79)));
        assert_eq!(row_span(&r, Line(1), 79), Some((0, 79)));
        assert_eq!(row_span(&r, Line(2), 79), Some((0, 1)));
        let b = SelectionRange::new(Point::new(Line(0), Column(5)), Point::new(Line(1), Column(2)), true);
        assert_eq!(row_span(&b, Line(1), 79), Some((2, 5)));
    }
}
