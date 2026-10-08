//! One file pane: location, listing, visible order, selection, cursor and view state.
//! Pure state — the background requests are issued by `crate::ops`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime};

use soyuz_core::{EntryInfo, FsError, Location, NamePattern, NamePatternSpec, OpControl, PatternKind, sort_entries};

use super::history::{HistoryEntry, NavHistory};
use super::listing::{self, GroupSpan, ViewFilter};
use super::selection::{EntryKey, Selection};
use super::view_memory::ViewState;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct PaneId(pub u64);

impl PaneId {
    pub fn next() -> PaneId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        PaneId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug)]
pub enum ListingStatus {
    Loading {
        since: Instant,
    },
    Ready,
    Failed(FsError),
    /// A virtual location that has no listing (Computer is drawn from volumes).
    Virtual,
}

/// Type-ahead jump buffer ("start typing to jump").
#[derive(Clone, Debug, Default)]
pub struct TypeAhead {
    pub buf: String,
    pub last: Option<Instant>,
}

impl TypeAhead {
    pub const TIMEOUT_SECS: f32 = 1.0;

    /// Append `text`, resetting the buffer when the previous keystroke is too old.
    pub fn push(&mut self, text: &str, now: Instant) -> &str {
        if self.last.is_none_or(|t| now.duration_since(t).as_secs_f32() > Self::TIMEOUT_SECS) {
            self.buf.clear();
        }
        self.buf.push_str(text);
        self.last = Some(now);
        &self.buf
    }

    pub fn is_active(&self, now: Instant) -> bool {
        self.last.is_some_and(|t| now.duration_since(t).as_secs_f32() <= Self::TIMEOUT_SECS) && !self.buf.is_empty()
    }
}

/// An in-progress rubber-band drag (content coordinates).
#[derive(Clone, Debug)]
pub struct Band {
    pub start: egui::Pos2,
    pub current: egui::Pos2,
    pub base: Selection,
    pub mode: super::selection::BandMode,
}

pub struct Pane {
    pub id: PaneId,
    pub location: Location,
    /// Sorted by `view.sort`.
    pub entries: Vec<EntryInfo>,
    /// Streamed entries not merged yet.
    incoming: Vec<EntryInfo>,
    /// Parallel to `entries`.
    pub keys: Vec<EntryKey>,
    index: HashMap<EntryKey, usize>,
    /// Indices into `entries` in display order (filtered, grouped).
    pub visible: Vec<usize>,
    pub groups: Vec<GroupSpan>,
    pub collapsed_groups: HashSet<String>,
    pub selection: Selection,
    pub cursor: Option<EntryKey>,
    pub view: ViewState,
    pub filter: String,
    filter_pattern: Option<NamePattern>,
    pub history: NavHistory,
    /// Listing generation; messages for older generations are ignored.
    pub generation: u64,
    pub ctl: Option<Arc<OpControl>>,
    pub status: ListingStatus,
    /// A soft relist (refresh / rescan) is running; contents stay visible.
    pub refreshing: bool,
    /// Visible order must be recomputed before the next draw.
    pub dirty: bool,
    /// Scroll the cursor into view on the next draw.
    pub scroll_to_cursor: bool,
    /// Apply this scroll offset on the next draw (history / session restore).
    pub restore_scroll: Option<f32>,
    /// Last drawn scroll offset.
    pub scroll_y: f32,
    /// Put the cursor on (and select) this name once it shows up in the listing.
    pub pending_cursor: Option<String>,
    /// Names to select once the listing completes (session restore).
    pub pending_selection: Vec<String>,
    pub type_ahead: TypeAhead,
    pub band: Option<Band>,
    /// When the listing was last rebuilt (for group date buckets).
    pub now: SystemTime,
    /// Items shown per "page" in the last draw (PageUp/PageDown).
    pub page_rows: usize,
    /// Layout of the last draw (keyboard navigation uses it); rebuilt when `layout_sig` changes.
    pub layout: Option<crate::ui::views::layout::FlowLayout>,
    pub layout_sig: Option<LayoutSig>,
    /// Height of the list viewport in the last draw.
    pub viewport_h: f32,
    /// Bumped whenever `visible`/`groups` are recomputed.
    pub vis_version: u64,
    /// What the open context menu applies to.
    pub context: ContextTarget,
    /// Column header drag (details view): `(visible column index, pointer x)`.
    pub col_drag: Option<(usize, f32)>,
}

