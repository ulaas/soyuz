//! Keys and mouse of the folder compare and sync screen, a full-screen overlay (Phase T5).
//!
//! The screen draws the core's [`CompareModel`] (`soyuz_appcore::models::compare`): the
//! same state machine as the GUI's compare window (setup → scanning → review → plan →
//! submitted → compared again). It opens for `ToolCmd::Compare` (Compare folders, Sync
//! folders: the two selected folders or both panes' folders) and `ToolCmd::CompareProfile`
//! from `crate::tools::drain`; `ToolCmd::SyncProfiles` opens it on its profiles page.
//! While it is open it owns the keyboard (`TuiFocus::Tool`) and the mouse.
//!
//! The focus moves between zones with Tab / Shift+Tab: the left and right folder fields
//! (with folder completion like the address bar), the method and the sync mode, then,
//! once a result shows, the name filter and the rows.
//!
//! | Key | Does |
//! |---|---|
//! | F5 / Ctrl+R | compare (again) |
//! | F6 | swap the sides |
//! | F8 | the sync profiles page (and back) |
//! | Esc | cancel a running scan; else back from the plan / profiles; else close |
//! | Enter (field) | compare; in the filter: to the rows |
//! | Left / Right, Space (method, mode) | pick |
//! | Up / Down (folder field) | pick a completion; Tab accepts it while a name is typed |
//! | Up / Down / PageUp / PageDown / Home / End (rows) | move |
//! | Space (rows) | cycle the row's action among the allowed ones |
//! | Left / Right / Delete / S (rows) | copy to the left / to the right, delete, skip |
//! | Shift + Left / Right / D / S (rows) | the same for every visible row |
//! | 1 – 5 (rows) | show / hide a category (only left, newer left, different, newer right, only right) |
//! | M / P (rows) | next method / next sync mode |
//! | C (rows) | resolve the two-way conflicts (`TuiDialog::CompareConflict`) |
//! | Enter (rows) | the conflicts while some are open, else review the plan |
//! | Enter (plan) | run it: one `JobKind::Sync` job |
//! | Enter / R / D / X (profiles) | load the profile and compare / run it now / dry run / cancel its run |
//! | N / S (profiles) | a new profile: blank / from the current setup (`TuiDialog::CompareProfile`) |
//! | E or F4 / F2 / Delete (profiles) | edit (folders, method, mode, schedule) / rename / delete the profile |
//!
//! Like the GUI's tool windows it is modal: other keys do not reach the keymap (Quit
//! still does).
//!
//! Mouse: a click focuses a field, picks a method / mode / completion, toggles a chip,
//! presses a button, selects a row (a click in the action column cycles the action) or a
//! profile; the wheel scrolls the rows, the plan and the profiles.
//!
//! The profiles page lists the saved sync profiles (`models::compare_profiles`) with their
//! schedule and next run or what they are doing; under the list the selected profile's
//! schedule, last run and run reports (loaded on a worker, `load_reports_async`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crossbeam_channel::Receiver;
use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Host;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::compare::{Category, CompareModel, Method, Preset, ReviewStep, RowAction, Stage};
use soyuz_appcore::models::compare_profiles::{self, RunReport, SyncProfile};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::state::crumbs;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, KeyChord, KeyName, Mods};

use super::{InputState, TuiFocus, key_context};
use crate::app::App;
use crate::input::dialogs::compare_profile as profile_dialog;
use crate::layout::HitTarget;
use crate::view::compare::{CompareVm, Geometry};
use crate::widgets::{EditOutcome, LineEdit, SelectList};

/// The compare model, shared with the conflicts dialog (`input::dialogs::compare_conflict`).
pub type Shared = Rc<RefCell<CompareModel>>;

/// The profile to select on the profiles page, set by the profile dialog
/// (`input::dialogs::compare_profile`) after a save or a rename.
pub type Select = Rc<RefCell<Option<String>>>;

/// Run reports listed for the selected profile at most.
pub const REPORTS_MAX: usize = 20;

/// While a scan, a sync job or a profile run is busy the screen redraws this often (the
/// elapsed time, the progress, the activity).
pub const BUSY_REDRAW: std::time::Duration = std::time::Duration::from_millis(500);

/// `HitTarget::CompareField` ids: the fields, then the options and completions.
pub mod field {
    pub const LEFT: u8 = 0;
    pub const RIGHT: u8 = 1;
    pub const METHOD: u8 = 2;
    pub const PRESET: u8 = 3;
    pub const FILTER: u8 = 4;
    /// `METHOD_OPTION + i`: option `i` of `Method::ALL`.
    pub const METHOD_OPTION: u8 = 10;
    /// `PRESET_OPTION + i`: option `i` of `Preset::ALL`.
    pub const PRESET_OPTION: u8 = 20;
    /// `COMPLETION + i`: completion `i` of the focused folder field.
    pub const COMPLETION: u8 = 30;
}

