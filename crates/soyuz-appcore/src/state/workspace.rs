//! Tabs and the workspace (one window). Each tab owns one, two or four panes (the layout
//! is per tab, like Files v4); extra panes are kept alive when the layout shrinks so
//! switching back restores them. Tabs can be collected in named, colored, collapsible
//! groups; a group's tabs are always adjacent in the strip.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use soyuz_core::config::Orientation;

use super::pane::{Pane, PaneId};
use super::session::TabSession;
use super::workspace_visits::VisitGraph;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TabId(pub u64);

impl TabId {
    pub fn next() -> TabId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        TabId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Identity of a tab group (unique across all windows of the process).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct GroupId(pub u64);

impl GroupId {
    pub fn next() -> GroupId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        GroupId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A named, colored, collapsible run of tabs.
#[derive(Clone, Debug, PartialEq)]
pub struct TabGroup {
    pub id: GroupId,
    pub name: String,
    /// Tag color index (0..8, `Palette::tags`).
    pub color: u8,
    pub collapsed: bool,
}

impl TabGroup {
    pub fn new(name: impl Into<String>, color: u8) -> Self {
        TabGroup { id: GroupId::next(), name: name.into(), color: color.min(7), collapsed: false }
    }
}

/// How a tab arranges its panes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneLayout {
    Single,
    Dual(Orientation),
    /// 2×2 grid.
    Quad,
}

pub struct Tab {
    pub id: TabId,
    /// One to four panes; extra panes stay alive while the layout shows fewer.
    pub panes: Vec<Pane>,
    pub focused: usize,
    pub dual: bool,
    pub orientation: Orientation,
    /// Fraction of the content given to the first pane (first column in quad).
    pub split: f32,
    pub pinned: bool,
    /// Synchronized navigation between the visible panes.
    pub sync_nav: bool,
    /// 2×2 layout (implies `dual`; needs four panes).
    pub quad: bool,
    /// Row split of the quad layout.
    pub split2: f32,
    pub group: Option<GroupId>,
    /// "Where was I": every place visited in this tab, branching at back/forward.
    pub visits: VisitGraph,
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
            quad: false,
            split2: 0.5,
            group: None,
            visits: VisitGraph::default(),
        }
    }

    /// Number of panes currently shown.
    pub fn shown(&self) -> usize {
        if self.quad && self.panes.len() >= 4 {
            4
        } else if self.dual {
            self.panes.len().min(2)
        } else {
            1
        }
    }

    pub fn layout(&self) -> PaneLayout {
        match self.shown() {
            4 => PaneLayout::Quad,
            2 => PaneLayout::Dual(self.orientation),
            _ => PaneLayout::Single,
        }
    }

    /// Switch the layout flags (the caller creates missing panes, see `ops::ensure_panes`).
    pub fn set_layout(&mut self, layout: PaneLayout) {
        match layout {
            PaneLayout::Single => {
                self.dual = false;
                self.quad = false;
            }
            PaneLayout::Dual(o) => {
                self.dual = true;
                self.quad = false;
                self.orientation = o;
                if self.focused > 1 {
                    self.focused = 0;
                }
            }
            PaneLayout::Quad => {
                self.dual = true;
                self.quad = true;
            }
        }
    }

    /// Panes the layout needs.
    pub fn wanted_panes(layout: PaneLayout) -> usize {
        match layout {
            PaneLayout::Single => 1,
            PaneLayout::Dual(_) => 2,
            PaneLayout::Quad => 4,
        }
    }

    pub fn active(&self) -> &Pane {
        &self.panes[self.focused.min(self.panes.len() - 1)]
    }

    pub fn active_mut(&mut self) -> &mut Pane {
        let i = self.focused.min(self.panes.len() - 1);
        &mut self.panes[i]
    }

    /// Index of the "other" pane: the neighbour in dual mode, the pane beside the
    /// focused one (same row) in the quad layout.
    fn other_index(&self) -> Option<usize> {
        match self.shown() {
            2 => Some(1 - self.focused.min(1)),
            4 => Some(self.focused.min(3) ^ 1),
            _ => None,
        }
    }

    /// The other visible pane in dual mode.
    pub fn other_mut(&mut self) -> Option<&mut Pane> {
        let i = self.other_index()?;
        self.panes.get_mut(i)
    }

    pub fn other(&self) -> Option<&Pane> {
        self.panes.get(self.other_index()?)
    }

    /// Next visible pane after the focused one (FocusOtherPane cycles through the quad).
    pub fn next_visible(&self) -> usize {
        let n = self.shown();
        if n < 2 { self.focused } else { (self.focused.min(n - 1) + 1) % n }
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
    /// Tab groups in no particular order (the strip orders them by their first tab).
    pub groups: Vec<TabGroup>,
}

