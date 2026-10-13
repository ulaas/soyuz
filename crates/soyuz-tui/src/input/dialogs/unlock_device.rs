//! The unlock dialog of a locked device container (`TuiDialog::UnlockDevice`, Phase T7,
//! agent B): a masked passphrase / recovery key field (Alt+V shows it), unlocking on a
//! worker (`models::devices::submit_unlock`, which normalizes a BitLocker recovery key).
//! The GUI's `ui/dialogs/unlock_volume.rs`: a wrong passphrase keeps the dialog open with
//! the error and the field focused again; success closes it (the mount and the
//! navigation follow from the core's device pump); a worker that never ran says so.
//!
//! The passphrase is never logged (`State`'s `Debug` leaves it out) and is wiped: moved
//! out of the field into a `Zeroizing` buffer on submit, zeroized when the dialog goes
//! (`Drop`), and the drawn copy of a revealed passphrase is wiped with its `FieldVm`. The
//! field's buffer is reserved up front so typing does not leave reallocated copies behind
//! (best effort, like the GUI's).
//!
//! | Key | Does |
//! |---|---|
//! | typing | the passphrase (bullets) |
//! | Enter / Alt+U | unlock |
//! | Alt+V | show / hide the passphrase |
//! | Esc / Alt+C | cancel |

use std::sync::mpsc::{Receiver, TryRecvError};

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::devices::{is_bitlocker, submit_unlock};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use zeroize::{Zeroize, Zeroizing};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog, form_mouse};
use crate::app::App;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome};

/// Button indices.
pub const UNLOCK: usize = 0;
pub const SHOW: usize = 1;
pub const CANCEL: usize = 2;

/// Bytes reserved for the passphrase, so typing never reallocates (and leaves copies).
const RESERVE: usize = 256;

/// What the dialog keeps while it is open.
pub struct State {
    /// The UDisks2 object of the container.
    pub object: String,
    pub name: String,
    /// `BitLocker` / `crypto_LUKS`, for the texts and the recovery key.
    pub kind: String,
    /// Where the unlocked volume opens.
    pub pane: PaneId,
    pub title: String,
    /// Field 0: the passphrase; buttons Unlock, Show / Hide, Cancel.
    pub form: Form,
    /// The passphrase shows in clear.
    pub reveal: bool,
    /// The unlock running on its worker.
    pub pending: Option<Receiver<Result<(), String>>>,
    /// Unlocked: the dialog closes ([`stale`]).
    pub done: bool,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the field's text.
        f.debug_struct("unlock_device::State")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("reveal", &self.reveal)
            .field("working", &self.pending.is_some())
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        wipe(&mut self.form);
    }
}

/// Zeroize the passphrase field's text (it stays empty).
fn wipe(form: &mut Form) {
    if let Some(f) = form.fields.first_mut() {
        f.take_text().zeroize();
    }
}

/// An empty passphrase field with its buffer reserved.
fn passphrase_field(kind: &str) -> Field {
    let label = if is_bitlocker(kind) { tr!("dialogs-unlock-hint-bitlocker") } else { tr!("dialogs-unlock-hint") };
    let mut f = Field::secret(label);
    if let FieldKind::Text { text, .. } = &mut f.kind {
        text.reserve(RESERVE);
    }
    f
}

impl State {
    pub fn new(object: String, name: String, kind: String, pane: PaneId) -> State {
        let title = tr!("dialogs-unlock-title", name = crate::dialog::safe(&name));
        let form = Form::new(
            vec![passphrase_field(&kind)],
            vec![
                Button::new(tr!("dialogs-unlock-button")).accel('u'),
                Button::new(tr!("dialogs-unlock-show")).accel('v'),
                Button::new(tr!("common-cancel")).accel('c'),
            ],
        );
        let mut s = State { object, name, kind, pane, title, form, reveal: false, pending: None, done: false };
        s.sync();
        s
    }

    /// The unlock runs.
    pub fn working(&self) -> bool {
        self.pending.is_some()
    }

    /// Something was typed.
    fn typed(&self) -> bool {
        self.form.fields.first().and_then(Field::value).is_some_and(|t| !t.is_empty())
    }

    /// Enable / label the controls for the state (working, typed, revealed).
    fn sync(&mut self) {
        let working = self.working();
        let typed = self.typed();
        let reveal = self.reveal;
        if let Some(f) = self.form.fields.first_mut() {
            f.enabled = !working;
            f.set_secret(!reveal);
        }
        self.form.buttons[UNLOCK].enabled = !working && typed;
        self.form.buttons[SHOW].enabled = !working;
        self.form.buttons[SHOW].label =
            if reveal { tr!("dialogs-unlock-hide") } else { tr!("dialogs-unlock-show") }.to_string();
        if working && self.form.focused_field().is_some() {
            self.form.focus_button(CANCEL);
        }
    }

