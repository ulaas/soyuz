//! Keys and mouse of the jobs panel (`TuiFocus::Jobs`) — Phase T3.
//!
//! The panel opens by itself when a job runs (`view::jobs`) without taking the keyboard;
//! `FocusActivityPanel` (Alt+J, [`focus`]) opens it and gives it the keyboard, a click on
//! it does too. While it has the keyboard:
//!
//! | Key | Jobs tab | History tab |
//! |---|---|---|
//! | Up / Down / PageUp / PageDown / Home / End | move the cursor | |
//! | Left / Right, Tab / Shift+Tab | switch tabs | |
//! | Space, P | pause / resume | |
//! | Delete, C | cancel (finished: remove from the list) | |
//! | Enter | answer the job's deferred prompt | undo / redo |
//! | R | retry the failed items | retry the failed items |
//! | U | | undo (redo when undone) |
//! | L | show / hide the job's details (errors and log) | |
//! | M | the job's "More" menu: priority, move up / down, start now, cancel | |
//! | Shift+P / Shift+S / Shift+C | pause all / resume (start) all / cancel all | |
//! | X | clear the finished jobs | |
//! | Esc | give the keyboard back to the file list | |
//!
//! Other keys go to the core's global bindings (Ctrl+Q, the palette, …). A mouse press
//! outside the panel gives the keyboard back too.
//!
//! Mouse ([`click`], [`wheel`]): `BottomTab` switches tabs, `JobRow` moves the cursor,
//! `JobButton` presses the button (`Close` closes the panel; the toolbar's act on every
//! job; `Details` shows / hides the log; `More` opens the menu); a right click on a job
//! opens its menu; each press focuses the panel. The wheel scrolls the list without
//! moving the cursor. While jobs run and the panel is open, [`tick`] asks for a redraw
//! every [`REDRAW_EVERY`] (progress, speed and ETA move).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Host;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::jobs_rows::{self, HistoryRow, JobMenuEntry, JobRow, RowButton, Tone, ToolbarButton};
use soyuz_appcore::models::panels::ActivityTab;
use soyuz_core::config::{KeyName, Mods};
use soyuz_jobs::JobId;

use super::InputState;
use super::menu::MenuCmd;
use crate::app::App;
use crate::hits::JobButton;
use crate::layout::HitTarget;
use crate::widgets::{Menu, MenuItem, SelectList};

/// How often the panel is redrawn while jobs run.
pub const REDRAW_EVERY: Duration = Duration::from_millis(250);

/// What the jobs panel keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The panel has the keyboard.
    pub focused: bool,
    /// Cursor and scroll of the list (rows of the active tab).
    pub list: SelectList,
    /// The tab the list belongs to (the history tab), to reset it on a switch.
    pub history: bool,
    /// Rows that fit (set by `view::jobs::prepare`).
    pub height: usize,
    /// Bring the cursor into view on the next frame.
    pub follow: bool,
    /// The details of the expanded jobs (read by `view::jobs::prepare`).
    pub details: HashMap<JobId, Vec<(Tone, String)>>,
    /// Screen-reader mode (T8): the progress text each job shows, throttled ([`due`];
    /// kept by `view::jobs::prepare`, empty outside screen-reader mode).
    pub spoken: HashMap<JobId, Spoken>,
}

/// Screen-reader mode: a job's progress text changes at most once per [`SPOKEN_STEP`]
/// of progress or every [`SPOKEN_EVERY`] ([`due`]), so a reader that speaks changed text
/// is not flooded. The core's rows (`jobs_rows`, shared with the GUI) stay live.
pub const SPOKEN_EVERY: Duration = Duration::from_secs(2);
/// Progress, in thousandths, that changes the text at once (10 percent).
pub const SPOKEN_STEP: u16 = 100;

/// A job's progress text as screen-reader mode shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spoken {
    /// When it was taken.
    pub at: Instant,
    /// The job's state (running, paused, waiting, failed, done, its buttons): a change
    /// shows at once.
    pub state: String,
    /// Done, in thousandths.
    pub permille: Option<u16>,
    /// The state line, the counts and the speed / time left.
    pub line: String,
    pub stats: String,
    pub right: String,
}

