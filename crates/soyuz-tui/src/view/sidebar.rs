//! The sidebar (step T2.6): the places sections and the folder tree, mirroring the GUI's
//! `ui::sidebar` / `ui::tree` (`[sidebar] sections` order, collapsible headers kept in
//! `SidebarState`, `[sidebar] mode` deciding where the tree shows).
//!
//! Everything the sidebar lists is one flat list of [`Entry`]s built by [`entries`] from
//! the core (section headers, place rows, tree rows, notes), used by the view, the
//! keyboard (`crate::input::sidebar`) and the click targets alike:
//! `HitTarget::SidebarRow(i)` is entry `i`.
//!
//! - `places`: Favorites (favorites, `[[bookmarks]]`, GTK bookmarks), Devices (Computer,
//!   the drives with an eject mark where they can be ejected, then the unmounted and
//!   locked devices of `[sidebar] unmounted_devices`, Linux / UDisks2: Phase T7), Portal
//!   pins, Cloud folders, Other (Recent, Timeline, Trash),
//!   Network (saved connections with their state dot, "Connect to server…"), Tags, Smart
//!   folders ("New smart folder") and saved searches;
//! - `both`: the same with the folder tree as the `tree` section (at the end when the
//!   section list leaves it out, like the GUI);
//! - `tree`: only the folder tree.
//!
//! The tree is `soyuz_appcore::models::tree`: [`prepare`] pumps it (background listings,
//! follow-the-pane, lazy loading) while it is shown; the keys go to its `keyboard`.
//!
//! Devices (`soyuz_appcore::models::devices`, like the GUI's `ui::sidebar`): [`prepare`]
//! starts the core's refresher while the sidebar shows (UDisks2 devices and the GTK
//! bookmarks) and re-classifies the unmounted devices into `SidebarState::devices`; an
//! [`Entry::Device`] row shows its state mark (locked, not mounted, busy), the name and
//! the filesystem (or what runs on it) on the right. A mounted volume that can be ejected
//! ends with an eject mark (`HitTarget::SidebarEject`).
//!
//! # The keyboard cursor
//!
//! New compared with the GUI (which moves widget focus with Tab): while the sidebar has
//! the keyboard (`Focus::Sidebar`) one cursor walks the whole list, headers included. On
//! a tree row it is the tree's own selection (`TreeState::selected`); elsewhere it is an
//! entry index. The cursor and the scroll offset are a [`SidebarNav`] in the TUI's input
//! state (`crate::input::sidebar::State::nav`), which the keys and clicks move and
//! [`prepare`] / [`SidebarVm::from_core`] read.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::widgets::Paragraph;
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::host::{Host, ThemePref, Wake, WindowCmd, WindowId};
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::devices;
use soyuz_appcore::models::jobs_rows::Tone;
use soyuz_appcore::models::sidebar::section_name;
use soyuz_appcore::models::tree::model::Visibility;
use soyuz_appcore::models::tree::{TreeArgs, TreeIo};
use soyuz_appcore::services::places::{drive_volumes, volume_label};
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{Config, SidebarMode, SidebarSection};
use soyuz_fs::udisks::{BlockDevice, CandidateState};

use super::text;
use crate::dialog::safe;
use crate::icons::{IconSet, glyph};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};

/// Collapsed-set key of the saved searches (the GUI's, persisted in the session).
const SEARCHES_KEY: &str = "Searches";

/// A collapsible section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Of(SidebarSection),
    /// Saved searches (not a `[sidebar] sections` entry; always last, like the GUI).
    Searches,
}

impl Section {
    pub fn is_open(self, core: &AppCore) -> bool {
        match self {
            Section::Of(s) => core.ui.sidebar.is_open(s),
            Section::Searches => !core.ui.sidebar.collapsed.contains(SEARCHES_KEY),
        }
    }

    pub fn set_open(self, core: &mut AppCore, open: bool) {
        match self {
            Section::Of(s) => core.ui.sidebar.set_open(s, open),
            Section::Searches if open => {
                core.ui.sidebar.collapsed.remove(SEARCHES_KEY);
            }
            Section::Searches => {
                core.ui.sidebar.collapsed.insert(SEARCHES_KEY.to_string());
            }
        }
    }

    /// The heading (the GUI's wording per section).
    fn label(self) -> String {
        match self {
            Section::Of(SidebarSection::Network) => tr!("remote-network").to_string(),
            Section::Of(SidebarSection::Tags) => tr!("tools-tags-section").to_string(),
            Section::Of(SidebarSection::Smart) => tr!("views-smart-section").to_string(),
            Section::Of(s) => section_name(s).to_string(),
            Section::Searches => tr!("views-search-sidebar").to_string(),
        }
    }
}