/// Inputs that determine a [`FlowLayout`](crate::ui::views::layout::FlowLayout).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutSig {
    pub vis_version: u64,
    pub width: f32,
    pub cell: egui::Vec2,
    pub mode: u8,
}

/// Target of the pane's context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ContextTarget {
    #[default]
    Background,
    Items,
}

impl Pane {
    pub fn new(location: Location, view: ViewState) -> Self {
        Pane {
            id: PaneId::next(),
            location,
            entries: Vec::new(),
            incoming: Vec::new(),
            keys: Vec::new(),
            index: HashMap::new(),
            visible: Vec::new(),
            groups: Vec::new(),
            collapsed_groups: HashSet::new(),
            selection: Selection::default(),
            cursor: None,
            view,
            filter: String::new(),
            filter_pattern: None,
            history: NavHistory::default(),
            generation: 0,
            ctl: None,
            status: ListingStatus::Loading { since: Instant::now() },
            refreshing: false,
            dirty: true,
            scroll_to_cursor: false,
            restore_scroll: None,
            scroll_y: 0.0,
            pending_cursor: None,
            pending_selection: Vec::new(),
            type_ahead: TypeAhead::default(),
            band: None,
            now: SystemTime::now(),
            page_rows: 10,
            layout: None,
            layout_sig: None,
            viewport_h: 400.0,
            vis_version: 0,
            context: ContextTarget::Background,
            col_drag: None,
        }
    }

    /// The history entry describing where the user is right now.
    pub fn here(&self) -> HistoryEntry {
        HistoryEntry {
            location: self.location.clone(),
            cursor: self.cursor.as_deref().map(str::to_string),
            scroll: self.scroll_y,
        }
    }