/// Whether the shown progress text `last` gives way to the live one (`state`,
/// `permille`) at `now`: nothing shown yet, another state, another 10 percent step, or
/// [`SPOKEN_EVERY`] passed.
pub fn due(last: Option<&Spoken>, state: &str, permille: Option<u16>, now: Instant) -> bool {
    let Some(last) = last else { return true };
    let step = |p: Option<u16>| p.map(|p| p / SPOKEN_STEP);
    last.state != state
        || step(last.permille) != step(permille)
        || now.saturating_duration_since(last.at) >= SPOKEN_EVERY
}

/// Whether the panel has the keyboard (it must be shown).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    st.jobs.focused && crate::view::jobs::shown(core)
}

/// `FocusActivityPanel`: open the panel and give it the keyboard.
pub fn focus(app: &mut App) {
    if !crate::view::jobs::shown(&app.core) {
        app.core.ui.bottom.open_activity();
    }
    app.input.jobs.focused = true;
    app.input.jobs.follow = true;
}

/// Every frame (`App::frame`): while the panel is open and a job is working, a
/// redraw in [`REDRAW_EVERY`].
pub fn tick(core: &AppCore, host: &dyn Host) {
    if crate::view::jobs::shown(core) && core.fileops.jobs.snapshots().iter().any(|s| s.state.is_active()) {
        host.redraw_after(REDRAW_EVERY);
    }
}

/// What a key does in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCmd {
    Leave,
    Tab(bool),
    /// Pause or resume.
    PauseResume,
    /// Cancel or remove.
    CancelRemove,
    Enter,
    Retry,
    UndoRedo,
    /// Show / hide the details.
    Log,
    /// The "More" menu.
    Menu,
    /// A toolbar button.
    All(ToolbarButton),
}

/// The panel's command for chord `ev` (`None`: not one of its keys).
pub fn key_cmd(ev: &InputEvent) -> Option<KeyCmd> {
    let InputEvent::Chord(c) = ev else { return None };
    if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) {
        return None;
    }
    let shift = c.mods.contains(Mods::SHIFT);
    Some(match (c.key, shift) {
        (KeyName::Escape, false) => KeyCmd::Leave,
        (KeyName::ArrowLeft, false) => KeyCmd::Tab(false),
        (KeyName::ArrowRight, false) => KeyCmd::Tab(true),
        (KeyName::Tab, s) => KeyCmd::Tab(!s),
        (KeyName::Space | KeyName::P, false) => KeyCmd::PauseResume,
        (KeyName::Delete | KeyName::C, false) => KeyCmd::CancelRemove,
        (KeyName::Enter, false) => KeyCmd::Enter,
        (KeyName::R, false) => KeyCmd::Retry,
        (KeyName::U, false) => KeyCmd::UndoRedo,
        (KeyName::L, false) => KeyCmd::Log,
        (KeyName::M, false) => KeyCmd::Menu,
        (KeyName::P, true) => KeyCmd::All(ToolbarButton::PauseAll),
        (KeyName::S, true) => KeyCmd::All(ToolbarButton::ResumeAll),
        (KeyName::C, true) => KeyCmd::All(ToolbarButton::CancelAll),
        (KeyName::X, false) => KeyCmd::All(ToolbarButton::ClearFinished),
        _ => return None,
    })
}

/// The row under the cursor.
enum Target {
    Job(JobRow),
    History(HistoryRow),
}

fn target(core: &AppCore, index: usize) -> Option<Target> {
    if crate::view::jobs::history_tab(core) {
        let all = core.fileops.jobs.history();
        let h = all.len().checked_sub(1 + index).map(|i| &all[i])?;
        Some(Target::History(HistoryRow::new(h, std::time::SystemTime::now())))
    } else {
        let snaps = jobs_rows::sorted_snapshots(core);
        snaps.get(index).map(|s| Target::Job(JobRow::new(core, s)))
    }
}