/// `HitTarget::CompareButton` ids.
pub mod button {
    pub const CLOSE: u8 = 0;
    /// Compare, or Cancel while a scan runs.
    pub const COMPARE: u8 = 1;
    pub const SWAP: u8 = 2;
    pub const PROFILES: u8 = 3;
    /// Review the plan.
    pub const REVIEW: u8 = 4;
    pub const CONFLICTS: u8 = 5;
    /// Back from the plan or the profiles page.
    pub const BACK: u8 = 6;
    /// Start the sync job.
    pub const RUN: u8 = 7;
    /// Profiles page: load into the setup and compare.
    pub const LOAD: u8 = 8;
    /// Profiles page: run now.
    pub const RUN_PROFILE: u8 = 9;
    /// Profiles page: a new profile (blank).
    pub const NEW: u8 = 10;
    /// Profiles page: edit the selected profile.
    pub const EDIT: u8 = 11;
    /// Profiles page: rename it.
    pub const RENAME: u8 = 12;
    /// Profiles page: delete it.
    pub const DELETE: u8 = 13;
    /// Profiles page: save the current setup as a profile.
    pub const SAVE_SETUP: u8 = 14;
    /// Profiles page: dry run.
    pub const DRY_RUN: u8 = 15;
    /// Profiles page: cancel the selected profile's run.
    pub const CANCEL_RUN: u8 = 16;
}

/// Where the keyboard is on the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zone {
    #[default]
    Left,
    Right,
    Method,
    Preset,
    Filter,
    Rows,
}

/// The screen's page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Compare,
    /// The saved sync profiles (run, load into the setup).
    Profiles,
}

/// What this part keeps between frames (`InputState::compare`).
#[derive(Default)]
pub struct State {
    /// The screen is open (it owns the keyboard: `TuiFocus::Tool`).
    pub open: bool,
    /// The compare (`None` while closed).
    pub model: Option<Shared>,
    pub zone: Zone,
    pub page: Page,
    pub left_edit: LineEdit,
    pub right_edit: LineEdit,
    pub filter_edit: LineEdit,
    /// First row shown of the table (an index into the visible rows).
    pub top: usize,
    /// First action shown of the plan.
    pub plan_top: usize,
    /// Cursor and scroll of the profiles page.
    pub profiles: SelectList,
    /// The profiles as last listed (`view::compare::prepare`).
    pub profile_list: Vec<SyncProfile>,
    /// The profile to select next frame (shared with the profile dialog).
    pub select: Select,
    /// The run reports of a profile (newest first) as last loaded.
    pub reports: Option<(String, Vec<RunReport>)>,
    /// Reports being loaded on a worker, for this profile.
    pub reports_rx: Option<(String, Receiver<Vec<RunReport>>)>,
    /// Completions of the focused folder field and the highlighted one.
    pub names: Vec<String>,
    pub pick: usize,
    /// A partial name is typed after the last separator (Tab accepts a completion).
    pub partial: bool,
    /// The rows were in review last frame (focus the rows when a result comes in).
    pub was_review: bool,
    /// The layout of the last frame.
    pub geo: Geometry,
    /// The model built by `view::compare::prepare`.
    pub vm: CompareVm,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("open", &self.open)
            .field("zone", &self.zone)
            .field("page", &self.page)
            .field("top", &self.top)
            .finish_non_exhaustive()
    }
}

impl State {
    /// The zones Tab visits now.
    fn zones(&self) -> Vec<Zone> {
        let mut z = vec![Zone::Left, Zone::Right, Zone::Method, Zone::Preset];
        if self.model.as_ref().is_some_and(|m| table_shown(&m.borrow())) {
            z.extend([Zone::Filter, Zone::Rows]);
        }
        z
    }

    fn move_zone(&mut self, forward: bool) {
        let z = self.zones();
        let at = z.iter().position(|x| *x == self.zone).unwrap_or(0);
        let n = z.len();
        self.zone = z[if forward { (at + 1) % n } else { (at + n - 1) % n }];
        self.names.clear();
        self.pick = 0;
    }
}

/// The review table shows (a result with rows, not the plan).
pub fn table_shown(m: &CompareModel) -> bool {
    matches!(m.stage(), Stage::Review) && m.step == ReviewStep::Table && !m.rows().is_empty()
}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(_core: &AppCore, st: &InputState) -> bool {
    st.compare.open
}

/// While it is open every mouse event is its own.
pub fn captures_mouse(_core: &AppCore, st: &InputState) -> bool {
    st.compare.open
}

fn new_state(model: CompareModel, page: Page) -> State {
    State {
        open: true,
        left_edit: LineEdit::at_end(&model.left),
        right_edit: LineEdit::at_end(&model.right),
        zone: if model.right.trim().is_empty() { Zone::Right } else { Zone::Left },
        model: Some(Rc::new(RefCell::new(model))),
        page,
        ..State::default()
    }
}

