//! Keys and mouse of the Trash and Computer pages (Phase T7 "places",
//! `crate::view::pane::places`): what the GUI's `ui/views/trash.rs` and
//! `ui/views/computer.rs` do with the mouse, plus a keyboard for the terminal.
//!
//! While the focused pane shows one of the pages, `crate::input::list::route` hands the
//! list's keys here first; what is not used goes on to the core's keymap (Delete and
//! Shift+Delete then ask to delete the selection permanently, as in the GUI; the palette,
//! tabs, panes, Quit, … work as usual).
//!
//! | Key | Trash | Computer |
//! |---|---|---|
//! | the cursor actions (Up / Down, Page Up / Down, Home / End, user bindings) | move and select only that item | move |
//! | Shift + those, `ExtendUp` / `ExtendDown` | extend the selection from the anchor | move |
//! | Ctrl + the arrows / Page / Home / End | move the cursor only | move |
//! | Space | toggle the cursor's item | |
//! | `SelectAll` (Ctrl+A) | select every item | |
//! | Esc | clear the selection (when there is one) | |
//! | `Open` (Enter) | the item's menu | open the drive |
//! | `ContextMenu` (Shift+F10) | the item's menu | the drive's menu |
//! | `Delete` / `DeletePermanent` (Del, Shift+Del) | the core asks to delete the selection (or the cursor's item) permanently | |
//! | `Refresh` (F5) | list the bins again | list the drives again |
//! | R (unbound letter) | restore the selection (or the cursor's item) | |
//! | E (unbound letter) | empty the trash (the core asks first) | |
//!
//! The mouse ([`on_mouse`], from `crate::input::mouse` for the `Places*` targets): a left
//! press on a trashed item selects it like the GUI (Ctrl toggles, Shift extends); on a
//! drive it opens the drive (the GUI's cards open on a click). A right press selects the
//! item when it is not selected and opens its menu; a middle press on a drive follows
//! `behaviour.middle_click`. A press on the page's empty area focuses the pane (the left
//! button also clears the trash selection); the header buttons run; the wheel scrolls.

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::context_menu::{self as catalogue, Entry, Item, ItemStyle, MenuIcon, Run};
use soyuz_appcore::models::devices;
use soyuz_appcore::models::trash;
use soyuz_appcore::ops::fileops::FileOpCmd;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::Location;
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, MiddleClick, Mods};

use crate::app::App;
use crate::input::menu::{self, Runs};
use crate::layout::HitTarget;
use crate::view::pane::places::{self as view, Page, button};
use crate::widgets::Menu;

/// The focused pane and its page, when it shows one.
pub fn active(core: &AppCore) -> Option<(PaneId, Page)> {
    let pane = core.ws.pane();
    view::page(pane).map(|p| (pane.id, p))
}

/// This frame's list keys while the focused pane shows a page: the page's keys first, the
/// rest to the core's keymap.
pub fn route(app: &mut App, events: &[Event]) {
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    let mut i = 0;
    while i < frame.events.len() {
        let Some((pane, page)) = active(&app.core) else { break };
        if app.focus() != crate::input::TuiFocus::List {
            break;
        }
        if !frame.consumed[i]
            && let InputEvent::Chord(c) = frame.events[i]
            && key(app, pane, page, c)
        {
            frame.consumed[i] = true;
            // The text a letter or Space typed goes with it.
            if let Some(InputEvent::Text(_)) = frame.events.get(i + 1) {
                frame.consumed[i + 1] = true;
                i += 1;
            }
        }
        i += 1;
    }
    if app.focus() == crate::input::TuiFocus::List {
        let (kctx, text) = crate::input::key_context(crate::input::TuiFocus::List);
        app.core.input(&app.host, &mut frame, kctx, text);
    }
}

/// How a move changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sel {
    /// Select only the new cursor item.
    Only,
    /// Extend from the anchor.
    Extend,
    /// Leave it.
    Keep,
}