/// Press button `b` of row `index` (when the row has it, enabled).
pub fn press(app: &mut App, index: usize, b: RowButton) {
    match target(&app.core, index) {
        Some(Target::Job(r)) if r.has(b) => jobs_rows::apply_job(&mut app.core, r.id, b),
        Some(Target::History(r)) if r.enabled(b) => jobs_rows::apply_history(&mut app.core, r.id, b),
        _ => {}
    }
}

/// Press toolbar button `b` (jobs tab, when it applies).
pub fn press_all(app: &mut App, b: ToolbarButton) {
    if crate::view::jobs::history_tab(&app.core) {
        return;
    }
    if jobs_rows::toolbar(&app.core).iter().any(|&(x, on)| x == b && on) {
        jobs_rows::apply_toolbar(&mut app.core, b);
    }
}

/// Show or hide the details of job row `index`.
pub fn toggle_log(app: &mut App, index: usize) {
    if let Some(Target::Job(r)) = target(&app.core, index) {
        jobs_rows::toggle_expanded(&mut app.core, r.id);
    }
}

/// Open the "More" menu of job row `index` at `anchor` (nothing for a finished job or a
/// history entry).
pub fn open_menu(app: &mut App, index: usize, anchor: (u16, u16)) {
    let Some(Target::Job(r)) = target(&app.core, index) else { return };
    let id = r.id;
    let mut items: Vec<MenuItem<MenuCmd>> = Vec::new();
    for e in jobs_rows::job_menu(&r) {
        match e {
            JobMenuEntry::Priority(prios) => {
                let sub = prios
                    .into_iter()
                    .map(|(cmd, label, on)| MenuItem::run(label, MenuCmd::Job(id, cmd)).checked(on))
                    .collect();
                items.push(MenuItem::submenu(soyuz_appcore::tr!("panels-priority"), sub));
            }
            JobMenuEntry::Item { cmd, label, enabled, .. } => {
                items.push(MenuItem::run(label, MenuCmd::Job(id, cmd)).enabled(enabled));
            }
            JobMenuEntry::Separator => items.push(MenuItem::Separator),
        }
    }
    if !items.is_empty() {
        super::menu::open(app, Menu::new(items, anchor));
    }
}

/// Where the keyboard-opened menu of row `index` goes: below its first line.
fn row_anchor(app: &App, index: usize) -> (u16, u16) {
    app.hits
        .iter()
        .find(|(_, t)| *t == HitTarget::JobRow(index))
        .map_or((1, 1), |(r, _)| (r.x.saturating_add(4.min(r.width)), r.y.saturating_add(1)))
}

/// Switch to the jobs (`false`) or history tab.
fn set_tab(app: &mut App, history: bool) {
    app.core.fileops.activity.tab = if history { ActivityTab::History } else { ActivityTab::Jobs };
}

/// Run a key command on the cursor row.
pub fn run(app: &mut App, cmd: KeyCmd) {
    let i = app.input.jobs.list.cursor;
    let row = target(&app.core, i);
    let job = |b: RowButton| matches!(&row, Some(Target::Job(r)) if r.has(b));
    let hist = |b: RowButton| matches!(&row, Some(Target::History(r)) if r.enabled(b));
    let first = |bs: &[RowButton]| bs.iter().copied().find(|&b| job(b) || hist(b));
    let b = match cmd {
        KeyCmd::Leave => {
            app.input.jobs.focused = false;
            return;
        }
        KeyCmd::Tab(_) => {
            let history = crate::view::jobs::history_tab(&app.core);
            set_tab(app, !history);
            return;
        }
        KeyCmd::Log => {
            toggle_log(app, i);
            app.input.jobs.follow = true;
            return;
        }
        KeyCmd::Menu => {
            let anchor = row_anchor(app, i);
            open_menu(app, i, anchor);
            return;
        }
        KeyCmd::All(b) => {
            press_all(app, b);
            return;
        }
        KeyCmd::PauseResume => first(&[RowButton::Pause, RowButton::Resume]),
        KeyCmd::CancelRemove => first(&[RowButton::Cancel, RowButton::Remove]),
        KeyCmd::Retry => first(&[RowButton::RetryFailed]),
        KeyCmd::UndoRedo => first(&[RowButton::Undo, RowButton::Redo]),
        KeyCmd::Enter => first(&[RowButton::Resolve, RowButton::Undo, RowButton::Redo]),
    };
    if let Some(b) = b {
        press(app, i, b);
    }
}

