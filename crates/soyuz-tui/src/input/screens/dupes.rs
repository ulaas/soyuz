//! The duplicate finder screen (Phase T7, agent C): `models::dupes::DupesModel` with the
//! roots, the minimum size, the groups and marks, "keep" rules and the trash confirm bar.
//!
//! The keyboard moves between zones with Tab / Shift+Tab: the roots, the add-folder field
//! (folder completion like the address bar; no disk check, the search reports errors),
//! the minimum size and the list. F2 goes to the setup (it opens over the list on short
//! screens until the list has the keyboard again).
//!
//! | Key | Does |
//! |---|---|
//! | F5 / Ctrl+R (`Action::Refresh`), Enter in the setup | find (again) |
//! | Esc | close the confirm bar, else stop the search, else close |
//! | Alt+O | add the other pane's folder |
//! | Left / Right (roots, sizes) | pick; Del / X / Backspace removes the root |
//! | Enter (field) | add the folder typed |
//! | Up / Down / PageUp / PageDown / Home / End (list) | move |
//! | Space (list) | mark / unmark the file (the last copy is refused); fold a group |
//! | Enter (list) | fold a group; reveal the file in the pane (the screen steps aside) |
//! | Left / Right (list) | fold / unfold the group |
//! | n / o / s (list) | keep the newest / oldest / shortest path in this group |
//! | N / O / S (list) | the same in every group |
//! | C (list) | clear the marks |
//! | Del / X (`Action::Delete`) | ask to move the marked files to the trash |
//! | Shift+F10 / Menu (`Action::ContextMenu`) | the file's menu: reveal, copy path |
//!
//! The confirm bar takes Enter / Y (one `JobKind::Delete { Trash }` job, undoable) and
//! Esc / N.
//!
//! Mouse: a click on a root removes it, chips pick the size, the buttons do what they say,
//! a click on a file toggles its mark (double click: reveal), a click on a group header
//! folds it, a right click opens the file's menu and the wheel scrolls the list.

use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::host::Wake;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::dupes::{DupesModel, Keep, SIZES, other_pane_dir};
use soyuz_appcore::ops::fileops::names::show_path;
use soyuz_appcore::state::crumbs;
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{Screen, chord, is_delete, is_menu, is_refresh, plain, shifted};
use crate::app::App;
use crate::input::menu::MenuCmd;
use crate::layout::HitTarget;
use crate::view::screens::chrome::confirm;
use crate::view::screens::dupes::{AREA_LIST, DupesVm, Geometry, button, chip, field_rect};
use crate::widgets::{EditOutcome, LineEdit, Menu, MenuItem, SelectList};

/// Where the keyboard is on the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zone {
    Roots,
    Add,
    Size,
    #[default]
    List,
}

const ZONES: [Zone; 4] = [Zone::Roots, Zone::Add, Zone::Size, Zone::List];

/// A line of the list: a group header, or file `i` of a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Header(usize),
    File(usize, usize),
}

/// The list's lines (folded groups show their header only).
pub fn lines(m: &DupesModel) -> Vec<Line> {
    let mut v = Vec::new();
    for (g, group) in m.groups.iter().enumerate() {
        v.push(Line::Header(g));
        if !group.collapsed {
            v.extend((0..group.files.len()).map(|i| Line::File(g, i)));
        }
    }
    v
}

/// What the duplicate finder screen keeps while it is open.
pub struct State {
    pub model: DupesModel,
    pub zone: Zone,
    /// The root chip under the keyboard.
    pub root_cursor: usize,
    /// The add-folder field.
    pub add_text: String,
    pub add_edit: LineEdit,
    /// Folder completions of the field and the highlighted one.
    pub names: Vec<String>,
    pub pick: usize,
    /// A partial name is typed after the last separator (Tab accepts a completion).
    pub partial: bool,
    /// The list's cursor (over [`lines`]) and scroll.
    pub list: SelectList,
    /// The layout of the last frame.
    pub geo: Geometry,
    /// The file the open menu is for.
    pub menu: Option<PathBuf>,
    /// The model built by `view::screens::prepare`.
    pub vm: DupesVm,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").field("model", &self.model).field("zone", &self.zone).finish_non_exhaustive()
    }
}