/// What activating a place row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Open the location in the active pane (`FocusPane` + `Navigate`, the GUI's click).
    Go(Location),
    /// Open smart folder `i` (`SmartCmd::Open`).
    Smart(usize),
    /// Run saved search `i`.
    Search(usize),
    /// Open saved connection `id` at `uri` (like `Go`; its menu knows the connection).
    Remote { id: String, uri: String },
    /// "Connect to server…" (`RemoteCmd::ConnectDialog`).
    Connect,
    /// "New smart folder" (`SmartCmd::Edit(None)`).
    NewSmart,
}

/// What a place row shows before its label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    #[default]
    None,
    /// A server's state dot (`models::remote::conn_dot`).
    Dot(Tone),
    /// An action row ("Connect to server…", "New smart folder").
    Plus,
    /// A mounted volume of the Devices section (nothing before the label; its menu has
    /// the device entries, an ejectable one ends with the eject mark).
    Volume,
}

/// One line of the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Header {
        section: Section,
        label: String,
        open: bool,
    },
    Place {
        label: String,
        target: Target,
        active: bool,
        mark: Mark,
    },
    /// Row `i` of the folder tree (`TreeState::rows`).
    Tree(usize),
    /// Unmounted / locked device `cand` of `SidebarState::devices` (Phase T7).
    Device {
        cand: usize,
        label: String,
        state: CandidateState,
        /// An operation runs on it (mount, unlock, …): `detail` says which.
        busy: bool,
        /// The filesystem badge, or what runs on it.
        detail: String,
    },
    /// A note (loading, no drives yet); the cursor skips it.
    Message(String),
}

impl Entry {
    /// The cursor can stop here.
    pub fn selectable(&self) -> bool {
        !matches!(self, Entry::Message(_))
    }
}

/// The keyboard cursor and scroll position (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SidebarNav {
    /// The sidebar had the keyboard at the last check (entering it places the cursor).
    pub focused: bool,
    /// The cursor is on the tree's selected row.
    pub on_tree: bool,
    /// The cursor's entry when it is not on the tree.
    pub line: usize,
    /// First entry shown.
    pub top: usize,
    /// Rows the sidebar had in the last frame (page size).
    pub rows: u16,
    /// Cursor entry the view last scrolled to.
    pub shown_cursor: Option<usize>,
}

/// Whether the folder tree is drawn (and pumped).
pub fn tree_shown(core: &AppCore) -> bool {
    match core.config.sidebar.mode {
        SidebarMode::Tree => true,
        SidebarMode::Both => core.ui.sidebar.is_open(SidebarSection::Tree),
        SidebarMode::Places => false,
    }
}

/// Every line of the sidebar, in order (see the module docs).
pub fn entries(core: &AppCore) -> Vec<Entry> {
    let mut out = Vec::new();
    let mode = core.config.sidebar.mode;
    if mode == SidebarMode::Tree {
        tree_rows(core, &mut out);
        return out;
    }
    let sections = &core.config.sidebar.sections;
    for &sec in sections {
        match sec {
            SidebarSection::Tree if mode == SidebarMode::Both => tree_section(core, &mut out),
            SidebarSection::Favorites => favorites(core, &mut out),
            SidebarSection::Devices => devices(core, &mut out),
            SidebarSection::Portal => portal(core, &mut out),
            SidebarSection::Cloud if !core.ui.cloud.is_empty() => cloud(core, &mut out),
            SidebarSection::Trash => other(core, &mut out),
            SidebarSection::Network => network(core, &mut out),
            SidebarSection::Tags => tags(core, &mut out),
            SidebarSection::Smart => smart(core, &mut out),
            _ => {}
        }
    }
    if mode == SidebarMode::Both && !sections.contains(&SidebarSection::Tree) {
        tree_section(core, &mut out);
    }
    if !sections.contains(&SidebarSection::Smart) {
        smart(core, &mut out);
    }
    searches(core, &mut out);
    out
}

/// Push the header of `section`; returns whether it is open.
fn header(core: &AppCore, out: &mut Vec<Entry>, section: Section) -> bool {
    let open = section.is_open(core);
    out.push(Entry::Header { section, label: section.label(), open });
    open
}

fn place(core: &AppCore, out: &mut Vec<Entry>, label: String, location: Location) {
    let active = core.ws.pane().location == location;
    out.push(Entry::Place { label: safe(&label), target: Target::Go(location), active, mark: Mark::None });
}

