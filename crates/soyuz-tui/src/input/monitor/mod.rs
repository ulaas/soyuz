//! Keys and mouse of the system monitor (`crate::view::monitor`).
//!
//! While the focused pane shows `monitor:`, `crate::input::list::route` hands its keys here.
//! A filter being typed gets everything it can use first; then the page's keys; then the
//! keys every page shares; what is left goes to the core in the global key context, so the
//! palette, tabs, panes and Quit keep working and file commands stay out of the way. Text
//! that nobody used is dropped (no type-ahead on a page).
//!
//! | Keys | Where | What |
//! |---|---|---|
//! | Tab, Shift+Tab, 1-9, 0 | all | next, previous or that page |
//! | `?` | all | the page's keys |
//! | `P`, `i` | all | pause, next reading interval |
//! | Arrows, Page Up / Down, Home, End | all | move the cursor or scroll |
//! | `/`, Esc, Ctrl+R | tables | filter, clear it, whole-query regular expression |
//! | `<` `>` `I` | tables | sort column, order |
//! | `V`, Left, Right, Space, `+`, `-` | Processes | view, fold, mark, expand or fold all |
//! | `d`, `[`, `]`, `{`, `}` | Processes | details, its tabs, its scroll |
//! | `c`, `e` | Processes | column chooser, export CSV |
//! | F9 or `k`, `K`, `t`, `s`, `r` | Processes | end, kill, end tree, suspend, resume (the core asks first) |
//! | `p`, `a`, `o`, `y`, Enter | Processes | priority, processors, file location, copy, menu |
//! | `v` | CPU, Network | core view; virtual interfaces |
//! | `x`, `X`, `R` | Services | start, stop, restart |
//! | `+`, `-`, Left, Right, `L`, `,`, `.` | History | range, pan, live, cursor |
//! | `n`, F2 or `r`, Del, `e`, `a`, `b`, Enter | Snapshots | new, rename, delete, export, sides, compare |
//! | `u`, `m`, Esc | Comparison | unchanged rows, match by, close |

pub mod dialogs;

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::context_menu::{Entry, Item, ItemStyle, MenuIcon, Run, Submenu, SubmenuKind};
use soyuz_appcore::models::monitor::cmd::CopyWhat;
use soyuz_appcore::models::monitor::{CompareTarget, DetailsTab, MonitorCmd, MonitorView, SignalScope, labels};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::monitor::{CoreView, CpuScale, Monitor, MonitorPage, ProcColumn};
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Mods};
use soyuz_sysmon::{DiffSort, MatchBy, ProcPriority, ProcSignal, RowId, ServiceOp, position_of};

use crate::app::App;
use crate::input::menu::{self, Runs};
use crate::layout::HitTarget;
use crate::view::monitor::{Hit, MonitorVm};
use crate::widgets::Menu;
use crate::widgets::line_edit::{EditOutcome, LineEdit};

/// Which filter the open field edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    ProcFilter,
    /// The filter of the Connections, Services or Startup page.
    ListFilter,
    DiffFilter,
}

/// The open filter field.
#[derive(Clone, Debug)]
pub struct Field {
    pub kind: FieldKind,
    pub text: String,
    pub edit: LineEdit,
}

/// What a list page's order was computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisKey {
    pub generation: u64,
    pub filter: String,
    pub sort: u8,
    pub descending: bool,
    pub rows: usize,
    pub slot: usize,
}

/// What a pane's page keeps between frames besides the core's view state.
#[derive(Debug, Default)]
pub struct PaneNav {
    /// First row of the process table.
    pub top: usize,
    /// The selected row's position last frame.
    pub cursor: usize,
    /// Table rows on screen.
    pub rows: usize,
    /// Bring the cursor into view at the next layout.
    pub follow: bool,
    /// The wheel moved the table: it stays where it is until a key moves the cursor.
    pub free_scroll: bool,
    /// First row of each list page.
    pub list_top: [usize; 3],
    pub snap_top: usize,
    pub diff_top: usize,
    /// Scroll offset of a page of blocks.
    pub page_scroll: usize,
    pub page_rows: usize,
    pub page_max: usize,
    pub details_top: usize,
    pub details_max: usize,
    /// The help overlay's scroll; `None`: closed.
    pub help: Option<usize>,
    pub field: Option<Field>,
    /// The list page's shown rows and what they were computed from.
    pub vis: Vec<usize>,
    pub vis_key: Option<VisKey>,
    /// The processors dialog is open.
    pub affinity_open: bool,
}

/// The monitor's part of the input state.
#[derive(Debug, Default)]
pub struct State {
    pub panes: HashMap<PaneId, PaneNav>,
    pub vms: HashMap<PaneId, Arc<MonitorVm>>,
}

/// The focused pane, when it shows the monitor.
pub fn active(core: &AppCore) -> Option<PaneId> {
    let pane = core.ws.pane();
    matches!(pane.location, Location::Monitor).then_some(pane.id)
}

fn push(app: &mut App, pane: PaneId, c: MonitorCmd) {
    app.core.cmds.push(Cmd::Monitor(pane, c));
}

fn nav(app: &mut App, pane: PaneId) -> &mut PaneNav {
    app.input.monitor.panes.entry(pane).or_default()
}

fn cfg(app: &App) -> Monitor {
    app.core.config.monitor.clone()
}

fn view(app: &mut App, pane: PaneId) -> &mut MonitorView {
    let c = app.core.config.monitor.clone();
    app.core.ui.monitor.view(pane, &c)
}

