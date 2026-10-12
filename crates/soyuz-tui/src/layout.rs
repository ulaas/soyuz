//! Where everything goes on the screen: [`Regions::compute`] splits the terminal into the
//! tab strip, the location line, the sidebar, the pane rects (single, dual in either
//! orientation with its split ratio, quad), the toast lines and the status line.
//!
//! Small terminals degrade in steps instead of drawing garbage:
//! - below [`LIST_ONLY_BELOW_ROWS`] rows only the focused pane's list is drawn;
//! - below [`SIDEBAR_MIN_COLS`] columns the sidebar is hidden;
//! - below [`STACK_BELOW_COLS`] columns dual panes stack (one above the other) and quad
//!   becomes a single column;
//! - when there is not even one row per pane, only the focused pane is drawn;
//! - with more than one pane on screen, all at least [`TITLE_MIN_ROWS`] high, each pane
//!   gives its top row to a title line (its location, the focused one highlighted), drawn by
//!   `view::tabs::draw_pane_chrome` together with the splitters.
//!
//! The parts each view step adds report whether they are shown through [`Facts`], built
//! by `Facts::from_core` from one function per part (`view::tabs::shown`,
//! `view::sidebar::shown` / `width`, `view::status::shown`, `view::toasts::lines`), so
//! a step changes its own file, not this one.

use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;
use soyuz_core::config::{Orientation, TuiSection};

pub use crate::hits::{HitTarget, Hits};

/// Below this many rows only the list is drawn.
pub const LIST_ONLY_BELOW_ROWS: u16 = 3;
/// Below this many columns the sidebar is hidden.
pub const SIDEBAR_MIN_COLS: u16 = 40;
/// Below this many columns dual panes stack and quad becomes one column.
pub const STACK_BELOW_COLS: u16 = 60;
/// Cells between panes (and between the sidebar and the panes): borders / splitters.
pub const GAP: u16 = 1;
/// A pane of a multi-pane layout needs this many rows for its title line (one title row
/// plus at least two list rows).
pub const TITLE_MIN_ROWS: u16 = 3;

/// What the core's state says about the layout (no sizes in here).
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    /// Panes shown by the tab's layout: 1, 2 or 4 (`Tab::shown`).
    pub shown: usize,
    /// Index of the focused pane in the tab.
    pub focused: usize,
    /// `Vertical`: side by side; `Horizontal`: one above the other (like the GUI).
    pub orientation: Orientation,
    /// Share of the first pane / column.
    pub split: f32,
    /// Share of the first row (quad).
    pub split2: f32,
    pub tab_strip: bool,
    pub sidebar: bool,
    /// Wanted sidebar width in columns (capped at a third of the screen).
    pub sidebar_width: u16,
    pub status: bool,
    /// Toast lines wanted.
    pub toasts: u16,
}

impl Default for Facts {
    fn default() -> Self {
        Facts {
            shown: 1,
            focused: 0,
            orientation: Orientation::Vertical,
            split: 0.5,
            split2: 0.5,
            tab_strip: false,
            sidebar: false,
            sidebar_width: 24,
            status: true,
            toasts: 0,
        }
    }
}

impl Facts {
    pub fn from_core(core: &AppCore) -> Facts {
        let tab = core.ws.tab();
        Facts {
            shown: tab.shown(),
            focused: tab.focused,
            orientation: tab.orientation,
            split: tab.split,
            split2: tab.split2,
            tab_strip: crate::view::tabs::shown(core),
            sidebar: crate::view::sidebar::shown(core),
            sidebar_width: crate::view::sidebar::width(core),
            status: crate::view::status::shown(core),
            toasts: crate::view::toasts::lines(core),
        }
    }
}

/// A pane on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneRect {
    /// Index in `Tab::panes`.
    pub index: usize,
    pub rect: Rect,
}

/// The screen split for one frame. Parts not shown have an empty rect.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Regions {
    pub area: Rect,
    pub tabs: Rect,
    /// The location line (breadcrumbs / address field).
    pub crumbs: Rect,
    pub sidebar: Rect,
    /// The panes drawn this frame, in tab order (their lists: title rows excluded).
    pub panes: Vec<PaneRect>,
    /// The title line of each pane that has one (multi-pane layouts, see the module docs).
    pub titles: Vec<PaneRect>,
    /// The gaps between panes (`true`: a vertical line between columns).
    pub splitters: Vec<(Rect, bool)>,
    pub toasts: Rect,
    pub status: Rect,
    /// Too small for anything but the list.
    pub list_only: bool,
    /// Dual / quad panes are stacked into one column.
    pub stacked: bool,
}

