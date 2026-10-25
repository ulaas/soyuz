//! Mouse input (step T2.9), only while `[tui] mouse` is on (otherwise the terminal keeps
//! its own text selection and nothing reaches here). Runs before the keyboard routing.
//!
//! Every press is looked up in the click targets of the last frame drawn (`app.hits`,
//! [`Hits`]); they are cleared on resize, so a click never lands on an older layout.
//! Row and group-header targets carry the pane's `vis_version`: when the listing changed
//! since that frame (or the pane is gone), the click is dropped.
//!
//! | Target | Left | Middle | Right |
//! |---|---|---|---|
//! | `PaneRow` | focus the pane, then [`row_press`] (below) | `behaviour.middle_click` (folders) | select the row, open its context menu |
//! | `PaneHeader` | focus, collapse / expand the group (as in the GUI) | | |
//! | `PaneBackground` | focus, clear the selection (unless Ctrl / Shift) | focus | focus, clear unless Ctrl, open the background menu |
//! | `PaneColumn` | sort by the column; the sorted one flips direction (GUI header click) | | the column chooser |
//! | `MillerRow` | folder: open it; file: open its folder with it selected (GUI) | | |
//! | `Splitter` | drag sets `split` (or `split2`, the quad rows) | | |
//! | `Tab` / `TabClose` / `NewTab` | activate / close / new tab | close the tab | the tab menu |
//! | `Crumb` | navigate there; the last crumb edits the address | open in a new tab | |
//! | `Address` | edit the address | | |
//! | `SidebarRow` | cursor there; a header toggles, a place / tree folder opens | a place / tree folder in a new tab | a place's menu |
//! | `Filter` | arm the filter field | | |
//! | `PaletteRow` | select and run (Ctrl: new tab, Shift: complete only) | | |
//! | `Toast` | dismiss | | |
//!
//! A left press on a row follows the GUI (`views/flow.rs`): in checkbox mode the mark
//! column ([`CHECK_COLS`]) toggles the row; with `general.single_click_open` a folder
//! opens; otherwise `Selection::click` (Ctrl toggles, Shift extends from the anchor) and
//! the cursor moves there. A double-click opens (`Cmd::Open`, i.e. `ops::open_entry`).
//!
//! A press outside the open palette closes it (and does nothing else). While a dialog is
//! open every mouse event goes to it (`crate::input::dialogs::mouse`, which hands it to
//! the dialog kind's `on_mouse`), while a popup menu is open to `crate::input::menu`,
//! while Quick Look is open to `crate::input::quicklook`. The Phase T3 / T4 panels get
//! presses and the wheel on their own targets: `BottomTab` / `JobRow` / `JobButton` →
//! `crate::input::jobs`, `PreviewScroll` / `ArchiveRow` → `crate::input::preview`,
//! `EditorTab` / `EditorClose` / `EditorCell` → `crate::input::editor`, `RenameField`
//! → `crate::input::rename`. `PaletteInput` and `Status` take presses without doing
//! anything.
//! A press outside the address / filter field leaves that field.
//!
//! Phase T5 / T6 parts get every mouse event (presses, drags, releases, the wheel) over
//! their own targets through their `on_mouse` (`input::{search, terminal, ai, compare,
//! settings}`), and every event anywhere while their `captures_mouse` says so; so do the
//! Phase T7 parts: the full-screen tools (`input::screens`: `Screen*`, `Treemap*`,
//! `Dupes*`, `ConfirmBar`) and the sidebar's eject mark (`input::sidebar::on_mouse`:
//! `SidebarEject`). The groups are fixed here; the owners decide what each event does. An
//! `Overlay` target (under the compare screen / settings editor) swallows the rest, so a
//! click never reaches the panes below. Not while the palette is open (a press outside it
//! closes it first).
//!
//! crossterm reports no double-clicks: two left presses on the same target within
//! `general.double_click_speed_ms` (clamped like the GUI's `apply_live_settings`) are one.
//!
//! The wheel scrolls the pane under the pointer by [`WHEEL_ROWS`] rows (one line of cells
//! in the grid and gallery views) through `pane.scroll_y` without moving the cursor (`scroll_to_cursor` stays as it is; the pane
//! view clamps the offset). The sidebar and the palette keep no scroll offset of their own
//! (the palette scrolls to its selection), so the wheel does nothing over them.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::state::pane::{ContextTarget, Pane, PaneId};
use soyuz_appcore::state::selection::{ClickMods, EntryKey};
use soyuz_core::Location;
use soyuz_core::config::{Action, Config, MiddleClick};

use crate::app::App;
use crate::layout::{GAP, HitTarget, Hits, Regions};

/// Rows one wheel notch scrolls.
pub const WHEEL_ROWS: i32 = 3;
/// Bounds of the double-click interval (the GUI's `apply_live_settings`).
pub const DOUBLE_CLICK_MS_MIN: u32 = 100;
pub const DOUBLE_CLICK_MS_MAX: u32 = 2000;
/// Bounds of a split ratio (`Cmd::SetSplit`).
pub const SPLIT_MIN: f32 = 0.15;
pub const SPLIT_MAX: f32 = 0.85;
/// Columns of a row's selection mark (`* `): the checkbox in checkbox mode.
pub const CHECK_COLS: u16 = 2;