fn install(app: &mut App, st: State) {
    close(app);
    app.input.compare = st;
}

/// Open the compare screen on `left` and `right` (`ToolCmd::Compare`, from
/// `crate::tools::drain`); compares at once when `right` is given.
pub fn open(app: &mut App, left: PathBuf, right: Option<PathBuf>) {
    let m = CompareModel::new(left, right, compare_profiles::profiles(), app.host.wake());
    install(app, new_state(m, Page::Compare));
}

/// Open the compare screen with saved sync profile `name` loaded and compared
/// (`ToolCmd::CompareProfile`).
pub fn open_profile(app: &mut App, name: String) {
    let profiles = compare_profiles::profiles();
    match profiles.iter().find(|p| p.name == name).cloned() {
        Some(pr) => {
            let m = CompareModel::with_profile(&pr, profiles, app.host.wake());
            install(app, new_state(m, Page::Compare));
        }
        None => {
            app.core.ui.toasts.push(Toast::info(tr!("tools-toast-no-profile", name = name)));
        }
    }
}

/// The sync profiles (`ToolCmd::SyncProfiles`): the screen on its profiles page.
/// Editing and scheduling profiles is the profile dialog's (`TuiDialog::CompareProfile`).
pub fn open_profiles(app: &mut App) {
    if app.input.compare.open {
        app.input.compare.page = Page::Profiles;
        return;
    }
    let m = CompareModel::new(PathBuf::new(), None, compare_profiles::profiles(), app.host.wake());
    install(app, new_state(m, Page::Profiles));
}

/// Close the screen (a running scan stops).
pub fn close(app: &mut App) {
    let st = &mut app.input.compare;
    if let Some(m) = st.model.take() {
        m.borrow_mut().close();
    }
    *st = State::default();
}

/// This frame's events while it has the focus: its own keys first (in order), the rest to
/// the core's bindings in the dialog context.
pub fn route(app: &mut App, events: &[Event]) {
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    for i in 0..frame.events.len() {
        if !app.input.compare.open || app.focus() != TuiFocus::Tool {
            break;
        }
        if frame.consumed[i] {
            continue;
        }
        let ev = frame.events[i].clone();
        if key(app, &ev) {
            frame.consumed[i] = true;
        }
    }
    if app.input.compare.open && app.focus() == TuiFocus::Tool {
        // Modal like the GUI's tool windows: the keymap does not reach the panes under
        // it, except Quit.
        let (kctx, text) = key_context(TuiFocus::Tool);
        for i in 0..frame.events.len() {
            let quit = chord(&frame.events[i])
                .is_some_and(|c| app.core.keymap.lookup(kctx, c).is_some_and(|a| *a == Action::Quit));
            if !quit {
                frame.consumed[i] = true;
            }
        }
        app.core.input(&app.host, &mut frame, kctx, text);
    }
}

fn chord(ev: &InputEvent) -> Option<KeyChord> {
    match ev {
        InputEvent::Chord(c) => Some(*c),
        _ => None,
    }
}

fn plain(c: &KeyChord) -> bool {
    !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT)
}

fn shifted(c: &KeyChord) -> bool {
    c.mods.contains(Mods::SHIFT) && !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD)
}

/// One key; returns whether the screen used it.
fn key(app: &mut App, ev: &InputEvent) -> bool {
    let Some(model) = app.input.compare.model.clone() else {
        close(app);
        return true;
    };
    if let Some(c) = chord(ev) {
        let ctrl = c.mods.contains(Mods::CTRL) && !c.mods.intersects(Mods::ALT | Mods::SHIFT);
        match c.key {
            KeyName::F5 if plain(&c) => {
                compare(app, &model);
                return true;
            }
            KeyName::R if ctrl => {
                compare(app, &model);
                return true;
            }
            KeyName::F6 if plain(&c) => {
                swap(app, &model);
                return true;
            }
            KeyName::F8 if plain(&c) => {
                toggle_profiles(app);
                return true;
            }
            KeyName::Escape if plain(&c) => {
                escape(app, &model);
                return true;
            }
            _ => {}
        }
    }
    if app.input.compare.page == Page::Profiles {
        return profiles_key(app, &model, ev);
    }
    let step = model.borrow().step;
    if step == ReviewStep::Plan && matches!(model.borrow().stage(), Stage::Review) {
        return plan_key(app, &model, ev);
    }
    match app.input.compare.zone {
        Zone::Left | Zone::Right | Zone::Filter => field_key(app, &model, ev),
        Zone::Method | Zone::Preset => choice_key(app, &model, ev),
        Zone::Rows => rows_key(app, &model, ev),
    }
}