fn favorites(core: &AppCore, out: &mut Vec<Entry>) {
    if !header(core, out, Section::Of(SidebarSection::Favorites)) {
        return;
    }
    let s = &core.ui.sidebar;
    for f in &s.favorites {
        place(core, out, f.name.clone(), Location::Dir(f.path.clone()));
    }
    let bookmarks = &core.config.bookmarks;
    for b in bookmarks {
        let label = if b.name.is_empty() { soyuz_core::entry::display_name(&b.path) } else { b.name.clone() };
        place(core, out, label, Location::Dir(b.path.clone()));
    }
    // GTK bookmarks not shown already (Linux).
    for b in soyuz_appcore::models::devices::gtk_bookmarks(&core.config.sidebar) {
        let loc = match &b.path {
            Some(p) if s.favorites.iter().any(|f| &f.path == p) || bookmarks.iter().any(|k| &k.path == p) => continue,
            Some(p) => Location::Dir(p.clone()),
            None if soyuz_core::location::looks_remote(&b.uri) => Location::Remote(b.uri.clone()),
            None => continue,
        };
        place(core, out, b.display_name(), loc);
    }
}

fn devices(core: &AppCore, out: &mut Vec<Entry>) {
    if !header(core, out, Section::Of(SidebarSection::Devices)) {
        return;
    }
    let computer = if cfg!(windows) { tr!("app-sidebar-this-pc") } else { tr!("app-sidebar-computer") };
    place(core, out, computer.to_string(), Location::Computer);
    let vols = drive_volumes(&core.ui.volumes, &core.ui.cloud);
    for v in vols.iter() {
        let location = Location::Dir(v.mount.clone());
        let active = core.ws.pane().location == location;
        let label = safe(&volume_label(v));
        out.push(Entry::Place { label, target: Target::Go(location), active, mark: Mark::Volume });
    }
    // Unmounted and locked devices (`prepare` classified them for the config).
    let busy = devices::busy();
    for (cand, c) in core.ui.sidebar.devices.list().iter().enumerate() {
        let work = busy.get(&c.dev.object).copied();
        let detail = match work {
            Some(_) => devices::candidate_status(work, c.state, c.dev.size),
            None => devices::candidate_badge(c),
        };
        let label = safe(&devices::candidate_name(c));
        out.push(Entry::Device { cand, label, state: c.state, busy: work.is_some(), detail });
    }
    if vols.is_empty() {
        out.push(Entry::Message(tr!("app-sidebar-scanning").to_string()));
    }
}

fn portal(core: &AppCore, out: &mut Vec<Entry>) {
    let pins = &core.config.sidebar.portal;
    // The GUI's empty section is a drop target; without drops there is nothing to show.
    if pins.is_empty() || !header(core, out, Section::Of(SidebarSection::Portal)) {
        return;
    }
    for p in pins {
        let label = if p.name.is_empty() { soyuz_core::entry::display_name(&p.path) } else { p.name.clone() };
        place(core, out, label, Location::Dir(p.path.clone()));
    }
}

fn cloud(core: &AppCore, out: &mut Vec<Entry>) {
    if !header(core, out, Section::Of(SidebarSection::Cloud)) {
        return;
    }
    for (name, path) in &core.ui.cloud {
        place(core, out, name.clone(), Location::Dir(path.clone()));
    }
}

fn other(core: &AppCore, out: &mut Vec<Entry>) {
    if !header(core, out, Section::Of(SidebarSection::Trash)) {
        return;
    }
    place(core, out, tr!("tools-recent").to_string(), Location::Recent);
    place(core, out, tr!("workspaces-timeline").to_string(), Location::Timeline);
    place(core, out, tr!("app-sidebar-trash").to_string(), Location::Trash);
}

fn network(core: &AppCore, out: &mut Vec<Entry>) {
    use soyuz_appcore::models::remote::conn_dot;
    use soyuz_appcore::services::remote as svc;
    if !header(core, out, Section::Of(SidebarSection::Network)) {
        return;
    }
    let here = match &core.ws.pane().location {
        Location::Remote(u) => soyuz_appcore::models::remote::RemoteUri::parse(u).ok().map(|u| u.conn_key()),
        _ => None,
    };
    for c in svc::connections() {
        let Some(uri) = c.parsed() else { continue };
        let mark = conn_dot(&svc::conn_state(&uri)).map_or(Mark::None, Mark::Dot);
        let active = here.as_deref() == Some(uri.conn_key().as_str());
        let target = Target::Remote { id: c.id.clone(), uri: uri.display() };
        out.push(Entry::Place { label: safe(&c.name), target, active, mark });
    }
    let connect = tr!("remote-connect").to_string();
    out.push(Entry::Place { label: connect, target: Target::Connect, active: false, mark: Mark::Plus });
}

