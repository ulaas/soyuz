//! Keys and mouse of the search bar and its status row (Phase T5), the terminal twin of
//! the GUI's bar (`soyuz-app/src/ui/search.rs`). The state and the logic live in the core
//! (`soyuz_appcore::models::search`); this module moves a focus ring over the bar.
//!
//! The bar opens when the core sets `ui.search.focus` (Search, SearchContent, Refine):
//! [`arm`] gives it the keyboard with the cursor at the end of the query. Focus stops, in
//! Tab order: the query field, the "in files" glob field (content search only), the pills
//! ([`pill`]: mode, case, contents, size, date, kind, hidden, .gitignore, scope) and the
//! buttons (Run / Stop, Save, close).
//!
//! Keys (after the core's `KeyContext::Omnibar` bindings, like the address bar):
//! - in a field: typing edits it; Enter runs the search (the bar keeps the keyboard); Esc
//!   closes the bar; Down gives the keyboard to the results (the bar stays; Ctrl+Shift+F /
//!   F3 or a click come back);
//! - on a pill: Space / Enter toggle it or pick the next option (Shift: the previous one),
//!   the size pill opens the size dialog; Left / Right move between pills and buttons;
//! - anywhere: Tab / Shift+Tab move, Ctrl+S saves the search (the name dialog).
//!
//! An option change re-runs the search on screen (`models::search::submit`). Mouse: a
//! click on the field gives it the keyboard; on a pill toggles / cycles it (right click and
//! the wheel cycle backwards / both ways); on a button presses it; on a status-row button
//! (Cancel, Relevance, Edit / Smart folder, Refine) presses it. A click anywhere else while
//! the bar has the keyboard takes it away.

use std::time::Duration;

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::host::Host as _;
use soyuz_appcore::keys::input::{InputEvent, InputFrame};
use soyuz_appcore::models::search::{self as model, StatusAct};
use soyuz_appcore::services::search as svc;
use soyuz_core::config::{KeyName, Mods};

use super::{InputState, TuiFocus, core_input, key_context};
use crate::app::App;
use crate::hits::SearchButton;
use crate::layout::HitTarget;
use crate::widgets::line_edit::{EditOutcome, LineEdit};

/// Pill ids ([`HitTarget::SearchPill`]), in drawing and Tab order.
pub mod pill {
    /// Name mode (contains / glob / regex / fuzzy); in a content search the regex toggle.
    pub const MODE: u8 = 0;
    pub const CASE: u8 = 1;
    pub const CONTENT: u8 = 2;
    /// Opens the size dialog.
    pub const SIZE: u8 = 3;
    pub const DATE: u8 = 4;
    pub const KIND: u8 = 5;
    pub const HIDDEN: u8 = 6;
    pub const GITIGNORE: u8 = 7;
    /// The scope: this folder or every indexed folder.
    pub const EVERYWHERE: u8 = 8;
    /// How many pills there are.
    pub const COUNT: u8 = 9;
    /// Not a pill: the "in files" glob field of a content search (its click target).
    pub const GLOB: u8 = 9;
}

/// Status-row button ids ([`HitTarget::SearchStatusButton`]).
pub mod status_button {
    pub const CANCEL: u8 = 0;
    pub const RELEVANCE: u8 = 1;
    pub const EDIT_SMART: u8 = 2;
    pub const MAKE_SMART: u8 = 3;
    pub const REFINE: u8 = 4;
}

/// A focus stop of the bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stop {
    /// The query field.
    #[default]
    Field,
    /// The "in files" glob field (content search).
    Glob,
    Pill(u8),
    Button(SearchButton),
}

/// What the bar keeps between frames (`InputState::search`).
#[derive(Debug, Default)]
pub struct State {
    /// The bar owns the keyboard (armed by the core's `ui.search.focus`).
    pub editing: bool,
    /// Cursor and scroll of the query field.
    pub edit: LineEdit,
    /// Cursor and scroll of the glob field.
    pub glob: LineEdit,
    /// The focused stop.
    pub stop: Stop,
}

/// Redraw this often while the search on screen runs (elapsed time, current folder).
const REDRAW_EVERY: Duration = Duration::from_millis(250);

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    core.ui.search.open && st.search.editing
}

/// The stops in Tab order for the current form.
fn stops(core: &AppCore) -> Vec<Stop> {
    let mut v = vec![Stop::Field];
    if core.ui.search.form.content {
        v.push(Stop::Glob);
    }
    v.extend((0..pill::COUNT).map(Stop::Pill));
    v.extend([SearchButton::Run, SearchButton::Save, SearchButton::Close].map(Stop::Button));
    v
}

