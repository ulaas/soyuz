//! Tabs and the workspace (one window). Each tab owns one or two panes (dual pane is
//! per tab, like Files v4); the second pane is kept alive when dual mode is switched
//! off so toggling back restores it.

use std::sync::atomic::{AtomicU64, Ordering};

use soyuz_core::config::Orientation;

use super::pane::{Pane, PaneId};
use super::session::TabSession;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TabId(pub u64);

impl TabId {
    pub fn next() -> TabId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        TabId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

pub struct Tab {
    pub id: TabId,
    /// One or two panes; `panes[1]` exists once dual mode was used.
    pub panes: Vec<Pane>,
    pub focused: usize,
    pub dual: bool,
    pub orientation: Orientation,
    /// Fraction of the content given to the first pane.
    pub split: f32,
    pub pinned: bool,
    /// Synchronized navigation between the two panes.
    pub sync_nav: bool,
}

impl Tab {
    pub fn new(pane: Pane) -> Self {
        Tab {
            id: TabId::next(),
            panes: vec![pane],
            focused: 0,
            dual: false,
            orientation: Orientation::Vertical,
            split: 0.5,
            pinned: false,
            sync_nav: false,
        }
    }

    /// Number of panes currently shown.
    pub fn shown(&self) -> usize {
        if self.dual { self.panes.len().min(2) } else { 1 }
    }

    pub fn active(&self) -> &Pane {
        &self.panes[self.focused.min(self.panes.len() - 1)]
    }

    pub fn active_mut(&mut self) -> &mut Pane {
        let i = self.focused.min(self.panes.len() - 1);
        &mut self.panes[i]
    }

    /// The other visible pane in dual mode.
    pub fn other_mut(&mut self) -> Option<&mut Pane> {
        if self.shown() < 2 {
            return None;
        }
        let i = 1 - self.focused.min(1);
        self.panes.get_mut(i)
    }

    pub fn other(&self) -> Option<&Pane> {
        if self.shown() < 2 {
            return None;
        }
        self.panes.get(1 - self.focused.min(1))
    }

    pub fn pane_index(&self, id: PaneId) -> Option<usize> {
        self.panes.iter().position(|p| p.id == id)
    }
}

/// A closed tab remembered for "reopen closed tab".
#[derive(Clone, Debug)]
pub struct ClosedTab {
    pub index: usize,
    pub session: TabSession,
}

pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub closed: Vec<ClosedTab>,
}

impl Workspace {
    pub const MAX_CLOSED: usize = 25;

    pub fn new(first: Tab) -> Self {
        Workspace { tabs: vec![first], active: 0, closed: Vec::new() }
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active.min(self.tabs.len() - 1)]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        let i = self.active.min(self.tabs.len() - 1);
        &mut self.tabs[i]
    }

    pub fn pane(&self) -> &Pane {
        self.tab().active()
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        self.tab_mut().active_mut()
    }

    /// Any pane in any tab by id.
    pub fn find_pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.tabs.iter_mut().flat_map(|t| t.panes.iter_mut()).find(|p| p.id == id)
    }

    pub fn find_pane(&self, id: PaneId) -> Option<&Pane> {
        self.tabs.iter().flat_map(|t| t.panes.iter()).find(|p| p.id == id)
    }

    /// `(tab index, pane index)` of a pane.
    pub fn locate(&self, id: PaneId) -> Option<(usize, usize)> {
        self.tabs.iter().enumerate().find_map(|(ti, t)| t.pane_index(id).map(|pi| (ti, pi)))
    }

    /// Index where a new tab is inserted: after the active one, but never among pinned tabs.
    pub fn insertion_index(&self) -> usize {
        let pinned = self.tabs.iter().filter(|t| t.pinned).count();
        (self.active + 1).max(pinned).min(self.tabs.len())
    }

    /// Move tab `from` to position `to`, keeping pinned tabs before unpinned ones.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() {
            return;
        }
        let pinned = self.tabs.iter().filter(|t| t.pinned).count();
        let is_pinned = self.tabs[from].pinned;
        let to = if is_pinned { to.min(pinned.saturating_sub(1)) } else { to.max(pinned).min(self.tabs.len() - 1) };
        if from == to {
            return;
        }
        let active_id = self.tab().id;
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.active = self.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
    }

    /// Pin / unpin a tab, moving it to the end of the pinned block / start of the rest.
    pub fn set_pinned(&mut self, index: usize, pinned: bool) {
        if index >= self.tabs.len() || self.tabs[index].pinned == pinned {
            return;
        }
        let active_id = self.tab().id;
        let mut t = self.tabs.remove(index);
        t.pinned = pinned;
        let pinned_count = self.tabs.iter().filter(|t| t.pinned).count();
        self.tabs.insert(pinned_count, t);
        self.active = self.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::view_memory::ViewState;
    use soyuz_core::Location;

    fn tab(name: &str) -> Tab {
        Tab::new(Pane::new(Location::Dir(name.into()), ViewState::default()))
    }

    fn order(w: &Workspace) -> Vec<String> {
        w.tabs.iter().map(|t| t.active().location.display()).collect()
    }

    #[test]
    fn move_and_pin() {
        let mut w = Workspace::new(tab("a"));
        w.tabs.push(tab("b"));
        w.tabs.push(tab("c"));
        w.active = 2;
        w.move_tab(2, 0);
        assert_eq!(order(&w), ["c", "a", "b"]);
        assert_eq!(w.active, 0, "active tab follows the move");
        w.set_pinned(2, true);
        assert_eq!(order(&w), ["b", "c", "a"]);
        assert!(w.tabs[0].pinned);
        // unpinned tabs cannot move in front of pinned ones
        w.move_tab(2, 0);
        assert_eq!(order(&w), ["b", "a", "c"]);
        assert_eq!(w.insertion_index(), 3);
        w.active = 0;
        assert_eq!(w.insertion_index(), 1, "after the active tab");
        w.set_pinned(0, false);
        assert!(w.tabs.iter().all(|t| !t.pinned));
    }

    #[test]
    fn dual_pane_accessors() {
        let mut t = tab("a");
        assert!(t.other().is_none());
        t.panes.push(Pane::new(Location::Dir("b".into()), ViewState::default()));
        assert!(t.other().is_none(), "second pane hidden until dual is on");
        t.dual = true;
        assert_eq!(t.other().unwrap().location.display(), "b");
        t.focused = 1;
        assert_eq!(t.active().location.display(), "b");
        assert_eq!(t.other_mut().unwrap().location.display(), "a");
    }
}