fn tags(core: &AppCore, out: &mut Vec<Entry>) {
    use soyuz_appcore::services::tags;
    if !header(core, out, Section::Of(SidebarSection::Tags)) {
        return;
    }
    for t in 0..tags::TAG_COUNT as u8 {
        place(core, out, tags::name(t), Location::Tags(tags::slug(t)));
    }
}

fn smart(core: &AppCore, out: &mut Vec<Entry>) {
    use soyuz_appcore::models::smart;
    let list = smart::folders();
    // Shown with smart folders or while the index runs (the GUI's rule).
    let shown = !list.is_empty() || soyuz_appcore::services::index::is_enabled();
    if !shown || !header(core, out, Section::Of(SidebarSection::Smart)) {
        return;
    }
    let current = smart::shown_name(&core.ws.pane().location);
    for (i, f) in list.iter().enumerate() {
        let active = current.as_deref() == Some(f.name.as_str());
        out.push(Entry::Place { label: safe(&f.name), target: Target::Smart(i), active, mark: Mark::None });
    }
    let new = tr!("views-smart-new").to_string();
    out.push(Entry::Place { label: new, target: Target::NewSmart, active: false, mark: Mark::Plus });
}

fn searches(core: &AppCore, out: &mut Vec<Entry>) {
    let names = core.ui.search.names();
    if names.is_empty() || !header(core, out, Section::Searches) {
        return;
    }
    for (i, n) in names.iter().enumerate() {
        out.push(Entry::Place { label: safe(n), target: Target::Search(i), active: false, mark: Mark::None });
    }
}

fn tree_section(core: &AppCore, out: &mut Vec<Entry>) {
    if header(core, out, Section::Of(SidebarSection::Tree)) {
        tree_rows(core, out);
    }
}

fn tree_rows(core: &AppCore, out: &mut Vec<Entry>) {
    let s = &core.ui.tree;
    if s.rows().is_empty() {
        let msg = if s.filter.trim().is_empty() { tr!("common-loading") } else { tr!("views-tree-no-matches") };
        out.push(Entry::Message(msg.to_string()));
    }
    out.extend((0..s.rows().len()).map(Entry::Tree));
}

/// The entry of the tree's selected row, if it is listed.
pub fn tree_selection(core: &AppCore, entries: &[Entry]) -> Option<usize> {
    let s = &core.ui.tree;
    let row = s.selected.as_ref().and_then(|k| s.row_index(k))?;
    entries.iter().position(|e| *e == Entry::Tree(row))
}

/// The cursor's entry while the sidebar has the keyboard.
pub fn cursor_index(core: &AppCore, entries: &[Entry], n: &SidebarNav) -> Option<usize> {
    if !n.focused || entries.is_empty() {
        return None;
    }
    if n.on_tree
        && let Some(i) = tree_selection(core, entries)
    {
        return Some(i);
    }
    Some(n.line.min(entries.len() - 1))
}

/// Keep [`SidebarNav::focused`] in step with the core's focus; entering the sidebar puts
/// the cursor on the current location's place, else on the tree's selection, else on the
/// first place.
pub fn sync_focus(core: &AppCore, n: &mut SidebarNav, entries: &[Entry]) {
    let focused = core.ui.focus == Focus::Sidebar;
    if focused && !n.focused {
        let active = entries.iter().position(|e| matches!(e, Entry::Place { active: true, .. }));
        let tree = tree_selection(core, entries);
        let tree_first = core.config.sidebar.mode == SidebarMode::Tree;
        n.on_tree = false;
        match (active, tree) {
            (_, Some(_)) if tree_first => n.on_tree = true,
            (Some(i), _) => n.line = i,
            (None, Some(_)) => n.on_tree = true,
            (None, None) => {
                n.line = entries
                    .iter()
                    .position(|e| matches!(e, Entry::Place { .. } | Entry::Tree(_)))
                    .or_else(|| entries.iter().position(Entry::selectable))
                    .unwrap_or(0);
            }
        }
        n.shown_cursor = None;
    }
    n.focused = focused;
}

/// The sidebar's rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarVm {
    /// The lines in view (from the scroll position).
    pub lines: Vec<SidebarLine>,
    /// The sidebar has the keyboard.
    pub focused: bool,
}