fn move_stop(app: &mut App, forward: bool) {
    let all = stops(&app.core);
    let n = all.len();
    let i = all.iter().position(|s| *s == app.input.search.stop).unwrap_or(0);
    let next = all[if forward { (i + 1) % n } else { (i + n - 1) % n }];
    focus_stop(app, next);
}

/// Focus `stop` (a field gets its cursor at the end).
fn focus_stop(app: &mut App, stop: Stop) {
    let st = &mut app.input.search;
    st.stop = stop;
    match stop {
        Stop::Field => st.edit = LineEdit::at_end(&app.core.ui.search.form.text),
        Stop::Glob => st.glob = LineEdit::at_end(&app.core.ui.search.form.name_glob),
        _ => {}
    }
}

/// Give the bar the keyboard on `stop`.
fn begin(app: &mut App, stop: Stop) {
    app.input.search.editing = true;
    focus_stop(app, stop);
}

/// Close the bar (the results stay) and give the keyboard to the list.
pub fn close(app: &mut App) {
    app.input.search.editing = false;
    app.input.search.stop = Stop::Field;
    model::close(&mut app.core);
}

/// Keep the bar open but give the keyboard to the list.
fn leave(app: &mut App) {
    app.input.search.editing = false;
    app.core.ui.focus = Focus::List;
}

/// This frame's events while it has the focus.
pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::Search);
    let mut frame = core_input(app, events, kctx, text);
    // The core's input may have closed the bar or opened something over it.
    if !has_focus(&app.core, &app.input) || super::focus(&app.core, &app.input) != TuiFocus::Search {
        return;
    }
    if take_ctrl_s(&mut frame) {
        crate::input::dialogs::save_search::open(app);
        return;
    }
    if !stops(&app.core).contains(&app.input.search.stop) {
        app.input.search.stop = Stop::Field;
    }
    match app.input.search.stop {
        Stop::Field | Stop::Glob => field_keys(app, &mut frame),
        Stop::Pill(_) | Stop::Button(_) => control_keys(app, &frame),
    }
}

/// Consume a live Ctrl+S (Save), if any.
fn take_ctrl_s(frame: &mut InputFrame) -> bool {
    let found = frame.live().find(|&i| {
        matches!(&frame.events[i], InputEvent::Chord(c)
            if c.key == KeyName::S && c.mods.intersects(Mods::CTRL | Mods::CMD) && !c.mods.intersects(Mods::ALT | Mods::SHIFT))
    });
    if let Some(i) = found {
        frame.consumed[i] = true;
    }
    found.is_some()
}

fn field_keys(app: &mut App, frame: &mut InputFrame) {
    let glob = app.input.search.stop == Stop::Glob;
    let form = &mut app.core.ui.search.form;
    let (text, edit) = if glob {
        (&mut form.name_glob, &mut app.input.search.glob)
    } else {
        (&mut form.text, &mut app.input.search.edit)
    };
    match edit.feed(text, frame) {
        // The bar keeps the keyboard (Down goes to the results).
        EditOutcome::Submit => model::submit(&mut app.core, false, true, false),
        EditOutcome::Cancel => close(app),
        EditOutcome::Tab => move_stop(app, true),
        EditOutcome::ShiftTab => move_stop(app, false),
        EditOutcome::Down => leave(app),
        _ => {}
    }
}

fn control_keys(app: &mut App, frame: &InputFrame) {
    for i in frame.live() {
        let InputEvent::Chord(c) = &frame.events[i] else { continue };
        if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) {
            continue;
        }
        let shift = c.mods.contains(Mods::SHIFT);
        match c.key {
            KeyName::Tab => move_stop(app, !shift),
            KeyName::ArrowRight => move_stop(app, true),
            KeyName::ArrowLeft => move_stop(app, false),
            KeyName::Escape => {
                close(app);
                return;
            }
            KeyName::ArrowDown => {
                leave(app);
                return;
            }
            KeyName::Enter | KeyName::Space => match app.input.search.stop {
                Stop::Pill(p) => activate(app, p, !shift),
                Stop::Button(b) => {
                    press(app, b);
                    return;
                }
                _ => {}
            },
            _ => {}
        }
        if !has_focus(&app.core, &app.input) {
            return;
        }
    }
}