/// A move of the cursor (rows; `isize::MIN` / `MAX`: the first / last).
fn moves(core: &AppCore, c: KeyChord) -> Option<(isize, Sel)> {
    let page = |n: usize| n.max(1) as isize;
    let rows = active(core).map_or(1, |(p, pg)| view::nav(p, pg).rows);
    let step = |a: &Action| match a {
        Action::CursorUp | Action::ExtendUp => Some(-1),
        Action::CursorDown | Action::ExtendDown => Some(1),
        Action::CursorPageUp => Some(-page(rows)),
        Action::CursorPageDown => Some(page(rows)),
        Action::CursorHome => Some(isize::MIN),
        Action::CursorEnd => Some(isize::MAX),
        _ => None,
    };
    if let Some(a) = core.keymap.lookup(KeyContext::FileList, c) {
        let sel = if matches!(a, Action::ExtendUp | Action::ExtendDown) { Sel::Extend } else { Sel::Only };
        // A chord bound to something else keeps its action.
        return step(a).map(|d| (d, sel));
    }
    // Shift / Ctrl with the plain keys: extend / move the cursor only.
    let sel = if c.mods == Mods::SHIFT {
        Sel::Extend
    } else if c.mods == Mods::CTRL {
        Sel::Keep
    } else {
        return None;
    };
    let base = KeyChord { mods: Mods::empty(), ..c };
    let d = core.keymap.lookup(KeyContext::FileList, base).and_then(step).or(match c.key {
        KeyName::ArrowUp => Some(-1),
        KeyName::ArrowDown => Some(1),
        KeyName::PageUp => Some(-page(rows)),
        KeyName::PageDown => Some(page(rows)),
        KeyName::Home => Some(isize::MIN),
        KeyName::End => Some(isize::MAX),
        _ => None,
    })?;
    Some((d, sel))
}

/// One chord on the page; returns whether it was used.
fn key(app: &mut App, pane: PaneId, page: Page, c: KeyChord) -> bool {
    let bound = app.core.keymap.lookup(KeyContext::FileList, c).cloned();
    if let Some((d, sel)) = moves(&app.core, c) {
        move_cursor(&mut app.core, pane, page, d, sel);
        return true;
    }
    let plain = c.mods.is_empty();
    let shifted = c.mods == Mods::SHIFT;
    match bound {
        Some(Action::ContextMenu) => return open_menu_at_cursor(app, pane, page),
        Some(Action::Refresh) => {
            refresh(&mut app.core, page);
            return true;
        }
        Some(Action::Open) => {
            match page {
                Page::Trash => {
                    open_menu_at_cursor(app, pane, page);
                }
                Page::Computer => open_drive(&mut app.core, pane, view::nav(pane, page).cursor),
            }
            return true;
        }
        Some(Action::SelectAll) if page == Page::Trash => {
            app.core.fileops.trash.select_all();
            return true;
        }
        // The core deletes the selection permanently (after asking); with none, the
        // cursor's item.
        Some(Action::Delete | Action::DeletePermanent | Action::SecureWipe) if page == Page::Trash => {
            let ts = &mut app.core.fileops.trash;
            let cursor = view::nav(pane, page).cursor;
            if ts.selected.is_empty() && cursor < ts.items.len() {
                ts.click(cursor, false, false);
            }
            return false;
        }
        _ => {}
    }
    if shifted && c.key == KeyName::F10 {
        return open_menu_at_cursor(app, pane, page);
    }
    if plain && c.key == KeyName::F5 {
        refresh(&mut app.core, page);
        return true;
    }
    if page != Page::Trash || !plain {
        return false;
    }
    let ts = &mut app.core.fileops.trash;
    let cursor = view::nav(pane, page).cursor;
    match c.key {
        KeyName::Space if cursor < ts.items.len() => {
            ts.click(cursor, false, true);
            true
        }
        KeyName::Escape if !ts.selected.is_empty() => {
            ts.selected.clear();
            true
        }
        KeyName::R if bound.is_none() => {
            restore(&mut app.core, pane);
            true
        }
        KeyName::E if bound.is_none() => {
            empty(&mut app.core);
            true
        }
        _ => false,
    }
}

/// Move pane `pane`'s cursor by `d` rows on `page`.
fn move_cursor(core: &mut AppCore, pane: PaneId, page: Page, d: isize, sel: Sel) {
    let n = view::len(core, page);
    if n == 0 {
        return;
    }
    let cur = view::nav(pane, page).cursor.min(n - 1);
    let to = match d {
        isize::MIN => 0,
        isize::MAX => n - 1,
        d => cur.saturating_add_signed(d).min(n - 1),
    };
    view::update(pane, page, |nav| {
        nav.cursor = to;
        nav.follow = true;
    });
    if page == Page::Trash {
        match sel {
            Sel::Only => core.fileops.trash.click(to, false, false),
            Sel::Extend => core.fileops.trash.click(to, true, false),
            Sel::Keep => {}
        }
    }
}