/// One drawn line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarLine {
    /// Entry index ([`entries`]; the click target).
    pub index: usize,
    pub kind: LineKind,
    pub label: String,
    /// Tree depth (0 for everything else).
    pub depth: usize,
    /// Shows the active pane's location.
    pub active: bool,
    /// The keyboard cursor is here.
    pub cursor: bool,
    /// Hidden folder or failed listing.
    pub dim: bool,
    /// What a place row shows before its label.
    pub mark: Mark,
    /// Right-aligned text (a device's filesystem or what runs on it).
    pub right: String,
    /// A mounted volume that can be ejected: the eject mark at the end.
    pub eject: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Header {
        open: bool,
    },
    Place,
    /// An unmounted / locked device (Phase T7).
    Device {
        state: CandidateState,
        busy: bool,
    },
    Folder {
        expandable: bool,
        expanded: bool,
        loading: bool,
        failed: bool,
    },
    Message,
}

impl SidebarVm {
    pub fn from_core(core: &AppCore, n: &SidebarNav) -> SidebarVm {
        if !shown(core) || n.rows == 0 {
            return SidebarVm::default();
        }
        let entries = entries(core);
        let cursor = cursor_index(core, &entries, n);
        let current = match &core.ws.pane().location {
            Location::Dir(d) => Some(d.as_path()),
            _ => None,
        };
        let rows = core.ui.tree.rows();
        let all = devices::block_devices();
        let lines = entries
            .iter()
            .enumerate()
            .skip(n.top)
            .take(usize::from(n.rows))
            .filter_map(|(index, e)| {
                let line = |kind, label: String, active| SidebarLine {
                    index,
                    kind,
                    label,
                    depth: 0,
                    active,
                    cursor: cursor == Some(index),
                    dim: false,
                    mark: Mark::None,
                    right: String::new(),
                    eject: false,
                };
                Some(match e {
                    Entry::Header { label, open, .. } => line(LineKind::Header { open: *open }, label.clone(), false),
                    Entry::Place { label, active, mark, target } => {
                        let eject = match (mark, target) {
                            (Mark::Volume, Target::Go(Location::Dir(p))) => ejectable(&all, p).is_some(),
                            _ => false,
                        };
                        SidebarLine { mark: *mark, eject, ..line(LineKind::Place, label.clone(), *active) }
                    }
                    Entry::Device { label, state, busy, detail, .. } => SidebarLine {
                        right: detail.clone(),
                        dim: *state == CandidateState::Unsupported,
                        ..line(LineKind::Device { state: *state, busy: *busy }, label.clone(), false)
                    },
                    Entry::Message(m) => line(LineKind::Message, m.clone(), false),
                    Entry::Tree(i) => {
                        let r = rows.get(*i)?;
                        let kind = LineKind::Folder {
                            expandable: r.expandable,
                            expanded: r.expanded,
                            loading: r.loading,
                            failed: r.failed,
                        };
                        SidebarLine {
                            depth: r.depth,
                            dim: r.hidden || r.failed,
                            ..line(kind, safe(&r.name), current == Some(r.key.path.as_path()))
                        }
                    }
                })
            })
            .collect();
        SidebarVm { lines, focused: n.focused }
    }
}

/// The device of the volume mounted at `mount` when its drive can be ejected
/// (`models::devices::{device_at, eject_plan}`, the GUI's eject button).
pub fn ejectable(all: &[BlockDevice], mount: &std::path::Path) -> Option<BlockDevice> {
    if all.is_empty() {
        return None;
    }
    devices::device_at(all, mount).filter(|d| devices::eject_plan(d, all).is_some())
}

/// Whether the sidebar takes columns (`layout::Facts::sidebar`; hidden anyway below
/// `layout::SIDEBAR_MIN_COLS`).
pub fn shown(core: &AppCore) -> bool {
    core.ui.sidebar_visible
}

/// Wanted sidebar width in columns (`layout::Facts::sidebar_width`): `[tui] sidebar_width`.
pub fn width(core: &AppCore) -> u16 {
    core.config.tui.sidebar_width_clamped()
}

/// The tree's host: wake-ups go to the frame loop through the core's [`Wake`]
/// (`prepare` has no `Host`). A delayed redraw wakes at once: the frame loop keeps frames
/// `MIN_FRAME` apart, so the tree's 220 ms keyboard-navigation delay costs a few frames.
struct WakeHost(Wake);