/// Keys while the panel has focus.
pub fn route(app: &mut App, events: &[Event]) {
    // A press outside the panel gives the keyboard back (`mouse::route` handled it).
    for e in events {
        if let Event::Mouse(m) = e
            && matches!(m.kind, MouseEventKind::Down(_))
        {
            let ours = matches!(
                crate::input::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t),
                Some(HitTarget::BottomTab(_) | HitTarget::JobRow(_) | HitTarget::JobButton { .. })
            );
            if !ours {
                app.input.jobs.focused = false;
            }
        }
    }
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    let mut i = 0;
    while i < frame.events.len() {
        // A key may have opened the job menu: it takes the rest.
        if !app.input.jobs.focused || super::menu::is_open(&app.input.menu) {
            break;
        }
        let len = crate::view::jobs::len(&app.core);
        let height = app.input.jobs.height.max(1);
        let ev = frame.events[i].clone();
        let handled = if app.input.jobs.list.apply(&ev, len, height) {
            true
        } else if let Some(cmd) = key_cmd(&ev) {
            run(app, cmd);
            true
        } else {
            false
        };
        if handled {
            frame.consumed[i] = true;
            // The text a letter key types.
            if let Some(InputEvent::Text(_)) = frame.events.get(i + 1) {
                frame.consumed[i + 1] = true;
                i += 1;
            }
        }
        i += 1;
    }
    // Whatever is left: the core's global bindings.
    let (kctx, text) = super::key_context(super::TuiFocus::Jobs);
    app.core.input(&app.host, &mut frame, kctx, text);
}

/// A mouse press on one of the panel's targets.
pub fn click(app: &mut App, target: HitTarget, m: &MouseEvent) {
    let MouseEventKind::Down(button) = m.kind else { return };
    let len = crate::view::jobs::len(&app.core);
    if button == MouseButton::Right {
        if let HitTarget::JobRow(row) | HitTarget::JobButton { row, .. } = target
            && !matches!(target, HitTarget::JobButton { button: b, .. } if crate::view::jobs::toolbar_button(b).is_some() || b == JobButton::Close)
        {
            app.input.jobs.focused = true;
            app.input.jobs.list.select(row, len);
            open_menu(app, row, (m.column, m.row));
        }
        return;
    }
    if button != MouseButton::Left {
        return;
    }
    match target {
        HitTarget::BottomTab(i) => {
            app.input.jobs.focused = true;
            set_tab(app, i == 1);
        }
        HitTarget::JobRow(i) => {
            app.input.jobs.focused = true;
            app.input.jobs.list.select(i, len);
        }
        HitTarget::JobButton { button: JobButton::Close, .. } => {
            app.input.jobs.focused = false;
            if crate::view::jobs::shown(&app.core) {
                app.core.ui.bottom.toggle(soyuz_appcore::models::panels::BottomTab::Activity);
            }
        }
        HitTarget::JobButton { button, .. } if crate::view::jobs::toolbar_button(button).is_some() => {
            app.input.jobs.focused = true;
            if let Some(b) = crate::view::jobs::toolbar_button(button) {
                press_all(app, b);
            }
        }
        HitTarget::JobButton { row, button: JobButton::Details } => {
            app.input.jobs.focused = true;
            app.input.jobs.list.select(row, len);
            toggle_log(app, row);
        }
        HitTarget::JobButton { row, button: JobButton::More } => {
            app.input.jobs.focused = true;
            app.input.jobs.list.select(row, len);
            open_menu(app, row, (m.column, m.row));
        }
        HitTarget::JobButton { row, button } => {
            app.input.jobs.focused = true;
            app.input.jobs.list.select(row, len);
            if let Some(b) = crate::view::jobs::row_button(button) {
                press(app, row, b);
            }
        }
        _ => {}
    }
}