/// List the bins / drives again.
fn refresh(core: &mut AppCore, page: Page) {
    match page {
        Page::Trash if !core.fileops.trash.loading => core.fileops.trash.reload(),
        Page::Trash => {}
        Page::Computer => core.services.places.refresh(),
    }
}

/// Restore the selection (or the cursor's item) through the jobs engine, like the GUI's
/// Restore button (undoable: the undo trashes the items again).
fn restore(core: &mut AppCore, pane: PaneId) {
    let ts = &mut core.fileops.trash;
    if ts.selected.is_empty() {
        let cursor = view::nav(pane, Page::Trash).cursor;
        if cursor >= ts.items.len() {
            return;
        }
        ts.click(cursor, false, false);
    }
    let items = ts.selected_items();
    ts.selected.clear();
    core.cmds.push(Cmd::FileOp(FileOpCmd::TrashRestore(items)));
}

/// Empty the trash after the core's confirmation.
fn empty(core: &mut AppCore) {
    if !core.fileops.trash.items.is_empty() {
        core.cmds.push(Cmd::FileOp(FileOpCmd::ConfirmEmptyTrash));
    }
}

/// Open drive `index` in pane `pane`.
fn open_drive(core: &mut AppCore, pane: PaneId, index: usize) {
    let Some(mount) = view::drives(core).get(index).map(|v| v.mount.clone()) else { return };
    core.cmds.push(Cmd::FocusPane(pane));
    core.cmds.push(Cmd::Navigate { pane, location: Location::Dir(mount), select: None });
}