impl State {
    /// The finder on `roots` (searching at once when there are some; else the add field
    /// has the keyboard).
    pub fn new(roots: Vec<PathBuf>, wake: Wake) -> State {
        let zone = if roots.is_empty() { Zone::Add } else { Zone::List };
        State {
            model: DupesModel::new(roots, wake),
            zone,
            root_cursor: 0,
            add_text: String::new(),
            add_edit: LineEdit::default(),
            names: Vec::new(),
            pick: 0,
            partial: false,
            list: SelectList::default(),
            geo: Geometry::default(),
            menu: None,
            vm: DupesVm::default(),
        }
    }

    fn move_zone(&mut self, forward: bool) {
        let at = ZONES.iter().position(|z| *z == self.zone).unwrap_or(0);
        let n = ZONES.len();
        self.zone = ZONES[if forward { (at + 1) % n } else { (at + n - 1) % n }];
        self.names.clear();
        self.pick = 0;
    }

    /// Rows of the list shown at once.
    pub fn list_height(&self) -> usize {
        usize::from(self.geo.list.height).max(1)
    }

    /// The line under the cursor.
    fn line(&self) -> Option<Line> {
        lines(&self.model).get(self.list.cursor).copied()
    }

    /// The group under the cursor (a header or one of its files).
    fn group(&self) -> Option<usize> {
        match self.line()? {
            Line::Header(g) | Line::File(g, _) => Some(g),
        }
    }

    /// Find (again): the list gets the keyboard.
    fn find(&mut self) {
        self.model.start();
        self.list = SelectList::default();
        if !self.model.roots.is_empty() {
            self.zone = Zone::List;
        }
    }

    /// Fold or unfold group `g` (`Some(true)`: fold); the cursor stays on its header.
    fn fold(&mut self, g: usize, fold: Option<bool>) {
        let Some(group) = self.model.groups.get_mut(g) else { return };
        group.collapsed = fold.unwrap_or(!group.collapsed);
        if let Some(i) = lines(&self.model).iter().position(|l| *l == Line::Header(g)) {
            self.list.cursor = i;
        }
    }

    /// Ask to move the marked files to the trash (nothing marked, or a job running: no).
    fn ask_trash(&mut self) {
        if self.model.marked().0 > 0 && !self.model.busy() {
            self.model.confirm = true;
        }
    }

    /// Add the field's folder (no disk check).
    fn add_typed(&mut self) -> bool {
        let added = self.model.add_root(&self.add_text);
        if added || !self.add_text.trim().is_empty() {
            self.add_text.clear();
            self.add_edit = LineEdit::default();
            self.names.clear();
        }
        added
    }

    /// Accept completion `i` into the field.
    fn accept(&mut self, i: usize) {
        let Some(name) = self.names.get(i).cloned() else { return };
        crumbs::accept(&mut self.add_text, &name);
        self.add_edit = LineEdit::at_end(&self.add_text);
        self.names.clear();
        self.pick = 0;
    }
}

fn state(app: &mut App) -> Option<&mut State> {
    match app.input.screens.screen.as_mut() {
        Some(Screen::Dupes(d)) => Some(d.as_mut()),
        _ => None,
    }
}