/// Compare (again) with what the fields say.
fn compare(app: &mut App, model: &Shared) {
    model.borrow_mut().start();
    let st = &mut app.input.compare;
    st.page = Page::Compare;
    st.top = 0;
    st.names.clear();
}

fn swap(app: &mut App, model: &Shared) {
    let mut m = model.borrow_mut();
    m.swap();
    let st = &mut app.input.compare;
    st.left_edit = LineEdit::at_end(&m.left);
    st.right_edit = LineEdit::at_end(&m.right);
}

fn toggle_profiles(app: &mut App) {
    let st = &mut app.input.compare;
    st.page = if st.page == Page::Profiles { Page::Compare } else { Page::Profiles };
}

/// Esc: cancel a scan, else step back, else close.
fn escape(app: &mut App, model: &Shared) {
    if model.borrow().is_scanning() {
        model.borrow_mut().cancel();
        return;
    }
    if app.input.compare.page == Page::Profiles {
        app.input.compare.page = Page::Compare;
        return;
    }
    let mut m = model.borrow_mut();
    if m.step != ReviewStep::Table {
        m.step = ReviewStep::Table;
        return;
    }
    drop(m);
    close(app);
}

/// The text of a field zone.
fn text_of(m: &mut CompareModel, z: Zone) -> &mut String {
    match z {
        Zone::Right => &mut m.right,
        Zone::Filter => &mut m.search,
        _ => &mut m.left,
    }
}

fn field_key(app: &mut App, model: &Shared, ev: &InputEvent) -> bool {
    let zone = app.input.compare.zone;
    let st = &mut app.input.compare;
    // Completions of a folder field.
    if zone != Zone::Filter
        && !st.names.is_empty()
        && let Some(c) = chord(ev)
        && plain(&c)
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
                accept(app, model, pick);
                return true;
            }
            _ => {}
        }
    }
    let out = {
        let mut m = model.borrow_mut();
        let edit = match zone {
            Zone::Right => &mut st.right_edit,
            Zone::Filter => &mut st.filter_edit,
            _ => &mut st.left_edit,
        };
        edit.apply(text_of(&mut m, zone), ev)
    };
    match out {
        EditOutcome::Consumed => {
            if zone == Zone::Filter {
                st.top = 0;
            }
            true
        }
        EditOutcome::Submit if zone == Zone::Filter => {
            st.zone = Zone::Rows;
            true
        }
        EditOutcome::Submit => {
            compare(app, model);
            true
        }
        EditOutcome::Cancel => {
            escape(app, model);
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
        EditOutcome::Ignored => false,
    }
}

/// Accept completion `i` into the focused folder field.
fn accept(app: &mut App, model: &Shared, i: usize) {
    let st = &mut app.input.compare;
    let Some(name) = st.names.get(i).cloned() else { return };
    let mut m = model.borrow_mut();
    let zone = st.zone;
    let text = text_of(&mut m, zone);
    crumbs::accept(text, &name);
    let edit = LineEdit::at_end(text);
    match zone {
        Zone::Right => st.right_edit = edit,
        _ => st.left_edit = edit,
    }
    st.names.clear();
    st.pick = 0;
}

fn next<T: Copy + PartialEq>(all: &[T], cur: T, forward: bool) -> T {
    let n = all.len();
    let at = all.iter().position(|x| *x == cur).unwrap_or(0);
    all[if forward { (at + 1) % n } else { (at + n - 1) % n }]
}

fn choice_key(app: &mut App, model: &Shared, ev: &InputEvent) -> bool {
    let Some(c) = chord(ev) else { return true };
    let st = &mut app.input.compare;
    let forward = match c.key {
        KeyName::Tab if plain(&c) => {
            st.move_zone(true);
            return true;
        }
        KeyName::Tab if shifted(&c) => {
            st.move_zone(false);
            return true;
        }
        KeyName::ArrowDown if plain(&c) => {
            st.move_zone(true);
            return true;
        }
        KeyName::ArrowUp if plain(&c) => {
            st.move_zone(false);
            return true;
        }
        KeyName::Enter if plain(&c) => {
            compare(app, model);
            return true;
        }
        KeyName::ArrowRight | KeyName::Space if plain(&c) => true,
        KeyName::ArrowLeft if plain(&c) => false,
        _ => return !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD),
    };
    let mut m = model.borrow_mut();
    if st.zone == Zone::Method {
        let v = next(&Method::ALL, m.method, forward);
        m.set_method(v);
    } else {
        let v = next(&Preset::ALL, m.preset, forward);
        m.set_preset(v);
    }
    true
}

/// The rows' cursor as a list (cursor in the model, scroll here).
fn rows_list(app: &App, m: &CompareModel) -> SelectList {
    SelectList { cursor: m.cursor, top: app.input.compare.top }
}