/// This frame's keys.
pub fn route(app: &mut App, events: &[Event]) {
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    let mut i = 0;
    while i < frame.events.len() {
        let Some(pane) = active(&app.core) else { break };
        if app.focus() != crate::input::TuiFocus::List {
            break;
        }
        if frame.consumed[i] {
            i += 1;
            continue;
        }
        let ev = frame.events[i].clone();
        if nav(app, pane).field.is_some() {
            if field_event(app, pane, &ev) {
                frame.consumed[i] = true;
            }
        } else if let InputEvent::Chord(c) = ev
            && key(app, pane, c)
        {
            frame.consumed[i] = true;
            if let Some(InputEvent::Text(_)) = frame.events.get(i + 1) {
                frame.consumed[i + 1] = true;
                i += 1;
            }
        }
        i += 1;
    }
    if app.focus() == crate::input::TuiFocus::List {
        // No type-ahead on a page.
        for (ev, used) in frame.events.iter().zip(frame.consumed.iter_mut()) {
            if matches!(ev, InputEvent::Text(_)) {
                *used = true;
            }
        }
        app.core.input(&app.host, &mut frame, KeyContext::Global, false);
    }
}

/// One event while a filter is being typed; returns whether it was used.
fn field_event(app: &mut App, pane: PaneId, ev: &InputEvent) -> bool {
    if let InputEvent::Chord(c) = ev
        && c.mods == Mods::CTRL
        && c.key == KeyName::R
    {
        push(app, pane, MonitorCmd::ToggleFilterRegex);
        return true;
    }
    let Some(field) = nav(app, pane).field.as_mut() else { return false };
    let before = field.text.clone();
    let outcome = field.edit.apply(&mut field.text, ev);
    let (kind, text) = (field.kind, field.text.clone());
    if text != before {
        set_filter(app, pane, kind, text.clone());
    }
    match outcome {
        EditOutcome::Cancel => {
            nav(app, pane).field = None;
            set_filter(app, pane, kind, String::new());
            true
        }
        EditOutcome::Submit | EditOutcome::Tab | EditOutcome::ShiftTab => {
            nav(app, pane).field = None;
            true
        }
        EditOutcome::Up => {
            move_by(app, pane, -1);
            true
        }
        EditOutcome::Down => {
            move_by(app, pane, 1);
            true
        }
        EditOutcome::PageUp => {
            let n = page_step(app, pane);
            move_by(app, pane, -n);
            true
        }
        EditOutcome::PageDown => {
            let n = page_step(app, pane);
            move_by(app, pane, n);
            true
        }
        EditOutcome::Consumed => true,
        EditOutcome::Ignored => false,
    }
}

/// Which list page's commands apply on `page`.
fn set_filter(app: &mut App, pane: PaneId, kind: FieldKind, text: String) {
    let page = view(app, pane).page;
    let cmd = match (kind, page) {
        (FieldKind::ProcFilter, _) => MonitorCmd::SetFilter(text),
        (FieldKind::DiffFilter, _) => MonitorCmd::DiffFilter(text),
        (FieldKind::ListFilter, MonitorPage::Services) => MonitorCmd::ServicesFilter(text),
        (FieldKind::ListFilter, MonitorPage::Startup) => MonitorCmd::StartupFilter(text),
        (FieldKind::ListFilter, _) => MonitorCmd::ConnectionsFilter(text),
    };
    push(app, pane, cmd);
}

fn open_field(app: &mut App, pane: PaneId, kind: FieldKind) {
    let page = view(app, pane).page;
    let v = view(app, pane);
    let text = match (kind, page) {
        (FieldKind::ProcFilter, _) => v.filter_text.clone(),
        (FieldKind::DiffFilter, _) => v.snapshots.filter.clone(),
        (FieldKind::ListFilter, MonitorPage::Services) => v.services.filter_text.clone(),
        (FieldKind::ListFilter, MonitorPage::Startup) => v.startup.filter_text.clone(),
        (FieldKind::ListFilter, _) => v.connections.filter_text.clone(),
    };
    let edit = LineEdit::at_end(&text);
    nav(app, pane).field = Some(Field { kind, text, edit });
}

/// A step of Page Up / Page Down on the page.
fn page_step(app: &mut App, pane: PaneId) -> isize {
    let n = nav(app, pane);
    n.rows.max(n.page_rows).max(2) as isize - 1
}

/// Move the page's cursor or scroll by `d` rows.
fn move_by(app: &mut App, pane: PaneId, d: isize) {
    let page = view(app, pane).page;
    match page {
        MonitorPage::Processes => move_process(app, pane, d),
        MonitorPage::Connections | MonitorPage::Services | MonitorPage::Startup => {
            let v = view(app, pane);
            let (cursor, len) = match page {
                MonitorPage::Services => (&mut v.services.cursor, v.services.rows.len()),
                MonitorPage::Startup => (&mut v.startup.cursor, v.startup.rows.len()),
                _ => (&mut v.connections.cursor, v.connections.rows.len()),
            };
            *cursor = apply_delta(*cursor, d, len);
            let shown = nav(app, pane).vis.len();
            let v = view(app, pane);
            let cursor = match page {
                MonitorPage::Services => &mut v.services.cursor,
                MonitorPage::Startup => &mut v.startup.cursor,
                _ => &mut v.connections.cursor,
            };
            *cursor = (*cursor).min(shown.saturating_sub(1));
            nav(app, pane).follow = true;
        }
        MonitorPage::Snapshots => {
            let v = view(app, pane);
            if v.snapshots.comparison.is_some() {
                v.snapshots.diff_cursor = apply_delta(v.snapshots.diff_cursor, d, v.snapshots.shown.len());
            } else {
                let to = apply_delta(v.snapshots.cursor, d, v.snapshots.metas.len());
                push(app, pane, MonitorCmd::SnapshotCursor(to));
            }
        }
        _ => {
            let n = nav(app, pane);
            n.page_scroll = apply_delta(n.page_scroll, d, n.page_max + 1);
        }
    }
}