/// One key; `upper`: the case of the letter it typed, when it typed one. Returns whether
/// the screen used it.
pub fn key(app: &mut App, ev: &InputEvent, upper: Option<bool>) -> bool {
    let Some(st) = state(app) else { return true };
    if st.model.confirm {
        if let Some(c) = chord(ev).filter(plain) {
            match c.key {
                KeyName::Enter | KeyName::Y => trash(app),
                KeyName::Escape | KeyName::N => st.model.confirm = false,
                _ => {}
            }
        }
        return true;
    }
    // The field reads its keys (and typed text) first.
    if st.zone == Zone::Add && field_key(app, ev) {
        return true;
    }
    let Some(c) = chord(ev) else { return true };
    if plain(&c) && c.key == KeyName::Escape {
        let st = state(app).expect("dupes");
        if st.model.is_running() {
            st.model.stop_search();
        } else {
            super::close(app);
        }
        return true;
    }
    if is_refresh(&app.core, &c) {
        state(app).expect("dupes").find();
        return true;
    }
    if is_menu(&app.core, &c) {
        open_menu(app, None);
        return true;
    }
    if c.mods == Mods::ALT && c.key == KeyName::O {
        add_other(app);
        return true;
    }
    let st = state(app).expect("dupes");
    match c.key {
        KeyName::Tab if plain(&c) => {
            st.move_zone(true);
            return true;
        }
        KeyName::Tab if shifted(&c) => {
            st.move_zone(false);
            return true;
        }
        KeyName::F2 if plain(&c) => {
            st.zone = Zone::Roots;
            return true;
        }
        _ => {}
    }
    match st.zone {
        Zone::Roots => roots_key(app, &c),
        Zone::Size => size_key(app, &c),
        Zone::Add => true,
        Zone::List => list_key(app, ev, &c, upper),
    }
}

/// The add-folder field; returns whether it used the event.
fn field_key(app: &mut App, ev: &InputEvent) -> bool {
    let st = state(app).expect("dupes");
    if let Some(c) = chord(ev).filter(plain)
        && !st.names.is_empty()
    {
        let n = st.names.len();
        match c.key {
            KeyName::ArrowDown => {
                st.pick = (st.pick + 1) % n;
                return true;
            }
            KeyName::ArrowUp => {
                st.pick = (st.pick + n - 1) % n;
                return true;
            }
            KeyName::Tab if st.partial => {
                let pick = st.pick;
                st.accept(pick);
                return true;
            }
            _ => {}
        }
    }
    match st.add_edit.apply(&mut st.add_text, ev) {
        EditOutcome::Consumed => true,
        EditOutcome::Submit => {
            if st.add_text.trim().is_empty() {
                st.find();
            } else {
                st.add_typed();
            }
            true
        }
        EditOutcome::Tab | EditOutcome::Down => {
            st.move_zone(true);
            true
        }
        EditOutcome::ShiftTab | EditOutcome::Up => {
            st.move_zone(false);
            true
        }
        EditOutcome::PageUp | EditOutcome::PageDown => true,
        // Esc and the rest go to the screen's keys.
        EditOutcome::Cancel | EditOutcome::Ignored => false,
    }
}

fn roots_key(app: &mut App, c: &soyuz_core::config::KeyChord) -> bool {
    let st = state(app).expect("dupes");
    if !plain(c) {
        return false;
    }
    let n = st.model.roots.len();
    match c.key {
        KeyName::ArrowLeft => st.root_cursor = st.root_cursor.saturating_sub(1),
        KeyName::ArrowRight => st.root_cursor = (st.root_cursor + 1).min(n.saturating_sub(1)),
        KeyName::Delete | KeyName::X | KeyName::Backspace => {
            st.model.remove_root(st.root_cursor);
            st.root_cursor = st.root_cursor.min(st.model.roots.len().saturating_sub(1));
        }
        KeyName::Enter => st.find(),
        KeyName::ArrowDown => st.move_zone(true),
        KeyName::ArrowUp => st.move_zone(false),
        _ => {}
    }
    true
}

fn size_key(app: &mut App, c: &soyuz_core::config::KeyChord) -> bool {
    let st = state(app).expect("dupes");
    if !plain(c) {
        return false;
    }
    let i = st.model.min_idx;
    match c.key {
        KeyName::ArrowLeft => st.model.set_min(i.saturating_sub(1)),
        KeyName::ArrowRight => st.model.set_min((i + 1).min(SIZES.len() - 1)),
        KeyName::Enter | KeyName::Space => st.find(),
        KeyName::ArrowDown => st.move_zone(true),
        KeyName::ArrowUp => st.move_zone(false),
        _ => {}
    }
    true
}