fn rows_key(app: &mut App, model: &Shared, ev: &InputEvent) -> bool {
    let Some(c) = chord(ev) else { return true };
    let vis = model.borrow().visible_rows();
    let height = usize::from(app.input.compare.geo.rows.height).max(1);
    {
        let mut m = model.borrow_mut();
        let mut list = rows_list(app, &m);
        if list.apply(ev, vis.len(), height) {
            m.cursor = list.cursor;
            app.input.compare.top = list.top;
            return true;
        }
    }
    let row = vis.get(model.borrow().cursor).copied();
    let set = |a: RowAction| {
        if let Some(i) = row {
            model.borrow_mut().set_action(i, a);
        }
    };
    let set_all = |a: RowAction| model.borrow_mut().set_actions(&vis, a);
    if shifted(&c) {
        match c.key {
            KeyName::ArrowLeft => set_all(RowAction::CopyLeft),
            KeyName::ArrowRight => set_all(RowAction::CopyRight),
            KeyName::D | KeyName::Delete => set_all(RowAction::Delete),
            KeyName::S => set_all(RowAction::Skip),
            KeyName::Tab => app.input.compare.move_zone(false),
            _ => return false,
        }
        return true;
    }
    if !plain(&c) {
        return false;
    }
    match c.key {
        KeyName::Tab => app.input.compare.move_zone(true),
        KeyName::Space => {
            if let Some(i) = row {
                model.borrow_mut().cycle_action(i);
            }
        }
        KeyName::ArrowLeft => set(RowAction::CopyLeft),
        KeyName::ArrowRight => set(RowAction::CopyRight),
        KeyName::Delete | KeyName::D => set(RowAction::Delete),
        KeyName::S | KeyName::Minus => set(RowAction::Skip),
        KeyName::Num1 | KeyName::Num2 | KeyName::Num3 | KeyName::Num4 | KeyName::Num5 => {
            let k = match c.key {
                KeyName::Num1 => 0,
                KeyName::Num2 => 1,
                KeyName::Num3 => 2,
                KeyName::Num4 => 3,
                _ => 4,
            };
            toggle_chip(app, model, k);
        }
        KeyName::M => {
            let mut m = model.borrow_mut();
            let v = next(&Method::ALL, m.method, true);
            m.set_method(v);
        }
        KeyName::P => {
            let mut m = model.borrow_mut();
            let v = next(&Preset::ALL, m.preset, true);
            m.set_preset(v);
        }
        KeyName::C => open_conflicts(app, model),
        KeyName::Enter => review(app, model),
        _ => {}
    }
    true
}

fn toggle_chip(app: &mut App, model: &Shared, k: usize) {
    if let Some(cat) = Category::ALL.get(k) {
        model.borrow_mut().toggle_filter(*cat);
        app.input.compare.top = 0;
    }
}

/// Enter on the table: the open conflicts first, then the plan.
fn review(app: &mut App, model: &Shared) {
    let (_, open) = model.borrow().conflict_counts();
    if open > 0 {
        open_conflicts(app, model);
        return;
    }
    let mut m = model.borrow_mut();
    if matches!(m.stage(), Stage::Review) && m.stats().total() > 0 {
        m.step = ReviewStep::Plan;
        app.input.compare.plan_top = 0;
    }
}

fn open_conflicts(app: &mut App, model: &Shared) {
    if model.borrow().conflict_counts().0 > 0 {
        crate::input::dialogs::compare_conflict::open(app, model.clone());
    }
}

fn run(app: &mut App, model: &Shared) {
    let job = model.borrow_mut().submit(&mut app.core);
    if job.is_some() {
        app.input.compare.zone = Zone::Left;
    }
}

fn plan_key(app: &mut App, model: &Shared, ev: &InputEvent) -> bool {
    let Some(c) = chord(ev) else { return true };
    if !plain(&c) {
        return false;
    }
    let len = model.borrow().plan().map_or(0, |p| p.actions.len());
    let height = usize::from(app.input.compare.geo.rows.height).max(1);
    let max = len.saturating_sub(height);
    let st = &mut app.input.compare;
    match c.key {
        KeyName::Enter => run(app, model),
        KeyName::Backspace | KeyName::ArrowLeft => model.borrow_mut().step = ReviewStep::Table,
        KeyName::ArrowUp => st.plan_top = st.plan_top.saturating_sub(1),
        KeyName::ArrowDown => st.plan_top = (st.plan_top + 1).min(max),
        KeyName::PageUp => st.plan_top = st.plan_top.saturating_sub(height),
        KeyName::PageDown => st.plan_top = (st.plan_top + height).min(max),
        KeyName::Home => st.plan_top = 0,
        KeyName::End => st.plan_top = max,
        _ => {}
    }
    true
}