impl Host for WakeHost {
    fn wake(&self) -> Wake {
        self.0.clone()
    }
    fn redraw(&self) {
        (self.0)();
    }
    fn redraw_after(&self, _after: Duration) {
        (self.0)();
    }
    fn copy_text(&self, _text: String) {}
    fn set_theme(&self, _pref: ThemePref) {}
    fn window(&self, _id: WindowId, _cmd: WindowCmd) {}
    fn config_changed(&self, _old: &Config, _new: &Config) {}
}

/// One frame of the folder tree (`models::tree::pump`, like the GUI's `ui::tree::show`).
fn pump_tree(core: &mut AppCore) {
    let host = WakeHost(core.wake.clone());
    let current = core.ws.pane().location.clone();
    let pane = core.ws.pane().id;
    let cfg = &core.config;
    let mode = cfg.sidebar.mode;
    let follow = cfg.sidebar.tree_follow;
    let drag_default = cfg.behaviour.drag_default;
    let debounce = Duration::from_millis(cfg.behaviour.watch_debounce_ms.max(50));
    let vis = Visibility { show_hidden: core.ui.show_hidden, show_system: core.ui.show_system };
    let keyboard = core.ui.focus == Focus::Sidebar;
    let vols = drive_volumes(&core.ui.volumes, &core.ui.cloud);
    let mut args = TreeArgs {
        state: &mut core.ui.tree,
        mode,
        vis,
        follow,
        // No drag and drop in the terminal.
        drag_expand: None,
        drag_default,
        keyboard,
        debounce,
    };
    let mut io = TreeIo { volumes: &vols, current: &current, pane, cmds: &mut core.cmds };
    soyuz_appcore::models::tree::pump(&host, &mut args, &mut io, volume_label);
}

/// Before the model is built: favorites follow the places probe, the tree is pumped, the
/// focus and scroll position are settled.
pub fn prepare(core: &mut AppCore, n: &mut SidebarNav, regions: &Regions) {
    let rect = regions.sidebar;
    if rect.is_empty() {
        // Hidden (or too narrow): the keyboard goes back to the list.
        if core.ui.focus == Focus::Sidebar {
            core.ui.focus = Focus::List;
        }
        n.focused = false;
        n.rows = 0;
        return;
    }
    // While the places probe runs this asks for frames; the idle tick brings them.
    let _ = core.ui.sidebar.refresh_favorites();
    // The device refresher (Linux: UDisks2 and the GTK bookmarks; once per process), then
    // the unmounted devices for the config (`[sidebar] unmounted_devices`, hidden ones).
    devices::start(&WakeHost(core.wake.clone()));
    let _ = devices::candidates(&mut core.ui.sidebar.devices, &core.config.sidebar);
    if tree_shown(core) {
        pump_tree(core);
    }
    let entries = entries(core);
    sync_focus(core, n, &entries);
    let h = usize::from(rect.height);
    n.rows = rect.height;
    let follow = core.ui.tree.take_scroll();
    let cursor = cursor_index(core, &entries, n);
    let target = if cursor.is_some() && cursor != n.shown_cursor {
        cursor
    } else if follow {
        tree_selection(core, &entries)
    } else {
        None
    };
    n.shown_cursor = cursor;
    if let Some(c) = target {
        if c < n.top {
            n.top = c;
        } else if c >= n.top + h {
            n.top = c + 1 - h;
        }
    }
    n.top = n.top.min(entries.len().saturating_sub(h));
}

/// The chevron of an open / closed header or folder.
fn chevron(open: bool, set: IconSet) -> &'static str {
    glyph(if open { IconId::CaretDown } else { IconId::CaretRight }, set)
}

/// The one-cell mark before a place's label.
fn mark_glyph(mark: Mark, set: IconSet) -> &'static str {
    match (mark, set) {
        (Mark::None | Mark::Volume, _) => " ",
        (Mark::Plus, _) => "+",
        (Mark::Dot(_), IconSet::Ascii) => "*",
        (Mark::Dot(_), _) => "●",
    }
}

/// The one-cell mark before a device's label: busy, locked, not mounted, unsupported.
fn device_glyph(state: CandidateState, busy: bool, set: IconSet) -> &'static str {
    let ascii = set == IconSet::Ascii;
    match state {
        _ if busy => {
            if ascii {
                "~"
            } else {
                "…"
            }
        }
        CandidateState::Locked => glyph(IconId::LockSimple, set),
        CandidateState::Unmounted => {
            if ascii {
                "-"
            } else {
                "○"
            }
        }
        CandidateState::Unsupported => {
            if ascii {
                "x"
            } else {
                "×"
            }
        }
    }
}

