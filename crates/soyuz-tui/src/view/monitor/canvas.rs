//! The display list a monitor page is built into: text runs, painted rects and click
//! targets, in page coordinates. Pages are laid out while the frame is prepared (that is
//! when `&mut AppCore` is at hand: the table cache and the series need it); drawing replays
//! the list, so `view::draw` stays free of the core.
//!
//! Everything is clipped here: a page can write anywhere, at any terminal size, without a
//! check of its own. `y` is signed so that a scrolled page simply starts above the top.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use soyuz_appcore::models::monitor::DetailsTab;
use soyuz_appcore::state::pane::PaneId;
use soyuz_core::config::monitor::{MonitorPage, ProcColumn};

use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::pane::put;
use crate::view::text;

/// What is under a cell of a monitor page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The rest of the page.
    Area,
    /// A page of the header strip.
    Tab(MonitorPage),
    /// The arrows of the collapsed header strip (`true`: next).
    Step(bool),
    /// The pause indicator.
    Pause,
    /// A column title of the process table.
    ProcHeader(ProcColumn),
    /// Row `i` of the process table.
    ProcRow(usize),
    /// The fold mark of row `i` of the process table.
    ProcFold(usize),
    /// A tab of the process details.
    Details(DetailsTab),
    /// The body of the process details.
    DetailsBody,
    /// The filter line of a page.
    FilterLine,
    /// A column title of a list page.
    ListHeader(u8),
    /// Row `i` of a list page, in shown order.
    ListRow(usize),
    /// Snapshot row `i`.
    SnapRow(usize),
    /// A column title of the comparison.
    DiffHeader(u8),
    /// Comparison row `i`, in shown order.
    DiffRow(usize),
}

/// One drawing step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Text {
        x: u16,
        y: u16,
        w: u16,
        text: String,
        role: Role,
        bold: bool,
    },
    /// Patch `role` over a rect (the cursor row, a chart's time cursor).
    Paint {
        rect: Rect,
        role: Role,
    },
    /// Clear a rect to the page's own style (under the help overlay).
    Blank {
        rect: Rect,
    },
}

/// A rectangle in page coordinates (above the top when scrolled).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Area {
    pub x: u16,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

impl Area {
    /// Row `i` of the area.
    pub fn row(self, i: u16) -> Area {
        Area { y: self.y + i32::from(i), h: 1, ..self }
    }

    /// The area with height `h`.
    pub fn h_of(self, h: u16) -> Area {
        Area { h, ..self }
    }

    /// The area without its first `n` rows.
    pub fn below(self, n: u16) -> Area {
        Area { y: self.y + i32::from(n.min(self.h)), h: self.h.saturating_sub(n), ..self }
    }

    /// Take the last `n` rows off the area.
    pub fn take_bottom(&mut self, n: u16) -> Area {
        let n = n.min(self.h);
        self.h -= n;
        Area { y: self.y + i32::from(self.h), h: n, ..*self }
    }

    /// Take the first `n` rows off the area.
    pub fn take_top(&mut self, n: u16) -> Area {
        let n = n.min(self.h);
        let top = Area { h: n, ..*self };
        self.y += i32::from(n);
        self.h -= n;
        top
    }

    /// `n` equal columns with `gap` cells between them.
    pub fn columns(self, n: u16, gap: u16) -> Vec<Area> {
        let n = n.max(1);
        let cw = self.w.saturating_sub(gap * (n - 1)) / n;
        (0..n).map(|i| Area { x: self.x + i * (cw + gap), w: cw, ..self }).collect()
    }
}

/// A page's display list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Canvas {
    pub w: u16,
    pub h: u16,
    pub ops: Vec<Op>,
    pub hits: Vec<(Rect, Hit)>,
    pub caret: Option<(u16, u16)>,
    /// Rows outside `clip_top..clip_bottom` are dropped.
    clip_top: i32,
    clip_bottom: i32,
}

impl Canvas {
    pub fn new(w: u16, h: u16) -> Canvas {
        Canvas { w, h, clip_top: 0, clip_bottom: i32::from(h), ..Canvas::default() }
    }

    /// Only rows of `a` are drawn until the next call; returns the old limits.
    pub fn clip(&mut self, a: Area) -> (i32, i32) {
        let old = (self.clip_top, self.clip_bottom);
        self.clip_top = a.y.max(0);
        self.clip_bottom = (a.y + i32::from(a.h)).min(i32::from(self.h));
        old
    }

    pub fn unclip(&mut self, old: (i32, i32)) {
        self.clip_top = old.0;
        self.clip_bottom = old.1;
    }

    fn row(&self, y: i32) -> Option<u16> {
        (y >= self.clip_top && y < self.clip_bottom).then_some(y as u16)
    }

    fn room(&self, x: u16, w: u16) -> u16 {
        w.min(self.w.saturating_sub(x))
    }

    /// `s` at `(x, y)` in at most `w` cells, cut with "…".
    pub fn text(&mut self, x: u16, y: i32, w: u16, s: &str, role: Role) {
        self.write(x, y, w, s, role, false);
    }

    /// Like [`Canvas::text`], bold.
    pub fn bold(&mut self, x: u16, y: i32, w: u16, s: &str, role: Role) {
        self.write(x, y, w, s, role, true);
    }

    fn write(&mut self, x: u16, y: i32, w: u16, s: &str, role: Role, bold: bool) {
        let (Some(y), w) = (self.row(y), self.room(x, w)) else { return };
        if w == 0 || s.is_empty() {
            return;
        }
        let text = text::truncate_end(s, usize::from(w));
        self.ops.push(Op::Text { x, y, w, text, role, bold });
    }