/// `cur` moved by `d`, kept in `0..len`; `isize::MIN` / `MAX` are the ends.
fn apply_delta(cur: usize, d: isize, len: usize) -> usize {
    match d {
        isize::MIN => 0,
        isize::MAX => len.saturating_sub(1),
        d => cur.saturating_add_signed(d).min(len.saturating_sub(1)),
    }
}

/// Move the process cursor: the selection stays on the row's identity.
fn move_process(app: &mut App, pane: PaneId, d: isize) {
    let Some(frame) = app.core.ui.monitor.frame.clone() else { return };
    let generation = app.core.ui.monitor.generation;
    let c = cfg(app);
    let v = view(app, pane);
    let selected = v.selected.clone();
    let rows = v.rows(&frame, generation, &c);
    if rows.is_empty() {
        return;
    }
    let cur = selected.as_ref().and_then(|id| position_of(rows, &frame, id)).unwrap_or(0);
    let to = apply_delta(cur, d, rows.len());
    let id = rows[to].id(&frame);
    view(app, pane).selected = Some(id);
    let n = nav(app, pane);
    n.follow = true;
    n.free_scroll = false;
    n.cursor = to;
    n.details_top = 0;
}

/// The selected process's key.
fn selected_key(app: &mut App, pane: PaneId) -> Option<soyuz_sysmon::ProcKey> {
    match view(app, pane).selected {
        Some(RowId::Proc(k)) => Some(k),
        _ => None,
    }
}

/// The next or previous page.
fn step_page(app: &mut App, pane: PaneId, next: bool) {
    let page = view(app, pane).page;
    let n = MonitorPage::ALL.len();
    let i = MonitorPage::ALL.iter().position(|p| *p == page).unwrap_or(0);
    let to = if next { (i + 1) % n } else { (i + n - 1) % n };
    set_page(app, pane, MonitorPage::ALL[to]);
}

fn set_page(app: &mut App, pane: PaneId, page: MonitorPage) {
    let n = nav(app, pane);
    n.page_scroll = 0;
    n.page_rows = 0;
    n.rows = 0;
    n.follow = true;
    n.field = None;
    n.vis_key = None;
    push(app, pane, MonitorCmd::SetPage(page));
}

/// One chord; returns whether it was used.
fn key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    // The help overlay takes what it can use, and closes on the rest of the page's keys.
    if nav(app, pane).help.is_some() {
        return help_key(app, pane, c);
    }
    let page = view(app, pane).page;
    // Keys a user bound to the context menu open the row's menu anywhere.
    if matches!(app.core.keymap.lookup(KeyContext::FileList, c), Some(Action::ContextMenu)) {
        return open_context(app);
    }
    if page_key(app, pane, page, c) {
        return true;
    }
    common_key(app, pane, c)
}

fn help_key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    let n = nav(app, pane);
    let plain = c.mods.is_empty();
    let help = n.help.get_or_insert(0);
    match c.key {
        KeyName::ArrowUp => *help = help.saturating_sub(1),
        KeyName::ArrowDown => *help += 1,
        KeyName::PageUp => *help = help.saturating_sub(10),
        KeyName::PageDown => *help += 10,
        KeyName::Home => *help = 0,
        KeyName::Escape => n.help = None,
        KeyName::Slash if c.mods == Mods::SHIFT => n.help = None,
        _ if plain || c.mods == Mods::SHIFT => {}
        _ => return false,
    }
    true
}

/// Keys every page has.
fn common_key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    let shifted = c.mods == Mods::SHIFT;
    let plain = c.mods.is_empty();
    match c.key {
        KeyName::Tab if plain || shifted => step_page(app, pane, plain),
        KeyName::Slash if shifted => nav(app, pane).help = Some(0),
        KeyName::P if shifted => push(app, pane, MonitorCmd::TogglePause),
        KeyName::I if plain => {
            const STEPS: [u32; 5] = [250, 500, 1000, 2000, 5000];
            let cur = app.core.config.monitor.interval_ms;
            let to = STEPS.iter().copied().find(|s| *s > cur).unwrap_or(STEPS[0]);
            push(app, pane, MonitorCmd::SetInterval(to));
        }
        KeyName::ArrowUp if plain => move_by(app, pane, -1),
        KeyName::ArrowDown if plain => move_by(app, pane, 1),
        KeyName::PageUp if plain => {
            let n = page_step(app, pane);
            move_by(app, pane, -n);
        }
        KeyName::PageDown if plain => {
            let n = page_step(app, pane);
            move_by(app, pane, n);
        }
        KeyName::Home if plain => move_by(app, pane, isize::MIN),
        KeyName::End if plain => move_by(app, pane, isize::MAX),
        k if plain => {
            let Some(i) = number_of(k) else { return false };
            let Some(p) = MonitorPage::ALL.get(i) else { return false };
            set_page(app, pane, *p);
        }
        _ => return false,
    }
    true
}

/// The page of a number key: 1-9, then 0.
fn number_of(k: KeyName) -> Option<usize> {
    use KeyName as N;
    Some(match k {
        N::Num1 => 0,
        N::Num2 => 1,
        N::Num3 => 2,
        N::Num4 => 3,
        N::Num5 => 4,
        N::Num6 => 5,
        N::Num7 => 6,
        N::Num8 => 7,
        N::Num9 => 8,
        N::Num0 => 9,
        _ => return None,
    })
}

