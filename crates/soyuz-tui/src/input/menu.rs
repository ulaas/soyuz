//! The popup menu (`TuiFocus::Menu`): the context menu of the file list — `ContextMenu`
//! (Shift+F10), the Menu key (when the terminal reports it, e.g. kitty's keyboard
//! protocol), a right click on a row (selected first, as in the GUI) or on the empty
//! area of a pane — and the menus of a sidebar place (right click, or Shift+F10 / the Menu
//! key while the sidebar has the keyboard: [`open_sidebar`]), a tab (right click,
//! [`open_tab`]) and the details header (right click, [`open_header`]: the column
//! chooser, which stays open while columns are ticked).
//!
//! The entries come from the core's catalogue (`soyuz_appcore::models::context_menu`),
//! the same one the GUI's right-click menu draws: the items menu for a selection, the
//! background menu otherwise. They become `widgets::Menu` rows; an activated row closes the
//! menu and runs its catalogue [`Run`] (kept in [`State::runs`], the rows carry its
//! index). Two GUI-only rows get a plain form: "Open with" opens the core's Open-with
//! chooser, "Folder color" is a submenu of colour names.
//!
//! While a menu is open it owns the keyboard and the mouse: keys go to
//! `widgets::Menu::apply`, a press on a `MenuItem` target to `Menu::click`, a press
//! anywhere else closes it (and does nothing else).

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::context_menu::{self as catalogue, Entry, ItemStyle, MenuCtx, MenuIcon, Run};
use soyuz_appcore::state::pane::{ContextTarget, PaneId};
use soyuz_appcore::tr;
use soyuz_core::config::Action;

use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::{Menu, MenuItem, MenuOutcome};

/// What a menu entry runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    /// A core action (`Cmd::Action`).
    Action(Action),
    /// Catalogue entry `i` ([`State::runs`]).
    Catalog(usize),
    /// A command of a job's "More" menu (`input::jobs::open_menu`).
    Job(soyuz_jobs::JobId, soyuz_appcore::models::jobs_rows::JobMenuCmd),
}

/// What the open menu's catalogue rows run.
#[derive(Debug, Default)]
pub struct Runs {
    /// By row payload (`None`: only close).
    pub runs: Vec<Option<Run>>,
    /// The rows that keep the menu open (column ticks), by payload.
    pub keep: Vec<bool>,
}

