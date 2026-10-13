//! The disk usage treemap screen (Phase T7, agent C): `models::treemap::TreemapModel`
//! drawn as cell tiles, with the breadcrumb, the side list (contents / largest), the
//! legend, the trash confirm bar and the tile menu.
//!
//! | Key | Does |
//! |---|---|
//! | Arrows (map) | select the nearest tile of this level in that direction |
//! | Enter (map) | zoom into the selected folder; open the selected file |
//! | Up / Down / PageUp / PageDown / Home / End (list) | move |
//! | Enter (list) | a folder zooms in; a file is selected, Enter again opens it |
//! | Backspace / Alt+Up | one level up |
//! | Tab | map ⇄ list (narrow screens: M, or Tab, switches what is shown) |
//! | C / L | the side list's Contents / Largest files |
//! | O | open the selected folder (or this level) in the pane; the screen steps aside |
//! | Del / X (`Action::Delete`) | ask to move the selection to the trash |
//! | Shift+F10 / Menu (`Action::ContextMenu`) | the tile's menu |
//! | F5 / Ctrl+R (`Action::Refresh`) | scan again |
//! | Esc | close the confirm bar, else stop the scan, else close |
//!
//! The confirm bar takes Enter / Y (move to the trash: `FileOpCmd::Delete { Trash }`
//! through the jobs engine, undoable) and Esc / N.
//!
//! Mouse: a click on a tile drills into the folder it belongs to (or selects a file), a
//! double click opens a file, a right click opens the menu; the breadcrumb, the tabs, the
//! buttons and the side rows are clickable and the wheel scrolls the side list.

use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::host::Wake;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::treemap::{SideTab, TreemapItem, TreemapModel};
use soyuz_core::config::{KeyName, Mods};

use super::{Screen, chord, is_delete, is_menu, is_refresh, plain};
use crate::app::App;
use crate::input::menu::MenuCmd;
use crate::layout::HitTarget;
use crate::view::screens::chrome::confirm;
use crate::view::screens::treemap::{AREA_SIDE, CellTile, Geometry, TreemapVm, View, button, side_row_h};
use crate::widgets::{Menu, MenuItem, SelectList};

/// What the treemap screen keeps while it is open.
pub struct State {
    pub model: TreemapModel,
    /// The side list has the keyboard.
    pub focus_list: bool,
    /// Narrow screens show the list instead of the map (M).
    pub list: bool,
    /// The side list's cursor and scroll.
    pub side: SelectList,
    /// The level the side list was last built for (its cursor resets on a new one).
    pub level: usize,
    /// The layout of the last frame.
    pub geo: Geometry,
    /// The map's tiles (cached until the level, the tree or the map size changes).
    pub tiles: Vec<CellTile>,
    pub cache: Option<(usize, u64, u16, u16, bool)>,
    /// The tile the open menu is for, and its entries.
    pub menu: Option<(usize, Vec<TreemapItem>)>,
    /// The model built by `view::screens::prepare`.
    pub vm: TreemapVm,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("model", &self.model)
            .field("focus_list", &self.focus_list)
            .field("list", &self.list)
            .finish_non_exhaustive()
    }
}

impl State {
    /// The treemap of `root`; its scan starts now.
    pub fn new(root: PathBuf, wake: Wake) -> State {
        State {
            model: TreemapModel::new(root, wake),
            focus_list: false,
            list: false,
            side: SelectList::default(),
            level: 0,
            geo: Geometry::default(),
            tiles: Vec::new(),
            cache: None,
            menu: None,
            vm: TreemapVm::default(),
        }
    }

    /// The list has the keyboard (focused, or the only thing shown).
    pub fn list_active(&self) -> bool {
        match self.geo.view {
            View::List => true,
            View::Map => false,
            _ => self.focus_list,
        }
    }

    /// Side-list items shown at once.
    pub fn side_height(&self) -> usize {
        usize::from(self.geo.side.height / side_row_h(self.geo.side.width).max(1)).max(1)
    }

    /// The top-level tiles (this level's children).
    fn top_tiles(&self) -> impl Iterator<Item = &CellTile> {
        self.tiles.iter().filter(|t| t.depth == 0)
    }