fn page_key(app: &mut App, pane: PaneId, page: MonitorPage, c: KeyChord) -> bool {
    match page {
        MonitorPage::Processes => process_key(app, pane, c),
        MonitorPage::Cpu if c.mods.is_empty() && c.key == KeyName::V => {
            let next = match app.core.config.monitor.core_view {
                CoreView::Graphs => CoreView::Heat,
                CoreView::Heat => CoreView::Bars,
                CoreView::Bars => CoreView::Graphs,
            };
            push(app, pane, MonitorCmd::SetCoreView(next));
            true
        }
        MonitorPage::Network if c.mods.is_empty() && c.key == KeyName::V => {
            push(app, pane, MonitorCmd::ToggleVirtualNets);
            true
        }
        MonitorPage::Connections | MonitorPage::Services | MonitorPage::Startup => list_key(app, pane, page, c),
        MonitorPage::History => history_key(app, pane, c),
        MonitorPage::Snapshots => snapshot_key(app, pane, c),
        _ => false,
    }
}

/// A change of the process sort column: previous (`dir` -1) or next among the shown ones.
fn cycle_sort(app: &mut App, pane: PaneId, dir: isize) {
    let c = cfg(app);
    let cols: Vec<ProcColumn> = c.columns.iter().map(|x| x.id).collect();
    if cols.is_empty() {
        return;
    }
    let i = cols.iter().position(|x| *x == c.sort_by).unwrap_or(0) as isize;
    let to = (i + dir).rem_euclid(cols.len() as isize) as usize;
    push(app, pane, MonitorCmd::SortBy(cols[to]));
}

fn process_key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    use KeyName as N;
    let (plain, shifted) = (c.mods.is_empty(), c.mods == Mods::SHIFT);
    let signal = |app: &mut App, sig, scope| push(app, pane, MonitorCmd::Signal { scope, sig });
    match c.key {
        N::Slash if plain => open_field(app, pane, FieldKind::ProcFilter),
        N::R if c.mods == Mods::CTRL => push(app, pane, MonitorCmd::ToggleFilterRegex),
        N::Escape if plain => {
            let v = view(app, pane);
            if !v.filter_text.is_empty() {
                push(app, pane, MonitorCmd::SetFilter(String::new()));
            } else if !v.marked.is_empty() {
                push(app, pane, MonitorCmd::ClearMarks);
            } else {
                return false;
            }
        }
        N::V if plain || shifted => {
            let next = crate::view::monitor::next_view(app.core.config.monitor.view);
            push(app, pane, MonitorCmd::SetView(next));
            nav(app, pane).follow = true;
        }
        N::ArrowLeft if plain => fold(app, pane, true),
        N::ArrowRight if plain => fold(app, pane, false),
        N::Plus if plain || shifted => push(app, pane, MonitorCmd::ExpandAll),
        N::Equals if shifted => push(app, pane, MonitorCmd::ExpandAll),
        N::Minus if plain => push(app, pane, MonitorCmd::CollapseAll),
        N::Space if plain => {
            if let Some(k) = selected_key(app, pane) {
                push(app, pane, MonitorCmd::ToggleMark(k));
                move_by(app, pane, 1);
            }
        }
        // `<` and `>` are Shift+, and Shift+. on the US layout.
        N::Comma if shifted => cycle_sort(app, pane, -1),
        N::Period if shifted => cycle_sort(app, pane, 1),
        N::I if shifted => {
            let col = app.core.config.monitor.sort_by;
            push(app, pane, MonitorCmd::SortBy(col));
        }
        N::C if plain => push(app, pane, MonitorCmd::ChooseColumns),
        N::D if plain => push(app, pane, MonitorCmd::ToggleDetails),
        N::OpenBracket if plain => details_step(app, pane, false),
        N::CloseBracket if plain => details_step(app, pane, true),
        N::OpenBracket if shifted => {
            let n = nav(app, pane);
            n.details_top = n.details_top.saturating_sub(5);
        }
        N::CloseBracket if shifted => {
            let n = nav(app, pane);
            n.details_top = (n.details_top + 5).min(n.details_max);
        }
        N::F9 if plain => signal(app, ProcSignal::Terminate, SignalScope::Selection),
        N::K if plain => signal(app, ProcSignal::Terminate, SignalScope::Selection),
        N::K if shifted => signal(app, ProcSignal::Kill, SignalScope::Selection),
        N::T if plain => signal(app, ProcSignal::Terminate, SignalScope::Tree),
        N::S if plain => signal(app, ProcSignal::Suspend, SignalScope::Selection),
        N::R if plain => signal(app, ProcSignal::Resume, SignalScope::Selection),
        N::P if plain => {
            if let Some(k) = selected_key(app, pane) {
                let items = priority_entries(pane, k);
                let at = row_anchor(app, pane);
                open_entries(app, items, at);
            }
        }
        N::A if plain => {
            if let Some(k) = selected_key(app, pane) {
                push(app, pane, MonitorCmd::EditAffinity(k));
            }
        }
        N::O if plain => {
            if let Some(k) = selected_key(app, pane) {
                push(app, pane, MonitorCmd::OpenLocation(k));
            }
        }
        N::Y if plain => {
            if let Some(k) = selected_key(app, pane) {
                let items = copy_entries(pane, k);
                let at = row_anchor(app, pane);
                open_entries(app, items, at);
            }
        }
        N::E if plain => push(app, pane, MonitorCmd::ExportProcesses),
        N::Enter if plain => {
            if let Some(RowId::Group(_)) = view(app, pane).selected {
                let collapse = !is_collapsed(app, pane);
                fold(app, pane, collapse);
            } else {
                let items = process_menu(app, pane);
                let at = row_anchor(app, pane);
                open_entries(app, items, at);
            }
        }
        _ => return false,
    }
    true
}