impl Runs {
    fn push(&mut self, r: Option<Run>, keep: bool) -> usize {
        self.runs.push(r);
        self.keep.push(keep);
        self.runs.len() - 1
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// The open menu, if any.
#[derive(Debug, Default)]
pub struct State {
    pub menu: Option<Menu<MenuCmd>>,
    /// What the open menu's catalogue rows run.
    pub runs: Runs,
    /// The open menu is pane `.0`'s column chooser, opened at `.1` (rebuilt after a tick).
    pub header: Option<(PaneId, (u16, u16))>,
}

/// Whether a menu is open.
pub fn is_open(st: &State) -> bool {
    st.menu.is_some()
}

/// `ContextMenu` (Shift+F10, the Menu key): the menu of the focused pane, at its cursor
/// row. With a selection it is the items menu; with only a cursor, the cursor's item is
/// selected first (like a right click); otherwise the background menu. While the sidebar
/// has the keyboard: the menu of the place under its cursor.
pub fn open_context(app: &mut App) {
    if app.focus() == crate::input::TuiFocus::Sidebar {
        let list = crate::view::sidebar::entries(&app.core);
        if let Some(i) = crate::view::sidebar::cursor_index(&app.core, &list, &app.input.sidebar.nav) {
            let anchor = app
                .hits
                .iter()
                .find(|(_, t)| *t == HitTarget::SidebarRow(i))
                .map_or((1, 1), |(r, _)| (r.x.saturating_add(2.min(r.width)), r.y.saturating_add(1)));
            open_sidebar(app, i, anchor);
        }
        return;
    }
    let pane = app.core.ws.pane();
    let id = pane.id;
    let cursor_row = pane.cursor.as_ref().and_then(|c| pane.visible_keys().iter().position(|k| k == c));
    let target = if !pane.selection.is_empty() {
        ContextTarget::Items
    } else if let Some(key) = pane.cursor.clone() {
        if let Some(p) = app.core.ws.find_pane_mut(id) {
            p.selection.select_only(key);
        }
        ContextTarget::Items
    } else {
        ContextTarget::Background
    };
    let anchor = anchor_for(app, id, cursor_row);
    open_for(app, id, target, anchor);
}

/// Where the keyboard-opened menu goes: below the cursor row of `pane` if it is on screen,
/// else at the top-left of the pane, else the top-left of the screen.
fn anchor_for(app: &App, pane: PaneId, row: Option<usize>) -> (u16, u16) {
    let mut fallback = None;
    for (r, t) in app.hits.iter() {
        match *t {
            HitTarget::PaneRow { pane: p, index, .. } if p == pane && Some(index) == row => {
                return (r.x.saturating_add(2.min(r.width)), r.y.saturating_add(1));
            }
            HitTarget::PaneBackground { pane: p } if p == pane && fallback.is_none() => {
                fallback = Some((r.x.saturating_add(1), r.y.saturating_add(1)));
            }
            _ => {}
        }
    }
    fallback.unwrap_or((1, 1))
}

/// Open the context menu of `pane` for `target` at `anchor` (a screen cell; the menu
/// opens below-right of it). A right click passes the clicked cell.
pub fn open_for(app: &mut App, pane: PaneId, target: ContextTarget, anchor: (u16, u16)) {
    let dual = app.core.ws.tab().shown() >= 2;
    let has_clip = catalogue::has_clip(&app.core.fileops);
    let Some(p) = app.core.ws.find_pane_mut(pane) else { return };
    p.context = target;
    let Some(p) = app.core.ws.find_pane(pane) else { return };
    let cx = MenuCtx {
        config: &app.core.config,
        keymap: &app.core.keymap,
        has_clip,
        dual,
        show_hidden: app.core.ui.show_hidden,
        checkbox_mode: app.core.ui.checkbox_mode,
    };
    let entries = catalogue::pane_menu(&cx, p);
    open_entries(app, entries, anchor);
}

/// The menu of sidebar entry `i` (`input::sidebar::menu`: a place on disk, a saved
/// connection, a smart folder, a saved search; other rows have none) at `anchor`.
pub fn open_sidebar(app: &mut App, i: usize, anchor: (u16, u16)) {
    let entries = crate::input::sidebar::menu(&app.core, i);
    open_entries(app, entries, anchor);
}

/// The menu of tab `i` at `anchor`.
pub fn open_tab(app: &mut App, i: usize, anchor: (u16, u16)) {
    let entries = catalogue::tab_menu(&app.core, i);
    open_entries(app, entries, anchor);
}

/// The column chooser of `pane`'s details header at `anchor`.
pub fn open_header(app: &mut App, pane: PaneId, anchor: (u16, u16)) {
    let Some(p) = app.core.ws.find_pane(pane) else { return };
    let entries = catalogue::column_menu(p);
    open_entries(app, entries, anchor);
    if app.input.menu.menu.is_some() {
        app.input.menu.header = Some((pane, anchor));
    }
}

/// Open catalogue `entries` at `anchor` (nothing when they are all empty).
fn open_entries(app: &mut App, entries: Vec<Entry>, anchor: (u16, u16)) {
    let mut runs = Runs::default();
    let items = rows(entries, &mut runs);
    if items.is_empty() {
        return;
    }
    app.input.menu.runs = runs;
    app.input.menu.header = None;
    open(app, Menu::new(items, anchor));
}

/// Catalogue entries as menu rows; their runs go to `runs`. Separators never lead, trail
/// or repeat; empty submenus are left out; a caption is a disabled row.
pub fn rows(entries: Vec<Entry>, runs: &mut Runs) -> Vec<MenuItem<MenuCmd>> {
    let mut out: Vec<MenuItem<MenuCmd>> = Vec::new();
    for e in entries {
        match e {
            Entry::Separator => {
                if !matches!(out.last(), None | Some(MenuItem::Separator)) {
                    out.push(MenuItem::Separator);
                }
            }
            Entry::Caption(c) => out.push(run_row(runs, c, None, false).enabled(false)),
            Entry::Item(it) => {
                let mut row = run_row(runs, it.label, it.run, it.keep_open).enabled(it.enabled);
                if let Some(c) = it.chord {
                    row = row.chord(c);
                }
                match (it.style, it.icon) {
                    (ItemStyle::Check(on), _) | (_, MenuIcon::Tag { checked: on, .. }) => row = row.checked(on),
                    (_, MenuIcon::Check) => row = row.checked(true),
                    (_, MenuIcon::Blank) => row = row.checked(false),
                    _ => {}
                }
                out.push(row);
            }
            Entry::Submenu(s) => {
                let items = rows(s.items, runs);
                if !items.is_empty() {
                    out.push(MenuItem::submenu(s.label, items));
                }
            }
            Entry::OpenWith(path) => {
                out.push(run_row(runs, tr!("tui-menu-open-with"), Some(Run::Cmd(Cmd::ChooseOpenWith(path))), false));
            }
            Entry::FolderColor(dirs) => {
                let items = rows(catalogue::folder_color_entries(&dirs), runs);
                out.push(MenuItem::submenu(tr!("app-looks-folder-color"), items));
            }
        }
    }
    if matches!(out.last(), Some(MenuItem::Separator)) {
        out.pop();
    }
    out
}

/// A row running `r` (stored in `runs`).
fn run_row(runs: &mut Runs, label: impl Into<String>, r: Option<Run>, keep: bool) -> MenuItem<MenuCmd> {
    let i = runs.push(r, keep);
    MenuItem::run(label, MenuCmd::Catalog(i))
}

/// Open `menu` (replacing an open one).
pub fn open(app: &mut App, menu: Menu<MenuCmd>) {
    app.input.menu.menu = Some(menu);
}

fn close(app: &mut App) {
    app.input.menu.menu = None;
    app.input.menu.runs = Runs::default();
    app.input.menu.header = None;
}

fn run(app: &mut App, c: MenuCmd) {
    let header = app.input.menu.header;
    let cursor = app.input.menu.menu.as_ref().and_then(|m| m.cursor);
    let runs = std::mem::take(&mut app.input.menu.runs);
    close(app);
    match c {
        MenuCmd::Action(a) => app.core.cmds.push(Cmd::Action(a)),
        MenuCmd::Job(id, c) => soyuz_appcore::models::jobs_rows::apply_menu(&mut app.core, id, c),
        MenuCmd::Catalog(i) => {
            let keep = runs.keep.get(i).copied().unwrap_or(false);
            match runs.runs.into_iter().nth(i).flatten() {
                // A column tick applies now and the chooser opens again where it was.
                Some(Run::Cmd(Cmd::Column(c))) if keep => {
                    catalogue::apply_column(&mut app.core, c);
                    if let Some((pane, anchor)) = header {
                        open_header(app, pane, anchor);
                        if let Some(m) = app.input.menu.menu.as_mut() {
                            m.cursor = cursor;
                        }
                    }
                }
                // Tags, the shelf, Recent, Compare & sync, …: `crate::tools::drain` applies them
                // later this frame.
                Some(Run::Tool(t)) => soyuz_appcore::models::tools::queue(t),
                Some(r) => catalogue::run(r, &mut app.core.cmds),
                None => {}
            }
        }
    }
}

/// The Menu key in the file list or the sidebar opens the context menu (crossterm reports
/// it only with the kitty keyboard protocol).
pub fn menu_key(app: &mut App, events: &[Event]) {
    let pressed =
        events.iter().any(|e| matches!(e, Event::Key(k) if k.code == KeyCode::Menu && k.kind != KeyEventKind::Release));
    if pressed && matches!(app.focus(), crate::input::TuiFocus::List | crate::input::TuiFocus::Sidebar) {
        open_context(app);
    }
}

/// Keys while a menu is open.
pub fn route(app: &mut App, events: &[Event]) {
    let screen = app.regions().area;
    let frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    for ev in &frame.events {
        let Some(menu) = app.input.menu.menu.as_mut() else { return };
        match menu.apply(ev, screen) {
            MenuOutcome::Activated(c) => run(app, c),
            MenuOutcome::Close => close(app),
            MenuOutcome::Consumed | MenuOutcome::Ignored => {}
        }
    }
}

/// A mouse event while a menu is open (`crate::input::mouse` hands it over).
pub fn mouse(app: &mut App, m: &MouseEvent) {
    let MouseEventKind::Down(button) = m.kind else { return };
    let screen = app.regions().area;
    let target = crate::input::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t);
    let Some(menu) = app.input.menu.menu.as_mut() else { return };
    match target {
        Some(HitTarget::MenuItem { depth, index }) if button == MouseButton::Left => {
            match menu.click(usize::from(depth), index, screen) {
                MenuOutcome::Activated(c) => run(app, c),
                MenuOutcome::Close => close(app),
                MenuOutcome::Consumed | MenuOutcome::Ignored => {}
            }
        }
        Some(HitTarget::MenuItem { .. }) => {}
        _ => close(app),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::EntryKind;

    fn item(label: &str, run: Option<Run>) -> Entry {
        Entry::Item(catalogue::Item {
            icon: MenuIcon::None,
            label: label.into(),
            chord: Some("Ctrl+X".into()),
            enabled: true,
            style: ItemStyle::Plain,
            run,
            keep_open: false,
            tip: None,
        })
    }

    #[test]
    fn rows_tidy_separators_and_index_runs() {
        let mut runs = Runs::default();
        let r = rows(
            vec![
                Entry::Separator,
                item("a", None),
                Entry::Separator,
                Entry::Separator,
                Entry::Submenu(catalogue::Submenu {
                    kind: catalogue::SubmenuKind::Ai,
                    icon: MenuIcon::None,
                    label: "sub".into(),
                    items: vec![item("b", Some(Run::Cmd(Cmd::Action(Action::Refresh))))],
                }),
                Entry::Submenu(catalogue::Submenu {
                    kind: catalogue::SubmenuKind::Ai,
                    icon: MenuIcon::None,
                    label: "empty".into(),
                    items: vec![Entry::Separator],
                }),
                Entry::OpenWith("x.txt".into()),
                Entry::Separator,
            ],
            &mut runs,
        );
        assert_eq!(r.len(), 4, "{r:?}");
        assert!(matches!(&r[0], MenuItem::Entry(e) if e.label == "a" && e.chord.as_deref() == Some("Ctrl+X")));
        assert_eq!(r[1], MenuItem::Separator);
        let MenuItem::Entry(sub) = &r[2] else { panic!() };
        let EntryKind::Submenu(inner) = &sub.kind else { panic!() };
        assert!(matches!(&inner[0], MenuItem::Entry(e) if e.kind == EntryKind::Run(MenuCmd::Catalog(1))));
        assert_eq!(runs.len(), 3);
        assert!(matches!(runs.runs[2], Some(Run::Cmd(Cmd::ChooseOpenWith(_)))));
    }
}