impl Regions {
    /// The rect of tab pane `index`, if it is drawn.
    pub fn pane(&self, index: usize) -> Option<Rect> {
        self.panes.iter().find(|p| p.index == index).map(|p| p.rect)
    }

    /// Split `area` for `facts`. `_tui` carries the `[tui]` settings T2.1 adds (none
    /// change the layout yet).
    pub fn compute(area: Rect, _tui: &TuiSection, facts: &Facts) -> Regions {
        let at = |x: u16, y: u16| Rect::new(x, y, 0, 0);
        let mut r = Regions {
            area,
            tabs: at(area.x, area.y),
            crumbs: at(area.x, area.y),
            sidebar: at(area.x, area.y),
            toasts: at(area.x, area.bottom()),
            status: at(area.x, area.bottom()),
            ..Regions::default()
        };
        if area.height < LIST_ONLY_BELOW_ROWS {
            r.list_only = true;
            r.panes = vec![PaneRect { index: facts.focused, rect: area }];
            return r;
        }
        let row = |y: u16| Rect::new(area.x, y, area.width, 1);
        let mut top = area.y;
        let mut bottom = area.bottom();
        if facts.status {
            bottom -= 1;
            r.status = row(bottom);
        }
        // The location line, then the tab strip when at least two list rows stay.
        r.crumbs = row(top);
        top += 1;
        if facts.tab_strip && bottom.saturating_sub(top) >= 3 {
            r.tabs = r.crumbs;
            r.crumbs = row(top);
            top += 1;
        }
        let toasts = facts.toasts.min(bottom.saturating_sub(top).saturating_sub(1));
        if toasts > 0 {
            bottom -= toasts;
            r.toasts = Rect::new(area.x, bottom, area.width, toasts);
        } else {
            r.toasts = at(area.x, bottom);
        }
        let mut body = Rect::new(area.x, top, area.width, bottom - top);
        if facts.sidebar && area.width >= SIDEBAR_MIN_COLS {
            let w = facts.sidebar_width.clamp(1, area.width / 3);
            r.sidebar = Rect { width: w, ..body };
            body.x += w + GAP;
            body.width -= w + GAP;
        } else {
            r.sidebar = at(body.x, body.y);
        }
        let (mut panes, splitters, stacked) = split_panes(body, facts);
        if panes.len() > 1 && panes.iter().all(|p| p.rect.height >= TITLE_MIN_ROWS) {
            for p in &mut panes {
                r.titles.push(PaneRect { index: p.index, rect: Rect { height: 1, ..p.rect } });
                p.rect.y += 1;
                p.rect.height -= 1;
            }
        }
        r.panes = panes;
        r.splitters = splitters;
        r.stacked = stacked;
        r
    }
}

/// `len` cells split at `share` with [`GAP`] between: the first part's length, keeping at
/// least one cell on each side.
fn cut(len: u16, share: f32) -> u16 {
    let usable = len.saturating_sub(GAP);
    let share = if share.is_finite() { share.clamp(0.0, 1.0) } else { 0.5 };
    ((f32::from(usable) * share).round() as u16).clamp(1, usable.saturating_sub(1).max(1))
}

type Split = (Vec<PaneRect>, Vec<(Rect, bool)>, bool);