fn details_step(app: &mut App, pane: PaneId, next: bool) {
    let tabs = DetailsTab::available();
    let cur = view(app, pane).details.tab;
    let i = tabs.iter().position(|t| *t == cur).unwrap_or(0);
    let to = if next { (i + 1) % tabs.len() } else { (i + tabs.len() - 1) % tabs.len() };
    nav(app, pane).details_top = 0;
    push(app, pane, MonitorCmd::SetDetailsTab(tabs[to]));
}

fn is_collapsed(app: &mut App, pane: PaneId) -> bool {
    let Some(frame) = app.core.ui.monitor.frame.clone() else { return false };
    let generation = app.core.ui.monitor.generation;
    let c = cfg(app);
    let v = view(app, pane);
    let selected = v.selected.clone();
    let rows = v.rows(&frame, generation, &c);
    selected.and_then(|id| position_of(rows, &frame, &id)).is_some_and(|i| rows[i].collapsed)
}

/// Left and Right on a row of the tree: fold, unfold; Left on a leaf goes to its parent.
fn fold(app: &mut App, pane: PaneId, collapse: bool) {
    let Some(frame) = app.core.ui.monitor.frame.clone() else { return };
    let generation = app.core.ui.monitor.generation;
    let c = cfg(app);
    let v = view(app, pane);
    let selected = v.selected.clone();
    let rows = v.rows(&frame, generation, &c);
    let Some(i) = selected.as_ref().and_then(|id| position_of(rows, &frame, id)) else { return };
    let row = &rows[i];
    if row.has_children && row.collapsed != collapse {
        let id = row.id(&frame);
        push(app, pane, MonitorCmd::ToggleCollapsed(id));
    } else if collapse && row.depth > 0 {
        let depth = row.depth;
        if let Some(parent) = rows[..i].iter().rposition(|r| r.depth < depth) {
            let id = rows[parent].id(&frame);
            view(app, pane).selected = Some(id);
            nav(app, pane).follow = true;
        }
    }
}

fn list_key(app: &mut App, pane: PaneId, page: MonitorPage, c: KeyChord) -> bool {
    use KeyName as N;
    let (plain, shifted) = (c.mods.is_empty(), c.mods == Mods::SHIFT);
    let (columns, sort, filter_empty) = {
        let v = view(app, pane);
        match page {
            MonitorPage::Services => (5u8, v.services.sort, v.services.filter_text.is_empty()),
            MonitorPage::Startup => (4, v.startup.sort, v.startup.filter_text.is_empty()),
            _ => (6, v.connections.sort, v.connections.filter_text.is_empty()),
        }
    };
    let sort_cmd = |col: u8| match page {
        MonitorPage::Services => MonitorCmd::ServicesSort(col),
        MonitorPage::Startup => MonitorCmd::StartupSort(col),
        _ => MonitorCmd::ConnectionsSort(col),
    };
    match c.key {
        N::Slash if plain => open_field(app, pane, FieldKind::ListFilter),
        N::Escape if plain && !filter_empty => set_filter(app, pane, FieldKind::ListFilter, String::new()),
        N::Comma if shifted => push(app, pane, sort_cmd((sort + columns - 1) % columns)),
        N::Period if shifted => push(app, pane, sort_cmd((sort + 1) % columns)),
        N::I if shifted => push(app, pane, sort_cmd(sort)),
        N::X if page == MonitorPage::Services && plain => service(app, pane, ServiceOp::Start),
        N::X if page == MonitorPage::Services && shifted => service(app, pane, ServiceOp::Stop),
        N::R if page == MonitorPage::Services && shifted => service(app, pane, ServiceOp::Restart),
        N::Enter if page == MonitorPage::Connections && plain => {
            let frame = app.core.ui.monitor.frame.clone();
            let i = view(app, pane).connections.cursor;
            let row = nav(app, pane).vis.get(i).copied();
            let v = view(app, pane);
            let pid = row.and_then(|r| v.connections.rows.get(r)).and_then(|c| c.pid);
            let key = soyuz_appcore::models::monitor::lists::process_key(frame.as_deref(), pid);
            if let Some(k) = key {
                push(app, pane, MonitorCmd::Select(Some(RowId::Proc(k))));
                set_page(app, pane, MonitorPage::Processes);
            }
        }
        _ => return false,
    }
    true
}

fn service(app: &mut App, pane: PaneId, op: ServiceOp) {
    let i = view(app, pane).services.cursor;
    let row = nav(app, pane).vis.get(i).copied();
    let name = row.and_then(|r| view(app, pane).services.rows.get(r).map(|s| s.name.clone()));
    if let Some(name) = name {
        push(app, pane, MonitorCmd::Service(name, op));
    }
}

