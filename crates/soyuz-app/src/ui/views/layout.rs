//! Virtualized flow layout shared by the details (one column), list and grid views:
//! visible items are laid out row-major into fixed-size cells, optionally broken into
//! groups that each start with a header line. All coordinates are *content* coordinates
//! (origin = top-left of the scrollable content); the views add the screen offset.

use std::ops::Range;

use egui::{Pos2, Rect, Vec2, pos2, vec2};

use crate::state::listing::GroupSpan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Group header for `groups[g]`.
    Header(usize),
    /// Items `start..start + len` of the visible list.
    Items { start: usize, len: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub kind: LineKind,
    pub top: f32,
    pub height: f32,
}

/// Keyboard navigation directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavDir {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Clone, Debug)]
pub struct FlowLayout {
    pub cols: usize,
    pub cell: Vec2,
    pub gap: Vec2,
    /// Left/top padding of the content.
    pub pad: Vec2,
    pub header_h: f32,
    pub total_h: f32,
    pub n: usize,
    lines: Vec<Line>,
    /// For each group (or one pseudo-group when ungrouped): index of its first item line.
    group_first_line: Vec<usize>,
    groups: Vec<(usize, usize)>,
}

impl FlowLayout {
    /// Lay out `n` items into cells of `cell` size across `width` (columns are derived
    /// from the width; at least one). `groups` empty = no headers.
    pub fn new(n: usize, groups: &[GroupSpan], width: f32, cell: Vec2, gap: Vec2, pad: Vec2, header_h: f32) -> Self {
        let usable = (width - 2.0 * pad.x + gap.x).max(cell.x + gap.x);
        let cols = ((usable / (cell.x + gap.x)).floor() as usize).max(1);
        let spans: Vec<(usize, usize)> =
            if groups.is_empty() { vec![(0, n)] } else { groups.iter().map(|g| (g.start, g.len)).collect() };
        let mut lines = Vec::new();
        let mut group_first_line = Vec::with_capacity(spans.len());
        let mut y = pad.y;
        for (gi, &(start, len)) in spans.iter().enumerate() {
            if !groups.is_empty() {
                lines.push(Line { kind: LineKind::Header(gi), top: y, height: header_h });
                y += header_h;
            }
            group_first_line.push(lines.len());
            let mut i = 0;
            while i < len {
                let take = cols.min(len - i);
                lines.push(Line { kind: LineKind::Items { start: start + i, len: take }, top: y, height: cell.y });
                y += cell.y + gap.y;
                i += take;
            }
        }
        let total_h = y + pad.y;
        FlowLayout { cols, cell, gap, pad, header_h, total_h, n, lines, group_first_line, groups: spans }
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Group index containing visible item `i`.
    fn group_of(&self, i: usize) -> usize {
        // First group ending after `i` (empty / collapsed groups are skipped).
        self.groups.partition_point(|&(start, len)| start + len <= i).min(self.groups.len().saturating_sub(1))
    }

    /// `(line index, column)` of item `i`.
    fn line_col(&self, i: usize) -> Option<(usize, usize)> {
        if i >= self.n {
            return None;
        }
        let g = self.group_of(i);
        let (start, _) = self.groups[g];
        let off = i - start;
        Some((self.group_first_line[g] + off / self.cols, off % self.cols))
    }

    /// Cell rect of item `i` in content coordinates.
    pub fn item_rect(&self, i: usize) -> Option<Rect> {
        let (li, col) = self.line_col(i)?;
        let line = &self.lines[li];
        let x = self.pad.x + col as f32 * (self.cell.x + self.gap.x);
        Some(Rect::from_min_size(pos2(x, line.top), self.cell))
    }

    /// Lines intersecting the vertical range `y0..y1`.
    pub fn lines_in(&self, y0: f32, y1: f32) -> Range<usize> {
        let first = self.lines.partition_point(|l| l.top + l.height < y0);
        let last = self.lines.partition_point(|l| l.top <= y1);
        first..last.max(first)
    }

    /// Item under `pos` (exact cell hit; gaps and headers miss).
    pub fn hit(&self, pos: Pos2) -> Option<usize> {
        let r = self.lines_in(pos.y, pos.y);
        for line in &self.lines[r] {
            if let LineKind::Items { start, len } = line.kind {
                if pos.y < line.top || pos.y > line.top + line.height {
                    continue;
                }
                let rel = pos.x - self.pad.x;
                if rel < 0.0 {
                    return None;
                }
                let col = (rel / (self.cell.x + self.gap.x)).floor() as usize;
                let within = rel - col as f32 * (self.cell.x + self.gap.x) <= self.cell.x;
                return (col < len && within).then_some(start + col);
            }
        }
        None
    }

    /// Group header under `pos`.
    pub fn header_at(&self, pos: Pos2) -> Option<usize> {
        let r = self.lines_in(pos.y, pos.y);
        self.lines[r].iter().find_map(|l| match l.kind {
            LineKind::Header(g) if pos.y >= l.top && pos.y <= l.top + l.height => Some(g),
            _ => None,
        })
    }

    /// Items whose cells intersect `rect` (rubber band), in visible order.
    pub fn items_in_rect(&self, rect: Rect) -> Vec<usize> {
        let mut out = Vec::new();
        for line in &self.lines[self.lines_in(rect.top(), rect.bottom())] {
            let LineKind::Items { start, len } = line.kind else { continue };
            if line.top > rect.bottom() || line.top + line.height < rect.top() {
                continue;
            }
            for c in 0..len {
                let x = self.pad.x + c as f32 * (self.cell.x + self.gap.x);
                if x <= rect.right() && x + self.cell.x >= rect.left() {
                    out.push(start + c);
                }
            }
        }
        out
    }

    /// Number of item lines that fit in `viewport_h` (for PageUp/PageDown).
    pub fn rows_per_page(&self, viewport_h: f32) -> usize {
        ((viewport_h / (self.cell.y + self.gap.y)).floor() as usize).max(1)
    }

    /// Where keyboard navigation from `from` lands (`None` from = start of list).
    pub fn nav(&self, from: Option<usize>, dir: NavDir, viewport_h: f32) -> Option<usize> {
        if self.n == 0 {
            return None;
        }
        let last = self.n - 1;
        let Some(i) = from.filter(|&i| i < self.n) else {
            return Some(match dir {
                NavDir::End | NavDir::Up | NavDir::PageUp => last,
                _ => 0,
            });
        };
        Some(match dir {
            NavDir::Left => i.saturating_sub(1),
            NavDir::Right => (i + 1).min(last),
            NavDir::Home => 0,
            NavDir::End => last,
            NavDir::Up => self.vertical(i, -1),
            NavDir::Down => self.vertical(i, 1),
            NavDir::PageUp | NavDir::PageDown => {
                let rows = self.rows_per_page(viewport_h).saturating_sub(1).max(1);
                let step = if dir == NavDir::PageUp { -1 } else { 1 };
                let mut cur = i;
                for _ in 0..rows {
                    let next = self.vertical(cur, step);
                    if next == cur {
                        break;
                    }
                    cur = next;
                }
                cur
            }
        })
    }

    /// Same column in the previous / next item line (clamped to that line's length).
    fn vertical(&self, i: usize, step: i32) -> usize {
        let Some((li, col)) = self.line_col(i) else { return i };
        let mut l = li as i64;
        loop {
            l += step as i64;
            if l < 0 || l as usize >= self.lines.len() {
                return i;
            }
            if let LineKind::Items { start, len } = self.lines[l as usize].kind {
                return start + col.min(len - 1);
            }
        }
    }

    /// Content size for the scroll area.
    pub fn content_size(&self, width: f32) -> Vec2 {
        vec2(width, self.total_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize, groups: &[GroupSpan]) -> FlowLayout {
        // 3 columns: cells 100 wide with 10 gap, pad 5 → width 5+100+10+100+10+100+5 = 330
        FlowLayout::new(n, groups, 330.0, vec2(100.0, 50.0), vec2(10.0, 10.0), vec2(5.0, 5.0), 24.0)
    }

    #[test]
    fn columns_and_rects() {
        let l = grid(7, &[]);
        assert_eq!(l.cols, 3);
        assert_eq!(l.lines().len(), 3);
        assert_eq!(l.item_rect(0).unwrap().min, pos2(5.0, 5.0));
        assert_eq!(l.item_rect(4).unwrap().min, pos2(115.0, 65.0));
        assert_eq!(l.item_rect(6).unwrap().min, pos2(5.0, 125.0));
        assert!(l.item_rect(7).is_none());
        assert_eq!(l.total_h, 5.0 + 3.0 * 60.0 + 5.0);
    }

    #[test]
    fn single_column_details() {
        let l = FlowLayout::new(100, &[], 800.0, vec2(800.0, 28.0), Vec2::ZERO, Vec2::ZERO, 0.0);
        assert_eq!(l.cols, 1);
        assert_eq!(l.hit(pos2(400.0, 28.0 * 10.0 + 3.0)), Some(10));
        assert_eq!(l.lines_in(280.0, 560.0), 9..21);
        assert_eq!(l.nav(Some(10), NavDir::Down, 280.0), Some(11));
        assert_eq!(l.nav(Some(10), NavDir::PageDown, 280.0), Some(19));
        assert_eq!(l.nav(Some(5), NavDir::PageUp, 280.0), Some(0));
        assert_eq!(l.nav(None, NavDir::Down, 280.0), Some(0));
        assert_eq!(l.nav(None, NavDir::End, 280.0), Some(99));
    }

    #[test]
    fn hits_and_gaps() {
        let l = grid(7, &[]);
        assert_eq!(l.hit(pos2(10.0, 10.0)), Some(0));
        assert_eq!(l.hit(pos2(110.0, 10.0)), None, "gap between cells");
        assert_eq!(l.hit(pos2(120.0, 70.0)), Some(4));
        assert_eq!(l.hit(pos2(120.0, 130.0)), None, "beyond the last item of a short line");
        assert_eq!(l.hit(pos2(10.0, 400.0)), None);
    }

    #[test]
    fn rubber_band_hits() {
        let l = grid(7, &[]);
        // Band from the middle of item 1 to the middle of item 3's row, first column.
        let band = Rect::from_two_pos(pos2(150.0, 30.0), pos2(50.0, 80.0));
        assert_eq!(l.items_in_rect(band), vec![0, 1, 3, 4]);
        let thin = Rect::from_two_pos(pos2(0.0, 0.0), pos2(2.0, 400.0));
        assert!(l.items_in_rect(thin).is_empty(), "padding only");
    }

    #[test]
    fn grid_navigation() {
        let l = grid(7, &[]);
        assert_eq!(l.nav(Some(1), NavDir::Down, 1000.0), Some(4));
        assert_eq!(l.nav(Some(5), NavDir::Down, 1000.0), Some(6), "clamped to the short last line");
        assert_eq!(l.nav(Some(6), NavDir::Down, 1000.0), Some(6));
        assert_eq!(l.nav(Some(4), NavDir::Up, 1000.0), Some(1));
        assert_eq!(l.nav(Some(3), NavDir::Left, 1000.0), Some(2));
        assert_eq!(l.nav(Some(6), NavDir::Right, 1000.0), Some(6));
    }

    #[test]
    fn grouped_layout() {
        let groups = vec![
            GroupSpan { label: "A".into(), start: 0, len: 4, count: 4 },
            GroupSpan { label: "B".into(), start: 4, len: 2, count: 2 },
        ];
        let l = grid(6, &groups);
        // header A, items 0..3, items 3..4, header B, items 4..6
        assert_eq!(l.lines().len(), 5);
        assert_eq!(l.lines()[3].kind, LineKind::Header(1));
        let r4 = l.item_rect(4).unwrap();
        assert_eq!(r4.min.x, 5.0, "a group starts on a new line");
        assert_eq!(l.header_at(pos2(50.0, l.lines()[3].top + 3.0)), Some(1));
        assert_eq!(l.nav(Some(3), NavDir::Down, 1000.0), Some(4), "down skips the header");
        assert_eq!(l.nav(Some(5), NavDir::Up, 1000.0), Some(3));
        assert_eq!(l.hit(pos2(10.0, r4.center().y)), Some(4));
    }

    #[test]
    fn collapsed_group_is_skipped() {
        let groups = vec![
            GroupSpan { label: "A".into(), start: 0, len: 2, count: 2 },
            GroupSpan { label: "B".into(), start: 2, len: 0, count: 0 },
            GroupSpan { label: "C".into(), start: 2, len: 1, count: 1 },
        ];
        let l = grid(3, &groups);
        let r2 = l.item_rect(2).unwrap();
        let c_header = l.lines().iter().find(|ln| ln.kind == LineKind::Header(2)).unwrap();
        assert!(r2.min.y > c_header.top, "item 2 belongs to group C, below its header");
        assert_eq!(l.nav(Some(0), NavDir::Down, 1000.0), Some(2));
    }
}