/// The eject mark at the end of an ejectable volume's row (one cell).
fn eject_glyph(set: IconSet) -> &'static str {
    if set == IconSet::Ascii { "^" } else { "⏏" }
}

/// `left` with `right` right-aligned in `w` cells when both fit (else `left` cut).
fn with_right(left: &str, right: &str, w: usize) -> String {
    let (lw, rw) = (text::width(left), text::width(right));
    if right.is_empty() || lw + 1 + rw > w {
        return text::truncate_end(left, w);
    }
    format!("{left}{:pad$}{right}", "", pad = w - lw - rw)
}

/// The text of `line` in `w` cells.
fn line_text(line: &SidebarLine, w: usize, set: IconSet) -> String {
    let s = match line.kind {
        LineKind::Header { open } => format!("{} {}", chevron(open, set), line.label),
        LineKind::Place if line.eject => {
            let body =
                text::truncate_end(&format!("{} {}", mark_glyph(line.mark, set), line.label), w.saturating_sub(2));
            return with_right(&body, eject_glyph(set), w);
        }
        LineKind::Place => format!("{} {}", mark_glyph(line.mark, set), line.label),
        LineKind::Device { state, busy } => {
            return with_right(&format!("{} {}", device_glyph(state, busy, set), line.label), &line.right, w);
        }
        LineKind::Message => format!("  {}", line.label),
        LineKind::Folder { expandable, expanded, loading, failed } => {
            // Two cells per level, but never more than half the width.
            let indent = (2 + 2 * line.depth).min(w / 2);
            let mark = if loading && expanded {
                "…"
            } else if expandable {
                chevron(expanded, set)
            } else {
                " "
            };
            let bang = if failed { " !" } else { "" };
            format!("{:indent$}{mark} {}{bang}", "", line.label)
        }
    };
    text::truncate_end(&s, w)
}