/// A header button.
fn run_button(core: &mut AppCore, page: Page, id: u8) {
    match id {
        button::RESTORE => {
            let ts = &mut core.fileops.trash;
            let items = ts.selected_items();
            if !items.is_empty() {
                ts.selected.clear();
                core.cmds.push(Cmd::FileOp(FileOpCmd::TrashRestore(items)));
            }
        }
        button::PURGE => {
            let items = core.fileops.trash.selected_items();
            core.cmds.push(Cmd::FileOp(FileOpCmd::ConfirmPurge(items)));
        }
        button::EMPTY => empty(core),
        button::REFRESH => refresh(core, page),
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------
// Menus.

/// The menu of row `index` (a trashed item: the core's trash menu for the selection; a
/// drive: open, the sidebar place's entries and its device's entries).
fn row_menu(core: &AppCore, pane: PaneId, page: Page, index: usize) -> Vec<Entry> {
    match page {
        Page::Trash => {
            let ts = &core.fileops.trash;
            let Some(it) = ts.items.get(index) else { return Vec::new() };
            trash::menu(ts.selected_items(), it.original.clone(), pane)
        }
        Page::Computer => {
            let drives = view::drives(core);
            let Some(v) = drives.get(index) else { return Vec::new() };
            let path = v.mount.clone();
            let mut out = vec![
                Entry::Item(Item {
                    icon: MenuIcon::Id(IconId::FolderOpen),
                    label: tr!("app-ctx-open").to_string(),
                    chord: None,
                    enabled: true,
                    style: ItemStyle::Plain,
                    run: Some(Run::Cmd(Cmd::Navigate { pane, location: Location::Dir(path.clone()), select: None })),
                    keep_open: false,
                    tip: None,
                }),
                Entry::Separator,
            ];
            let removable = core.ui.sidebar.favorites.iter().any(|f| f.path == path);
            out.extend(catalogue::place_menu(&path, removable));
            let all = devices::block_devices();
            if !all.is_empty()
                && let Some(dev) = devices::device_at(&all, &path)
            {
                let label = soyuz_appcore::services::places::volume_label(v);
                out.push(Entry::Separator);
                out.extend(devices::device_menu(&dev, &all, &label));
            }
            out
        }
    }
}

/// Open `entries` as a popup menu at `anchor`.
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

/// The cursor item's menu (keyboard), below its row when it is on screen. With nothing
/// selected the cursor's trashed item is selected first (like a right click).
fn open_menu_at_cursor(app: &mut App, pane: PaneId, page: Page) -> bool {
    let n = view::len(&app.core, page);
    if n == 0 {
        return true;
    }
    let cursor = view::nav(pane, page).cursor.min(n - 1);
    if page == Page::Trash && !app.core.fileops.trash.is_selected(cursor) {
        app.core.fileops.trash.click(cursor, false, false);
    }
    let mut anchor = None;
    for (r, t) in app.hits.iter() {
        match *t {
            HitTarget::PlacesRow { pane: p, index } if p == pane && index == cursor => {
                anchor = Some((r.x.saturating_add(2.min(r.width)), r.y.saturating_add(1)));
            }
            HitTarget::PlacesArea { pane: p } if p == pane && anchor.is_none() => {
                anchor = Some((r.x.saturating_add(1), r.y.saturating_add(1)));
            }
            _ => {}
        }
    }
    let entries = row_menu(&app.core, pane, page, cursor);
    open_entries(app, entries, anchor.unwrap_or((1, 1)));
    true
}

/// `ContextMenu` from the palette or the Menu key while the list (not the sidebar) has
/// the keyboard and the focused pane shows a page
/// (`crate::input::menu::open_context`): the cursor item's menu. Returns whether it
/// opened one (`false`: not on a page).
pub fn open_context(app: &mut App) -> bool {
    if app.focus() != crate::input::TuiFocus::List {
        return false;
    }
    match active(&app.core) {
        Some((pane, page)) => open_menu_at_cursor(app, pane, page),
        None => false,
    }
}

// ---------------------------------------------------------------------------------------
// Mouse.

/// A mouse event over a `Places*` target (see the module docs); always taken.
pub fn on_mouse(app: &mut App, target: HitTarget, m: &MouseEvent) -> bool {
    let pane = match target {
        HitTarget::PlacesRow { pane, .. } | HitTarget::PlacesButton { pane, .. } | HitTarget::PlacesArea { pane } => {
            pane
        }
        _ => return false,
    };
    let Some(page) = app.core.ws.find_pane(pane).and_then(view::page) else { return true };
    match m.kind {
        MouseEventKind::Down(b) => press(app, pane, page, target, b, m),
        MouseEventKind::ScrollDown => scroll(&app.core, pane, page, crate::input::mouse::WHEEL_ROWS),
        MouseEventKind::ScrollUp => scroll(&app.core, pane, page, -crate::input::mouse::WHEEL_ROWS),
        _ => {}
    }
    true
}

fn press(app: &mut App, pane: PaneId, page: Page, target: HitTarget, b: MouseButton, m: &MouseEvent) {
    use crossterm::event::KeyModifiers;
    app.core.cmds.push(Cmd::FocusPane(pane));
    let anchor = (m.column, m.row);
    match target {
        HitTarget::PlacesRow { index, .. } if index < view::len(&app.core, page) => {
            view::update(pane, page, |n| n.cursor = index);
            match (page, b) {
                (Page::Trash, MouseButton::Left) => {
                    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
                    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
                    app.core.fileops.trash.click(index, shift, ctrl);
                }
                (Page::Trash, MouseButton::Right) => {
                    if !app.core.fileops.trash.is_selected(index) {
                        app.core.fileops.trash.click(index, false, false);
                    }
                    let entries = row_menu(&app.core, pane, page, index);
                    open_entries(app, entries, anchor);
                }
                (Page::Computer, MouseButton::Left) => open_drive(&mut app.core, pane, index),
                (Page::Computer, MouseButton::Right) => {
                    let entries = row_menu(&app.core, pane, page, index);
                    open_entries(app, entries, anchor);
                }
                (Page::Computer, MouseButton::Middle) => {
                    let Some(mount) = view::drives(&app.core).get(index).map(|v| v.mount.clone()) else { return };
                    let location = Location::Dir(mount);
                    match app.core.config.behaviour.middle_click {
                        MiddleClick::NewTab => app.core.cmds.push(Cmd::OpenInNewTab { location, activate: false }),
                        MiddleClick::OtherPane => app.core.cmds.push(Cmd::OpenInOtherPane(location)),
                        MiddleClick::Nothing => {}
                    }
                }
                (Page::Trash, MouseButton::Middle) => {}
            }
        }
        HitTarget::PlacesButton { button, .. } if b == MouseButton::Left => run_button(&mut app.core, page, button),
        HitTarget::PlacesArea { .. } if b == MouseButton::Left && page == Page::Trash => {
            app.core.fileops.trash.selected.clear();
        }
        _ => {}
    }
}

/// Scroll pane `pane`'s page by `rows` (the cursor stays).
fn scroll(core: &AppCore, pane: PaneId, page: Page, rows: i32) {
    let n = view::len(core, page);
    view::update(pane, page, |nav| {
        let max = n.saturating_sub(nav.rows.max(1));
        nav.top = nav.top.saturating_add_signed(rows as isize).min(max);
    });
}