/// The pane rects of `area`.
fn split_panes(area: Rect, f: &Facts) -> Split {
    let only = |i: usize| (vec![PaneRect { index: i, rect: area }], Vec::new(), false);
    let narrow = area.width < STACK_BELOW_COLS;
    match f.shown {
        2 => {
            let stacked = f.orientation == Orientation::Horizontal || narrow;
            if stacked {
                if area.height < 2 + GAP {
                    return only(f.focused);
                }
                let a = cut(area.height, f.split);
                let first = Rect { height: a, ..area };
                let gap = Rect { y: area.y + a, height: GAP, ..area };
                let second = Rect { y: area.y + a + GAP, height: area.height - a - GAP, ..area };
                let panes = vec![PaneRect { index: 0, rect: first }, PaneRect { index: 1, rect: second }];
                (panes, vec![(gap, false)], true)
            } else {
                if area.width < 2 + GAP {
                    return only(f.focused);
                }
                let a = cut(area.width, f.split);
                let first = Rect { width: a, ..area };
                let gap = Rect { x: area.x + a, width: GAP, ..area };
                let second = Rect { x: area.x + a + GAP, width: area.width - a - GAP, ..area };
                let panes = vec![PaneRect { index: 0, rect: first }, PaneRect { index: 1, rect: second }];
                (panes, vec![(gap, true)], false)
            }
        }
        4 if !narrow && area.height >= 2 + GAP => {
            let a = cut(area.width, f.split);
            let b = cut(area.height, f.split2);
            let (x0, x1) = (area.x, area.x + a + GAP);
            let (y0, y1) = (area.y, area.y + b + GAP);
            let (w0, w1) = (a, area.width - a - GAP);
            let (h0, h1) = (b, area.height - b - GAP);
            let panes = vec![
                PaneRect { index: 0, rect: Rect::new(x0, y0, w0, h0) },
                PaneRect { index: 1, rect: Rect::new(x1, y0, w1, h0) },
                PaneRect { index: 2, rect: Rect::new(x0, y1, w0, h1) },
                PaneRect { index: 3, rect: Rect::new(x1, y1, w1, h1) },
            ];
            let splitters = vec![
                (Rect { x: area.x + a, width: GAP, ..area }, true),
                (Rect { y: area.y + b, height: GAP, ..area }, false),
            ];
            (panes, splitters, false)
        }
        4 if area.height >= 4 + 3 * GAP => {
            // One column: equal rows, the remainder to the last pane.
            let each = (area.height - 3 * GAP) / 4;
            let mut panes = Vec::with_capacity(4);
            let mut splitters = Vec::with_capacity(3);
            let mut y = area.y;
            for i in 0..4 {
                let h = if i == 3 { area.bottom() - y } else { each };
                panes.push(PaneRect { index: i, rect: Rect { y, height: h, ..area } });
                y += h;
                if i < 3 {
                    splitters.push((Rect { y, height: GAP, ..area }, false));
                    y += GAP;
                }
            }
            (panes, splitters, true)
        }
        _ => only(f.focused),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(w: u16, h: u16, f: &Facts) -> Regions {
        Regions::compute(Rect::new(0, 0, w, h), &TuiSection::default(), f)
    }

    #[test]
    fn single_pane_matches_phase_t1() {
        // Path line, list, toasts, status line.
        let f = Facts { toasts: 2, ..Facts::default() };
        let r = regions(80, 24, &f);
        assert_eq!(r.crumbs, Rect::new(0, 0, 80, 1));
        assert_eq!(r.panes, vec![PaneRect { index: 0, rect: Rect::new(0, 1, 80, 20) }]);
        assert_eq!(r.toasts, Rect::new(0, 21, 80, 2));
        assert_eq!(r.status, Rect::new(0, 23, 80, 1));
        assert!(r.tabs.is_empty() && r.sidebar.is_empty() && !r.list_only);
    }

    #[test]
    fn tiny_terminals() {
        let f = Facts { shown: 2, focused: 1, sidebar: true, tab_strip: true, toasts: 3, ..Facts::default() };
        let r = regions(80, 2, &f);
        assert!(r.list_only);
        assert_eq!(r.panes, vec![PaneRect { index: 1, rect: Rect::new(0, 0, 80, 2) }]);
        assert!(r.crumbs.is_empty() && r.status.is_empty() && r.toasts.is_empty());
        // Three rows: location, one list row, status; no toasts, no tab strip.
        let r = regions(80, 3, &f);
        assert!(!r.list_only && r.tabs.is_empty() && r.toasts.is_empty());
        assert_eq!(r.pane(1).map(|p| p.height), Some(1));
        // Narrow: no sidebar.
        assert!(regions(39, 24, &f).sidebar.is_empty());
        assert_eq!(regions(40, 24, &f).sidebar.width, 13);
        assert_eq!(regions(0, 0, &f).panes.len(), 1);
    }

    #[test]
    fn dual_both_orientations_and_ratio() {
        let mut f = Facts { shown: 2, split: 0.25, ..Facts::default() };
        let r = regions(81, 10, &f);
        let (a, b) = (r.pane(0).unwrap(), r.pane(1).unwrap());
        assert_eq!((a.x, a.width, b.x, b.width), (0, 20, 21, 60));
        assert_eq!((a.y, a.height, b.y, b.height), (2, 7, 2, 7));
        assert_eq!(
            r.titles.iter().map(|t| t.rect).collect::<Vec<_>>(),
            [Rect::new(0, 1, 20, 1), Rect::new(21, 1, 60, 1)]
        );
        assert_eq!(r.splitters, vec![(Rect::new(20, 1, 1, 8), true)]);
        f.orientation = Orientation::Horizontal;
        let r = regions(81, 22, &f);
        let (a, b) = (r.pane(0).unwrap(), r.pane(1).unwrap());
        assert!(r.stacked && a.width == 81 && a.y + a.height + GAP + 1 == b.y && b.bottom() == 21);
        // Each pane's title row sits right above its list.
        assert_eq!(
            r.titles,
            vec![
                PaneRect { index: 0, rect: Rect::new(0, 1, 81, 1) },
                PaneRect { index: 1, rect: Rect::new(0, b.y - 1, 81, 1) },
            ]
        );
        // Narrow terminals stack a side-by-side split.
        f.orientation = Orientation::Vertical;
        assert!(regions(59, 22, &f).stacked);
        // Not even a row each: the focused pane alone, without a title.
        f.focused = 1;
        let r = regions(59, 4, &f);
        assert_eq!(r.panes, vec![PaneRect { index: 1, rect: Rect::new(0, 1, 59, 2) }]);
        assert!(r.titles.is_empty());
        // Too short for a title row: the panes keep every row.
        let r = regions(81, 4, &Facts { focused: 0, ..f.clone() });
        assert!(r.titles.is_empty() && r.panes.iter().all(|p| p.rect.height == 2), "{r:?}");
    }

    #[test]
    fn quad_grid_column_or_single() {
        let f = Facts { shown: 4, focused: 2, ..Facts::default() };
        let r = regions(100, 30, &f);
        assert_eq!(r.panes.len(), 4);
        assert_eq!(r.titles.len(), 4);
        assert_eq!(r.titles[2].rect.bottom(), r.pane(2).unwrap().y);
        assert_eq!(r.pane(3).map(|p| (p.right(), p.bottom())), Some((100, 29)));
        assert!(!r.stacked && r.splitters.len() == 2);
        let r = regions(50, 30, &f);
        assert!(r.stacked && r.panes.len() == 4 && r.panes.iter().all(|p| p.rect.width == 50));
        assert_eq!(r.pane(3).map(|p| p.bottom()), Some(29));
        let r = regions(50, 8, &f);
        assert_eq!(r.panes.len(), 1);
        assert_eq!(r.panes[0].index, 2);
    }

    #[test]
    fn every_size_is_consistent() {
        for shown in [1, 2, 4] {
            for orientation in [Orientation::Vertical, Orientation::Horizontal] {
                let f = Facts { shown, orientation, sidebar: true, tab_strip: true, toasts: 3, ..Facts::default() };
                for w in 0..=130 {
                    for h in 0..=40 {
                        let r = regions(w, h, &f);
                        assert!(!r.panes.is_empty());
                        assert!(r.titles.is_empty() || r.titles.len() == r.panes.len(), "{w}x{h}");
                        let rects: Vec<Rect> = r
                            .panes
                            .iter()
                            .chain(&r.titles)
                            .map(|p| p.rect)
                            .chain(r.splitters.iter().map(|s| s.0))
                            .filter(|r| !r.is_empty())
                            .collect();
                        for p in &rects {
                            assert!(r.area.union(*p) == r.area, "{w}x{h} {p:?}");
                            assert!(!p.intersects(r.tabs) && !p.intersects(r.crumbs) && !p.intersects(r.status));
                        }
                        // Panes and titles never overlap (splitters may cross each other).
                        let boxes: Vec<Rect> = r.panes.iter().chain(&r.titles).map(|p| p.rect).collect();
                        for (i, a) in boxes.iter().enumerate() {
                            for b in &boxes[i + 1..] {
                                assert!(!a.intersects(*b), "{w}x{h} {a:?} {b:?}");
                            }
                            for (s, _) in &r.splitters {
                                assert!(!a.intersects(*s), "{w}x{h} {a:?} {s:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}