fn profiles_key(app: &mut App, model: &Shared, ev: &InputEvent) -> bool {
    let Some(c) = chord(ev) else { return true };
    let len = app.input.compare.profile_list.len();
    let height = list_height(app);
    if app.input.compare.profiles.apply(ev, len, height) {
        return true;
    }
    if !plain(&c) {
        return false;
    }
    let i = app.input.compare.profiles.cursor;
    match c.key {
        KeyName::Enter => load_profile(app, model, i),
        KeyName::R => run_profile(app, i),
        KeyName::D => dry_run(app, i),
        KeyName::X => cancel_run(app, i),
        KeyName::N => new_profile(app),
        KeyName::S => save_setup(app, model),
        KeyName::E | KeyName::F4 => edit_profile(app, i),
        KeyName::F2 => rename_profile(app, i),
        KeyName::Delete => delete_profile(app, i),
        KeyName::Backspace => app.input.compare.page = Page::Compare,
        _ => {}
    }
    true
}

/// Rows of the profiles list (the details take the rest of the body).
fn list_height(app: &App) -> usize {
    usize::from(crate::view::compare::profile_split(app.input.compare.geo.rows).0.height).max(1)
}

/// A new, blank profile (`TuiDialog::CompareProfile`).
fn new_profile(app: &mut App) {
    let p = SyncProfile::new("", PathBuf::new(), PathBuf::new(), Preset::Update);
    let s = profile_dialog::State::new_profile(p, Some(app.input.compare.select.clone()));
    profile_dialog::open(app, s);
}

/// The compare screen's setup as a profile named after the loaded one (a hand-edited
/// table saves as its base mode).
pub fn setup_profile(m: &CompareModel) -> SyncProfile {
    let preset = match m.preset {
        Preset::Custom if m.two_way().is_some() => Preset::TwoWay,
        Preset::Custom => Preset::Update,
        other => other,
    };
    SyncProfile {
        name: m.profile_name.trim().to_string(),
        left: PathBuf::from(m.left.trim()),
        right: PathBuf::from(m.right.trim()),
        method: m.method,
        preset,
        schedule: Default::default(),
    }
}

/// Save the current setup: edits the loaded profile when it still exists (its schedule
/// stays), else a new profile prefilled with it.
fn save_setup(app: &mut App, model: &Shared) {
    let mut p = setup_profile(&model.borrow());
    let select = Some(app.input.compare.select.clone());
    let s = match compare_profiles::profile(&p.name) {
        Some(old) => {
            p.schedule = old.schedule;
            profile_dialog::State::edit(&old.name, &p, select)
        }
        None => profile_dialog::State::new_profile(p, select),
    };
    profile_dialog::open(app, s);
}

fn edit_profile(app: &mut App, i: usize) {
    let Some(p) = app.input.compare.profile_list.get(i).cloned() else { return };
    let s = profile_dialog::State::edit(&p.name, &p, Some(app.input.compare.select.clone()));
    profile_dialog::open(app, s);
}

fn rename_profile(app: &mut App, i: usize) {
    let Some(name) = app.input.compare.profile_list.get(i).map(|p| p.name.clone()) else { return };
    let s = profile_dialog::State::rename(&name, Some(app.input.compare.select.clone()));
    profile_dialog::open(app, s);
}

fn delete_profile(app: &mut App, i: usize) {
    let Some(name) = app.input.compare.profile_list.get(i).map(|p| p.name.clone()) else { return };
    profile_dialog::open(app, profile_dialog::State::delete(&name));
}

/// A dry run of profile `i`: only its report is written.
fn dry_run(app: &mut App, i: usize) {
    let Some(pr) = app.input.compare.profile_list.get(i) else { return };
    if compare_profiles::is_running(&pr.name) {
        return;
    }
    compare_profiles::run_now(&pr.name, true);
    app.core.ui.toasts.push(Toast::info(tr!("tui-cmpprof-toast-dry-run", name = pr.name.clone())));
}

/// Cancel the planning or the sync job of profile `i`.
fn cancel_run(app: &mut App, i: usize) {
    let Some(name) = app.input.compare.profile_list.get(i).map(|p| p.name.clone()) else { return };
    if compare_profiles::is_running(&name) {
        compare_profiles::cancel(&app.core, &name);
        app.core.ui.toasts.push(Toast::info(tr!("tui-cmpprof-toast-cancelled", name = name)));
    }
}

/// Load profile `i` into the setup and compare.
fn load_profile(app: &mut App, model: &Shared, i: usize) {
    let Some(pr) = app.input.compare.profile_list.get(i).cloned() else { return };
    let mut m = model.borrow_mut();
    m.load_profile(pr);
    let st = &mut app.input.compare;
    st.left_edit = LineEdit::at_end(&m.left);
    st.right_edit = LineEdit::at_end(&m.right);
    st.page = Page::Compare;
    st.zone = Zone::Left;
    st.top = 0;
}