/// The wheel over one of the panel's targets (`rows`: negative scrolls up).
pub fn wheel(app: &mut App, _target: HitTarget, rows: i32) {
    let len = crate::view::jobs::len(&app.core);
    let height = app.input.jobs.height.max(1);
    app.input.jobs.list.wheel(rows, len, height);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn keys() {
        assert_eq!(key_cmd(&chord("Escape")), Some(KeyCmd::Leave));
        assert_eq!(key_cmd(&chord("Space")), Some(KeyCmd::PauseResume));
        assert_eq!(key_cmd(&chord("P")), Some(KeyCmd::PauseResume));
        assert_eq!(key_cmd(&chord("Delete")), Some(KeyCmd::CancelRemove));
        assert_eq!(key_cmd(&chord("C")), Some(KeyCmd::CancelRemove));
        assert_eq!(key_cmd(&chord("R")), Some(KeyCmd::Retry));
        assert_eq!(key_cmd(&chord("U")), Some(KeyCmd::UndoRedo));
        assert_eq!(key_cmd(&chord("L")), Some(KeyCmd::Log));
        assert_eq!(key_cmd(&chord("M")), Some(KeyCmd::Menu));
        assert_eq!(key_cmd(&chord("Shift+P")), Some(KeyCmd::All(ToolbarButton::PauseAll)));
        assert_eq!(key_cmd(&chord("Shift+S")), Some(KeyCmd::All(ToolbarButton::ResumeAll)));
        assert_eq!(key_cmd(&chord("Shift+C")), Some(KeyCmd::All(ToolbarButton::CancelAll)));
        assert_eq!(key_cmd(&chord("X")), Some(KeyCmd::All(ToolbarButton::ClearFinished)));
        assert_eq!(key_cmd(&chord("Shift+Tab")), Some(KeyCmd::Tab(false)));
        assert_eq!(key_cmd(&chord("Right")), Some(KeyCmd::Tab(true)));
        assert_eq!(key_cmd(&chord("Ctrl+C")), None, "Copy stays global");
        assert_eq!(key_cmd(&chord("Shift+U")), None);
        assert_eq!(key_cmd(&InputEvent::Text("p".into())), None);
    }

    #[test]
    fn spoken_progress_changes_per_ten_percent_or_every_two_seconds() {
        let t0 = Instant::now();
        let shown = |permille: Option<u16>| Spoken {
            at: t0,
            state: "running".into(),
            permille,
            line: String::new(),
            stats: String::new(),
            right: String::new(),
        };
        // Nothing shown yet.
        assert!(due(None, "running", Some(0), t0));
        let at_42 = shown(Some(420));
        // Within the same step and under 2 s: held.
        for p in [420, 450, 499] {
            assert!(!due(Some(&at_42), "running", Some(p), t0 + Duration::from_millis(1999)), "{p}");
        }
        // The next step, at once.
        assert!(due(Some(&at_42), "running", Some(500), t0 + Duration::from_millis(10)));
        assert!(due(Some(&at_42), "running", Some(1000), t0));
        // 2 s later, even without a new step (speed and time left move).
        assert!(due(Some(&at_42), "running", Some(430), t0 + SPOKEN_EVERY));
        // Another state (paused, done, failed) at once.
        assert!(due(Some(&at_42), "paused", Some(420), t0));
        // A known fraction appears or goes.
        assert!(due(Some(&shown(None)), "running", Some(10), t0));
        assert!(due(Some(&at_42), "running", None, t0));
        assert!(!due(Some(&shown(None)), "running", None, t0 + Duration::from_secs(1)));
    }
}