/// What the mouse keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The last left press, for double-click detection.
    last: Option<(Instant, HitTarget)>,
    /// A splitter is being dragged (`true`: the one between columns).
    drag: Option<bool>,
}

impl State {
    /// A left press on `target` at `now`; returns whether it completes a double-click.
    /// A double-click is not the first half of the next one.
    pub fn press(&mut self, target: HitTarget, now: Instant, interval: Duration) -> bool {
        let double = self.last.is_some_and(|(at, t)| t == target && now.saturating_duration_since(at) <= interval);
        self.last = if double { None } else { Some((now, target)) };
        double
    }
}

/// The double-click interval of `config`.
pub fn double_click_interval(config: &Config) -> Duration {
    let ms = config.general.double_click_speed_ms.clamp(DOUBLE_CLICK_MS_MIN, DOUBLE_CLICK_MS_MAX);
    Duration::from_millis(u64::from(ms))
}

/// The top-most target at `(x, y)` with its rect.
pub fn hit(hits: &Hits, x: u16, y: u16) -> Option<(Rect, HitTarget)> {
    let p = Position { x, y };
    hits.iter().filter(|(r, _)| r.contains(p)).last().copied()
}

pub fn route(app: &mut App, events: &[Event]) {
    if !app.core.config.tui.mouse {
        return;
    }
    let now = Instant::now();
    for ev in events {
        if let Event::Mouse(m) = ev {
            handle(app, m, now);
        }
    }
}

fn handle(app: &mut App, m: &MouseEvent, now: Instant) {
    if super::dialogs::is_open(&app.core, &app.input.dialogs) {
        app.input.mouse.drag = None;
        super::dialogs::mouse(app, m);
        return;
    }
    if super::menu::is_open(&app.input.menu) {
        app.input.mouse.drag = None;
        super::menu::mouse(app, m);
        return;
    }
    if super::quicklook::open(&app.core, &app.input) {
        app.input.mouse.drag = None;
        super::quicklook::mouse(app, m);
        return;
    }
    if !app.core.palette.open && phase_t56(app, m) {
        app.input.mouse.drag = None;
        return;
    }
    match m.kind {
        MouseEventKind::Down(b) => press(app, m, b, now),
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(vertical) = app.input.mouse.drag {
                drag_splitter(app, vertical, m.column, m.row);
            }
        }
        MouseEventKind::Up(_) => app.input.mouse.drag = None,
        MouseEventKind::ScrollDown => wheel(app, m, WHEEL_ROWS),
        MouseEventKind::ScrollUp => wheel(app, m, -WHEEL_ROWS),
        _ => {}
    }
}

/// The Phase T5 / T6 parts' mouse (see the module docs); returns whether one took `m`.
fn phase_t56(app: &mut App, m: &MouseEvent) -> bool {
    let target = hit(&app.hits, m.column, m.row).map(|(_, t)| t);
    let any = target.unwrap_or(HitTarget::Overlay);
    if super::settings::captures_mouse(&app.core, &app.input) {
        return super::settings::on_mouse(app, any, m);
    }
    if super::compare::captures_mouse(&app.core, &app.input) {
        return super::compare::on_mouse(app, any, m);
    }
    if super::screens::captures_mouse(&app.core, &app.input) {
        return super::screens::on_mouse(app, any, m);
    }
    if super::terminal::captures_mouse(&app.core, &app.input) {
        return super::terminal::on_mouse(app, any, m);
    }
    if super::ai::captures_mouse(&app.core, &app.input) {
        return super::ai::on_mouse(app, any, m);
    }
    if super::search::captures_mouse(&app.core, &app.input) {
        return super::search::on_mouse(app, any, m);
    }
    let Some(t) = target else { return false };
    match t {
        HitTarget::Overlay => true,
        HitTarget::SearchField
        | HitTarget::SearchPill(_)
        | HitTarget::SearchButton(_)
        | HitTarget::SearchStatusButton(_) => super::search::on_mouse(app, t, m),
        HitTarget::TermTab(_)
        | HitTarget::TermTabClose(_)
        | HitTarget::TermNew
        | HitTarget::TermExternal
        | HitTarget::TermCell { .. }
        | HitTarget::TermClip(_) => super::terminal::on_mouse(app, t, m),
        HitTarget::AiTab(_)
        | HitTarget::AiComposer
        | HitTarget::AiButton(_)
        | HitTarget::AiCard { .. }
        | HitTarget::AiCardButton { .. }
        | HitTarget::AiRow(_)
        | HitTarget::AiMention(_)
        | HitTarget::AiMcpUndo(_)
        | HitTarget::AiChat => super::ai::on_mouse(app, t, m),
        HitTarget::CompareField(_)
        | HitTarget::CompareRow(_)
        | HitTarget::CompareButton(_)
        | HitTarget::CompareChip(_)
        | HitTarget::CompareProfile(_) => super::compare::on_mouse(app, t, m),
        HitTarget::SettingsNav(_)
        | HitTarget::SettingsRow(_)
        | HitTarget::SettingsSearch
        | HitTarget::SettingsClose
        | HitTarget::SettingsKeyRow(_)
        | HitTarget::SettingsKeyChip { .. }
        | HitTarget::SettingsKeyAdd(_) => super::settings::on_mouse(app, t, m),
        // ---- Phase T7 ----
        HitTarget::SidebarEject(_) => super::sidebar::on_mouse(app, t, m),
        HitTarget::ScreenButton(_)
        | HitTarget::ScreenChip(_)
        | HitTarget::ScreenField(_)
        | HitTarget::ScreenTab(_)
        | HitTarget::ScreenClose
        | HitTarget::ScreenArea(_)
        | HitTarget::TreemapTile(_)
        | HitTarget::TreemapCrumb(_)
        | HitTarget::TreemapSide(_)
        | HitTarget::DupesRow(_)
        | HitTarget::DupesRoot(_)
        | HitTarget::DupesHeader(_)
        | HitTarget::ConfirmBar(_) => super::screens::on_mouse(app, t, m),
        HitTarget::PlacesRow { .. } | HitTarget::PlacesButton { .. } | HitTarget::PlacesArea { .. } => {
            super::list::places::on_mouse(app, t, m)
        }
        _ => false,
    }
}