/// Run profile `i` now (the scheduler runs it as a job, like the GUI's "Run now").
fn run_profile(app: &mut App, i: usize) {
    let Some(pr) = app.input.compare.profile_list.get(i) else { return };
    if compare_profiles::is_running(&pr.name) {
        return;
    }
    compare_profiles::run_now(&pr.name, false);
    app.core.ui.toasts.push(Toast::info(tr!("tui-compare-profile-started", name = pr.name.clone())));
}

/// A mouse event over one of its click targets (or anywhere while it is open); returns
/// whether it was taken.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    if !app.input.compare.open {
        return false;
    }
    let Some(model) = app.input.compare.model.clone() else { return true };
    match ev.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let rows = if ev.kind == MouseEventKind::ScrollDown {
                super::mouse::WHEEL_ROWS
            } else {
                -super::mouse::WHEEL_ROWS
            };
            wheel(app, &model, rows);
        }
        MouseEventKind::Down(MouseButton::Left) => click(app, &model, target, ev),
        _ => {}
    }
    true
}

fn wheel(app: &mut App, model: &Shared, rows: i32) {
    let height = usize::from(app.input.compare.geo.rows.height).max(1);
    if app.input.compare.page == Page::Profiles {
        let len = app.input.compare.profile_list.len();
        let height = list_height(app);
        app.input.compare.profiles.wheel(rows, len, height);
        return;
    }
    let m = model.borrow();
    if m.step == ReviewStep::Plan {
        let len = m.plan().map_or(0, |p| p.actions.len());
        let mut l = SelectList { cursor: 0, top: app.input.compare.plan_top };
        l.wheel(rows, len, height);
        app.input.compare.plan_top = l.top;
    } else {
        let len = m.visible_rows().len();
        let mut l = rows_list(app, &m);
        l.wheel(rows, len, height);
        app.input.compare.top = l.top;
    }
}

fn click(app: &mut App, model: &Shared, target: HitTarget, ev: &MouseEvent) {
    match target {
        HitTarget::CompareField(f) => click_field(app, model, f),
        HitTarget::CompareChip(k) => toggle_chip(app, model, usize::from(k)),
        HitTarget::CompareRow(k) => {
            let st = &mut app.input.compare;
            st.zone = Zone::Rows;
            let vis = model.borrow().visible_rows();
            if let Some(&i) = vis.get(k) {
                model.borrow_mut().cursor = k;
                let g = &st.geo;
                if ev.column >= g.act_x && ev.column < g.act_x.saturating_add(g.act_w) {
                    model.borrow_mut().cycle_action(i);
                }
            }
        }
        HitTarget::CompareProfile(i) => {
            let st = &mut app.input.compare;
            let again = st.profiles.cursor == i;
            st.profiles.select(i, st.profile_list.len());
            if again {
                load_profile(app, model, i);
            }
        }
        HitTarget::CompareButton(b) => press(app, model, b),
        _ => {}
    }
}

fn click_field(app: &mut App, model: &Shared, f: u8) {
    let st = &mut app.input.compare;
    st.page = Page::Compare;
    match f {
        field::LEFT => st.zone = Zone::Left,
        field::RIGHT => st.zone = Zone::Right,
        field::METHOD => st.zone = Zone::Method,
        field::PRESET => st.zone = Zone::Preset,
        field::FILTER => st.zone = Zone::Filter,
        f if (field::METHOD_OPTION..field::PRESET_OPTION).contains(&f) => {
            st.zone = Zone::Method;
            if let Some(m) = Method::ALL.get(usize::from(f - field::METHOD_OPTION)) {
                model.borrow_mut().set_method(*m);
            }
        }
        f if (field::PRESET_OPTION..field::COMPLETION).contains(&f) => {
            st.zone = Zone::Preset;
            if let Some(p) = Preset::ALL.get(usize::from(f - field::PRESET_OPTION)) {
                model.borrow_mut().set_preset(*p);
            }
        }
        f if f >= field::COMPLETION => accept(app, model, usize::from(f - field::COMPLETION)),
        _ => {}
    }
}

fn press(app: &mut App, model: &Shared, b: u8) {
    match b {
        button::CLOSE => close(app),
        button::COMPARE if model.borrow().is_scanning() => model.borrow_mut().cancel(),
        button::COMPARE => compare(app, model),
        button::SWAP => swap(app, model),
        button::PROFILES => toggle_profiles(app),
        button::REVIEW => review(app, model),
        button::CONFLICTS => open_conflicts(app, model),
        button::BACK => {
            app.input.compare.page = Page::Compare;
            model.borrow_mut().step = ReviewStep::Table;
        }
        button::RUN => run(app, model),
        button::LOAD => {
            let i = app.input.compare.profiles.cursor;
            load_profile(app, model, i);
        }
        button::RUN_PROFILE => {
            let i = app.input.compare.profiles.cursor;
            run_profile(app, i);
        }
        button::NEW => new_profile(app),
        button::SAVE_SETUP => save_setup(app, model),
        b @ (button::EDIT | button::RENAME | button::DELETE | button::DRY_RUN | button::CANCEL_RUN) => {
            let i = app.input.compare.profiles.cursor;
            match b {
                button::EDIT => edit_profile(app, i),
                button::RENAME => rename_profile(app, i),
                button::DELETE => delete_profile(app, i),
                button::DRY_RUN => dry_run(app, i),
                _ => cancel_run(app, i),
            }
        }
        _ => {}
    }
}

