//! One file pane: location, listing, visible order, selection, cursor and view state.
//! Pure state — the background requests are issued by `crate::ops`.

use std::cell::Cell;
use std::collections::HashSet;
use std::hash::{BuildHasher, RandomState};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
use soyuz_core::config::CursorAfterDelete;
use soyuz_core::{EntryInfo, FsError, Location, NamePattern, NamePatternSpec, OpControl, PatternKind, sort_entries};

use super::history::{HistoryEntry, NavHistory};
use super::listing::{self, GroupSpan, ViewFilter};
use super::selection::{ClickMods, EntryKey, Selection};
use super::view_memory::ViewState;

/// Streamed chunks are merged into a pane at most this often: a merge moves every entry and
/// recomputes the visible order, which a large folder cannot afford per frame.
pub const MERGE_INTERVAL: Duration = Duration::from_millis(100);

/// [`Pane::vis_pos`] slot of an entry that is not visible.
const NOT_VISIBLE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct PaneId(pub u64);

impl PaneId {
    pub fn next() -> PaneId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        PaneId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A listing error the pane does not show as a failure.
#[derive(Clone, Debug)]
pub struct ListingWarning {
    /// A refresh failed and the old entries stayed; otherwise the listing stopped partway
    /// and the pane shows what it got.
    pub kept_old: bool,
    pub error: FsError,
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
    pub start: crate::geom::Point,
    pub current: crate::geom::Point,
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
    /// Name → index into `entries`. Rebuilt once a listing is complete; while one streams
    /// it is out of date (`index_stale`) and lookups scan the entries.
    index: NameIndex,
    index_stale: bool,
    /// When streamed chunks were last merged.
    last_merge: Option<Instant>,
    /// Indices into `entries` in display order (filtered, grouped).
    pub visible: Vec<usize>,
    /// `entries` index → position in `visible` ([`NOT_VISIBLE`]: not shown).
    vis_pos: Vec<u32>,
    /// Group of every entry, kept until `entries` or the grouping change.
    group_cache: Option<listing::GroupIds>,
    pub groups: Vec<GroupSpan>,
    pub collapsed_groups: HashSet<String>,
    pub selection: Selection,
    /// [`Pane::selection_stats`] and what it was computed from.
    stats_cache: Cell<Option<SelectionStats>>,
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
    /// Typed into the address bar and not listed yet: if it does not exist, stay and show
    /// the error instead of moving to the nearest existing folder.
    pub typed_location: bool,
    /// A listing ended with an error the pane does not show as a failure; a toast tells
    /// the user (taken by `ops::handle_msg`).
    pub listing_warning: Option<ListingWarning>,
    /// Visible order must be recomputed before the next draw.
    pub dirty: bool,
    /// Changes reported while a listing was running (it may already have enumerated past
    /// them); re-stat'ed once it completes ([`Pane::take_deferred`]).
    pub deferred_paths: Vec<std::path::PathBuf>,
    /// Relist once the running listing completes (a job finished meanwhile).
    pub relist_pending: bool,
    /// UI language generation the group labels were built with.
    pub i18n_gen: u64,
    /// Scroll the cursor into view on the next draw.
    pub scroll_to_cursor: bool,
    /// Apply this scroll offset on the next draw (history / session restore).
    pub restore_scroll: Option<f32>,
    /// Last drawn scroll offset.
    pub scroll_y: f32,
    /// Put the cursor on (and select) this name once it shows up in the listing.
    pub pending_cursor: Option<String>,
    /// `[behaviour] cursor_after_delete`, kept current by `prepare_visible_panes`.
    pub after_delete: CursorAfterDelete,
    /// Takes over the cursor in the next `after_change`: the item under it is leaving.
    cursor_heir: Option<EntryKey>,
    /// Names to select once the listing completes (session restore).
    pub pending_selection: Vec<String>,
    pub type_ahead: TypeAhead,
    pub band: Option<Band>,
    /// When the listing was last rebuilt (for group date buckets).
    pub now: SystemTime,
    /// Items shown per "page" in the last draw (PageUp/PageDown).
    pub page_rows: usize,
    /// Layout of the last draw (keyboard navigation uses it); rebuilt when `layout_sig` changes.
    pub layout: Option<crate::state::layout::FlowLayout>,
    pub layout_sig: Option<LayoutSig>,
    /// Height of the list viewport in the last draw.
    pub viewport_h: f32,
    /// Bumped whenever `visible`/`groups` are recomputed.
    pub vis_version: u64,
    /// `(show_hidden, show_system)` of the last [`Pane::refresh_visible`], so a change to
    /// `entries` can recompute `visible` at once instead of leaving stale indices.
    vis_flags: (bool, bool),
    /// What the open context menu applies to.
    pub context: ContextTarget,
    /// The open context menu shows `[context_menu] extended` rows inline (opened with
    /// Shift+right-click).
    pub context_full: bool,
    /// Column header drag (details view): `(visible column index, pointer x)`.
    pub col_drag: Option<(usize, f32)>,
}

/// A computed [`Pane::selection_stats`].
#[derive(Clone, Copy)]
struct SelectionStats {
    selection: u64,
    vis_version: u64,
    count: usize,
    bytes: u64,
}

/// Inputs that determine a [`FlowLayout`](crate::state::layout::FlowLayout).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutSig {
    pub vis_version: u64,
    pub width: f32,
    pub cell: crate::geom::Vec2,
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
            index: NameIndex::default(),
            index_stale: false,
            last_merge: None,
            visible: Vec::new(),
            vis_pos: Vec::new(),
            group_cache: None,
            groups: Vec::new(),
            collapsed_groups: HashSet::new(),
            selection: Selection::default(),
            stats_cache: Cell::new(None),
            cursor: None,
            view,
            filter: String::new(),
            filter_pattern: None,
            history: NavHistory::default(),
            generation: 0,
            ctl: None,
            status: ListingStatus::Loading { since: Instant::now() },
            refreshing: false,
            typed_location: false,
            listing_warning: None,
            dirty: true,
            deferred_paths: Vec::new(),
            relist_pending: false,
            i18n_gen: crate::i18n::generation(),
            scroll_to_cursor: false,
            restore_scroll: None,
            scroll_y: 0.0,
            pending_cursor: None,
            after_delete: CursorAfterDelete::default(),
            cursor_heir: None,
            pending_selection: Vec::new(),
            type_ahead: TypeAhead::default(),
            band: None,
            now: SystemTime::now(),
            page_rows: 10,
            layout: None,
            layout_sig: None,
            viewport_h: 400.0,
            vis_version: 0,
            vis_flags: (false, false),
            context: ContextTarget::Background,
            context_full: false,
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
        self.index = NameIndex::default();
        self.index_stale = false;
        self.last_merge = None;
        self.visible.clear();
        self.vis_pos.clear();
        self.group_cache = None;
        self.groups.clear();
        self.selection.clear();
        self.cursor = None;
        self.band = None;
        self.filter.clear();
        self.filter_pattern = None;
        self.type_ahead = TypeAhead::default();
        self.refreshing = false;
        self.deferred_paths.clear();
        self.relist_pending = false;
        self.dirty = true;
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.status, ListingStatus::Loading { .. })
    }

    /// A complete listing is shown and no relist is running: in-place patches apply.
    pub fn accepts_patches(&self) -> bool {
        matches!(self.status, ListingStatus::Ready) && !self.refreshing
    }

    /// Remember changed paths that arrived while a listing was running (many → relist).
    pub fn defer_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        if self.relist_pending {
            return;
        }
        for p in paths {
            if !self.deferred_paths.contains(&p) {
                self.deferred_paths.push(p);
            }
        }
        if self.deferred_paths.len() > soyuz_fs::watch::MAX_CHANGED_PATHS {
            self.deferred_paths.clear();
            self.relist_pending = true;
        }
    }

    /// Work deferred while listing, once patches apply again: `(relist, paths to re-stat)`.
    pub fn take_deferred(&mut self) -> (bool, Vec<std::path::PathBuf>) {
        if !self.accepts_patches() {
            return (false, Vec::new());
        }
        let relist = std::mem::take(&mut self.relist_pending);
        let paths = std::mem::take(&mut self.deferred_paths);
        if relist { (true, Vec::new()) } else { (false, paths) }
    }

    fn rebuild_index(&mut self) {
        self.index.rebuild(&self.entries);
        self.index_stale = false;
    }

    /// Index into `entries` of the entry named `key`.
    pub fn index_of(&self, key: &str) -> Option<usize> {
        if self.index_stale {
            // A repeated name maps to its last entry, as in the index.
            return self.entries.iter().rposition(|e| e.name == key);
        }
        self.index.get(&self.entries, key)
    }

    /// Set the size of the entry at `path` (a folder size arrived). Returns whether the
    /// pane has that entry. Names of a folder listing are the last path component, so the
    /// name index finds it; other locations may name entries differently and are scanned.
    pub fn set_entry_size(&mut self, path: &Path, bytes: u64) -> bool {
        let found = if matches!(self.location, Location::Dir(_)) {
            let name = path.file_name().map(|n| n.to_string_lossy());
            name.and_then(|n| self.index_of(&n)).filter(|&i| self.entries[i].path == path)
        } else {
            self.entries.iter().position(|e| e.path == path)
        };
        match found {
            Some(i) => {
                self.entries[i].size = bytes;
                true
            }
            None => false,
        }
    }

    /// Selection key of `entries[i]`.
    pub fn key_of(&self, i: usize) -> EntryKey {
        EntryKey::from(self.entries[i].name.as_str())
    }

    /// Apply one listing message. Returns `true` when the listing finished.
    pub fn on_listing(&mut self, chunk: Vec<EntryInfo>, done: bool, err: Option<FsError>) -> bool {
        if self.refreshing {
            if done {
                self.refreshing = false;
                if let Some(e) = err {
                    if e.kind == soyuz_core::FsErrorKind::NotFound {
                        self.status = ListingStatus::Failed(e);
                        self.entries.clear();
                    } else {
                        // The folder is still there: keep what the pane shows, cursor and
                        // selection included, and say the view may be out of date.
                        self.status = ListingStatus::Ready;
                        self.listing_warning = Some(ListingWarning { kept_old: true, error: e });
                    }
                } else {
                    let mut fresh = chunk;
                    sort_entries(&mut fresh, &self.view.sort);
                    self.note_cursor_heir(&fresh);
                    self.entries = fresh;
                    self.status = ListingStatus::Ready;
                }
                self.after_change();
            }
            return done;
        }
        self.incoming.extend(chunk);
        if !done {
            // Merged by `refresh_visible`, at most every `MERGE_INTERVAL`.
            return false;
        }
        let incoming = std::mem::take(&mut self.incoming);
        listing::merge_sorted(&mut self.entries, incoming, &self.view.sort);
        {
            self.status = match err {
                Some(e) if self.entries.is_empty() => ListingStatus::Failed(e),
                Some(e) => {
                    log::warn!("listing {} incomplete: {e}", self.location.display());
                    self.listing_warning = Some(ListingWarning { kept_old: false, error: e });
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
        if self.cursor.as_deref().is_some_and(|c| removed.iter().any(|r| r == c)) {
            let mut gone: HashSet<&str> = removed.iter().map(String::as_str).collect();
            for e in &updated {
                gone.remove(e.name.as_str());
            }
            self.cursor_heir = self.heir_of_cursor(|n| gone.contains(n));
        }
        // Exact growth: a doubled buffer would be shrunk right back in `after_change`.
        self.entries.reserve_exact(updated.len());
        listing::apply_patch(&mut self.entries, updated, removed, &self.view.sort);
        self.after_change();
    }

    /// Before `fresh` replaces the entries: find who takes over a cursor that is not in it.
    fn note_cursor_heir(&mut self, fresh: &[EntryInfo]) {
        let Some(c) = self.cursor.as_deref() else { return };
        if fresh.iter().any(|e| e.name == c) {
            return;
        }
        let kept: HashSet<&str> = fresh.iter().map(|e| e.name.as_str()).collect();
        self.cursor_heir = self.heir_of_cursor(|n| !kept.contains(n));
    }

    /// The nearest visible entry that stays, in the direction `after_delete` asks for and
    /// then the other way. Items of the selection are passed over while others are left:
    /// they are probably leaving too, and the cursor should not hop along with them.
    /// Call before the entries change (`visible` holds indices into them).
    fn heir_of_cursor(&self, gone: impl Fn(&str) -> bool) -> Option<EntryKey> {
        if self.after_delete == CursorAfterDelete::None || self.pending_cursor.is_some() {
            return None;
        }
        let pos = self.cursor_pos()?;
        let (before, after) = (&self.visible[..pos], &self.visible[pos + 1..]);
        let find = |past_selected: bool| {
            let fits = |i: &&usize| {
                let name = &self.entries[**i].name;
                !gone(name) && (past_selected || !self.selection.contains(name))
            };
            if self.after_delete == CursorAfterDelete::Next {
                after.iter().find(fits).or_else(|| before.iter().rev().find(fits))
            } else {
                before.iter().rev().find(fits).or_else(|| after.iter().find(fits))
            }
        };
        find(false).or_else(|| find(true)).map(|&i| self.key_of(i))
    }

    fn after_change(&mut self) {
        if !self.is_loading() {
            // A finished listing stops growing: give back the doubling slack (up to half
            // of the buffer).
            self.entries.shrink_to_fit();
        }
        self.group_cache = None;
        if self.is_loading() {
            // Rebuilt once, when the listing completes.
            self.index_stale = true;
        } else {
            self.rebuild_index();
            let had_selection = !self.selection.is_empty();
            let (index, entries) = (&self.index, &self.entries);
            self.selection.retain(|k| index.get(entries, k).is_some());
            if self.cursor.as_deref().is_some_and(|c| self.index_of(c).is_none()) {
                self.cursor = self.cursor_heir.take();
                self.scroll_to_cursor |= self.cursor.is_some();
            }
            // Delete again acts on what is highlighted: once the last selected item has
            // left, the highlight belongs to the cursor.
            if had_selection
                && self.selection.is_empty()
                && let Some(c) = self.cursor.clone()
            {
                self.selection.select_only(c);
            }
        }
        self.cursor_heir = None;
        if self.pending_cursor.as_deref().is_some_and(|n| self.index_of(n).is_some())
            && let Some(name) = self.pending_cursor.take()
        {
            let key = EntryKey::from(name);
            self.selection.select_only(key.clone());
            self.cursor = Some(key);
            self.scroll_to_cursor = self.restore_scroll.is_none();
        }
        if !self.is_loading() && !self.pending_selection.is_empty() {
            for name in std::mem::take(&mut self.pending_selection) {
                if self.index_of(&name).is_some() {
                    self.selection.insert(EntryKey::from(name));
                }
            }
        }
        // `visible` holds indices into `entries`: rebuild it now, so nothing reads stale
        // indices between this change and the next frame's `refresh_visible`.
        self.recompute_visible();
    }

    /// Replace the entries keeping the given order (no re-sort), e.g. search relevance.
    pub fn set_entries_ordered(&mut self, entries: Vec<EntryInfo>) {
        self.incoming.clear();
        self.note_cursor_heir(&entries);
        self.entries = entries;
        self.after_change();
    }

    /// Re-sort after a sort spec change.
    pub fn resort(&mut self) {
        sort_entries(&mut self.entries, &self.view.sort);
        self.group_cache = None;
        self.rebuild_index();
        self.recompute_visible();
    }

    /// Re-sort by a key the entries don't carry (a script column's text; see
    /// `soyuz_core::sort_entries_keyed`).
    pub fn resort_keyed(&mut self, key: impl Fn(&EntryInfo) -> Option<String>) {
        soyuz_core::sort_entries_keyed(&mut self.entries, &self.view.sort, key);
        self.group_cache = None;
        self.rebuild_index();
        self.recompute_visible();
    }

    /// Merge streamed chunks received so far.
    pub fn flush_incoming(&mut self) {
        if self.incoming.is_empty() {
            return;
        }
        let incoming = std::mem::take(&mut self.incoming);
        listing::merge_sorted(&mut self.entries, incoming, &self.view.sort);
        self.last_merge = Some(Instant::now());
        self.after_change();
    }

    /// Streamed chunks wait to be merged: the frontend needs another frame within
    /// [`MERGE_INTERVAL`], even if the listing goes quiet.
    pub fn merge_pending(&self) -> bool {
        !self.incoming.is_empty()
    }

    /// The first chunk shows at once; later ones at most every [`MERGE_INTERVAL`].
    fn merge_due(&self) -> bool {
        self.merge_pending()
            && (self.entries.is_empty() || self.last_merge.is_none_or(|t| t.elapsed() >= MERGE_INTERVAL))
    }

    /// Merge what streamed in (when due) and recompute the visible order if needed.
    pub fn refresh_visible(&mut self, show_hidden: bool, show_system: bool) {
        if self.merge_due() {
            self.flush_incoming();
        }
        // Group labels are translated: rebuild them after a language switch.
        let i18n_gen = crate::i18n::generation();
        if self.i18n_gen != i18n_gen {
            self.i18n_gen = i18n_gen;
            self.collapsed_groups.clear();
            self.dirty = true;
        }
        if self.vis_flags != (show_hidden, show_system) {
            self.vis_flags = (show_hidden, show_system);
            self.dirty = true;
        }
        if self.dirty {
            self.recompute_visible();
        }
    }

    /// Rebuild `visible` / `groups` from `entries` with the last view flags.
    fn recompute_visible(&mut self) {
        let (show_hidden, show_system) = self.vis_flags;
        self.now = SystemTime::now();
        let f = ViewFilter { show_hidden, show_system, pattern: self.filter_pattern.as_ref() };
        let mut vis = listing::filter_visible(&self.entries, &f);
        let mut groups = Vec::new();
        if self.view.group != soyuz_core::GroupBy::None {
            let i18n_gen = crate::i18n::generation();
            let reuse = self.group_cache.take().filter(|c| c.reusable(self.entries.len(), self.view.group, i18n_gen));
            let ids =
                reuse.unwrap_or_else(|| listing::GroupIds::build(&self.entries, self.view.group, self.now, i18n_gen));
            groups = ids.apply(&mut vis);
            self.group_cache = Some(ids);
        }
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
        self.vis_pos.clear();
        self.vis_pos.resize(self.entries.len(), NOT_VISIBLE);
        for (pos, &i) in vis.iter().enumerate() {
            self.vis_pos[i] = pos as u32;
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
        self.recompute_visible();
        // A cursor the filter hides must not stay the target of Delete, Rename or Enter.
        if self.cursor.is_some() && self.cursor_pos().is_none() {
            self.cursor = None;
        }
    }

    pub fn filter_active(&self) -> bool {
        !self.filter.is_empty()
    }

    /// Which empty state to draw (after [`Pane::refresh_visible`] with the same flags).
    pub fn empty_kind(&self, show_hidden: bool, show_system: bool) -> listing::EmptyKind {
        listing::classify_empty(
            &self.entries,
            self.visible.len(),
            self.groups.len(),
            self.filter_active(),
            show_hidden,
            show_system,
        )
    }

    pub fn entry(&self, key: &str) -> Option<&EntryInfo> {
        self.index_of(key).map(|i| &self.entries[i])
    }

    /// Key (name) of the entry at visible position `visible_pos`.
    pub fn key_at(&self, visible_pos: usize) -> Option<&str> {
        self.visible.get(visible_pos).map(|&i| self.entries[i].name.as_str())
    }

    pub fn entry_at(&self, visible_pos: usize) -> Option<&EntryInfo> {
        self.visible.get(visible_pos).map(|&i| &self.entries[i])
    }

    /// Visible position of `key`.
    pub fn visible_pos(&self, key: &str) -> Option<usize> {
        let idx = self.index_of(key)?;
        match self.vis_pos.get(idx).copied() {
            Some(NOT_VISIBLE) => None,
            Some(pos) if self.visible.get(pos as usize) == Some(&idx) => Some(pos as usize),
            // `visible` was edited behind the map.
            _ => self.visible.iter().position(|&i| i == idx),
        }
    }

    pub fn cursor_pos(&self) -> Option<usize> {
        self.cursor.as_deref().and_then(|c| self.visible_pos(c))
    }

    /// Keys in visible order.
    pub fn visible_keys(&self) -> Vec<EntryKey> {
        self.visible.iter().map(|&i| self.key_of(i)).collect()
    }

    /// Selected entries in visible order (falls back to the cursor entry when nothing
    /// visible is selected and the cursor itself is on screen: an entry the filter hides
    /// is never acted on).
    pub fn selected_entries(&self) -> Vec<&EntryInfo> {
        let mut v: Vec<&EntryInfo> = self
            .visible
            .iter()
            .filter(|&&i| self.selection.contains(&self.entries[i].name))
            .map(|&i| &self.entries[i])
            .collect();
        if v.is_empty()
            && let Some(pos) = self.cursor_pos()
            && let Some(e) = self.entry_at(pos)
        {
            v.push(e);
        }
        v
    }

    /// Number of selected items and the total size of the selected files. The status bar
    /// asks every frame: computed once per change of the selection or the listing (only
    /// folders change size without a new `vis_version`, and they do not count).
    pub fn selection_stats(&self) -> (usize, u64) {
        let (selection, vis_version) = (self.selection.generation(), self.vis_version);
        if let Some(c) = self.stats_cache.get()
            && (c.selection, c.vis_version) == (selection, vis_version)
        {
            return (c.count, c.bytes);
        }
        let (mut count, mut bytes) = (0, 0);
        for k in self.selection.iter() {
            // Only what an operation would act on: an item the filter hides does not count.
            if self.visible_pos(k).is_some()
                && let Some(e) = self.entry(k)
            {
                count += 1;
                if !e.is_dir_like() {
                    bytes += e.size;
                }
            }
        }
        self.stats_cache.set(Some(SelectionStats { selection, vis_version, count, bytes }));
        (count, bytes)
    }

    /// Click semantics of [`Selection::click`] in the visible order. Only a Shift range
    /// looks at the order, and it allocates a key per item of the range alone.
    pub fn click(&mut self, key: EntryKey, mods: ClickMods) {
        match self.selection.anchor().cloned() {
            Some(anchor) if mods.range => self.select_visible_range(&anchor, &key, mods.toggle),
            _ => self.selection.click(&[], key, mods),
        }
    }

    /// [`Selection::select_range`] over the visible order, without building a key list.
    fn select_visible_range(&mut self, from: &str, to: &str, additive: bool) {
        let pos = |key: &str| self.visible.iter().position(|&i| self.entries[i].name == key);
        let (a, b) = (pos(from), pos(to));
        if !additive {
            self.selection.clear_keys();
        }
        let range = match (a, b) {
            (Some(a), Some(b)) => a.min(b)..a.max(b) + 1,
            (_, Some(b)) => b..b + 1,
            _ => return,
        };
        for vis_index in range {
            self.selection.insert(self.key_of(self.visible[vis_index]));
        }
    }

    /// Move the cursor to visible position `pos`; `extend` grows the selection from the
    /// anchor, `keep` leaves the selection alone (Ctrl+arrows), otherwise the cursor item
    /// becomes the selection.
    pub fn move_cursor(&mut self, pos: usize, extend: bool, keep: bool) {
        let Some(key) = self.key_at(pos).map(EntryKey::from) else { return };
        if extend {
            let anchor = self.selection.anchor().cloned().or_else(|| self.cursor.clone()).unwrap_or(key.clone());
            self.select_visible_range(&anchor, &key, false);
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
        let lp = soyuz_core::fold_for_match(prefix);
        let cycling = lp.chars().count() > 1 && lp.chars().all(|c| lp.starts_with(c));
        let needle: String = if cycling { lp.chars().take(1).collect() } else { lp };
        let start =
            self.cursor_pos().map(|p| if cycling || needle.chars().count() == 1 { p + 1 } else { p }).unwrap_or(0);
        let n = self.visible.len();
        (0..n)
            .map(|k| (start + k) % n)
            .find(|&p| self.entry_at(p).is_some_and(|e| soyuz_core::fold_starts_with(&e.name, &needle)))
    }
}

/// Name → index into a pane's `entries`. The table holds only indices (4 bytes a slot);
/// names are read from the entries, so a listing keeps no second copy of each name.
#[derive(Default)]
struct NameIndex {
    table: HashTable<u32>,
    hasher: RandomState,
}

impl NameIndex {
    /// Index every entry by name; a repeated name (search results) maps to its last entry.
    fn rebuild(&mut self, entries: &[EntryInfo]) {
        let NameIndex { table, hasher } = self;
        let rehash = |&j: &u32| hasher.hash_one(entries[j as usize].name.as_str());
        *table = HashTable::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            let i = u32::try_from(i).expect("fewer than 2^32 entries");
            let eq = |&j: &u32| entries[j as usize].name == e.name;
            match table.entry(hasher.hash_one(e.name.as_str()), eq, rehash) {
                Entry::Occupied(mut o) => *o.get_mut() = i,
                Entry::Vacant(v) => {
                    v.insert(i);
                }
            }
        }
    }

    fn get(&self, entries: &[EntryInfo], key: &str) -> Option<usize> {
        self.table.find(self.hasher.hash_one(key), |&j| entries[j as usize].name == key).map(|&j| j as usize)
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

    /// The cursor on `at` with `selected` selected, as if the user had moved there.
    fn on(p: &mut Pane, at: &str, selected: &[&str]) {
        p.cursor = Some(EntryKey::from(at));
        p.selection.clear();
        for s in selected {
            p.selection.insert(EntryKey::from(*s));
        }
    }

    #[test]
    fn deleting_the_item_under_the_cursor_moves_to_the_next() {
        let mut p = pane_with(&["a", "b", "c"]);
        on(&mut p, "b", &["b"]);
        p.on_patch(Vec::new(), &["b".into()]);
        assert_eq!(p.cursor.as_deref(), Some("c"));
        assert!(p.selection.contains("c"), "highlighted, so Delete again takes it");
        assert_eq!(p.selected_entries().len(), 1);
        assert!(p.scroll_to_cursor);
    }

    #[test]
    fn at_the_end_of_the_list_the_cursor_moves_back() {
        let mut p = pane_with(&["a", "b", "c"]);
        on(&mut p, "c", &["c"]);
        p.on_patch(Vec::new(), &["c".into()]);
        assert_eq!(p.cursor.as_deref(), Some("b"));
        p.on_patch(Vec::new(), &["b".into()]);
        assert_eq!(p.cursor.as_deref(), Some("a"));
        // Nothing left: no cursor.
        p.on_patch(Vec::new(), &["a".into()]);
        assert_eq!(p.cursor, None);
        assert!(p.selection.is_empty());
    }

    #[test]
    fn a_deleted_selection_is_passed_over_whatever_order_it_goes_in() {
        for order in [["b", "c", "d"], ["d", "c", "b"], ["c", "b", "d"]] {
            let mut p = pane_with(&["a", "b", "c", "d", "e"]);
            on(&mut p, "c", &["b", "c", "d"]);
            for gone in order {
                p.on_patch(Vec::new(), &[gone.to_string()]);
            }
            assert_eq!(p.cursor.as_deref(), Some("e"), "{order:?}");
            assert!(p.selection.contains("e"), "{order:?}");
            assert_eq!(p.selection.len(), 1, "{order:?}");
        }
        // Up to the end of the list: back to the nearest one before the selection.
        let mut p = pane_with(&["a", "b", "c", "d"]);
        on(&mut p, "c", &["c", "d"]);
        p.on_patch(Vec::new(), &["c".into(), "d".into()]);
        assert_eq!(p.cursor.as_deref(), Some("b"));
    }

    #[test]
    fn the_setting_chooses_the_direction() {
        let mut p = pane_with(&["a", "b", "c"]);
        p.after_delete = CursorAfterDelete::Previous;
        on(&mut p, "b", &["b"]);
        p.on_patch(Vec::new(), &["b".into()]);
        assert_eq!(p.cursor.as_deref(), Some("a"));
        p.on_patch(Vec::new(), &["a".into()]);
        assert_eq!(p.cursor.as_deref(), Some("c"), "nothing before it: the other way");

        let mut p = pane_with(&["a", "b", "c"]);
        p.after_delete = CursorAfterDelete::None;
        on(&mut p, "b", &["b"]);
        p.on_patch(Vec::new(), &["b".into()]);
        assert_eq!(p.cursor, None);
        assert!(p.selection.is_empty());
    }

    #[test]
    fn the_cursor_stays_when_something_else_is_deleted() {
        let mut p = pane_with(&["a", "b", "c"]);
        on(&mut p, "a", &["a", "c"]);
        p.on_patch(Vec::new(), &["c".into()]);
        assert_eq!(p.cursor.as_deref(), Some("a"));
        assert_eq!(p.selection.len(), 1);
    }

    #[test]
    fn a_relist_finds_the_neighbour_too() {
        let mut p = pane_with(&["a", "b", "c"]);
        on(&mut p, "b", &["b"]);
        p.refreshing = true;
        p.on_listing(vec![file("a"), file("c")], true, None);
        assert_eq!(p.cursor.as_deref(), Some("c"));
        assert!(p.selection.contains("c"));
    }

    #[test]
    fn a_rename_keeps_its_own_cursor() {
        let mut p = pane_with(&["a", "b", "c"]);
        on(&mut p, "a", &["a"]);
        p.pending_cursor = Some("x".into());
        p.on_patch(Vec::new(), &["a".into()]);
        assert_eq!(p.cursor, None, "not on the neighbour for a moment");
        p.on_patch(vec![file("x")], &[]);
        assert_eq!(p.cursor.as_deref(), Some("x"));
        // Updated in the same patch: it did not leave.
        let mut p = pane_with(&["a", "b"]);
        on(&mut p, "a", &["a"]);
        p.on_patch(vec![file("a")], &["a".into()]);
        assert_eq!(p.cursor.as_deref(), Some("a"));
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
    fn visible_follows_entries_before_the_next_refresh() {
        // A watcher patch or relist that shrinks `entries` must not leave `visible`
        // pointing past the end until the frame's `refresh_visible` (a panic in
        // `selected_entries` when an action ran in between).
        let mut p = pane_with(&["a", "b", "c"]);
        p.selection.insert("a".into());
        p.on_patch(vec![], &["b".to_string(), "c".to_string()]);
        assert_eq!(p.visible_keys().iter().map(|k| &**k).collect::<Vec<_>>(), ["a"]);
        assert_eq!(p.selected_entries().len(), 1);
        p.refreshing = true;
        p.on_listing(vec![], true, None);
        assert!(p.visible.is_empty());
        assert!(p.selected_entries().is_empty());
    }

    fn sized(name: &str, size: u64) -> EntryInfo {
        let mut e = file(name);
        e.size = size;
        e
    }

    fn shown(p: &Pane) -> Vec<String> {
        p.visible_keys().iter().map(|k| k.to_string()).collect()
    }

    /// Watcher patches (create / modify / rename / delete) always reach `visible`.
    #[test]
    fn watcher_patch_sequence_updates_visible() {
        let mut p = pane_with(&["b", "d"]);
        let v0 = p.vis_version;
        p.on_patch(vec![file("c")], &[]);
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["b", "c", "d"], "create merged in sort order");
        assert!(p.vis_version > v0, "layout caches are keyed on vis_version");
        p.on_patch(vec![sized("c", 42)], &[]);
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["b", "c", "d"], "modify keeps one entry");
        assert_eq!(p.entry("c").map(|e| e.size), Some(42));
        p.on_patch(vec![file("a")], &["c".to_string()]);
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["a", "b", "d"], "rename = old removed + new added");
        p.on_patch(vec![], &["d".to_string()]);
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["a", "b"]);
        // A patch for a file the filter hides keeps it out of `visible` but in `entries`.
        p.set_filter("a");
        p.on_patch(vec![file("zz")], &[]);
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["a"]);
        assert!(p.entry("zz").is_some());
    }

    #[test]
    fn an_entry_the_filter_hides_is_never_the_target() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.on_listing(vec![file("a.txt"), file("b.txt")], true, None);
        p.refresh_visible(true, true);
        p.move_cursor(0, false, false);
        assert_eq!(p.selected_entries()[0].name, "a.txt");
        p.set_filter("b");
        assert!(p.selected_entries().is_empty(), "a.txt is selected but off screen");
        assert_eq!(p.cursor_pos(), None);
        assert_eq!(p.cursor, None);
        // A cursor hidden some other way (hidden files turned off) is not a target either.
        p.set_filter("");
        p.refresh_visible(true, true);
        p.cursor = Some("a.txt".into());
        p.selection.clear();
        assert_eq!(p.selected_entries().len(), 1);
        p.visible.retain(|&i| p.entries[i].name != "a.txt");
        assert!(p.selected_entries().is_empty());
    }

    #[test]
    fn a_failed_refresh_keeps_the_entries_unless_the_folder_is_gone() {
        use soyuz_core::FsErrorKind;
        let err = |kind| FsError::with_message(kind, "read_dir", None, "boom");
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.on_listing(vec![file("a"), file("b")], true, None);
        p.move_cursor(0, false, false);
        p.refreshing = true;
        p.on_listing(Vec::new(), true, Some(err(FsErrorKind::Io)));
        assert_eq!(p.entries.len(), 2, "the pane stays as it was");
        assert!(matches!(p.status, ListingStatus::Ready));
        assert!(p.cursor.is_some() && !p.selection.is_empty());
        assert!(p.listing_warning.take().is_some_and(|w| w.kept_old));
        p.refreshing = true;
        p.on_listing(Vec::new(), true, Some(err(FsErrorKind::NotFound)));
        assert!(p.entries.is_empty() && matches!(p.status, ListingStatus::Failed(_)));
        assert!(p.listing_warning.is_none());
    }

    #[test]
    fn a_listing_that_stops_partway_warns() {
        let err = FsError::with_message(soyuz_core::FsErrorKind::Io, "read_dir", None, "stalled");
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.status = ListingStatus::Loading { since: Instant::now() };
        p.on_listing(vec![file("a")], true, Some(err));
        assert!(matches!(p.status, ListingStatus::Ready));
        assert!(p.listing_warning.take().is_some_and(|w| !w.kept_old));
    }

    /// Changes reported while a listing runs wait for it; a relist request wins.
    #[test]
    fn deferred_work_waits_for_the_listing() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        assert!(!p.on_listing(vec![file("a")], false, None));
        assert!(!p.accepts_patches());
        p.defer_paths(vec!["/d/x".into(), "/d/x".into()]);
        assert_eq!(p.deferred_paths.len(), 1, "deduplicated");
        assert_eq!(p.take_deferred(), (false, Vec::new()), "nothing while loading");
        assert!(p.on_listing(vec![], true, None));
        assert_eq!(p.take_deferred(), (false, vec![std::path::PathBuf::from("/d/x")]));
        assert_eq!(p.take_deferred(), (false, Vec::new()), "taken once");
        // Too many changes → one relist instead.
        p.refreshing = true;
        p.defer_paths((0..100).map(|i| std::path::PathBuf::from(format!("/d/f{i}"))).collect());
        assert!(p.relist_pending && p.deferred_paths.is_empty());
        p.refreshing = false;
        assert_eq!(p.take_deferred(), (true, Vec::new()));
        // A new location forgets deferred work.
        p.defer_paths(vec!["/d/y".into()]);
        p.relist_pending = true;
        p.clear_listing();
        assert!(p.deferred_paths.is_empty() && !p.relist_pending);
    }

    /// A finished listing holds no growth slack, and lookups go through the name index.
    #[test]
    fn finished_listing_is_compact_and_indexed() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        for c in 0..10 {
            p.on_listing((0..100).map(|i| file(&format!("f{c}_{i:03}"))).collect(), false, None);
            p.flush_incoming();
        }
        assert!(p.entries.capacity() > p.entries.len(), "streaming grows by doubling");
        p.on_listing(Vec::new(), true, None);
        assert_eq!(p.entries.capacity(), p.entries.len());
        assert_eq!(p.entry("f3_042").map(|e| e.name.as_str()), Some("f3_042"));
        assert_eq!(p.index_of("nope"), None);
        p.on_patch(vec![file("zz")], &["f0_000".to_string()]);
        assert_eq!(p.entries.capacity(), p.entries.len());
        assert!(p.entry("zz").is_some() && p.entry("f0_000").is_none());
        // Search results can repeat a name: the last one wins, as with a map.
        let mut dup = file("same");
        dup.size = 7;
        p.set_entries_ordered(vec![file("same"), dup]);
        assert_eq!(p.entry("same").map(|e| e.size), Some(7));
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

    /// Shift clicks and Shift+arrows select the same set as the key-list form in `Selection`.
    #[test]
    fn range_selection_matches_the_key_list_form() {
        let names = ["a", "b", "c", "d", "e", "f"];
        let order: Vec<EntryKey> = names.iter().map(|n| EntryKey::from(*n)).collect();
        let shift = ClickMods { toggle: false, range: true };
        let ctrl = ClickMods { toggle: true, range: false };
        let ctrl_shift = ClickMods { toggle: true, range: true };
        let plain = ClickMods::default();
        let script = [
            ("b", plain),
            ("e", shift),
            ("a", shift),
            ("f", ctrl),
            ("c", ctrl_shift),
            ("d", ctrl),
            ("b", plain),
            ("b", shift),
        ];
        let mut p = pane_with(&names);
        let mut reference = Selection::default();
        for (key, mods) in script {
            p.click(key.into(), mods);
            reference.click(&order, key.into(), mods);
            assert_eq!(p.selection, reference, "{key} {mods:?}");
        }
        // A range to a key the visible order lacks selects just the target, or nothing.
        p.selection.select_only("b".into());
        p.selection.set_anchor(Some("gone".into()));
        p.click("d".into(), shift);
        assert_eq!(p.selection.iter().map(|k| &**k).collect::<Vec<_>>(), ["d"]);
        p.click("gone-too".into(), shift);
        assert!(p.selection.is_empty());
    }

    #[test]
    fn selection_stats_follow_the_selection_and_the_listing() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.on_listing(vec![sized("a", 10), sized("b", 20), sized("c", 40)], true, None);
        p.refresh_visible(true, true);
        assert_eq!(p.selection_stats(), (0, 0));
        p.selection.insert("a".into());
        p.selection.insert("b".into());
        assert_eq!(p.selection_stats(), (2, 30));
        assert_eq!(p.selection_stats(), (2, 30), "from the cache");
        p.selection.toggle("b".into());
        assert_eq!(p.selection_stats(), (1, 10));
        p.on_patch(vec![sized("a", 15)], &[]);
        assert_eq!(p.selection_stats(), (1, 15), "the file changed size");
        p.selection =
            Selection::from_band(&Selection::default(), &["c".into()], super::super::selection::BandMode::Add);
        assert_eq!(p.selection_stats(), (1, 40));
        p.on_patch(vec![], &["c".to_string()]);
        assert_eq!(p.selection_stats(), (0, 0));
        p.selection.insert("a".into());
        assert_eq!(p.selection_stats(), (1, 15));
        p.set_filter("b");
        assert_eq!(p.selection_stats(), (0, 0), "the filter hides it, so nothing would act on it");
        p.set_filter("");
        assert_eq!(p.selection_stats(), (1, 15), "and it counts again");
    }

    #[test]
    fn visible_position_map_follows_every_change() {
        let mut p = pane_with(&["a", "b", "c", "d"]);
        let check = |p: &Pane| {
            for (i, e) in p.entries.iter().enumerate() {
                let linear = p.visible.iter().position(|&v| v == i);
                assert_eq!(p.visible_pos(&e.name), linear, "{}", e.name);
            }
        };
        check(&p);
        p.set_filter("b");
        check(&p);
        p.set_filter("");
        p.view.sort.dir = soyuz_core::SortDir::Desc;
        p.resort();
        check(&p);
        p.on_patch(vec![file("aa")], &["c".to_string()]);
        check(&p);
        p.view.group = soyuz_core::GroupBy::FirstLetter;
        p.dirty = true;
        p.refresh_visible(true, true);
        check(&p);
        assert_eq!(p.visible_pos("nope"), None);
    }

    #[test]
    fn folder_size_finds_the_entry_by_path() {
        let mut p = pane_with(&["a", "b"]);
        assert!(p.set_entry_size(&PathBuf::from("/d").join("b"), 7));
        assert_eq!(p.entry("b").map(|e| e.size), Some(7));
        assert!(!p.set_entry_size(&PathBuf::from("/other").join("b"), 9), "same name, other folder");
        assert!(!p.set_entry_size(&PathBuf::from("/d").join("missing"), 9));
        assert_eq!(p.entry("b").map(|e| e.size), Some(7));
        // Other locations: names may repeat, every path is found.
        p.location = Location::Recent;
        let (x, y) = (PathBuf::from("/x").join("same"), PathBuf::from("/y").join("same"));
        p.set_entries_ordered(vec![
            EntryInfo::minimal(x.clone(), EntryKind::File),
            EntryInfo::minimal(y, EntryKind::File),
        ]);
        assert!(p.set_entry_size(&x, 5));
        assert_eq!(p.entries[0].size, 5);
    }

    /// While a listing streams, chunks wait for the merge interval, lookups still work and
    /// nothing changes about the finished result.
    #[test]
    fn streaming_merges_are_throttled() {
        let mut p = Pane::new(Location::Dir("/d".into()), ViewState::default());
        p.on_listing(vec![file("c")], false, None);
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["c"], "the first chunk shows at once");
        let v0 = p.vis_version;
        p.on_listing(vec![file("a")], false, None);
        assert!(p.merge_pending());
        // (Just merged, however slow this machine is.)
        p.last_merge = Some(Instant::now() + Duration::from_secs(3600));
        p.refresh_visible(true, true);
        assert_eq!(shown(&p), ["c"], "the next one waits");
        assert_eq!(p.vis_version, v0, "and a waiting frame recomputes nothing");
        // Lookups during the stream see what is merged, through the stale index.
        assert_eq!(p.index_of("c"), Some(0));
        assert_eq!(p.visible_pos("c"), Some(0));
        p.pending_cursor = Some("a".into());
        p.last_merge = Some(Instant::now() - MERGE_INTERVAL);
        p.refresh_visible(true, true);
        assert!(!p.merge_pending());
        assert_eq!(shown(&p), ["a", "c"]);
        assert_eq!(p.cursor.as_deref(), Some("a"), "a pending cursor lands mid-stream");
        assert_eq!(p.index_of("c"), Some(1));
        assert_eq!(p.visible_pos("c"), Some(1));
        // `done` merges the rest at once and rebuilds the index.
        p.on_listing(vec![file("b")], false, None);
        p.on_listing(vec![file("d")], true, None);
        assert_eq!(shown(&p), ["a", "b", "c", "d"]);
        assert!(!p.index_stale);
        assert_eq!(p.index_of("d"), Some(3));
    }

    #[test]
    fn group_cache_follows_entries_and_grouping() {
        let mut p = pane_with(&["a.rs", "b.md", "c.rs"]);
        p.view.group = soyuz_core::GroupBy::Extension;
        p.dirty = true;
        p.refresh_visible(true, true);
        let labels = |p: &Pane| p.groups.iter().map(|g| (g.label.clone(), g.len)).collect::<Vec<_>>();
        assert_eq!(labels(&p), [(".md".to_string(), 1), (".rs".to_string(), 2)]);
        p.set_filter("a");
        assert_eq!(labels(&p), [(".rs".to_string(), 1)], "the filter reuses the cached groups");
        p.set_filter("");
        p.on_patch(vec![file("d.txt")], &["b.md".to_string()]);
        assert_eq!(labels(&p), [(".rs".to_string(), 2), (".txt".to_string(), 1)], "new entries");
        p.view.group = soyuz_core::GroupBy::FirstLetter;
        p.dirty = true;
        p.refresh_visible(true, true);
        assert_eq!(p.groups.iter().map(|g| g.label.as_str()).collect::<Vec<_>>(), ["A", "C", "D"], "new grouping");
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

    /// Reported bug: a folder that holds only protected OS files (here `desktop.ini`,
    /// hidden + system) showed "Only hidden items" and suggested Ctrl+H, which cannot
    /// reveal them. Replays: browse a full folder, navigate into it, toggle hidden.
    #[test]
    fn protected_only_folder_reads_as_empty() {
        use super::listing::EmptyKind;
        use soyuz_core::Attributes;
        let mut p = pane_with(&["a.txt"]);
        p.refresh_visible(false, false);
        assert_eq!(p.empty_kind(false, false), EmptyKind::NotEmpty);

        // Navigate into the "empty" folder.
        p.location = Location::Dir("/d/sub".into());
        p.generation += 1;
        p.clear_listing();
        p.status = ListingStatus::Loading { since: Instant::now() };
        p.refresh_visible(false, false);
        let mut ini = file("desktop.ini");
        ini.attrs = Attributes::HIDDEN | Attributes::SYSTEM;
        assert!(p.on_listing(vec![ini], true, None));
        for show_hidden in [false, true] {
            p.dirty = true;
            p.refresh_visible(show_hidden, false);
            assert!(p.visible.is_empty());
            assert_eq!(p.empty_kind(show_hidden, false), EmptyKind::Empty, "show_hidden={show_hidden}");
        }
        // With system files shown, it is visible.
        p.dirty = true;
        p.refresh_visible(true, true);
        assert_eq!(p.empty_kind(true, true), EmptyKind::NotEmpty);

        // A real hidden file next to it: "only hidden" (Ctrl+H reveals it) ...
        let mut dot = file(".git");
        dot.attrs = Attributes::HIDDEN;
        p.on_patch(vec![dot], &[]);
        p.refresh_visible(false, false);
        assert_eq!(p.empty_kind(false, false), EmptyKind::OnlyHidden);
        // ... and deleting it (watcher patch) brings the empty state back.
        p.on_patch(vec![], &[".git".to_string()]);
        p.refresh_visible(false, false);
        assert_eq!(p.empty_kind(false, false), EmptyKind::Empty);
        // Deleting the last file of a folder: empty, not "only hidden".
        p.on_patch(vec![], &["desktop.ini".to_string()]);
        p.refresh_visible(false, false);
        assert!(p.entries.is_empty());
        assert_eq!(p.empty_kind(false, false), EmptyKind::Empty);
    }
}