fn list_key(app: &mut App, ev: &InputEvent, c: &soyuz_core::config::KeyChord, upper: Option<bool>) -> bool {
    if is_delete(&app.core, c) {
        state(app).expect("dupes").ask_trash();
        return true;
    }
    let st = state(app).expect("dupes");
    let len = lines(&st.model).len();
    let height = st.list_height();
    if st.list.apply(ev, len, height) {
        return true;
    }
    // Letters: the case typed decides (Shift: every group).
    let letter = matches!(c.key, KeyName::N | KeyName::O | KeyName::S | KeyName::C);
    if letter && (plain(c) || shifted(c)) {
        let all = upper.unwrap_or(shifted(c));
        let keep = match c.key {
            KeyName::N => Some(Keep::Newest),
            KeyName::O => Some(Keep::Oldest),
            KeyName::S => Some(Keep::ShortestPath),
            _ => None,
        };
        match (keep, all) {
            (Some(k), true) => st.model.keep_all(k),
            (Some(k), false) => {
                if let Some(g) = st.group() {
                    st.model.keep_in(g, k);
                }
            }
            (None, _) => st.model.clear_marks(),
        }
        return true;
    }
    if !plain(c) {
        return false;
    }
    match (c.key, st.line()) {
        (KeyName::Space | KeyName::Enter, Some(Line::Header(g))) => st.fold(g, None),
        (KeyName::Space, Some(Line::File(g, i))) => {
            st.model.toggle(g, i);
        }
        (KeyName::Enter, Some(Line::File(g, i))) => {
            let path = st.model.groups[g].files[i].path.clone();
            reveal(app, path);
        }
        (KeyName::ArrowLeft, Some(Line::Header(g) | Line::File(g, _))) => st.fold(g, Some(true)),
        (KeyName::ArrowRight, Some(Line::Header(g))) => st.fold(g, Some(false)),
        _ => {}
    }
    true
}

/// Show `path` in the pane; the screen steps aside with its results.
fn reveal(app: &mut App, path: PathBuf) {
    app.core.cmds.push(Cmd::RevealPath(path));
    super::hide(app);
}

fn add_other(app: &mut App) {
    if let Some(dir) = other_pane_dir(&app.core) {
        state(app).expect("dupes").model.add_path(dir);
    }
}

fn trash(app: &mut App) {
    let core = &mut app.core;
    if let Some(Screen::Dupes(st)) = app.input.screens.screen.as_mut() {
        st.model.trash_marked(core);
    }
}

/// The file under the cursor's menu (or the file of list line `at`, at a cell).
pub fn open_menu(app: &mut App, at: Option<(usize, (u16, u16))>) {
    let Some(st) = state(app) else { return };
    let (line, anchor) = match at {
        Some((l, a)) => (l, a),
        None => {
            let y = st.geo.list.y + (st.list.cursor.saturating_sub(st.list.top) as u16).min(st.geo.list.height);
            (st.list.cursor, (st.geo.list.x + 4, y))
        }
    };
    let Some(Line::File(g, i)) = lines(&st.model).get(line).copied() else { return };
    st.list.cursor = line;
    st.menu = Some(st.model.groups[g].files[i].path.clone());
    let rows = vec![
        MenuItem::run(tr!("tools-shelf-reveal"), MenuCmd::Tool(0)).chord("Enter"),
        MenuItem::run(tr!("tools-copy-path"), MenuCmd::Tool(1)),
    ];
    crate::input::menu::open(app, Menu::new(rows, anchor));
}

/// A row of the file menu.
pub fn menu_cmd(app: &mut App, k: u32) {
    let Some(st) = state(app) else { return };
    let Some(path) = st.menu.take() else { return };
    match k {
        0 => reveal(app, path),
        _ => app.core.cmds.push(Cmd::CopyText(show_path(&path))),
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
            let len = lines(&st.model).len();
            let height = st.list_height();
            st.list.wheel(rows, len, height);
        }
        MouseEventKind::Down(MouseButton::Left) => click(app, target, ev),
        MouseEventKind::Down(MouseButton::Right) => {
            if let HitTarget::DupesRow(line) = target {
                open_menu(app, Some((line, (ev.column, ev.row))));
            }
        }
        _ => {}
    }
}