    /// Unlock with the typed passphrase (moved out of the field, never copied).
    fn submit(&mut self) {
        if self.working() || !self.typed() {
            return;
        }
        let Some(f) = self.form.fields.first_mut() else { return };
        let typed = Zeroizing::new(f.take_text());
        if let FieldKind::Text { text, .. } = &mut f.kind {
            text.reserve(RESERVE);
        }
        self.pending = Some(submit_unlock(&self.object, self.name.clone(), &self.kind, typed, self.pane));
        self.form.error = None;
        self.sync();
    }

    /// Back to typing (after an error).
    fn retry(&mut self, error: String) {
        self.pending = None;
        self.form.error = Some(error);
        self.sync();
        self.form.focus = 0;
    }
}

/// Open the dialog for `FrontendCmd::UnlockDevice` (mount into `pane` once unlocked).
pub fn open(app: &mut App, object: String, name: String, kind: String, pane: PaneId) {
    app.push_dialog(TuiDialog::UnlockDevice(State::new(object, name, kind, pane)));
}

fn press(s: &mut State, b: usize) -> Outcome {
    match b {
        UNLOCK => s.submit(),
        SHOW => {
            s.reveal = !s.reveal;
            s.sync();
        }
        _ => return Outcome::Close(None),
    }
    Outcome::Keep
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Pressed(b) => {
                if let Outcome::Close(c) = press(s, b) {
                    return Outcome::Close(c);
                }
            }
            FormOutcome::Changed(_) => {
                s.form.error = None;
                s.sync();
            }
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    match form_mouse(&mut s.form, None, m) {
        FormOutcome::Pressed(b) => press(s, b),
        _ => Outcome::Keep,
    }
}

/// The dialog went away by itself: unlocked.
pub fn stale(_core: &AppCore, s: &State) -> bool {
    s.done
}

/// Every frame while it is open (`input::dialogs::tick`, from `view::prepare`): the
/// worker's answer (the frame loop's idle tick polls it at least every 250 ms).
pub fn tick(_core: &mut AppCore, s: &mut State) {
    let Some(rx) = &s.pending else { return };
    match rx.try_recv() {
        Ok(Ok(())) => {
            s.pending = None;
            s.done = true;
        }
        Ok(Err(e)) => s.retry(e),
        // The worker never ran (another operation on this device) or died.
        Err(TryRecvError::Disconnected) => s.retry(tr!("dialogs-unlock-interrupted").to_string()),
        Err(TryRecvError::Empty) => {}
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn input(ev: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Chord(ev.parse().unwrap())] }
    }

    fn text(t: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Text(t.into())] }
    }

    #[test]
    fn debug_never_shows_the_passphrase() {
        let mut s = State::new("/org/x/sdb1".into(), "Backup".into(), "crypto_LUKS".into(), PaneId(1));
        for ev in &text("hunter2").events {
            s.form.apply(ev);
        }
        assert_eq!(s.form.fields[0].value(), Some("hunter2"));
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("hunter2") && dbg.contains("Backup"), "{dbg}");
    }

    #[test]
    fn reveal_toggles_and_the_buttons_follow_the_text() {
        let mut s = State::new("/org/x/sdb1".into(), "Backup".into(), "BitLocker".into(), PaneId(1));
        assert!(!s.form.buttons[UNLOCK].enabled, "nothing typed");
        assert_eq!(s.form.fields[0].label, tr!("dialogs-unlock-hint-bitlocker"));
        for ev in &text("pw").events {
            if let FormOutcome::Changed(_) = s.form.apply(ev) {
                s.sync();
            }
        }
        assert!(s.form.buttons[UNLOCK].enabled);
        let shown = |s: &State| match &s.form.field_vms()[0].kind {
            crate::widgets::form::FieldVmKind::Text { text, .. } => text.clone(),
            _ => String::new(),
        };
        assert_eq!(shown(&s), "••");
        assert!(matches!(press(&mut s, SHOW), Outcome::Keep));
        assert_eq!(shown(&s), "pw");
        assert_eq!(s.form.buttons[SHOW].label, tr!("dialogs-unlock-hide"));
        assert!(matches!(press(&mut s, CANCEL), Outcome::Close(None)));
        // Alt+V is the show button's accelerator, even in the field.
        let mut s = State::new("o".into(), "n".into(), "crypto_LUKS".into(), PaneId(1));
        assert_eq!(s.form.apply(&input("Alt+V").events[0]), FormOutcome::Pressed(SHOW));
    }

    #[test]
    fn the_field_buffer_is_reserved_and_wiped() {
        let mut s = State::new("o".into(), "n".into(), "crypto_LUKS".into(), PaneId(1));
        let cap = |s: &State| match &s.form.fields[0].kind {
            FieldKind::Text { text, .. } => text.capacity(),
            _ => 0,
        };
        assert!(cap(&s) >= RESERVE);
        for ev in &text("secret").events {
            s.form.apply(ev);
        }
        wipe(&mut s.form);
        assert_eq!(s.form.fields[0].value(), Some(""));
    }
}