impl Workspace {
    pub const MAX_CLOSED: usize = 25;

    pub fn new(first: Tab) -> Self {
        Workspace { tabs: vec![first], active: 0, closed: Vec::new(), groups: Vec::new() }
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

    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Index where a new tab is inserted: after the active one, but never among pinned
    /// tabs. Inside a group the new tab joins it (see [`Self::insertion_group`]).
    pub fn insertion_index(&self) -> usize {
        let pinned = self.tabs.iter().filter(|t| t.pinned).count();
        (self.active + 1).max(pinned).min(self.tabs.len())
    }

    /// The group a tab inserted at [`Self::insertion_index`] belongs to: the active tab's
    /// group, so "new tab" next to a grouped tab stays in the group.
    pub fn insertion_group(&self) -> Option<GroupId> {
        self.tabs.get(self.active).and_then(|t| t.group)
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

    /// Drag-and-drop move: like [`Self::move_tab`], then the tab joins `group` (`None` =
    /// leaves its group) and groups are made contiguous again.
    pub fn move_tab_to_group(&mut self, from: usize, to: usize, group: Option<GroupId>) {
        let Some(id) = self.tabs.get(from).map(|t| t.id) else { return };
        self.move_tab(from, to);
        if let Some(i) = self.tab_index(id)
            && !self.tabs[i].pinned
        {
            self.tabs[i].group = group.filter(|g| self.group(*g).is_some());
        }
        self.normalize_groups();
    }

    /// Pin / unpin a tab, moving it to the end of the pinned block / start of the rest.
    /// Pinned tabs leave their group.
    pub fn set_pinned(&mut self, index: usize, pinned: bool) {
        if index >= self.tabs.len() || self.tabs[index].pinned == pinned {
            return;
        }
        let active_id = self.tab().id;
        let mut t = self.tabs.remove(index);
        t.pinned = pinned;
        if pinned {
            t.group = None;
        }
        let pinned_count = self.tabs.iter().filter(|t| t.pinned).count();
        self.tabs.insert(pinned_count, t);
        self.active = self.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
        self.normalize_groups();
    }

    // ---- tab groups ----

    pub fn group(&self, id: GroupId) -> Option<&TabGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut TabGroup> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// Indices of the group's tabs (contiguous after [`Self::normalize_groups`]).
    pub fn group_span(&self, id: GroupId) -> Option<Range<usize>> {
        let first = self.tabs.iter().position(|t| t.group == Some(id))?;
        let last = self.tabs.iter().rposition(|t| t.group == Some(id))?;
        Some(first..last + 1)
    }

    pub fn group_tab_count(&self, id: GroupId) -> usize {
        self.tabs.iter().filter(|t| t.group == Some(id)).count()
    }

    /// Put tab `index` into a new group; returns its id (`None` for a pinned tab).
    pub fn new_group(&mut self, index: usize, name: impl Into<String>, color: u8) -> Option<GroupId> {
        let t = self.tabs.get(index)?;
        if t.pinned {
            return None;
        }
        let g = TabGroup::new(name, color);
        let id = g.id;
        self.groups.push(g);
        self.tabs[index].group = Some(id);
        self.normalize_groups();
        Some(id)
    }

    /// Add tab `index` to `group` (it moves to the end of the group).
    pub fn add_to_group(&mut self, index: usize, group: GroupId) {
        if index >= self.tabs.len() || self.tabs[index].pinned || self.group(group).is_none() {
            return;
        }
        let tab_id = self.tabs[index].id;
        let active_id = self.tab().id;
        if let Some(span) = self.group_span(group).filter(|_| self.tabs[index].group != Some(group)) {
            let mut t = self.tabs.remove(index);
            t.group = Some(group);
            let end = if index < span.end { span.end - 1 } else { span.end };
            self.tabs.insert(end.min(self.tabs.len()), t);
        } else if let Some(i) = self.tab_index(tab_id) {
            self.tabs[i].group = Some(group);
        }
        self.active = self.tab_index(active_id).unwrap_or(0);
        self.normalize_groups();
    }

    /// Take tab `index` out of its group (it moves right after the group).
    pub fn remove_from_group(&mut self, index: usize) {
        let Some(g) = self.tabs.get(index).and_then(|t| t.group) else { return };
        let span = self.group_span(g).unwrap_or(index..index + 1);
        let active_id = self.tab().id;
        let mut t = self.tabs.remove(index);
        t.group = None;
        self.tabs.insert((span.end - 1).min(self.tabs.len()), t);
        self.active = self.tab_index(active_id).unwrap_or(0);
        self.normalize_groups();
    }

    /// Dissolve a group; its tabs stay where they are.
    pub fn ungroup(&mut self, group: GroupId) {
        for t in &mut self.tabs {
            if t.group == Some(group) {
                t.group = None;
            }
        }
        self.groups.retain(|g| g.id != group);
    }

    pub fn toggle_group_collapsed(&mut self, group: GroupId) {
        // Collapsing the group of the active tab activates the nearest tab outside it.
        let collapse = self.group(group).is_some_and(|g| !g.collapsed);
        if collapse && self.tab().group == Some(group) {
            let outside = (0..self.tabs.len())
                .filter(|&i| self.tabs[i].group != Some(group))
                .min_by_key(|&i| i.abs_diff(self.active));
            match outside {
                Some(i) => self.active = i,
                None => return, // every tab is in this group: nothing to show instead
            }
        }
        if let Some(g) = self.group_mut(group) {
            g.collapsed = collapse;
        }
    }

    /// Restore invariants: drop empty / unknown groups, pinned tabs are never grouped, a
    /// group's tabs are contiguous (gathered at the group's first tab), and the active
    /// tab's group is expanded.
    pub fn normalize_groups(&mut self) {
        let known: Vec<GroupId> = self.groups.iter().map(|g| g.id).collect();
        for t in &mut self.tabs {
            if t.pinned || t.group.is_some_and(|g| !known.contains(&g)) {
                t.group = None;
            }
        }
        self.groups.retain(|g| self.tabs.iter().any(|t| t.group == Some(g.id)));
        let order = contiguous_order(&self.tabs.iter().map(|t| t.group).collect::<Vec<_>>());
        if order.iter().enumerate().any(|(i, &k)| i != k) {
            let active_id = self.tabs.get(self.active).map(|t| t.id);
            let mut slots: Vec<Option<Tab>> = std::mem::take(&mut self.tabs).into_iter().map(Some).collect();
            self.tabs = order.iter().filter_map(|&k| slots[k].take()).collect();
            self.active = active_id.and_then(|id| self.tab_index(id)).unwrap_or(0);
        }
        if let Some(g) = self.tabs.get(self.active).and_then(|t| t.group)
            && let Some(g) = self.group_mut(g)
        {
            g.collapsed = false;
        }
    }

    // ---- moving tabs between windows ----

    /// Remove tab `index` for another window. Returns it with a copy of its group (so
    /// the destination can recreate it). The last tab cannot be taken.
    pub fn take_tab(&mut self, index: usize) -> Option<(Tab, Option<TabGroup>)> {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        let group = tab.group.and_then(|g| self.group(g).cloned());
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.normalize_groups();
        Some((tab, group))
    }

    /// Insert a tab from another window at `at` (clamped after the pinned tabs unless the
    /// tab is pinned) and activate it. Its group joins the existing group with the same
    /// id, or is recreated here.
    pub fn insert_tab(&mut self, mut tab: Tab, at: usize, group: Option<TabGroup>) -> usize {
        let pinned = self.tabs.iter().filter(|t| t.pinned).count();
        let at = if tab.pinned { at.min(pinned) } else { at.clamp(pinned, self.tabs.len()) };
        match group {
            Some(g) if !tab.pinned => {
                if self.group(g.id).is_none() {
                    self.groups.push(g.clone());
                }
                tab.group = Some(g.id);
            }
            _ => tab.group = None,
        }
        let id = tab.id;
        self.tabs.insert(at, tab);
        self.active = at;
        self.normalize_groups();
        self.active = self.tab_index(id).unwrap_or(at);
        self.active
    }

    /// Remove every tab of a group (for "move group to new window"). Leaves at least one
    /// tab behind: `None` when the group holds every tab.
    pub fn take_group(&mut self, group: GroupId) -> Option<(Vec<Tab>, TabGroup)> {
        let g = self.group(group)?.clone();
        let n = self.group_tab_count(group);
        if n == 0 || n >= self.tabs.len() {
            return None;
        }
        let active_id = self.tab().id;
        let (taken, kept): (Vec<Tab>, Vec<Tab>) =
            std::mem::take(&mut self.tabs).into_iter().partition(|t| t.group == Some(group));
        self.tabs = kept;
        self.active = self.tab_index(active_id).unwrap_or(0).min(self.tabs.len() - 1);
        self.groups.retain(|x| x.id != group);
        self.normalize_groups();
        Some((taken, g))
    }
}

/// Stable order that gathers each group's members at the position of its first member;
/// ungrouped entries keep their place. `groups[i]` is entry `i`'s group.
pub fn contiguous_order(groups: &[Option<GroupId>]) -> Vec<usize> {
    let mut out = Vec::with_capacity(groups.len());
    let mut done: Vec<GroupId> = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        match g {
            None => out.push(i),
            Some(g) if done.contains(g) => {}
            Some(g) => {
                done.push(*g);
                out.extend((i..groups.len()).filter(|&k| groups[k] == Some(*g)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::view_memory::ViewState;
    use soyuz_core::Location;

    fn tab(name: &str) -> Tab {
        Tab::new(Pane::new(Location::Dir(name.into()), ViewState::default()))
    }

    fn ws(names: &[&str]) -> Workspace {
        let mut w = Workspace::new(tab(names[0]));
        for n in &names[1..] {
            w.tabs.push(tab(n));
        }
        w
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

    #[test]
    fn quad_layout() {
        let mut t = tab("a");
        t.set_layout(PaneLayout::Quad);
        assert_eq!(t.shown(), 1, "quad needs four panes");
        for n in ["b", "c", "d"] {
            t.panes.push(Pane::new(Location::Dir(n.into()), ViewState::default()));
        }
        assert_eq!(t.shown(), 4);
        assert_eq!(t.layout(), PaneLayout::Quad);
        t.focused = 2;
        assert_eq!(t.other().unwrap().location.display(), "d", "same-row neighbour");
        assert_eq!(t.next_visible(), 3);
        t.focused = 3;
        assert_eq!(t.next_visible(), 0, "cycles");
        t.set_layout(PaneLayout::Dual(Orientation::Horizontal));
        assert_eq!(t.shown(), 2);
        assert_eq!(t.focused, 0, "focus moves into the visible panes");
        assert_eq!(t.layout(), PaneLayout::Dual(Orientation::Horizontal));
        t.set_layout(PaneLayout::Single);
        assert_eq!(t.shown(), 1);
        assert_eq!(Tab::wanted_panes(PaneLayout::Quad), 4);
    }

    #[test]
    fn group_operations() {
        let mut w = ws(&["a", "b", "c", "d", "e"]);
        let g = w.new_group(1, "Work", 5).unwrap();
        assert_eq!(w.group_span(g), Some(1..2));
        // Adding a far tab pulls it next to the group.
        w.add_to_group(4, g);
        assert_eq!(order(&w), ["a", "b", "e", "c", "d"]);
        assert_eq!(w.group_span(g), Some(1..3));
        // Adding a tab from the left keeps the group contiguous.
        w.add_to_group(0, g);
        assert_eq!(order(&w), ["b", "e", "a", "c", "d"]);
        assert_eq!(w.group_span(g), Some(0..3));
        // Leaving moves the tab right after the group.
        w.remove_from_group(0);
        assert_eq!(order(&w), ["e", "a", "b", "c", "d"]);
        assert_eq!(w.group_span(g), Some(0..2));
        w.group_mut(g).unwrap().name = "Renamed".into();
        assert_eq!(w.group(g).unwrap().name, "Renamed");
        // Collapsing the active tab's group activates a tab outside it.
        w.active = 1;
        w.toggle_group_collapsed(g);
        assert!(w.group(g).unwrap().collapsed);
        assert_eq!(w.active, 2);
        // Selecting a tab inside expands it again.
        w.active = 0;
        w.normalize_groups();
        assert!(!w.group(g).unwrap().collapsed);
        // Pinning takes a tab out of its group; the last member leaving dissolves it.
        w.set_pinned(0, true);
        assert_eq!(w.tabs[0].group, None);
        let i = w.tabs.iter().position(|t| t.group == Some(g)).unwrap();
        w.remove_from_group(i);
        assert!(w.groups.is_empty(), "empty groups disappear");
        assert!(w.new_group(0, "pinned", 0).is_none(), "pinned tabs can't be grouped");
    }

    #[test]
    fn drag_into_and_out_of_group() {
        let mut w = ws(&["a", "b", "c", "d"]);
        let g = w.new_group(1, "G", 0).unwrap();
        w.add_to_group(2, g);
        // Drop "d" between b and c → joins.
        w.move_tab_to_group(3, 2, Some(g));
        assert_eq!(order(&w), ["a", "b", "d", "c"]);
        assert_eq!(w.group_tab_count(g), 3);
        // Drag "b" to the front with no group → leaves, the group stays contiguous.
        w.move_tab_to_group(1, 0, None);
        assert_eq!(order(&w), ["b", "a", "d", "c"]);
        assert_eq!(w.group_span(g), Some(2..4));
        // Moving a member out without changing its group keeps it with the group.
        w.move_tab_to_group(2, 0, Some(g));
        assert_eq!(w.group_span(g).map(|r| r.len()), Some(2));
        w.ungroup(g);
        assert!(w.groups.is_empty() && w.tabs.iter().all(|t| t.group.is_none()));
    }

    #[test]
    fn contiguity() {
        let (a, b) = (Some(GroupId(1)), Some(GroupId(2)));
        assert_eq!(contiguous_order(&[None, a, None, a, b, None, b]), vec![0, 1, 3, 2, 4, 6, 5]);
        assert_eq!(contiguous_order(&[None, None]), vec![0, 1]);
    }

    #[test]
    fn move_tabs_between_windows() {
        let mut src = ws(&["a", "b", "c"]);
        let mut dst = ws(&["x", "y"]);
        let g = src.new_group(1, "G", 2).unwrap();
        src.active = 2;
        let (t, grp) = src.take_tab(1).unwrap();
        assert_eq!(order(&src), ["a", "c"]);
        assert_eq!(src.active, 1, "active tab index follows");
        assert!(src.groups.is_empty(), "group left empty is dropped");
        assert_eq!(grp.as_ref().map(|g| g.name.as_str()), Some("G"));
        let at = dst.insert_tab(t, 1, grp);
        assert_eq!(order(&dst), ["x", "b", "y"]);
        assert_eq!(at, 1);
        assert_eq!(dst.active, 1);
        assert_eq!(dst.group(g).map(|g| g.color), Some(2), "group recreated in the destination");
        // The last tab of a window can't be taken.
        let mut one = ws(&["z"]);
        assert!(one.take_tab(0).is_none());
        // Whole groups move together.
        let mut w = ws(&["a", "b", "c", "d"]);
        let g2 = w.new_group(1, "two", 1).unwrap();
        w.add_to_group(3, g2);
        let (tabs, meta) = w.take_group(g2).unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(meta.name, "two");
        assert_eq!(order(&w), ["a", "c"]);
        let mut all = ws(&["a"]);
        let g3 = all.new_group(0, "all", 0).unwrap();
        assert!(all.take_group(g3).is_none(), "a window keeps at least one tab");
    }
}
