//! The shortcut recorder of the settings editor (`TuiDialog::Keybind`): for an action of
//! the key bindings page (add a binding, or replace one) or for a key that holds a chord.
//!
//! Every key press is recorded as it arrives from the terminal (`crate::keys` conversion
//! only: nothing reaches the core's dispatcher, so Esc / Enter / bound chords are not run),
//! with the GUI recorder's rules (`settings_keys::record_outcome`): Esc cancels, Enter
//! saves what was recorded (Enter as the first key is recorded itself), up to three chords
//! make a sequence. The dialog shows what the keys are already used for. Save, Clear and
//! Cancel are also buttons for the mouse. The result goes back to the editor through its
//! [`Inbox`].

use crossterm::event::{MouseButton, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::conflicts::users_of;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::settings_keys::{RecordOutcome, record_outcome};
use soyuz_appcore::palette::catalog;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, KeyChord, format_sequence};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::input::settings::{Commit, Inbox, post};
use crate::layout::HitTarget;
use crate::widgets::{Button, Form};

/// What the recording is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A binding of `action`; `replace`: the binding it replaces (else it is added).
    Action { action: Action, replace: Option<String> },
    /// A key holding one chord string (`bookmarks[2].hotkey`).
    Field { path: String },
}

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub target: Option<Target>,
    /// Recorded so far.
    pub chords: Vec<KeyChord>,
    /// Buttons: Save, Clear, Cancel (mouse; the keyboard is recorded).
    pub form: Form,
    pub inbox: Option<Inbox>,
}

fn form() -> Form {
    Form::new(
        Vec::new(),
        vec![
            Button::new(tr!("common-save")),
            Button::new(tr!("tui-settings-record-clear")),
            Button::new(tr!("common-cancel")),
        ],
    )
}

/// Record a binding for `action` (replacing `replace` when given).
pub fn open_action(app: &mut App, action: Action, replace: Option<String>, inbox: Inbox) {
    let title = tr!("tui-settings-record-title", action = catalog::title(&action));
    let s = State {
        title,
        target: Some(Target::Action { action, replace }),
        chords: Vec::new(),
        form: form(),
        inbox: Some(inbox),
    };
    app.push_dialog(TuiDialog::Keybind(s));
}

/// Record the chord stored at `path`.
pub fn open_field(app: &mut App, path: &str, inbox: Inbox) {
    let s = State {
        title: tr!("tui-settings-record-title", action = path),
        target: Some(Target::Field { path: path.to_string() }),
        chords: Vec::new(),
        form: form(),
        inbox: Some(inbox),
    };
    app.push_dialog(TuiDialog::Keybind(s));
}

/// What the recorded keys are already bound to (`(context, action)` names).
pub fn used_by(core: &AppCore, s: &State) -> Vec<String> {
    if s.chords.is_empty() {
        return Vec::new();
    }
    let own = match &s.target {
        Some(Target::Action { action, .. }) => Some(action),
        _ => None,
    };
    users_of(&core.keymap, &s.chords)
        .into_iter()
        .filter(|(_, a)| Some(a) != own)
        .map(|(ctx, a)| format!("{} ({})", catalog::title(&a), soyuz_appcore::i18n::labels::key_context(ctx)))
        .collect()
}

/// Hand `binding` to the editor.
fn save(core: &AppCore, s: &State, binding: String) {
    let (Some(inbox), Some(target)) = (&s.inbox, &s.target) else { return };
    let c = match target {
        Target::Action { action, replace } => {
            let mut list = crate::input::settings_keys::base_bindings(&core.config, action);
            match replace.as_ref().and_then(|r| list.iter().position(|b| b == r)) {
                Some(i) => list[i] = binding,
                None if !list.contains(&binding) => list.push(binding),
                None => {}
            }
            list.dedup();
            Commit::Bindings { action: action.clone(), list }
        }
        Target::Field { path } => Commit::Set { path: path.clone(), value: toml::Value::String(binding) },
    };
    post(inbox, core, c);
}

/// A key press: recorded (see the module docs).
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let InputEvent::Chord(c) = ev else { continue };
        match record_outcome(&mut s.chords, *c) {
            RecordOutcome::Pending => {}
            RecordOutcome::Cancel => return Outcome::Close(None),
            RecordOutcome::Save(binding) => {
                save(core, s, binding);
                return Outcome::Close(None);
            }
        }
    }
    Outcome::Keep
}

/// A mouse event: the buttons.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    if !matches!(m.event.kind, MouseEventKind::Down(MouseButton::Left)) {
        return Outcome::Keep;
    }
    match m.target {
        Some(HitTarget::DialogButton(0)) if !s.chords.is_empty() => {
            save(core, s, format_sequence(&s.chords));
            Outcome::Close(None)
        }
        Some(HitTarget::DialogButton(1)) => {
            s.chords.clear();
            Outcome::Keep
        }
        Some(HitTarget::DialogButton(2)) => Outcome::Close(None),
        _ => Outcome::Keep,
    }
}

/// The dialog went away by itself: never (the editor may close under it; the result is
/// then dropped with the inbox).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