fn history_key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    use KeyName as N;
    use soyuz_appcore::models::monitor::HistoryState;
    let (plain, shifted) = (c.mods.is_empty(), c.mods == Mods::SHIFT);
    let now = app.core.ui.monitor.frame.as_ref().map_or(0, |f| f.at_ms);
    let h = &view(app, pane).history;
    let range = h.range_secs;
    let i = HistoryState::RANGES.iter().position(|r| *r == range).unwrap_or(2);
    let width = app.input.monitor.vms.get(&pane).map_or(80, |v| v.canvas.w);
    match c.key {
        N::R if plain => {
            let off = soyuz_sysmon::Hub::global().recording() == soyuz_sysmon::Recording::Off;
            push(app, pane, if off { MonitorCmd::RecordStart } else { MonitorCmd::RecordStop });
        }
        N::Space if plain && soyuz_sysmon::Hub::global().recording() != soyuz_sysmon::Recording::Off => {
            push(app, pane, MonitorCmd::RecordPause);
        }
        N::Plus | N::Equals if plain || shifted => {
            // A wider range.
            let to = (i + 1).min(HistoryState::RANGES.len() - 1);
            push(app, pane, MonitorCmd::HistoryRange(HistoryState::RANGES[to]));
        }
        N::Minus if plain => push(app, pane, MonitorCmd::HistoryRange(HistoryState::RANGES[i.saturating_sub(1)])),
        N::ArrowLeft if plain => push(app, pane, MonitorCmd::HistoryPan(i64::from(range / 4))),
        N::ArrowRight if plain => push(app, pane, MonitorCmd::HistoryPan(-i64::from(range / 4))),
        N::L if plain || shifted => push(app, pane, MonitorCmd::HistoryLive),
        N::Comma if plain => {
            let h = &view(app, pane).history;
            let t = crate::view::monitor::step_cursor(h, now, width, -1);
            push(app, pane, MonitorCmd::HistoryCursor(Some(t)));
        }
        N::Period if plain => {
            let h = &view(app, pane).history;
            let t = crate::view::monitor::step_cursor(h, now, width, 1);
            push(app, pane, MonitorCmd::HistoryCursor(Some(t)));
        }
        N::Escape if plain && view(app, pane).history.cursor_ms.is_some() => {
            push(app, pane, MonitorCmd::HistoryCursor(None));
        }
        _ => return false,
    }
    true
}

/// The snapshot under the cursor.
fn cursor_snapshot(app: &mut App, pane: PaneId) -> Option<String> {
    let s = &view(app, pane).snapshots;
    s.metas.get(s.cursor).map(|m| m.id.clone())
}

fn snapshot_key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    use KeyName as N;
    let (plain, shifted) = (c.mods.is_empty(), c.mods == Mods::SHIFT);
    if view(app, pane).snapshots.comparison.is_some() {
        return compare_key(app, pane, c);
    }
    match c.key {
        N::N if plain => {
            dialogs::open_name(app, pane, dialogs::NameKind::New, &labels::default_snapshot_label());
        }
        N::F2 | N::R if plain => {
            if let Some(id) = cursor_snapshot(app, pane) {
                let label = view(app, pane).snapshots.label_of(&id).unwrap_or("").to_string();
                dialogs::open_name(app, pane, dialogs::NameKind::Rename(id), &label);
            }
        }
        N::Delete if plain => {
            if let Some(id) = cursor_snapshot(app, pane) {
                push(app, pane, MonitorCmd::DeleteSnapshot(id));
            }
        }
        N::E if plain => {
            if let Some(id) = cursor_snapshot(app, pane) {
                let items = vec![
                    item(
                        tr!("tui-monitor-export-json"),
                        None,
                        pane,
                        MonitorCmd::ExportSnapshot { id: id.clone(), csv: false },
                    ),
                    item(tr!("tui-monitor-export-csv"), None, pane, MonitorCmd::ExportSnapshot { id, csv: true }),
                ];
                let at = row_anchor(app, pane);
                open_entries(app, items, at);
            }
        }
        N::A if plain => {
            if let Some(id) = cursor_snapshot(app, pane) {
                push(app, pane, MonitorCmd::CompareFrom(id));
            }
        }
        // `b` on the snapshot that already is the right side goes back to now.
        N::B if plain => {
            if let Some(id) = cursor_snapshot(app, pane) {
                let target = CompareTarget::Snapshot(id);
                let to = if view(app, pane).snapshots.b == target { CompareTarget::Live } else { target };
                push(app, pane, MonitorCmd::CompareWith(to));
            }
        }
        N::B if shifted => push(app, pane, MonitorCmd::CompareWith(CompareTarget::Live)),
        N::Enter if plain => {
            if view(app, pane).snapshots.a.is_none()
                && let Some(id) = cursor_snapshot(app, pane)
            {
                push(app, pane, MonitorCmd::CompareFrom(id));
            }
            push(app, pane, MonitorCmd::RunCompare);
        }
        _ => return false,
    }
    true
}

