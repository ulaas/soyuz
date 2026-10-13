//! `SOYUZ_SCRIPT` in the terminal UI (Phase T8): the core runs the script
//! (`soyuz_appcore::devscript::Runner`, `App::script`); this module feeds it the
//! frontend's facts ([`facts`]) and carries out the frontend steps:
//! - `keys:<spec>`: crossterm events for the next frame (`Paste` → `Event::Paste`);
//! - `shot:<file>`: the last drawn buffer (`App::last_buffer`) saved by `crate::shot`;
//! - `size:<w>x<h>`: resizes the test terminal in headless mode; a real terminal ignores
//!   it with a warning.
//!
//! [`pump`] runs once per loop iteration, after the frame (`app::run_loop`). The script
//! comes from `SOYUZ_SCRIPT` in `crate::run` only, or from `App::with_script` in tests.
//!
//! Wave 0 skeleton: the loop and the facts; the steps come with Phase T8 P4.

use crossterm::event::Event;
use soyuz_appcore::devscript::{FrontStep, KeyTok, ScriptFacts};

use crate::app::App;
use crate::input::TuiFocus;

/// What the script's checks need from the terminal UI (`dialog=`, `settings=`, `focus=`,
/// `screen~`).
pub fn facts(app: &App) -> ScriptFacts {
    let focus = app.focus();
    ScriptFacts {
        dialog_open: app.view().is_some_and(|v| v.dialog.is_some()),
        settings_open: app.view().is_some_and(|v| v.settings.visible),
        focus: Some(focus_name(focus)),
        screen_text: app.last_buffer().map(|b| crate::snapshot::lines(b).join("\n")),
    }
}

/// `TuiFocus` by name, as `expect:focus=<name>` writes it (`List`, `Dialog`, …).
pub fn focus_name(focus: TuiFocus) -> String {
    format!("{focus:?}")
}

/// Run the script's due steps; synthetic input goes to `events` (the next frame's).
pub fn pump(app: &mut App, events: &mut Vec<Event>) {
    let Some(mut runner) = app.script.take() else { return };
    let facts = facts(app);
    while let Some(step) = runner.pump(&mut app.core, &app.host, &facts) {
        let yields = step.yields();
        handle(app, step, events);
        if yields {
            break;
        }
    }
    app.script = Some(runner);
}

/// One frontend step.
fn handle(app: &mut App, step: FrontStep, events: &mut Vec<Event>) {
    let _ = app;
    match step {
        FrontStep::Keys(keys) => events.extend(key_events(&keys)),
        other => log::warn!("SOYUZ_SCRIPT: {other:?} is not supported in the terminal UI yet"),
    }
}

/// `keys:` tokens → crossterm events.
pub fn key_events(keys: &[KeyTok]) -> Vec<Event> {
    let _ = keys;
    Vec::new()
}