fn click_mods(m: &MouseEvent) -> ClickMods {
    ClickMods { toggle: m.modifiers.contains(KeyModifiers::CONTROL), range: m.modifiers.contains(KeyModifiers::SHIFT) }
}

fn press(app: &mut App, m: &MouseEvent, button: MouseButton, now: Instant) {
    let target = hit(&app.hits, m.column, m.row);
    let in_palette = matches!(target, Some((_, HitTarget::PaletteRow(_) | HitTarget::PaletteInput)));
    if app.core.palette.open && !in_palette {
        soyuz_appcore::palette::close(&mut app.core);
        return;
    }
    let Some((rect, target)) = target else { return };
    let double = button == MouseButton::Left && {
        let interval = double_click_interval(&app.core.config);
        app.input.mouse.press(target, now, interval)
    };
    leave_fields(app, target);
    let mods = click_mods(m);
    match (target, button) {
        (HitTarget::PaneRow { pane, vis_version, index }, _) => {
            let ctx = RowCtx {
                button,
                mods,
                double,
                checkbox: app.core.ui.checkbox_mode && m.column < rect.x.saturating_add(CHECK_COLS),
                single_click_open: app.core.config.general.single_click_open,
            };
            let Some(p) = app.core.ws.find_pane_mut(pane) else { return };
            if p.vis_version != vis_version {
                return;
            }
            let effect = row_press(p, index, ctx);
            app.core.cmds.push(Cmd::FocusPane(pane));
            match effect {
                Some(RowEffect::Open(key)) => app.core.cmds.push(Cmd::Open { pane, key }),
                Some(RowEffect::Middle(location)) => match app.core.config.behaviour.middle_click {
                    MiddleClick::NewTab => app.core.cmds.push(Cmd::OpenInNewTab { location, activate: false }),
                    MiddleClick::OtherPane => app.core.cmds.push(Cmd::OpenInOtherPane(location)),
                    MiddleClick::Nothing => {}
                },
                None => {}
            }
            if button == MouseButton::Right {
                super::menu::open_for(app, pane, ContextTarget::Items, (m.column, m.row));
            }
        }
        (HitTarget::PaneHeader { pane, vis_version, group }, MouseButton::Left) => {
            let Some(p) = app.core.ws.find_pane_mut(pane) else { return };
            if p.vis_version != vis_version {
                return;
            }
            toggle_group(p, group);
            app.core.cmds.push(Cmd::FocusPane(pane));
        }
        (HitTarget::PaneBackground { pane }, _) => {
            let Some(p) = app.core.ws.find_pane_mut(pane) else { return };
            let clear = match button {
                MouseButton::Left => !mods.toggle && !mods.range,
                MouseButton::Right => !mods.toggle,
                MouseButton::Middle => false,
            };
            if clear {
                p.selection.clear();
            }
            app.core.cmds.push(Cmd::FocusPane(pane));
            if button == MouseButton::Right {
                let target = if p.selection.is_empty() { ContextTarget::Background } else { ContextTarget::Items };
                super::menu::open_for(app, pane, target, (m.column, m.row));
            }
        }
        (HitTarget::PaneColumn { pane, column }, MouseButton::Left) => {
            app.core.cmds.push(Cmd::PaneAction(pane, Action::SortBy(column)));
        }
        (HitTarget::MillerRow { pane, parent, index }, MouseButton::Left) => {
            if let Some((path, is_dir)) = crate::view::pane::miller::entry(pane, parent, index) {
                app.core.cmds.extend(miller_click(pane, path, is_dir));
            }
        }
        (HitTarget::Splitter { vertical }, MouseButton::Left) => app.input.mouse.drag = Some(vertical),
        (HitTarget::Tab(i), MouseButton::Left) => app.core.cmds.push(Cmd::SelectTab(i)),
        (HitTarget::Tab(i) | HitTarget::TabClose(i), MouseButton::Middle)
        | (HitTarget::TabClose(i), MouseButton::Left) => app.core.cmds.push(Cmd::CloseTab(i)),
        (HitTarget::NewTab, MouseButton::Left) => app.core.cmds.push(Cmd::Action(Action::NewTab)),
        (HitTarget::Crumb(i), MouseButton::Left | MouseButton::Middle) => crumb(app, i, button),
        (HitTarget::Address, MouseButton::Left) => edit_address(app),
        (HitTarget::SidebarRow(i), MouseButton::Left) => super::sidebar::click(app, i),
        (HitTarget::SidebarRow(i), MouseButton::Middle) => super::sidebar::middle_click(app, i),
        (HitTarget::Filter, MouseButton::Left) => {
            app.core.ui.filter_focus = true;
            super::filter::arm(app);
        }
        (HitTarget::PaletteRow(i), MouseButton::Left) => palette_row(app, i, m.modifiers),
        (HitTarget::Toast(i), MouseButton::Left) => dismiss_toast(app, i),
        // The menus of a tab, the details header and a sidebar place (`input::menu`).
        (HitTarget::Tab(i), MouseButton::Right) => super::menu::open_tab(app, i, (m.column, m.row)),
        (HitTarget::PaneColumn { pane, .. }, MouseButton::Right) => {
            app.core.cmds.push(Cmd::FocusPane(pane));
            super::menu::open_header(app, pane, (m.column, m.row));
        }
        (HitTarget::SidebarRow(i), MouseButton::Right) => super::menu::open_sidebar(app, i, (m.column, m.row)),
        // Other buttons on targets that only take the left one.
        (
            HitTarget::PaneHeader { .. }
            | HitTarget::PaneColumn { .. }
            | HitTarget::MillerRow { .. }
            | HitTarget::Splitter { .. }
            | HitTarget::TabClose(_)
            | HitTarget::NewTab
            | HitTarget::Crumb(_)
            | HitTarget::Address
            | HitTarget::Filter
            | HitTarget::PaletteRow(_)
            | HitTarget::Toast(_),
            _,
        ) => {}
        (t @ (HitTarget::BottomTab(_) | HitTarget::JobRow(_) | HitTarget::JobButton { .. }), _) => {
            super::jobs::click(app, t, m)
        }
        (t @ (HitTarget::PreviewScroll | HitTarget::ArchiveRow(_) | HitTarget::PlayerPlay | HitTarget::PlayerSeek), _) => {
            super::preview::click(app, t, m)
        }
        (t @ (HitTarget::EditorTab(_) | HitTarget::EditorClose(_) | HitTarget::EditorCell { .. }), _) => {
            super::editor::click(app, t, m)
        }
        (HitTarget::RenameField, _) => super::rename::click(app, m),
        // Nothing to do (the palette's field keeps the keyboard; dialogs, menus and Quick
        // Look took the mouse before).
        (
            HitTarget::PaletteInput
            | HitTarget::Status
            | HitTarget::DialogButton(_)
            | HitTarget::DialogField(_)
            | HitTarget::DialogRow(_)
            | HitTarget::MenuItem { .. }
            | HitTarget::QuickLook
            | HitTarget::DialogText(_),
            _,
        ) => {}
        // Phase T5 / T6 targets went to their owners before (`phase_t56`).
        (
            HitTarget::Overlay
            | HitTarget::SearchField
            | HitTarget::SearchPill(_)
            | HitTarget::SearchButton(_)
            | HitTarget::SearchStatusButton(_)
            | HitTarget::TermTab(_)
            | HitTarget::TermTabClose(_)
            | HitTarget::TermNew
            | HitTarget::TermExternal
            | HitTarget::TermCell { .. }
            | HitTarget::TermClip(_)
            | HitTarget::AiTab(_)
            | HitTarget::AiComposer
            | HitTarget::AiButton(_)
            | HitTarget::AiCard { .. }
            | HitTarget::AiCardButton { .. }
            | HitTarget::AiRow(_)
            | HitTarget::AiMention(_)
            | HitTarget::AiMcpUndo(_)
            | HitTarget::AiChat
            | HitTarget::CompareField(_)
            | HitTarget::CompareRow(_)
            | HitTarget::CompareButton(_)
            | HitTarget::CompareChip(_)
            | HitTarget::CompareProfile(_)
            | HitTarget::SettingsNav(_)
            | HitTarget::SettingsRow(_)
            | HitTarget::SettingsSearch
            | HitTarget::SettingsClose
            | HitTarget::SettingsKeyRow(_)
            | HitTarget::SettingsKeyChip { .. }
            | HitTarget::SettingsKeyAdd(_)
            // Phase T7 targets went to their owners before (`phase_t56`).
            | HitTarget::SidebarEject(_)
            | HitTarget::ScreenButton(_)
            | HitTarget::ScreenChip(_)
            | HitTarget::ScreenField(_)
            | HitTarget::ScreenTab(_)
            | HitTarget::ScreenClose
            | HitTarget::ScreenArea(_)
            | HitTarget::TreemapTile(_)
            | HitTarget::TreemapCrumb(_)
            | HitTarget::TreemapSide(_)
            | HitTarget::DupesRow(_)
            | HitTarget::DupesRoot(_)
            | HitTarget::DupesHeader(_)
            | HitTarget::ConfirmBar(_)
            | HitTarget::PlacesRow { .. }
            | HitTarget::PlacesButton { .. }
            | HitTarget::PlacesArea { .. },
            _,
        ) => {}
    }
}