    /// Reset listing state for a new location (generation bump handled by the caller).
    pub fn clear_listing(&mut self) {
        self.entries.clear();
        self.incoming.clear();
        self.keys.clear();
        self.index.clear();
        self.visible.clear();
        self.groups.clear();
        self.selection.clear();
        self.cursor = None;
        self.band = None;
        self.filter.clear();
        self.filter_pattern = None;
        self.type_ahead = TypeAhead::default();
        self.refreshing = false;
        self.dirty = true;
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.status, ListingStatus::Loading { .. })
    }

    fn rebuild_index(&mut self) {
        self.keys = self.entries.iter().map(|e| EntryKey::from(e.name.as_str())).collect();
        self.index = self.keys.iter().enumerate().map(|(i, k)| (k.clone(), i)).collect();
    }

    /// Apply one listing message. Returns `true` when the listing finished.
    pub fn on_listing(&mut self, chunk: Vec<EntryInfo>, done: bool, err: Option<FsError>) -> bool {
        if self.refreshing {
            if done {
                self.refreshing = false;
                if let Some(e) = err {
                    self.status = ListingStatus::Failed(e);
                    self.entries.clear();
                } else {
                    let mut fresh = chunk;
                    sort_entries(&mut fresh, &self.view.sort);
                    self.entries = fresh;
                    self.status = ListingStatus::Ready;
                }
                self.after_change();
            }
            return done;
        }
        self.incoming.extend(chunk);
        if !done {
            // Merged once per frame in `flush_incoming` (cheap even for 100k entries).
            self.dirty = true;
            return false;
        }
        let incoming = std::mem::take(&mut self.incoming);
        listing::merge_sorted(&mut self.entries, incoming, &self.view.sort);
        {
            self.status = match err {
                Some(e) if self.entries.is_empty() => ListingStatus::Failed(e),
                Some(e) => {
                    log::warn!("listing {} incomplete: {e}", self.location.display());
                    ListingStatus::Ready
                }
                None => ListingStatus::Ready,
            };
        }
        self.after_change();
        done
    }

    /// Apply a watcher patch.
    pub fn on_patch(&mut self, updated: Vec<EntryInfo>, removed: &[String]) {
        listing::apply_patch(&mut self.entries, updated, removed, &self.view.sort);
        self.after_change();
    }

    fn after_change(&mut self) {
        self.rebuild_index();
        if !self.is_loading() {
            let index = &self.index;
            self.selection.retain(|k| index.contains_key(k));
            if self.cursor.as_deref().is_some_and(|c| !self.index.contains_key(c)) {
                self.cursor = None;
            }
        }
        if let Some(name) = self.pending_cursor.clone()
            && let Some(&i) = self.index.get(name.as_str())
        {
            let key = self.keys[i].clone();
            self.selection.select_only(key.clone());
            self.cursor = Some(key);
            self.scroll_to_cursor = self.restore_scroll.is_none();
            self.pending_cursor = None;
        }
        if !self.is_loading() && !self.pending_selection.is_empty() {
            for name in std::mem::take(&mut self.pending_selection) {
                if let Some(&i) = self.index.get(name.as_str()) {
                    self.selection.insert(self.keys[i].clone());
                }
            }
        }
        self.dirty = true;
    }

    /// Replace the entries keeping the given order (no re-sort), e.g. search relevance.
    pub fn set_entries_ordered(&mut self, entries: Vec<EntryInfo>) {
        self.incoming.clear();
        self.entries = entries;
        self.after_change();
    }

    /// Re-sort after a sort spec change.
    pub fn resort(&mut self) {
        sort_entries(&mut self.entries, &self.view.sort);
        self.rebuild_index();
        self.dirty = true;
    }

    /// Recompute the visible order if needed.
    /// Merge streamed chunks received since the last frame.
    pub fn flush_incoming(&mut self) {
        if self.incoming.is_empty() {
            return;
        }
        let incoming = std::mem::take(&mut self.incoming);
        listing::merge_sorted(&mut self.entries, incoming, &self.view.sort);
        self.after_change();
    }

    pub fn refresh_visible(&mut self, show_hidden: bool, show_system: bool) {
        self.flush_incoming();
        if !self.dirty {
            return;
        }
        self.now = SystemTime::now();
        let f = ViewFilter { show_hidden, show_system, pattern: self.filter_pattern.as_ref() };
        let (mut vis, mut groups) = listing::compute_visible(&self.entries, &f, self.view.group, self.now);
        if !self.collapsed_groups.is_empty() && !groups.is_empty() {
            // Collapsed groups keep their header but hide their items.
            let mut kept = Vec::with_capacity(vis.len());
            let mut spans = Vec::with_capacity(groups.len());
            for g in &groups {
                let start = kept.len();
                if !self.collapsed_groups.contains(&g.label) {
                    kept.extend_from_slice(&vis[g.start..g.start + g.len]);
                }
                spans.push(GroupSpan { label: g.label.clone(), start, len: kept.len() - start, count: g.count });
            }
            vis = kept;
            groups = spans;
        }
        self.visible = vis;
        self.groups = groups;
        self.dirty = false;
        self.vis_version += 1;
    }

    pub fn set_filter(&mut self, text: &str) {
        if self.filter == text {
            return;
        }
        self.filter = text.to_string();
        self.filter_pattern = compile_filter(text);
        self.dirty = true;
    }

    pub fn filter_active(&self) -> bool {
        !self.filter.is_empty()
    }

    pub fn entry(&self, key: &str) -> Option<&EntryInfo> {
        self.index.get(key).map(|&i| &self.entries[i])
    }

    pub fn key_at(&self, visible_pos: usize) -> Option<&EntryKey> {
        self.visible.get(visible_pos).map(|&i| &self.keys[i])
    }

    pub fn entry_at(&self, visible_pos: usize) -> Option<&EntryInfo> {
        self.visible.get(visible_pos).map(|&i| &self.entries[i])
    }

    /// Visible position of `key`.
    pub fn visible_pos(&self, key: &str) -> Option<usize> {
        let idx = *self.index.get(key)?;
        self.visible.iter().position(|&i| i == idx)
    }

    pub fn cursor_pos(&self) -> Option<usize> {
        self.cursor.as_deref().and_then(|c| self.visible_pos(c))
    }

    /// Keys in visible order.
    pub fn visible_keys(&self) -> Vec<EntryKey> {
        self.visible.iter().map(|&i| self.keys[i].clone()).collect()
    }

    /// Selected entries in visible order (falls back to the cursor entry when nothing
    /// is selected).
    pub fn selected_entries(&self) -> Vec<&EntryInfo> {
        let mut v: Vec<&EntryInfo> = self
            .visible
            .iter()
            .filter(|&&i| self.selection.contains(&self.keys[i]))
            .map(|&i| &self.entries[i])
            .collect();
        if v.is_empty()
            && let Some(e) = self.cursor.as_deref().and_then(|c| self.entry(c))
        {
            v.push(e);
        }
        v
    }

    /// Total size of selected files and the number of selected items.
    pub fn selection_stats(&self) -> (usize, u64) {
        let mut n = 0;
        let mut bytes = 0;
        for k in self.selection.iter() {
            if let Some(e) = self.entry(k) {
                n += 1;
                if !e.is_dir_like() {
                    bytes += e.size;
                }
            }
        }
        (n, bytes)
    }

    /// Move the cursor to visible position `pos`; `extend` grows the selection from the
    /// anchor, `keep` leaves the selection alone (Ctrl+arrows), otherwise the cursor item
    /// becomes the selection.
    pub fn move_cursor(&mut self, pos: usize, extend: bool, keep: bool) {
        let Some(key) = self.key_at(pos).cloned() else { return };
        if extend {
            let anchor = self.selection.anchor().cloned().or_else(|| self.cursor.clone()).unwrap_or(key.clone());
            let order = self.visible_keys();
            self.selection.select_range(&order, &anchor, &key, false);
            self.selection.set_anchor(Some(anchor));
        } else if !keep {
            self.selection.select_only(key.clone());
        }
        self.cursor = Some(key);
        self.scroll_to_cursor = true;
    }

    /// Type-ahead: first visible entry (from the cursor on, wrapping) whose name starts
    /// with `prefix`, case-insensitively. Repeating the same single character cycles.
    pub fn find_prefix(&self, prefix: &str) -> Option<usize> {
        if prefix.is_empty() || self.visible.is_empty() {
            return None;
        }
        let lp = prefix.to_lowercase();
        let cycling = lp.chars().count() > 1 && lp.chars().all(|c| lp.starts_with(c));
        let needle: String = if cycling { lp.chars().take(1).collect() } else { lp };
        let start =
            self.cursor_pos().map(|p| if cycling || needle.chars().count() == 1 { p + 1 } else { p }).unwrap_or(0);
        let n = self.visible.len();
        (0..n)
            .map(|k| (start + k) % n)
            .find(|&p| self.entry_at(p).is_some_and(|e| e.name.to_lowercase().starts_with(&needle)))
    }
}