    /// `s` right-aligned in `w` cells at `x`.
    pub fn right(&mut self, x: u16, y: i32, w: u16, s: &str, role: Role) {
        let t = text::truncate_end(s, usize::from(w));
        let tw = text::width(&t) as u16;
        self.write(x + w.saturating_sub(tw), y, tw, &t, role, false);
    }

    /// Pieces of text one after the other; returns the column after the last one.
    pub fn spans(&mut self, a: Area, spans: &[(&str, Role)]) -> u16 {
        let end = a.x.saturating_add(a.w);
        let mut x = a.x;
        for (s, role) in spans {
            if x >= end {
                break;
            }
            self.text(x, a.y, end - x, s, *role);
            x = x.saturating_add(text::width(s) as u16).min(end);
        }
        x
    }

    fn rect(&self, x: u16, y: i32, w: u16, h: u16) -> Option<Rect> {
        let top = y.max(self.clip_top);
        let bottom = (y + i32::from(h)).min(self.clip_bottom);
        let w = self.room(x, w);
        (bottom > top && w > 0).then(|| Rect::new(x, top as u16, w, (bottom - top) as u16))
    }

    /// Patch `role` over a rect.
    pub fn paint(&mut self, a: Area, role: Role) {
        if let Some(rect) = self.rect(a.x, a.y, a.w, a.h) {
            self.ops.push(Op::Paint { rect, role });
        }
    }

    /// Clear a rect.
    pub fn blank(&mut self, a: Area) {
        if let Some(rect) = self.rect(a.x, a.y, a.w, a.h) {
            self.ops.push(Op::Blank { rect });
        }
    }

    /// A click target (later ones win).
    pub fn hit(&mut self, a: Area, hit: Hit) {
        if let Some(rect) = self.rect(a.x, a.y, a.w, a.h) {
            self.hits.push((rect, hit));
        }
    }

    /// The focused element's cell, for screen readers.
    pub fn set_caret(&mut self, x: u16, y: i32) {
        if let Some(y) = self.row(y) {
            self.caret = Some((x.min(self.w.saturating_sub(1)), y));
        }
    }

    /// Replay the list into `rect` of `buf`.
    pub fn draw(&self, buf: &mut Buffer, rect: Rect, pane: PaneId, focused: bool, theme: &TuiTheme, hits: &mut Hits) {
        hits.push(rect, HitTarget::Monitor { pane, hit: Hit::Area });
        if rect.is_empty() {
            return;
        }
        if focused {
            hits.set_caret(Position { x: rect.x, y: rect.y });
        }
        buf.set_style(rect, theme.style(Role::Text));
        let place = |r: Rect| {
            let moved = Rect::new(rect.x.saturating_add(r.x), rect.y.saturating_add(r.y), r.width, r.height);
            moved.intersection(rect)
        };
        for op in &self.ops {
            match op {
                Op::Text { x, y, w, text, role, bold } => {
                    let r = place(Rect::new(*x, *y, *w, 1));
                    if r.is_empty() {
                        continue;
                    }
                    let mut style: Style = theme.style(Role::Text).patch(theme.style(*role));
                    if *bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    put(buf, r.x, r.y, text, r.width, style);
                }
                Op::Paint { rect: a, role } => {
                    let r = place(*a);
                    if !r.is_empty() {
                        buf.set_style(r, theme.style(*role));
                    }
                }
                Op::Blank { rect: a } => {
                    let r = place(*a);
                    for y in r.top()..r.bottom() {
                        for x in r.left()..r.right() {
                            buf[(x, y)].reset();
                        }
                    }
                    if !r.is_empty() {
                        buf.set_style(r, theme.style(Role::Text));
                    }
                }
            }
        }
        for (a, hit) in &self.hits {
            let r = place(*a);
            if !r.is_empty() {
                hits.push(r, HitTarget::Monitor { pane, hit: *hit });
            }
        }
        if focused && let Some((x, y)) = self.caret {
            let p = Position { x: rect.x.saturating_add(x), y: rect.y.saturating_add(y) };
            if rect.contains(p) {
                hits.set_caret(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_outside_the_canvas_are_dropped() {
        let mut c = Canvas::new(4, 2);
        c.text(0, -1, 4, "up", Role::Text);
        c.text(0, 2, 4, "down", Role::Text);
        c.text(9, 0, 4, "right", Role::Text);
        c.paint(Area { x: 0, y: -5, w: 9, h: 2 }, Role::Cursor);
        assert!(c.ops.is_empty());
        c.text(1, 1, 99, "abcdef", Role::Text);
        assert_eq!(c.ops, vec![Op::Text { x: 1, y: 1, w: 3, text: "ab…".into(), role: Role::Text, bold: false }]);
    }

    #[test]
    fn scrolled_rects_are_cut_at_the_clip() {
        let mut c = Canvas::new(9, 9);
        let old = c.clip(Area { x: 0, y: 2, w: 9, h: 3 });
        c.paint(Area { x: 0, y: 0, w: 2, h: 9 }, Role::Cursor);
        c.unclip(old);
        assert_eq!(c.ops, vec![Op::Paint { rect: Rect::new(0, 2, 2, 3), role: Role::Cursor }]);
    }

    #[test]
    fn areas_split() {
        let mut a = Area { x: 1, y: 2, w: 21, h: 10 };
        assert_eq!(a.take_bottom(3).y, 9);
        assert_eq!(a.h, 7);
        assert_eq!(a.take_top(2).h, 2);
        assert_eq!(a.y, 4);
        let cols = a.columns(2, 1);
        assert_eq!((cols[0].x, cols[0].w, cols[1].x, cols[1].w), (1, 10, 12, 10));
    }
}