/// Draw the sidebar into `rect`.
pub fn draw_sidebar(f: &mut Frame<'_>, rect: Rect, m: &SidebarVm, theme: &TuiTheme, hits: &mut Hits) {
    let w = usize::from(rect.width);
    for (i, line) in m.lines.iter().enumerate().take(usize::from(rect.height)) {
        let r = Rect { y: rect.y + i as u16, height: 1, ..rect };
        let mut style = theme.style(match line.kind {
            LineKind::Header { .. } => Role::GroupHeader,
            LineKind::Message => Role::Muted,
            LineKind::Device { .. } if line.dim => Role::Disabled,
            LineKind::Device { .. } => Role::Muted,
            LineKind::Folder { .. } if line.dim => Role::Muted,
            LineKind::Folder { .. } | LineKind::Place => Role::Text,
        });
        if line.active {
            style = style.patch(theme.style(Role::Accent)).add_modifier(Modifier::BOLD);
        }
        if line.cursor && m.focused {
            style = style.patch(theme.style(Role::Cursor));
        }
        f.render_widget(Paragraph::new(line_text(line, w, theme.icons())), r);
        f.buffer_mut().set_style(r, style);
        // The mark in its colour (a state dot keeps it on the cursor row too).
        let mark_role = match line.mark {
            Mark::Dot(t) => Some(crate::view::jobs::role(t)),
            Mark::Plus => Some(Role::Accent),
            Mark::None | Mark::Volume => None,
        };
        if let Some(role) = mark_role
            && line.kind == LineKind::Place
            && r.width > 0
        {
            f.buffer_mut().set_style(Rect { width: 1, ..r }, theme.style(role));
        }
        if let LineKind::Device { state: CandidateState::Locked, busy: false } = line.kind
            && r.width > 0
        {
            f.buffer_mut().set_style(Rect { width: 1, ..r }, theme.style(Role::Warning));
        }
        if line.kind != LineKind::Message {
            hits.push(r, HitTarget::SidebarRow(line.index));
        }
        // The eject mark is its own target (pushed last: it wins over the row).
        if line.eject && r.width >= 3 {
            let mark = Rect { x: r.right() - 1, width: 1, ..r };
            f.buffer_mut().set_style(mark, theme.style(Role::Accent));
            hits.push(mark, HitTarget::SidebarEject(line.index));
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn line(index: usize, kind: LineKind, label: &str) -> SidebarLine {
        SidebarLine {
            index,
            kind,
            label: label.into(),
            depth: 0,
            active: false,
            cursor: false,
            dim: false,
            mark: Mark::None,
            right: String::new(),
            eject: false,
        }
    }

    fn model() -> SidebarVm {
        let folder = LineKind::Folder { expandable: true, expanded: true, loading: false, failed: false };
        let leaf = LineKind::Folder { expandable: false, expanded: false, loading: false, failed: true };
        SidebarVm {
            lines: vec![
                line(0, LineKind::Header { open: true }, "Favorites"),
                SidebarLine { active: true, ..line(1, LineKind::Place, "Documents") },
                SidebarLine { cursor: true, ..line(2, LineKind::Place, "Downloads") },
                line(3, LineKind::Header { open: false }, "Devices"),
                line(4, LineKind::Message, "Scanning drives…"),
                SidebarLine { depth: 1, ..line(5, folder, "projects") },
                SidebarLine { depth: 2, dim: true, ..line(6, leaf, "a_very_long_folder_name") },
            ],
            focused: true,
        }
    }

    fn draw(m: &SidebarVm, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_sidebar(f, Rect::new(0, 0, w, h), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, buf, hits)
    }

    #[test]
    fn headers_places_and_tree_rows() {
        let (lines, buf, hits) = draw(&model(), 20, 8);
        assert_eq!(lines[0], "v Favorites");
        assert_eq!(lines[1], "  Documents");
        assert_eq!(lines[2], "  Downloads");
        assert_eq!(lines[3], "> Devices");
        assert_eq!(lines[4], "  Scanning drives…");
        assert_eq!(lines[5], "    v projects");
        assert_eq!(lines[6], "        a_very_long…", "deep rows are cut, failed ones say so when they fit");
        // Headers bold, the active place bold, the cursor row reversed across the width.
        assert!(buf[(2, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(2, 1)].modifier.contains(Modifier::BOLD));
        assert!(buf[(0, 2)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(19, 2)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(0, 1)].modifier.contains(Modifier::REVERSED));
        // Every row but the note is a click target; nothing below the rows.
        assert_eq!(hits.at(5, 2), Some(HitTarget::SidebarRow(2)));
        assert_eq!(hits.at(5, 5), Some(HitTarget::SidebarRow(5)));
        assert_eq!(hits.at(5, 4), None);
        assert_eq!(hits.at(5, 7), None);
    }

    #[test]
    fn the_cursor_shows_only_with_focus() {
        let mut m = model();
        m.focused = false;
        let (_, buf, _) = draw(&m, 20, 8);
        assert!(!buf[(0, 2)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn device_rows_and_the_eject_mark() {
        let dev = |state, busy| LineKind::Device { state, busy };
        let m = SidebarVm {
            lines: vec![
                SidebarLine { mark: Mark::Volume, eject: true, ..line(0, LineKind::Place, "STICK") },
                SidebarLine { right: "NTFS".into(), ..line(1, dev(CandidateState::Unmounted, false), "Games") },
                SidebarLine { right: "BitLocker".into(), ..line(2, dev(CandidateState::Locked, false), "Vault") },
                SidebarLine { right: "Mounting…".into(), ..line(3, dev(CandidateState::Unmounted, true), "Data") },
                SidebarLine {
                    right: "APFS".into(),
                    dim: true,
                    ..line(4, dev(CandidateState::Unsupported, false), "Mac")
                },
            ],
            focused: false,
        };
        let (lines, buf, hits) = draw(&m, 20, 6);
        assert_eq!(lines[0], "  STICK            ^");
        assert_eq!(lines[1], "- Games         NTFS");
        assert_eq!(lines[2], "L Vault    BitLocker");
        assert_eq!(lines[3], "~ Data     Mounting…");
        assert_eq!(lines[4], "x Mac           APFS");
        // The eject mark is its own target; the rest of the row is the row.
        assert_eq!(hits.at(19, 0), Some(HitTarget::SidebarEject(0)));
        assert_eq!(hits.at(5, 0), Some(HitTarget::SidebarRow(0)));
        assert_eq!(hits.at(5, 2), Some(HitTarget::SidebarRow(2)));
        // Narrow: the badge goes first, the label is cut.
        let (lines, _, _) = draw(&m, 9, 6);
        assert_eq!(lines[2], "L Vault");
        assert!(!buf[(0, 1)].modifier.contains(Modifier::REVERSED));
        for w in 0..=6 {
            for h in 0..=3 {
                let _ = draw(&m, w, h);
            }
        }
    }

    #[test]
    fn failed_marker_and_tiny_sizes() {
        let mut m = model();
        m.lines[6].label = "x".into();
        let (lines, _, _) = draw(&m, 20, 8);
        assert_eq!(lines[6], "        x !");
        for w in 0..=6 {
            for h in 0..=3 {
                let _ = draw(&model(), w, h);
            }
        }
    }
}