/// A press on a Miller side column's entry `path` (`is_dir`: a folder), as in the GUI
/// (`ui/views/miller.rs`): a folder opens in pane `pane`, a file opens its folder with
/// the file selected.
pub fn miller_click(pane: PaneId, path: std::path::PathBuf, is_dir: bool) -> Vec<Cmd> {
    let nav = if is_dir {
        Cmd::Navigate { pane, location: Location::Dir(path), select: None }
    } else {
        let Some(dir) = path.parent().map(std::path::Path::to_path_buf) else { return Vec::new() };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        Cmd::Navigate { pane, location: Location::Dir(dir), select: name }
    };
    vec![Cmd::FocusPane(pane), nav]
}

/// A press anywhere but the address / filter field leaves it.
fn leave_fields(app: &mut App, target: HitTarget) {
    if !matches!(target, HitTarget::Address | HitTarget::Crumb(_)) && app.core.ui.address.editing {
        app.core.ui.address.editing = false;
        app.core.ui.address.request_focus = false;
    }
    if target != HitTarget::Filter {
        app.input.filter.editing = false;
    }
}

/// How a press on a row is handled.
#[derive(Clone, Copy, Debug)]
pub struct RowCtx {
    pub button: MouseButton,
    pub mods: ClickMods,
    /// The second press of a double-click.
    pub double: bool,
    /// Checkbox mode, and the press is on the row's mark column.
    pub checkbox: bool,
    /// `general.single_click_open`.
    pub single_click_open: bool,
}