fn compare_key(app: &mut App, pane: PaneId, c: KeyChord) -> bool {
    use KeyName as N;
    let (plain, shifted) = (c.mods.is_empty(), c.mods == Mods::SHIFT);
    let sorts = &crate::view::monitor::DIFF_SORTS;
    let s = &view(app, pane).snapshots;
    let (sort, show_same, by, filter_empty) = (s.sort, s.show_same, s.match_by, s.filter.is_empty());
    let i = sorts.iter().position(|x| *x == sort).unwrap_or(0);
    match c.key {
        N::Slash if plain => open_field(app, pane, FieldKind::DiffFilter),
        N::Escape if plain && !filter_empty => push(app, pane, MonitorCmd::DiffFilter(String::new())),
        N::Escape if plain => push(app, pane, MonitorCmd::CloseCompare),
        N::U if plain => push(app, pane, MonitorCmd::DiffShowSame(!show_same)),
        N::M if plain => {
            let to = if by == MatchBy::Process { MatchBy::Program } else { MatchBy::Process };
            push(app, pane, MonitorCmd::SetMatchBy(to));
        }
        N::Comma if shifted => {
            push(app, pane, MonitorCmd::DiffSortBy(sorts[(i + sorts.len() - 1) % sorts.len()]));
        }
        N::Period if shifted => push(app, pane, MonitorCmd::DiffSortBy(sorts[(i + 1) % sorts.len()])),
        N::I if shifted => push(app, pane, MonitorCmd::DiffSortBy(sort)),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------------------
// Menus.

fn item(label: &str, chord: Option<&str>, pane: PaneId, c: MonitorCmd) -> Entry {
    Entry::Item(Item {
        icon: MenuIcon::None,
        label: label.to_string(),
        chord: chord.map(str::to_string),
        enabled: true,
        style: ItemStyle::Plain,
        run: Some(Run::Cmd(Cmd::Monitor(pane, c))),
        keep_open: false,
        tip: None,
    })
}

fn priority_entries(pane: PaneId, key: soyuz_sysmon::ProcKey) -> Vec<Entry> {
    [
        ProcPriority::Idle,
        ProcPriority::BelowNormal,
        ProcPriority::Normal,
        ProcPriority::AboveNormal,
        ProcPriority::High,
        ProcPriority::Realtime,
    ]
    .into_iter()
    .map(|p| item(labels::priority(p), None, pane, MonitorCmd::SetPriority(key, p)))
    .collect()
}

fn copy_entries(pane: PaneId, key: soyuz_sysmon::ProcKey) -> Vec<Entry> {
    [
        (labels::column_title(ProcColumn::Pid), CopyWhat::Pid),
        (labels::column_title(ProcColumn::Name), CopyWhat::Name),
        (labels::column_title(ProcColumn::Path), CopyWhat::Path),
        (labels::column_title(ProcColumn::Command), CopyWhat::Command),
    ]
    .into_iter()
    .map(|(l, w)| item(l, None, pane, MonitorCmd::CopyProcess(key, w)))
    .collect()
}

fn submenu(label: &str, items: Vec<Entry>) -> Entry {
    Entry::Submenu(Submenu { kind: SubmenuKind::View, icon: MenuIcon::None, label: label.to_string(), items })
}

/// The menu of the selected process.
fn process_menu(app: &mut App, pane: PaneId) -> Vec<Entry> {
    let Some(k) = selected_key(app, pane) else { return Vec::new() };
    let signal = |sig, scope| MonitorCmd::Signal { scope, sig };
    let c = cfg(app);
    let scale_to = if c.cpu_scale == CpuScale::Machine { CpuScale::PerCore } else { CpuScale::Machine };
    vec![
        item(
            labels::signal(ProcSignal::Terminate),
            Some("k"),
            pane,
            signal(ProcSignal::Terminate, SignalScope::Selection),
        ),
        item(labels::signal(ProcSignal::Kill), Some("K"), pane, signal(ProcSignal::Kill, SignalScope::Selection)),
        item(tr!("tui-monitor-end-tree"), Some("t"), pane, signal(ProcSignal::Terminate, SignalScope::Tree)),
        item(labels::signal(ProcSignal::Suspend), Some("s"), pane, signal(ProcSignal::Suspend, SignalScope::Selection)),
        item(labels::signal(ProcSignal::Resume), Some("r"), pane, signal(ProcSignal::Resume, SignalScope::Selection)),
        Entry::Separator,
        submenu(tr!("tui-monitor-menu-priority"), priority_entries(pane, k)),
        item(tr!("tui-monitor-menu-affinity"), Some("a"), pane, MonitorCmd::EditAffinity(k)),
        item(tr!("tui-monitor-menu-open-location"), Some("o"), pane, MonitorCmd::OpenLocation(k)),
        submenu(tr!("tui-monitor-menu-copy"), copy_entries(pane, k)),
        Entry::Separator,
        item(tr!("tui-monitor-menu-mark"), Some("Space"), pane, MonitorCmd::ToggleMark(k)),
        item(tr!("tui-monitor-menu-details"), Some("d"), pane, MonitorCmd::ToggleDetails),
        item(tr!("tui-monitor-menu-columns"), Some("c"), pane, MonitorCmd::ChooseColumns),
        item(tr!("tui-monitor-menu-export"), Some("e"), pane, MonitorCmd::ExportProcesses),
        Entry::Separator,
        item(tr!("tui-monitor-menu-kernel"), None, pane, MonitorCmd::ToggleKernel),
        item(tr!("tui-monitor-menu-other-users"), None, pane, MonitorCmd::ToggleOtherUsers),
        item(tr!("tui-monitor-menu-heat"), None, pane, MonitorCmd::ToggleHeat),
        item(tr!("tui-monitor-menu-scale"), None, pane, MonitorCmd::SetCpuScale(scale_to)),
    ]
}

fn open_entries(app: &mut App, entries: Vec<Entry>, anchor: (u16, u16)) {
    let mut runs = Runs::default();
    let items = menu::rows(entries, &mut runs);
    if items.is_empty() {
        return;
    }
    app.input.menu.runs = runs;
    app.input.menu.header = None;
    menu::open(app, Menu::new(items, anchor));
}

/// Below the cursor row when it is on screen.
fn row_anchor(app: &mut App, pane: PaneId) -> (u16, u16) {
    let cursor = nav(app, pane).cursor;
    let mut fallback = (1, 1);
    for (r, t) in app.hits.iter() {
        if let HitTarget::Monitor { pane: p, hit } = *t
            && p == pane
        {
            match hit {
                Hit::ProcRow(i) | Hit::ListRow(i) | Hit::SnapRow(i) | Hit::DiffRow(i) if i == cursor => {
                    return (r.x.saturating_add(2.min(r.width)), r.y.saturating_add(1));
                }
                Hit::Area => fallback = (r.x.saturating_add(1), r.y.saturating_add(1)),
                _ => {}
            }
        }
    }
    fallback
}

/// `ContextMenu` while the list has the keyboard and the focused pane shows the monitor:
/// the selected process's menu. Returns whether the pane shows the monitor.
pub fn open_context(app: &mut App) -> bool {
    if app.focus() != crate::input::TuiFocus::List {
        return false;
    }
    let Some(pane) = active(&app.core) else { return false };
    if view(app, pane).page == MonitorPage::Processes {
        let items = process_menu(app, pane);
        let at = row_anchor(app, pane);
        open_entries(app, items, at);
    }
    true
}

// ---------------------------------------------------------------------------------------
// Mouse.

/// A mouse event over a monitor target; always taken.
pub fn on_mouse(app: &mut App, target: HitTarget, m: &MouseEvent) -> bool {
    let HitTarget::Monitor { pane, hit } = target else { return false };
    match m.kind {
        MouseEventKind::Down(b) => press(app, pane, hit, b, (m.column, m.row)),
        MouseEventKind::ScrollDown => wheel(app, pane, hit, crate::input::mouse::WHEEL_ROWS as isize),
        MouseEventKind::ScrollUp => wheel(app, pane, hit, -(crate::input::mouse::WHEEL_ROWS as isize)),
        _ => {}
    }
    true
}

fn wheel(app: &mut App, pane: PaneId, hit: Hit, d: isize) {
    let page = view(app, pane).page;
    let n = nav(app, pane);
    if let Some(h) = n.help.as_mut() {
        *h = h.saturating_add_signed(d);
        return;
    }
    if hit == Hit::DetailsBody {
        n.details_top = n.details_top.saturating_add_signed(d).min(n.details_max);
        return;
    }
    match page {
        MonitorPage::Processes => {
            n.top = n.top.saturating_add_signed(d);
            n.free_scroll = true;
        }
        MonitorPage::Connections => n.list_top[0] = n.list_top[0].saturating_add_signed(d),
        MonitorPage::Services => n.list_top[1] = n.list_top[1].saturating_add_signed(d),
        MonitorPage::Startup => n.list_top[2] = n.list_top[2].saturating_add_signed(d),
        MonitorPage::Snapshots => move_by(app, pane, d),
        _ => n.page_scroll = n.page_scroll.saturating_add_signed(d).min(n.page_max),
    }
}

fn press(app: &mut App, pane: PaneId, hit: Hit, b: MouseButton, at: (u16, u16)) {
    app.core.cmds.push(Cmd::FocusPane(pane));
    let left = b == MouseButton::Left;
    if nav(app, pane).help.is_some() {
        nav(app, pane).help = None;
        return;
    }
    match hit {
        Hit::Tab(p) if left => set_page(app, pane, p),
        Hit::Step(next) if left => step_page(app, pane, next),
        Hit::Pause if left => push(app, pane, MonitorCmd::TogglePause),
        Hit::ProcHeader(c) if left => push(app, pane, MonitorCmd::SortBy(c)),
        Hit::ProcHeader(_) if b == MouseButton::Right => push(app, pane, MonitorCmd::ChooseColumns),
        Hit::ProcFold(i) if left => {
            if let Some(id) = row_id(app, pane, i) {
                push(app, pane, MonitorCmd::ToggleCollapsed(id));
            }
        }
        Hit::ProcRow(i) | Hit::ProcFold(i) => {
            if let Some(id) = row_id(app, pane, i) {
                view(app, pane).selected = Some(id);
                nav(app, pane).details_top = 0;
                if b == MouseButton::Right {
                    let items = process_menu(app, pane);
                    open_entries(app, items, at);
                }
            }
        }
        Hit::Details(t) if left => {
            nav(app, pane).details_top = 0;
            push(app, pane, MonitorCmd::SetDetailsTab(t));
        }
        Hit::FilterLine if left => {
            let page = view(app, pane).page;
            let kind = match page {
                MonitorPage::Processes => FieldKind::ProcFilter,
                MonitorPage::Snapshots => FieldKind::DiffFilter,
                _ => FieldKind::ListFilter,
            };
            open_field(app, pane, kind);
        }
        Hit::ListHeader(c) if left => {
            let page = view(app, pane).page;
            let cmd = match page {
                MonitorPage::Services => MonitorCmd::ServicesSort(c),
                MonitorPage::Startup => MonitorCmd::StartupSort(c),
                _ => MonitorCmd::ConnectionsSort(c),
            };
            push(app, pane, cmd);
        }
        Hit::ListRow(i) => {
            let page = view(app, pane).page;
            let v = view(app, pane);
            match page {
                MonitorPage::Services => v.services.cursor = i,
                MonitorPage::Startup => v.startup.cursor = i,
                _ => v.connections.cursor = i,
            }
        }
        Hit::SnapRow(i) => push(app, pane, MonitorCmd::SnapshotCursor(i)),
        Hit::DiffHeader(c) if left => {
            let sort: DiffSort = crate::view::monitor::DIFF_SORTS[usize::from(c).min(7)];
            push(app, pane, MonitorCmd::DiffSortBy(sort));
        }
        Hit::DiffRow(i) => view(app, pane).snapshots.diff_cursor = i,
        _ => {}
    }
}

/// The identity of table row `i`.
fn row_id(app: &mut App, pane: PaneId, i: usize) -> Option<RowId> {
    let frame = app.core.ui.monitor.frame.clone()?;
    let generation = app.core.ui.monitor.generation;
    let c = cfg(app);
    let rows = view(app, pane).rows(&frame, generation, &c);
    rows.get(i).map(|r| r.id(&frame))
}