    /// The first (largest) top-level tile.
    fn first_tile(&self) -> Option<usize> {
        self.top_tiles().next().map(|t| t.id)
    }

    /// The top-level tile holding the selection (a deeper selection maps to its folder).
    fn selected_tile(&self) -> Option<CellTile> {
        let sel = self.model.selected?;
        let target = self.model.click_target(sel);
        self.top_tiles().find(|t| t.id == sel || t.id == target).copied()
    }

    /// Select the tile nearest to the selection in direction (`dx`, `dy`).
    fn step(&mut self, dx: i32, dy: i32) {
        let Some(from) = self.selected_tile() else {
            self.model.selected = self.first_tile();
            return;
        };
        let (fx, fy) = from.centre2();
        // Distances in "pixels": a cell is about twice as high as wide.
        let score = |t: &CellTile| {
            let (cx, cy) = t.centre2();
            let (ax, ay) = (cx - fx, (cy - fy) * 2);
            let along = ax * dx + ay * dy;
            let across = (ax * dy - ay * dx).abs();
            (along > 0).then_some((along <= across, i64::from(ax).pow(2) + i64::from(ay).pow(2)))
        };
        let best = self.top_tiles().filter(|t| t.id != from.id).filter_map(|t| Some((score(t)?, t.id))).min();
        if let Some((_, id)) = best {
            self.model.selected = Some(id);
        }
    }
}

fn state(app: &mut App) -> Option<&mut State> {
    match app.input.screens.screen.as_mut() {
        Some(Screen::Treemap(t)) => Some(t.as_mut()),
        _ => None,
    }
}

/// One key; returns whether the screen used it.
pub fn key(app: &mut App, ev: &InputEvent) -> bool {
    let Some(c) = chord(ev) else { return true };
    let Some(st) = state(app) else { return true };
    if st.model.confirm_delete.is_some() {
        match c.key {
            KeyName::Enter | KeyName::Y if plain(&c) => confirm_trash(app),
            KeyName::Escape | KeyName::N if plain(&c) => st.model.confirm_delete = None,
            _ => {}
        }
        return true;
    }
    if plain(&c) && c.key == KeyName::Escape {
        if st.model.is_scanning() {
            st.model.cancel();
        } else {
            super::close(app);
        }
        return true;
    }
    if is_refresh(&app.core, &c) {
        if let Some(st) = state(app) {
            st.model.rescan();
            st.side = SelectList::default();
        }
        return true;
    }
    if is_menu(&app.core, &c) {
        open_menu(app, None);
        return true;
    }
    if is_delete(&app.core, &c) {
        let st = state(app).expect("treemap");
        if let Some(id) = st.model.selected {
            st.model.ask_trash(id);
        }
        return true;
    }
    let alt_up = c.mods == Mods::ALT && c.key == KeyName::ArrowUp;
    if alt_up || (plain(&c) && c.key == KeyName::Backspace) {
        state(app).expect("treemap").model.up();
        return true;
    }
    if c.mods == Mods::SHIFT && c.key == KeyName::Tab {
        toggle_focus(state(app).expect("treemap"));
        return true;
    }
    if !plain(&c) {
        return false;
    }
    let st = state(app).expect("treemap");
    match c.key {
        KeyName::Tab => toggle_focus(st),
        KeyName::M => toggle_view(st),
        KeyName::C => set_tab(st, SideTab::Contents),
        KeyName::L => set_tab(st, SideTab::Largest),
        KeyName::O => {
            let id = st.model.selected.filter(|&i| st.model.tree.nodes[i].is_dir).unwrap_or(st.model.current);
            run_item(app, id, TreemapItem::OpenInPane);
        }
        KeyName::Enter => activate(app),
        _ if st.list_active() => {
            let len = st.model.side_items().len();
            let height = st.side_height();
            if st.side.apply(ev, len, height) {
                follow_list(st);
            }
        }
        KeyName::ArrowLeft => st.step(-1, 0),
        KeyName::ArrowRight => st.step(1, 0),
        KeyName::ArrowUp => st.step(0, -1),
        KeyName::ArrowDown => st.step(0, 1),
        _ => {}
    }
    true
}