/// What a row press asks of the core.
#[derive(Clone, Debug, PartialEq)]
pub enum RowEffect {
    /// Open the entry (`Cmd::Open`).
    Open(EntryKey),
    /// Middle-clicked folder (`behaviour.middle_click`).
    Middle(Location),
}

/// A press on visible item `index` of `p`: the GUI's selection and cursor semantics
/// (`views/flow.rs`). Returns what the core still has to do.
pub fn row_press(p: &mut Pane, index: usize, ctx: RowCtx) -> Option<RowEffect> {
    let key = EntryKey::from(p.key_at(index)?);
    let any_mod = ctx.mods.toggle || ctx.mods.range;
    match ctx.button {
        MouseButton::Left if ctx.double && !ctx.checkbox => Some(RowEffect::Open(key)),
        MouseButton::Left => {
            if ctx.checkbox {
                p.selection.toggle(key.clone());
                p.cursor = Some(key);
                None
            } else if ctx.single_click_open && !any_mod && p.entry(&key).is_some_and(|e| e.is_dir_like()) {
                Some(RowEffect::Open(key))
            } else {
                let order = p.visible_keys();
                p.selection.click(&order, key.clone(), ctx.mods);
                p.cursor = Some(key);
                None
            }
        }
        MouseButton::Right => {
            // The GUI selects the row and opens its context menu (the caller opens it).
            if !p.selection.contains(&key) {
                p.selection.select_only(key.clone());
            }
            p.cursor = Some(key);
            None
        }
        MouseButton::Middle => {
            p.entry(&key).filter(|e| e.is_dir_like()).map(|e| RowEffect::Middle(Location::Dir(e.path.clone())))
        }
    }
}

/// Collapse or expand group `group` of `p`.
pub fn toggle_group(p: &mut Pane, group: usize) {
    let Some(label) = p.groups.get(group).map(|g| g.label.clone()) else { return };
    if !p.collapsed_groups.remove(&label) {
        p.collapsed_groups.insert(label);
    }
    p.dirty = true;
}

/// Scroll `p` by `rows` (negative: up). The cursor stays; `view::pane::prepare_pane`
/// clamps the offset to the end of the list.
pub fn scroll(p: &mut Pane, rows: i32) {
    let top = if p.scroll_y.is_finite() { p.scroll_y.max(0.0) } else { 0.0 };
    p.scroll_y = (top + rows as f32).max(0.0);
}

fn wheel(app: &mut App, m: &MouseEvent, rows: i32) {
    if app.core.palette.open {
        return;
    }
    let pane = match hit(&app.hits, m.column, m.row) {
        Some((
            _,
            HitTarget::PaneRow { pane, .. } | HitTarget::PaneHeader { pane, .. } | HitTarget::PaneBackground { pane },
        )) => pane,
        Some((_, t @ (HitTarget::BottomTab(_) | HitTarget::JobRow(_) | HitTarget::JobButton { .. }))) => {
            return super::jobs::wheel(app, t, rows);
        }
        Some((
            _,
            t @ (HitTarget::PreviewScroll | HitTarget::ArchiveRow(_) | HitTarget::PlayerPlay | HitTarget::PlayerSeek),
        )) => {
            return super::preview::wheel(app, t, rows);
        }
        Some((_, t @ (HitTarget::EditorTab(_) | HitTarget::EditorClose(_) | HitTarget::EditorCell { .. }))) => {
            return super::editor::wheel(app, t, rows);
        }
        _ => return,
    };
    if let Some(p) = app.core.ws.find_pane_mut(pane) {
        scroll(p, wheel_rows(p, rows));
    }
}