fn click(app: &mut App, target: HitTarget, ev: &MouseEvent) {
    let interval = crate::input::mouse::double_click_interval(&app.core.config);
    let double = app.input.mouse.press(target, Instant::now(), interval);
    let Some(st) = state(app) else { return };
    match target {
        HitTarget::DupesRoot(i) => {
            st.model.remove_root(i);
            st.root_cursor = st.root_cursor.min(st.model.roots.len().saturating_sub(1));
        }
        HitTarget::ScreenField(_) => {
            st.zone = Zone::Add;
            let fr = field_rect(st.geo.add, &st.vm.labels, st.vm.other_pane, !st.vm.completions.is_empty());
            if ev.column >= fr.x && !fr.is_empty() {
                st.add_edit.click(&st.add_text, ev.column - fr.x, fr.width);
            }
        }
        HitTarget::ScreenChip(i) if i >= chip::COMPLETION => st.accept(usize::from(i - chip::COMPLETION)),
        HitTarget::ScreenChip(i) => {
            st.zone = Zone::Size;
            st.model.set_min(usize::from(i));
        }
        HitTarget::ScreenButton(button::FIND) if st.model.is_running() => st.model.stop_search(),
        HitTarget::ScreenButton(button::FIND) => st.find(),
        HitTarget::ScreenButton(button::ADD) => {
            st.add_typed();
        }
        HitTarget::ScreenButton(button::OTHER_PANE) => add_other(app),
        HitTarget::ScreenButton(b) if (button::KEEP..button::KEEP + 3).contains(&b) => {
            st.model.keep_all(Keep::ALL[usize::from(b - button::KEEP)]);
        }
        HitTarget::ScreenButton(button::CLEAR) => st.model.clear_marks(),
        HitTarget::ScreenButton(button::TRASH) => st.ask_trash(),
        HitTarget::ScreenButton(button::SETUP) => st.zone = Zone::Roots,
        HitTarget::DupesHeader(g) => {
            st.zone = Zone::List;
            st.fold(g, None);
        }
        HitTarget::DupesRow(line) => {
            st.zone = Zone::List;
            let len = lines(&st.model).len();
            st.list.select(line, len);
            match lines(&st.model).get(line).copied() {
                Some(Line::File(g, i)) if double => {
                    let path = st.model.groups[g].files[i].path.clone();
                    reveal(app, path);
                }
                Some(Line::File(g, i)) => {
                    st.model.toggle(g, i);
                }
                _ => {}
            }
        }
        HitTarget::ScreenArea(AREA_LIST) => st.zone = Zone::List,
        HitTarget::ConfirmBar(confirm::YES) => trash(app),
        HitTarget::ConfirmBar(_) => st.model.confirm = false,
        HitTarget::ScreenClose => super::close(app),
        _ => {}
    }
}

/// Every frame (`view::screens::prepare`): the search's results, finished trash jobs, the
/// field's completions; the cursors are clamped.
pub fn pump(core: &mut soyuz_appcore::core::AppCore, st: &mut State) {
    st.model.poll();
    st.model.poll_trash(&core.fileops.jobs);
    let len = lines(&st.model).len();
    let height = st.list_height();
    st.list.clamp(len, height);
    st.root_cursor = st.root_cursor.min(st.model.roots.len().saturating_sub(1));
    if st.zone == Zone::Add {
        let c = crumbs::completion(&mut core.services, &st.add_text);
        st.partial = !c.prefix.is_empty();
        if c.names != st.names {
            st.names = c.names;
            st.pick = 0;
        }
    } else {
        st.names.clear();
        st.partial = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_cycle_and_start_in_the_field_without_roots() {
        let mut st = State::new(Vec::new(), soyuz_appcore::host::no_wake());
        assert_eq!(st.zone, Zone::Add);
        st.move_zone(true);
        assert_eq!(st.zone, Zone::Size);
        st.move_zone(true);
        st.move_zone(true);
        assert_eq!(st.zone, Zone::Roots);
        st.move_zone(false);
        assert_eq!(st.zone, Zone::List);
        st.add_text = "  ".into();
        assert!(!st.add_typed());
        st.add_text = "/no/such/folder".into();
        assert!(st.add_typed(), "no disk check");
        assert!(st.add_text.is_empty());
        assert!(lines(&st.model).is_empty());
    }
}