fn toggle_focus(st: &mut State) {
    if st.geo.view == View::Both {
        st.focus_list = !st.focus_list;
        if st.focus_list {
            follow_list(st);
        }
    } else {
        toggle_view(st);
    }
}

fn toggle_view(st: &mut State) {
    if st.geo.view != View::Both {
        st.list = !st.list;
        st.focus_list = st.list;
    }
}

fn set_tab(st: &mut State, tab: SideTab) {
    if st.model.side != tab {
        st.model.side = tab;
        st.side = SelectList::default();
    }
}

/// The map shows what the list's cursor is on.
fn follow_list(st: &mut State) {
    if let Some(&id) = st.model.side_items().get(st.side.cursor) {
        st.model.selected = Some(id);
    }
}

/// Enter: zoom into a folder, open a file (a list row is selected first).
fn activate(app: &mut App) {
    let Some(st) = state(app) else { return };
    let id = if st.list_active() {
        let Some(&id) = st.model.side_items().get(st.side.cursor) else { return };
        if st.model.selected != Some(id) && !st.model.tree.nodes[id].is_dir {
            st.model.selected = Some(id);
            return;
        }
        id
    } else {
        match st.model.selected {
            Some(id) => id,
            None => {
                st.model.selected = st.first_tile();
                return;
            }
        }
    };
    open(app, id);
}

/// Zoom into `id`'s folder, or open the file.
fn open(app: &mut App, id: usize) {
    let Some(st) = state(app) else { return };
    if st.model.launchable(id) {
        let path = st.model.tree.nodes[id].path.clone();
        soyuz_appcore::ops::launch(&mut app.core, &path);
    } else {
        st.model.click(id);
    }
}

fn confirm_trash(app: &mut App) {
    let core = &mut app.core;
    if let Some(Screen::Treemap(st)) = app.input.screens.screen.as_mut() {
        st.model.confirm_trash(core);
    }
}

/// Run menu entry `item` on tile `id` (the screen steps aside for Reveal / Open in pane).
fn run_item(app: &mut App, id: usize, item: TreemapItem) {
    let core = &mut app.core;
    let Some(Screen::Treemap(st)) = app.input.screens.screen.as_mut() else { return };
    if st.model.apply_item(core, id, item) {
        super::hide(app);
    }
}

/// Open the menu of tile `id` (the selection, else this level) at `anchor` (else next to
/// the tile).
pub fn open_menu(app: &mut App, at: Option<(usize, (u16, u16))>) {
    let del = super::delete_label(&app.core);
    let Some(st) = state(app) else { return };
    let (id, anchor) = match at {
        Some(a) => a,
        None => {
            let id = st.model.selected.unwrap_or(st.model.current);
            let g = st.geo;
            let anchor = match st.top_tiles().find(|t| t.id == id) {
                Some(t) if !st.list_active() => (g.map.x + t.x + t.w / 2, g.map.y + t.y + t.h / 2),
                _ => (g.toolbar.x + 2, g.toolbar.y + 1),
            };
            (id, anchor)
        }
    };
    let items = st.model.menu_items(id);
    let mut rows = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if *item == TreemapItem::Trash {
            rows.push(MenuItem::Separator);
        }
        let mut row = MenuItem::run(item.label(), MenuCmd::Tool(i as u32)).enabled(st.model.item_enabled(id, *item));
        row = match item {
            TreemapItem::Trash => row.chord(del.clone()),
            TreemapItem::OpenInPane => row.chord("O"),
            _ => row,
        };
        rows.push(row);
    }
    let st = state(app).expect("treemap");
    st.menu = Some((id, items));
    crate::input::menu::open(app, Menu::new(rows, anchor));
}

/// A row of the tile menu.
pub fn menu_cmd(app: &mut App, k: u32) {
    let Some(st) = state(app) else { return };
    let Some((id, items)) = st.menu.take() else { return };
    if let Some(item) = items.get(k as usize) {
        run_item(app, id, *item);
    }
}