/// What a wheel step of `rows` scrolls `p` by: a grid / gallery line is a whole row of
/// cells (several lines tall), so one notch moves one of them there.
pub fn wheel_rows(p: &Pane, rows: i32) -> i32 {
    if crate::view::pane::PaneMode::of(p).is_grid() { rows.signum() } else { rows }
}

/// The split a splitter drag to `(x, y)` sets: `(second, share)` where `second` is the quad
/// layout's row split (`split2`). `None` when the drawn panes have no ratio to drag
/// (one pane, or quad stacked into equal rows).
pub fn split_share(regions: &Regions, shown: usize, vertical: bool, x: u16, y: u16) -> Option<(bool, f32)> {
    if regions.panes.len() < 2 || (shown == 4 && regions.stacked) {
        return None;
    }
    // The panes with their title lines: the area the layout split.
    let body = regions.panes.iter().chain(&regions.titles).map(|p| p.rect).reduce(|a, b| a.union(b))?;
    let (start, len, at) = if vertical { (body.x, body.width, x) } else { (body.y, body.height, y) };
    let usable = len.saturating_sub(GAP);
    if usable < 2 {
        return None;
    }
    let share = f32::from(at.saturating_sub(start)) / f32::from(usable);
    Some((shown == 4 && !vertical, share.clamp(SPLIT_MIN, SPLIT_MAX)))
}

fn drag_splitter(app: &mut App, vertical: bool, x: u16, y: u16) {
    let shown = app.core.ws.tab().shown();
    match split_share(app.regions(), shown, vertical, x, y) {
        Some((true, share)) => app.core.ws.tab_mut().split2 = share,
        Some((false, share)) => app.core.cmds.push(Cmd::SetSplit(share)),
        None => {}
    }
}

/// Breadcrumb `i` of the focused pane: segment `i` of
/// `soyuz_appcore::state::crumbs::crumbs(location, Some(home))`, as in the GUI.
fn crumb(app: &mut App, i: usize, button: MouseButton) {
    let pane = app.core.ws.pane();
    let id = pane.id;
    let home = soyuz_fs::home();
    let list = soyuz_appcore::state::crumbs::crumbs(&pane.location, Some(&home));
    let Some(c) = list.get(i) else { return };
    let location = c.location.clone();
    let last = i + 1 == list.len();
    match button {
        MouseButton::Middle => app.core.cmds.push(Cmd::OpenInNewTab { location, activate: false }),
        _ if last => edit_address(app),
        _ => {
            app.core.cmds.push(Cmd::FocusPane(id));
            app.core.cmds.push(Cmd::Navigate { pane: id, location, select: None });
        }
    }
}

fn edit_address(app: &mut App) {
    if !app.core.ui.address.editing {
        app.core.cmds.push(Cmd::Action(Action::FocusAddressBar));
    }
}

fn palette_row(app: &mut App, i: usize, m: KeyModifiers) {
    let Some(exec) = app.core.palette.results.get(i).map(|it| it.exec.clone()) else { return };
    app.core.palette.selected = i;
    let mods = soyuz_appcore::palette::RunMods {
        command: m.contains(KeyModifiers::CONTROL),
        shift: m.contains(KeyModifiers::SHIFT),
    };
    soyuz_appcore::palette::run(&mut app.core, &app.host, exec, mods);
}