/// Live-filter text → pattern: glob when it contains `*`/`?`, else substring.
pub fn compile_filter(text: &str) -> Option<NamePattern> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let kind = if t.contains(['*', '?']) { PatternKind::Glob } else { PatternKind::Substring };
    NamePatternSpec::new(kind, t).compile().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::EntryKind;
    use std::path::PathBuf;

    fn file(name: &str) -> EntryInfo {
        EntryInfo::minimal(PathBuf::from("/d").join(name), EntryKind::File)
    }

    fn pane_with(names: &[&str]) -> Pane {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.on_listing(names.iter().map(|n| file(n)).collect(), true, None);
        p.refresh_visible(true, true);
        p
    }

    #[test]
    fn streaming_then_ready() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.pending_cursor = Some("b".into());
        assert!(!p.on_listing(vec![file("c"), file("a")], false, None));
        assert!(p.is_loading());
        assert!(p.on_listing(vec![file("b")], true, None));
        p.refresh_visible(true, true);
        assert_eq!(p.visible_keys().iter().map(|k| &**k).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(p.cursor.as_deref(), Some("b"), "pending cursor applied (go-up selects origin)");
        assert!(p.selection.contains("b"));
    }

    #[test]
    fn selection_survives_patch_and_resort() {
        let mut p = pane_with(&["a", "b", "c"]);
        p.selection.insert("c".into());
        p.on_patch(vec![file("aa")], &["a".to_string()]);
        p.refresh_visible(true, true);
        assert!(p.selection.contains("c"));
        assert_eq!(p.visible_pos("c"), Some(2));
        p.view.sort.dir = soyuz_core::SortDir::Desc;
        p.resort();
        p.refresh_visible(true, true);
        assert_eq!(p.visible_pos("c"), Some(0));
        p.on_patch(vec![], &["c".to_string()]);
        assert!(!p.selection.contains("c"));
    }

    #[test]
    fn soft_refresh_swaps_atomically() {
        let mut p = pane_with(&["a", "b"]);
        p.selection.insert("b".into());
        p.refreshing = true;
        p.on_listing(vec![], false, None);
        assert_eq!(p.entries.len(), 2, "old contents stay until the relist completes");
        p.on_listing(vec![file("b"), file("z")], true, None);
        p.refresh_visible(true, true);
        assert_eq!(p.entries.len(), 2);
        assert!(p.selection.contains("b"));
    }

    #[test]
    fn filter_and_cursor() {
        let mut p = pane_with(&["alpha.rs", "beta.md", "gamma.rs"]);
        p.set_filter("*.rs");
        p.refresh_visible(true, true);
        assert_eq!(p.visible.len(), 2);
        p.move_cursor(1, false, false);
        assert_eq!(p.cursor.as_deref(), Some("gamma.rs"));
        p.set_filter("BETA");
        p.refresh_visible(true, true);
        assert_eq!(p.visible.len(), 1);
        assert_eq!(p.cursor_pos(), None);
    }

    #[test]
    fn extend_selection_with_keyboard() {
        let mut p = pane_with(&["a", "b", "c", "d"]);
        p.move_cursor(1, false, false);
        p.move_cursor(2, true, false);
        p.move_cursor(3, true, false);
        let sel: Vec<&str> = p.selection.iter().map(|k| &**k).collect();
        assert_eq!(sel, ["b", "c", "d"]);
        p.move_cursor(0, true, false);
        let sel: Vec<&str> = p.selection.iter().map(|k| &**k).collect();
        assert_eq!(sel, ["a", "b"]);
    }

    #[test]
    fn type_ahead_jumps_and_cycles() {
        let mut p = pane_with(&["apple", "banana", "blueberry", "cherry"]);
        assert_eq!(p.find_prefix("bl"), Some(2));
        p.move_cursor(1, false, false);
        assert_eq!(p.find_prefix("b"), Some(2), "single char moves to the next match");
        p.move_cursor(2, false, false);
        assert_eq!(p.find_prefix("bb"), Some(1), "repeated char cycles and wraps");
        assert_eq!(p.find_prefix("zzz"), None);
        let mut ta = TypeAhead::default();
        let t0 = Instant::now();
        assert_eq!(ta.push("a", t0), "a");
        assert_eq!(ta.push("b", t0), "ab");
        assert_eq!(ta.push("c", t0 + std::time::Duration::from_secs(3)), "c");
    }

    #[test]
    fn collapsed_groups_hide_items() {
        let mut p = pane_with(&["a.rs", "b.md", "c.rs"]);
        p.view.group = soyuz_core::GroupBy::Extension;
        p.dirty = true;
        p.refresh_visible(true, true);
        assert_eq!(p.groups.len(), 2);
        p.collapsed_groups.insert(".rs".into());
        p.dirty = true;
        p.refresh_visible(true, true);
        assert_eq!(p.visible.len(), 1);
        assert_eq!(p.groups.len(), 2);
        assert_eq!(p.groups.iter().find(|g| g.label == ".rs").unwrap().len, 0);
    }
}
