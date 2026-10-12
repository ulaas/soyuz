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
//! | Esc | give the keyboard back to the file list | |
//!
//! Other keys go to the core's global bindings (Ctrl+Q, the palette, …). A mouse press
//! outside the panel gives the keyboard back too.
//!
//! Mouse ([`click`], [`wheel`]): `BottomTab` switches tabs, `JobRow` moves the cursor,
//! `JobButton` presses the button (`Close` closes the panel); each press focuses the
//! panel. The wheel scrolls the list without moving the cursor.

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::jobs_rows::{self, HistoryRow, JobRow, RowButton};
use soyuz_appcore::models::panels::ActivityTab;
use soyuz_core::config::{KeyName, Mods};

use super::InputState;
use crate::app::App;
use crate::hits::JobButton;
use crate::layout::HitTarget;
use crate::widgets::SelectList;

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
        if !app.input.jobs.focused {
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
    if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    let len = crate::view::jobs::len(&app.core);
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
        assert_eq!(key_cmd(&chord("Shift+Tab")), Some(KeyCmd::Tab(false)));
        assert_eq!(key_cmd(&chord("Right")), Some(KeyCmd::Tab(true)));
        assert_eq!(key_cmd(&chord("Ctrl+C")), None, "Copy stays global");
        assert_eq!(key_cmd(&chord("Shift+P")), None);
        assert_eq!(key_cmd(&InputEvent::Text("p".into())), None);
    }
}