/// Every frame (`view::compare::prepare`): the scan result, the job's end, the completions
/// of the focused folder field; the cursor is clamped to the visible rows.
pub fn pump(core: &mut AppCore, st: &mut State) {
    let Some(model) = st.model.clone() else { return };
    let mut m = model.borrow_mut();
    m.poll(core);
    let review = table_shown(&m);
    if review && !st.was_review && st.zone != Zone::Filter {
        st.zone = Zone::Rows;
        st.top = 0;
    }
    if !review && matches!(st.zone, Zone::Rows | Zone::Filter) {
        st.zone = Zone::Left;
    }
    st.was_review = review;
    let len = m.visible_rows().len();
    m.cursor = m.cursor.min(len.saturating_sub(1));
    // Folder completion while a folder field has the keyboard.
    if st.page == Page::Compare && matches!(st.zone, Zone::Left | Zone::Right) {
        let text = if st.zone == Zone::Left { &m.left } else { &m.right };
        let c = crumbs::completion(&mut core.services, text);
        st.partial = !c.prefix.is_empty();
        if c.names != st.names {
            st.names = c.names;
            st.pick = 0;
        }
    } else {
        st.names.clear();
        st.partial = false;
    }
    drop(m);
    pump_profiles(st);
}

/// The profiles page: the list, a selection asked for by the profile dialog, and the
/// selected profile's run reports (loaded on a worker; again when a newer run ended).
fn pump_profiles(st: &mut State) {
    if st.page != Page::Profiles {
        st.profile_list.clear();
        return;
    }
    st.profile_list = compare_profiles::profiles();
    let n = st.profile_list.len();
    let height = usize::from(crate::view::compare::profile_split(st.geo.rows).0.height).max(1);
    let wanted = st.select.borrow_mut().take();
    if let Some(name) = wanted
        && let Some(i) = st.profile_list.iter().position(|p| p.name == name)
    {
        st.profiles.select(i, n);
        st.profiles.show_cursor(height);
    }
    st.profiles.clamp(n, height);
    let Some(name) = st.profile_list.get(st.profiles.cursor).map(|p| p.name.clone()) else {
        st.reports = None;
        st.reports_rx = None;
        return;
    };
    if let Some((for_name, rx)) = &st.reports_rx
        && let Ok(v) = rx.try_recv()
    {
        st.reports = Some((for_name.clone(), v));
        st.reports_rx = None;
    }
    let loaded = st.reports.as_ref().filter(|(n, _)| *n == name);
    let loading = st.reports_rx.as_ref().is_some_and(|(n, _)| *n == name);
    let newer = loaded.is_some_and(|(_, v)| {
        compare_profiles::last_report(&name).is_some_and(|last| v.first().is_none_or(|f| f.started < last.started))
    });
    if !loading && (loaded.is_none() || newer) {
        st.reports_rx = Some((name.clone(), compare_profiles::load_reports_async(&name, REPORTS_MAX)));
    }
}

/// Every frame (`App::frame`): while the screen shows something busy (a scan, a sync
/// job, a profile run on the profiles page) a redraw in [`BUSY_REDRAW`].
pub fn tick(st: &State, host: &dyn Host) {
    if !st.open {
        return;
    }
    let busy = st.model.as_ref().is_some_and(|m| {
        let m = m.borrow();
        m.is_scanning() || matches!(m.stage(), Stage::Submitted(_) | Stage::Recording(_))
    });
    let running = st.page == Page::Profiles && st.profile_list.iter().any(|p| compare_profiles::is_running(&p.name));
    if busy || running || st.reports_rx.is_some() {
        host.redraw_after(BUSY_REDRAW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_wraps_both_ways() {
        assert_eq!(next(&Method::ALL, Method::Content, true), Method::DateSize);
        assert_eq!(next(&Method::ALL, Method::DateSize, false), Method::Content);
        // Custom (not in the list) starts from the first.
        assert_eq!(next(&Preset::ALL, Preset::Custom, true), Preset::ALL[1]);
    }

    #[test]
    fn zones_follow_the_result() {
        let mut st = State::default();
        assert_eq!(st.zones(), vec![Zone::Left, Zone::Right, Zone::Method, Zone::Preset]);
        st.zone = Zone::Preset;
        st.move_zone(true);
        assert_eq!(st.zone, Zone::Left);
        st.move_zone(false);
        assert_eq!(st.zone, Zone::Preset);
    }
}