/// Toggle pill `p`, or pick its next (`forward`) or previous option; the size pill opens
/// the size dialog. A changed option re-runs the search on screen.
pub fn activate(app: &mut App, p: u8, forward: bool) {
    let here = model::current_dir(&app.core);
    let form = &mut app.core.ui.search.form;
    let changed = match p {
        pill::MODE if form.content => {
            form.content_regex = !form.content_regex;
            true
        }
        pill::MODE => {
            model::cycle_mode(form, forward);
            true
        }
        pill::CASE => {
            form.case_sensitive = !form.case_sensitive;
            true
        }
        pill::CONTENT => {
            // Like the GUI: the query changes meaning, so nothing re-runs.
            form.content = !form.content;
            false
        }
        pill::SIZE => {
            crate::input::dialogs::search_size::open(app);
            return;
        }
        pill::DATE => {
            model::cycle_date(form, forward);
            true
        }
        pill::KIND => {
            model::cycle_kind(form, forward);
            true
        }
        pill::HIDDEN => {
            form.include_hidden = !form.include_hidden;
            true
        }
        pill::GITIGNORE => {
            form.respect_gitignore = !form.respect_gitignore;
            true
        }
        pill::EVERYWHERE => {
            form.everywhere = !form.everywhere;
            if !form.everywhere
                && let Some(d) = here
            {
                form.root = d;
            }
            true
        }
        _ => false,
    };
    model::submit(&mut app.core, false, false, changed);
}

/// Press bar button `b`.
pub fn press(app: &mut App, b: SearchButton) {
    match b {
        SearchButton::Run => {
            let running = model::shown_search(&app.core).is_some_and(svc::is_running);
            model::submit(&mut app.core, running, true, false);
        }
        SearchButton::Save => crate::input::dialogs::save_search::open(app),
        SearchButton::Close => close(app),
    }
}

/// Press status-row button `i` ([`status_button`]).
fn press_status(app: &mut App, i: u8) {
    let Some(id) = model::shown_search(&app.core) else { return };
    let act = match i {
        status_button::CANCEL => StatusAct::Cancel,
        status_button::RELEVANCE => StatusAct::Relevance,
        status_button::EDIT_SMART => StatusAct::EditSmart,
        status_button::MAKE_SMART => StatusAct::MakeSmart,
        status_button::REFINE => StatusAct::Refine,
        _ => return,
    };
    model::status_act(&mut app.core, id, act);
}

fn is_search_target(t: HitTarget) -> bool {
    matches!(
        t,
        HitTarget::SearchField
            | HitTarget::SearchPill(_)
            | HitTarget::SearchButton(_)
            | HitTarget::SearchStatusButton(_)
    )
}

/// A mouse event over one of its click targets (`input::mouse` dispatches by target);
/// returns whether it was taken. While the bar has the keyboard it sees every event: a
/// press elsewhere takes the keyboard away and goes on to its target.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    if !is_search_target(target) {
        if matches!(ev.kind, MouseEventKind::Down(_)) {
            app.input.search.editing = false;
        }
        return false;
    }
    let cycles = |p: u8| matches!(p, pill::MODE | pill::DATE | pill::KIND);
    match (target, ev.kind) {
        (HitTarget::SearchField, MouseEventKind::Down(MouseButton::Left)) => begin(app, Stop::Field),
        (HitTarget::SearchPill(pill::GLOB), MouseEventKind::Down(MouseButton::Left)) => begin(app, Stop::Glob),
        (HitTarget::SearchPill(p), MouseEventKind::Down(b @ (MouseButton::Left | MouseButton::Right))) => {
            if app.input.search.editing {
                app.input.search.stop = Stop::Pill(p);
            }
            if b == MouseButton::Left || cycles(p) {
                activate(app, p, b == MouseButton::Left);
            }
        }
        (HitTarget::SearchPill(p), k @ (MouseEventKind::ScrollDown | MouseEventKind::ScrollUp)) if cycles(p) => {
            activate(app, p, k == MouseEventKind::ScrollDown);
        }
        (HitTarget::SearchButton(b), MouseEventKind::Down(MouseButton::Left)) => press(app, b),
        (HitTarget::SearchStatusButton(i), MouseEventKind::Down(MouseButton::Left)) => press_status(app, i),
        _ => {}
    }
    true
}

/// While `true`, every mouse event goes to [`on_mouse`] wherever it is: while the bar has
/// the keyboard, so a click elsewhere can take it away.
pub fn captures_mouse(core: &AppCore, st: &InputState) -> bool {
    has_focus(core, st)
}

/// Consume the core's request to focus the search bar (`ui.search.focus`, set by
/// `Search` / `SearchContent` / Refine), like `filter::arm`. Called every frame before
/// the routing; also asks for redraws while the search on screen runs.
pub fn arm(app: &mut App) {
    if app.core.ui.search.focus {
        app.core.ui.search.focus = false;
        if app.core.ui.search.open {
            begin(app, Stop::Field);
        }
    }
    if !app.core.ui.search.open {
        app.input.search.editing = false;
    }
    if model::shown_search(&app.core).is_some_and(svc::is_running) {
        app.host.redraw_after(REDRAW_EVERY);
    }
}