/// Toast `i` of the toast lines (`view::toasts`: the newest [`crate::view::MAX_TOASTS`]
/// that are not closing, newest last).
fn dismiss_toast(app: &mut App, i: usize) {
    let mut live: Vec<_> = app.core.ui.toasts.live.iter_mut().filter(|l| l.closing.is_none()).collect();
    let first = live.len().saturating_sub(crate::view::MAX_TOASTS);
    if let Some(l) = live.get_mut(first + i) {
        l.closing = Some(0.0);
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::state::pane::ListingStatus;
    use soyuz_appcore::state::pane::PaneId;
    use soyuz_appcore::state::view_memory::ViewState;
    use soyuz_core::{EntryInfo, EntryKind};

    use super::*;
    use crate::layout::{Facts, PaneRect};

    fn pane() -> Pane {
        let mut p = Pane::new(Location::Dir("/data".into()), ViewState::default());
        let mut entries = vec![EntryInfo::minimal("/data/sub".into(), EntryKind::Dir)];
        entries.extend((0..30).map(|i| EntryInfo::minimal(format!("/data/f{i:02}.txt").into(), EntryKind::File)));
        p.set_entries_ordered(entries);
        p.status = ListingStatus::Ready;
        p.refresh_visible(true, true);
        p
    }

    fn ctx(button: MouseButton, toggle: bool, range: bool) -> RowCtx {
        RowCtx { button, mods: ClickMods { toggle, range }, double: false, checkbox: false, single_click_open: false }
    }

    fn key(p: &Pane, i: usize) -> EntryKey {
        EntryKey::from(p.key_at(i).unwrap())
    }

    fn selected(p: &Pane) -> Vec<usize> {
        (0..p.visible.len()).filter(|&i| p.selection.contains(&key(p, i))).collect()
    }

    #[test]
    fn hits_resolve_with_their_rect() {
        let mut h = Hits::default();
        let id = PaneId(3);
        h.push(Rect::new(0, 1, 40, 10), HitTarget::PaneBackground { pane: id });
        h.push(Rect::new(0, 2, 40, 1), HitTarget::PaneRow { pane: id, vis_version: 1, index: 1 });
        assert_eq!(
            hit(&h, 5, 2),
            Some((Rect::new(0, 2, 40, 1), HitTarget::PaneRow { pane: id, vis_version: 1, index: 1 }))
        );
        assert_eq!(hit(&h, 5, 5).map(|(_, t)| t), Some(HitTarget::PaneBackground { pane: id }));
        assert_eq!(hit(&h, 50, 5), None);
    }

    #[test]
    fn plain_ctrl_and_shift_clicks() {
        let mut p = pane();
        assert_eq!(row_press(&mut p, 2, ctx(MouseButton::Left, false, false)), None);
        assert_eq!((p.cursor_pos(), selected(&p)), (Some(2), vec![2]));
        row_press(&mut p, 4, ctx(MouseButton::Left, true, false));
        assert_eq!((p.cursor_pos(), selected(&p)), (Some(4), vec![2, 4]));
        row_press(&mut p, 2, ctx(MouseButton::Left, true, false));
        assert_eq!(selected(&p), vec![4]);
        row_press(&mut p, 6, ctx(MouseButton::Left, false, true));
        // The Ctrl+click made row 2 the anchor.
        assert_eq!(selected(&p), vec![2, 3, 4, 5, 6]);
        assert!(!p.scroll_to_cursor, "a click never scrolls the view");
        // Gone row: nothing.
        assert_eq!(row_press(&mut p, 999, ctx(MouseButton::Left, false, false)), None);
        assert_eq!(selected(&p), vec![2, 3, 4, 5, 6]);
    }

    #[test]
    fn double_click_checkbox_and_single_click_open() {
        let mut p = pane();
        let dbl = RowCtx { double: true, ..ctx(MouseButton::Left, false, false) };
        assert_eq!(row_press(&mut p, 1, dbl), Some(RowEffect::Open(key(&p, 1))));
        // Checkbox mode toggles without touching the rest of the selection.
        let cb = RowCtx { checkbox: true, ..ctx(MouseButton::Left, false, false) };
        row_press(&mut p, 3, cb);
        row_press(&mut p, 5, cb);
        assert_eq!((p.cursor_pos(), selected(&p)), (Some(5), vec![3, 5]));
        row_press(&mut p, 3, cb);
        assert_eq!(selected(&p), vec![5]);
        // Single-click open: folders open, files select; modifiers select.
        let so = RowCtx { single_click_open: true, ..ctx(MouseButton::Left, false, false) };
        assert_eq!(row_press(&mut p, 0, so), Some(RowEffect::Open(key(&p, 0))));
        assert_eq!(row_press(&mut p, 2, so), None);
        assert_eq!(selected(&p), vec![2]);
        let so_ctrl = RowCtx { mods: ClickMods { toggle: true, range: false }, ..so };
        assert_eq!(row_press(&mut p, 0, so_ctrl), None);
        assert_eq!(selected(&p), vec![0, 2]);
    }

    #[test]
    fn right_and_middle_clicks() {
        let mut p = pane();
        row_press(&mut p, 2, ctx(MouseButton::Left, false, false));
        row_press(&mut p, 3, ctx(MouseButton::Left, true, false));
        // On a selected row the selection stays; elsewhere it becomes that row.
        assert_eq!(row_press(&mut p, 3, ctx(MouseButton::Right, false, false)), None);
        assert_eq!(selected(&p), vec![2, 3]);
        row_press(&mut p, 7, ctx(MouseButton::Right, false, false));
        assert_eq!((p.cursor_pos(), selected(&p)), (Some(7), vec![7]));
        // Middle: folders only, the selection untouched.
        assert_eq!(
            row_press(&mut p, 0, ctx(MouseButton::Middle, false, false)),
            Some(RowEffect::Middle(Location::Dir("/data/sub".into())))
        );
        assert_eq!(row_press(&mut p, 1, ctx(MouseButton::Middle, false, false)), None);
        assert_eq!(selected(&p), vec![7]);
    }

    #[test]
    fn double_click_timing() {
        let mut s = State::default();
        let iv = Duration::from_millis(400);
        let t0 = Instant::now();
        let a = HitTarget::PaneRow { pane: PaneId(1), vis_version: 1, index: 0 };
        let b = HitTarget::PaneRow { pane: PaneId(1), vis_version: 1, index: 1 };
        assert!(!s.press(a, t0, iv));
        assert!(s.press(a, t0 + Duration::from_millis(300), iv));
        // The third press starts over.
        assert!(!s.press(a, t0 + Duration::from_millis(350), iv));
        // Too slow, or another target, or the listing changed: no double-click.
        assert!(!s.press(a, t0 + Duration::from_millis(800), iv));
        assert!(!s.press(b, t0 + Duration::from_millis(850), iv));
        let b2 = HitTarget::PaneRow { pane: PaneId(1), vis_version: 2, index: 1 };
        assert!(!s.press(b2, t0 + Duration::from_millis(900), iv));
    }

    #[test]
    fn double_click_interval_follows_the_setting() {
        let mut c = Config::default();
        c.general.double_click_speed_ms = 250;
        assert_eq!(double_click_interval(&c), Duration::from_millis(250));
        c.general.double_click_speed_ms = 0;
        assert_eq!(double_click_interval(&c), Duration::from_millis(u64::from(DOUBLE_CLICK_MS_MIN)));
        c.general.double_click_speed_ms = 100_000;
        assert_eq!(double_click_interval(&c), Duration::from_millis(u64::from(DOUBLE_CLICK_MS_MAX)));
    }

    #[test]
    fn wheel_moves_one_grid_line_per_notch() {
        let mut p = pane();
        assert_eq!(wheel_rows(&p, WHEEL_ROWS), WHEEL_ROWS);
        for mode in [soyuz_core::ViewMode::Grid, soyuz_core::ViewMode::Gallery] {
            p.view.mode = mode;
            assert_eq!((wheel_rows(&p, WHEEL_ROWS), wheel_rows(&p, -WHEEL_ROWS)), (1, -1), "{mode:?}");
        }
    }

    #[test]
    fn wheel_scrolls_without_the_cursor() {
        let mut p = pane();
        p.move_cursor(1, false, false);
        crate::view::pane::prepare_pane(&mut p, 40, 10);
        scroll(&mut p, WHEEL_ROWS);
        crate::view::pane::prepare_pane(&mut p, 40, 10);
        assert_eq!((p.scroll_y, p.cursor_pos()), (3.0, Some(1)));
        scroll(&mut p, 100);
        crate::view::pane::prepare_pane(&mut p, 40, 10);
        // 31 items in 9 list rows (the details header takes one).
        assert_eq!(p.scroll_y, 22.0, "clamped to the end");
        scroll(&mut p, -100);
        assert_eq!(p.scroll_y, 0.0);
        p.scroll_y = f32::NAN;
        scroll(&mut p, -WHEEL_ROWS);
        assert_eq!(p.scroll_y, 0.0);
    }

    #[test]
    fn miller_clicks_like_the_gui() {
        let id = PaneId(4);
        let dir = std::path::PathBuf::from("/data/sub");
        let go = |cmds: &[Cmd]| match cmds {
            [Cmd::FocusPane(f), Cmd::Navigate { pane, location: Location::Dir(d), select }]
                if *f == id && *pane == id =>
            {
                Some((d.clone(), select.clone()))
            }
            _ => None,
        };
        assert_eq!(go(&miller_click(id, dir.clone(), true)), Some((dir.clone(), None)));
        // A file: its folder, with the file selected.
        assert_eq!(go(&miller_click(id, dir.join("a.txt"), false)), Some((dir, Some("a.txt".to_string()))));
    }

    #[test]
    fn group_header_toggles_collapse() {
        let mut p = pane();
        toggle_group(&mut p, 5);
        assert!(p.collapsed_groups.is_empty(), "no such group");
    }

    #[test]
    fn splitter_drag_shares() {
        let area = Rect::new(0, 0, 81, 10);
        let dual = Regions::compute(area, &Default::default(), &Facts { shown: 2, ..Facts::default() });
        // Body is 81 wide, 80 usable: column 40 is the middle.
        assert_eq!(split_share(&dual, 2, true, 40, 3), Some((false, 0.5)));
        assert_eq!(split_share(&dual, 2, true, 0, 3), Some((false, SPLIT_MIN)));
        assert_eq!(split_share(&dual, 2, true, 80, 3), Some((false, SPLIT_MAX)));
        let quad =
            Regions::compute(Rect::new(0, 0, 101, 31), &Default::default(), &Facts { shown: 4, ..Facts::default() });
        let (_, s) = split_share(&quad, 4, false, 0, 15).unwrap();
        assert!(split_share(&quad, 4, false, 0, 15).unwrap().0 && (s - 14.0 / 28.0).abs() < 1e-6, "{s}");
        assert!(!split_share(&quad, 4, true, 50, 3).unwrap().0);
        // Quad stacked into equal rows, or a single pane: nothing to drag.
        let stacked =
            Regions::compute(Rect::new(0, 0, 50, 30), &Default::default(), &Facts { shown: 4, ..Facts::default() });
        assert_eq!(split_share(&stacked, 4, false, 0, 10), None);
        let one = Regions { panes: vec![PaneRect { index: 0, rect: area }], ..Regions::default() };
        assert_eq!(split_share(&one, 1, true, 10, 1), None);
    }
}
