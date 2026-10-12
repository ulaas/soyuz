//! Click targets of the last frame: draw functions record what they put where, the mouse
//! handler (T2.9) looks positions up. Cleared at the start of every draw and on resize, so
//! a click never lands on something from an older layout. Row targets carry the pane's
//! `vis_version`: a click after the listing changed under the pointer is ignored.

use ratatui::layout::{Position, Rect};
use soyuz_appcore::state::pane::PaneId;

/// What is under a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    /// Item `index` (in the pane's visible order) of pane `pane`, as of `vis_version`.
    PaneRow { pane: PaneId, vis_version: u64, index: usize },
    /// Group header `group` of pane `pane`.
    PaneHeader { pane: PaneId, vis_version: u64, group: usize },
    /// Empty space of a pane's list.
    PaneBackground { pane: PaneId },
    /// The splitter between panes (`vertical`: between columns).
    Splitter { vertical: bool },
    /// Tab `index` of the tab strip.
    Tab(usize),
    /// The close mark of tab `index`.
    TabClose(usize),
    /// The "new tab" button.
    NewTab,
    /// Breadcrumb segment `index`.
    Crumb(usize),
    /// The address field (click to edit).
    Address,
    /// Sidebar row `index`.
    SidebarRow(usize),
    /// Filter field.
    Filter,
    /// Palette result `index`.
    PaletteRow(usize),
    /// The palette's input line.
    PaletteInput,
    /// Toast `index` (newest last).
    Toast(usize),
    /// The status line.
    Status,
    /// Button `index` of the open dialog.
    DialogButton(usize),
}

/// The targets of one frame, in drawing order.
#[derive(Clone, Debug, Default)]
pub struct Hits {
    items: Vec<(Rect, HitTarget)>,
}

impl Hits {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Record `target` at `rect` (empty rects are skipped).
    pub fn push(&mut self, rect: Rect, target: HitTarget) {
        if !rect.is_empty() {
            self.items.push((rect, target));
        }
    }

    /// The top-most target at cell `(x, y)`: the one drawn last.
    pub fn at(&self, x: u16, y: u16) -> Option<HitTarget> {
        let p = Position { x, y };
        self.items.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, t)| *t)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Every recorded target with its rect (tests, debugging).
    pub fn iter(&self) -> impl Iterator<Item = &(Rect, HitTarget)> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_drawn_wins() {
        let mut h = Hits::default();
        h.push(Rect::new(0, 0, 10, 10), HitTarget::Status);
        h.push(Rect::new(2, 2, 2, 1), HitTarget::Tab(1));
        h.push(Rect::new(5, 5, 0, 3), HitTarget::NewTab);
        assert_eq!(h.len(), 2);
        assert_eq!(h.at(2, 2), Some(HitTarget::Tab(1)));
        assert_eq!(h.at(4, 2), Some(HitTarget::Status));
        assert_eq!(h.at(10, 0), None);
        h.clear();
        assert!(h.is_empty() && h.at(2, 2).is_none());
    }
}