/// A mouse event while the screen shows.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) {
    match ev.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let rows = if ev.kind == MouseEventKind::ScrollDown {
                crate::input::mouse::WHEEL_ROWS
            } else {
                -crate::input::mouse::WHEEL_ROWS
            };
            let Some(st) = state(app) else { return };
            if matches!(target, HitTarget::ScreenArea(AREA_SIDE) | HitTarget::TreemapSide(_)) {
                let len = st.model.side_items().len();
                let height = st.side_height();
                st.side.wheel(rows, len, height);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => click(app, target),
        MouseEventKind::Down(MouseButton::Right) => {
            let Some(st) = state(app) else { return };
            let id = match target {
                HitTarget::TreemapTile(id) => Some(id as usize),
                HitTarget::TreemapSide(i) => st.model.side_items().get(i).copied(),
                _ => None,
            };
            if let Some(id) = id.filter(|&i| i < st.model.tree.nodes.len()) {
                st.model.selected = Some(id);
                open_menu(app, Some((id, (ev.column, ev.row))));
            }
        }
        _ => {}
    }
}

fn click(app: &mut App, target: HitTarget) {
    let interval = crate::input::mouse::double_click_interval(&app.core.config);
    let double = app.input.mouse.press(target, Instant::now(), interval);
    let Some(st) = state(app) else { return };
    match target {
        HitTarget::TreemapTile(id) => {
            let id = id as usize;
            if id >= st.model.tree.nodes.len() {
                return;
            }
            st.focus_list = false;
            if double && st.model.launchable(id) {
                open(app, id);
            } else {
                st.model.click(id);
            }
        }
        HitTarget::TreemapCrumb(id) => st.model.go(id as usize),
        HitTarget::TreemapSide(i) => {
            st.focus_list = true;
            let items = st.model.side_items();
            st.side.select(i, items.len());
            let Some(&id) = items.get(i) else { return };
            if double && st.model.launchable(id) {
                open(app, id);
            } else {
                st.model.side_click(id);
            }
        }
        HitTarget::ScreenTab(0) => set_tab(st, SideTab::Contents),
        HitTarget::ScreenTab(_) => set_tab(st, SideTab::Largest),
        HitTarget::ScreenButton(button::UP) => {
            st.model.up();
        }
        HitTarget::ScreenButton(button::RESCAN) if st.model.is_scanning() => st.model.cancel(),
        HitTarget::ScreenButton(button::RESCAN) => {
            st.model.rescan();
            st.side = SelectList::default();
        }
        HitTarget::ScreenButton(button::VIEW) => toggle_view(st),
        HitTarget::ConfirmBar(confirm::YES) => confirm_trash(app),
        HitTarget::ConfirmBar(_) => st.model.confirm_delete = None,
        HitTarget::ScreenClose => super::close(app),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(id: usize, x: u16, y: u16, w: u16, h: u16) -> CellTile {
        CellTile { id, x, y, w, h, depth: 0, frame: false }
    }

    #[test]
    fn arrows_move_to_the_nearest_tile_in_that_direction() {
        let mut st = State::new(std::env::temp_dir().join("soyuz-no-such-treemap"), soyuz_appcore::host::no_wake());
        st.model.cancel();
        //  1 | 2
        //  --+--
        //  3 | 4
        st.tiles = vec![tile(1, 0, 0, 20, 5), tile(2, 20, 0, 20, 5), tile(3, 0, 5, 20, 5), tile(4, 20, 5, 20, 5)];
        // Ids beyond the tree are fine here: only the geometry is read once one is
        // selected, and click_target needs the node: give the tree enough nodes.
        for _ in 0..4 {
            st.model.tree.add_child(0, soyuz_appcore::models::treemap::scan::SubTree::file("/x".into(), 1));
        }
        st.step(1, 0);
        assert_eq!(st.model.selected, Some(1), "the first tile when nothing is selected");
        st.step(1, 0);
        assert_eq!(st.model.selected, Some(2));
        st.step(0, 1);
        assert_eq!(st.model.selected, Some(4));
        st.step(-1, 0);
        assert_eq!(st.model.selected, Some(3));
        st.step(0, -1);
        assert_eq!(st.model.selected, Some(1));
        st.step(0, -1);
        assert_eq!(st.model.selected, Some(1), "nothing above: stays");
    }
}
