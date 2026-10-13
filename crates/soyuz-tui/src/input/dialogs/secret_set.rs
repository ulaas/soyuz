//! "Set secret…" of the settings editor (`TuiDialog::SecretSet`): a masked field whose
//! text goes to the OS credential store (`settings_draft::store_secret`, the same
//! `ai_service` call the GUI's AI page makes); the config only ever gets the
//! `credential:<name>` reference. Remove deletes the stored secret and clears the
//! reference.
//!
//! The table editor's element form shows it inside its own dialog (`table_row`), for the
//! fields marked `secret_ref` (`ai.profiles[].key`, `ai.mcp_clients[].token`,
//! `ai.mcp_servers[].token`). The store runs on a worker thread; the [`Pending`] job it
//! hands back is polled by the dialog that owns it ([`poll`]): a toast tells how it went,
//! and a stored MCP client token makes the MCP server pick up the new token
//! (`McpBridge::refresh_tokens`, as the GUI's Connections page does).
//!
//! | Key | Does |
//! |---|---|
//! | typing | the secret (shown as bullets) |
//! | Enter / Alt+S | store it |
//! | Alt+R | remove the stored secret and the reference |
//! | Esc | cancel |

use crossbeam_channel::Receiver;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::settings_draft;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::services::ai_service;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, form_mouse};
use crate::input::settings::{Commit, Inbox, post};
use crate::widgets::{Button, Field, Form, FormOutcome};

/// Button indices.
const SAVE: usize = 0;
const REMOVE: usize = 1;

/// A credential store write still running (or finished, not yet reported).
#[derive(Debug)]
pub struct Pending {
    pub rx: Receiver<Result<(), String>>,
    /// The secret was stored (else removed).
    pub stored: bool,
    /// It is an MCP client token: the MCP server re-reads its tokens when it is done.
    pub mcp_client: bool,
}

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Draft path of the reference field (`"ai.profiles[0].key"`).
    pub path: String,
    /// The credential name a new secret is stored under (an existing `credential:<name>`
    /// reference keeps its name).
    pub name: String,
    /// The reference the field holds now.
    pub current: String,
    /// Field 0: the secret (masked); buttons Save, Remove, Cancel.
    pub form: Form,
    /// Standalone (`TuiDialog::SecretSet`): where the reference goes.
    pub inbox: Option<Inbox>,
}

/// What a key did.
#[derive(Debug)]
pub enum Done {
    Keep,
    Cancel,
    /// Put `reference` into the field; `pending` is the store write.
    Set {
        reference: String,
        pending: Pending,
    },
}

/// The dialog for the reference field at `path` (schema row path `row_path`) of the
/// element named `element_name`, holding `current`.
pub fn state(path: &str, element_name: &str, current: &str) -> State {
    let stored = settings_draft::credential_of(current);
    let has = stored.is_some();
    let name = stored.unwrap_or_else(|| settings_draft::secret_name(path, element_name));
    let mut remove = Button::new(tr!("tui-tables-secret-remove")).accel('r');
    remove.enabled = has;
    let form = Form::new(
        vec![Field::secret(tr!("tui-tables-secret-field"))],
        vec![Button::new(tr!("common-save")).accel('s'), remove, Button::new(tr!("common-cancel"))],
    );
    State {
        title: tr!("tui-tables-secret-title", name = crate::dialog::safe(element_name)),
        path: path.to_string(),
        name,
        current: current.to_string(),
        form,
        inbox: None,
    }
}

/// The body text: where the secret goes.
pub fn body(s: &State) -> String {
    tr!("tui-tables-secret-body", name = s.name.as_str())
}

fn is_mcp_client(path: &str) -> bool {
    soyuz_appcore::models::settings_schema::normalize(path) == "ai.mcp_clients[].token"
}

fn pressed(core: &AppCore, s: &mut State, b: usize) -> Done {
    match b {
        SAVE => {
            let text = s.form.fields.first().and_then(Field::value).unwrap_or_default().to_string();
            if text.trim().is_empty() {
                s.form.error = Some(tr!("tui-tables-secret-empty").into());
                return Done::Keep;
            }
            let (reference, rx) =
                settings_draft::store_secret_text(&s.current, s.name.clone(), text, core.wake.clone());
            // The typed text does not stay in the form.
            s.form.fields = vec![Field::secret(tr!("tui-tables-secret-field"))];
            Done::Set { reference, pending: Pending { rx, stored: true, mcp_client: is_mcp_client(&s.path) } }
        }
        REMOVE => {
            let rx = ai_service::delete_secret(s.name.clone(), core.wake.clone());
            Done::Set {
                reference: String::new(),
                pending: Pending { rx, stored: false, mcp_client: is_mcp_client(&s.path) },
            }
        }
        _ => Done::Cancel,
    }
}

/// A key press or paste, for the dialog that shows this one inside its own.
pub fn input(core: &AppCore, s: &mut State, i: &DialogInput) -> Done {
    for ev in &i.events {
        match s.form.apply(ev) {
            FormOutcome::Pressed(b) => match pressed(core, s, b) {
                Done::Keep => {}
                done => return done,
            },
            FormOutcome::Cancel => return Done::Cancel,
            FormOutcome::Changed(_) => s.form.error = None,
            _ => {}
        }
    }
    Done::Keep
}

/// A mouse event, for the dialog that shows this one inside its own.
pub fn mouse(core: &AppCore, s: &mut State, m: &DialogMouse) -> Done {
    match form_mouse(&mut s.form, None, m) {
        FormOutcome::Pressed(b) => pressed(core, s, b),
        _ => Done::Keep,
    }
}

/// Report finished store writes (a toast each; a new MCP client token is picked up by the
/// MCP server). Returns whether any is still running.
pub fn poll(core: &mut AppCore, pending: &mut Vec<Pending>) -> bool {
    pending.retain(|p| match p.rx.try_recv() {
        Ok(Ok(())) => {
            let title = if p.stored { tr!("tui-tables-secret-stored") } else { tr!("tui-tables-secret-removed") };
            core.ui.toasts.push(Toast::info(title));
            if p.mcp_client {
                core.services.mcp.refresh_tokens();
            }
            false
        }
        Ok(Err(e)) => {
            core.ui.toasts.push(Toast::error(tr!("tui-tables-secret-failed", error = e)));
            false
        }
        Err(crossbeam_channel::TryRecvError::Empty) => true,
        Err(crossbeam_channel::TryRecvError::Disconnected) => false,
    });
    !pending.is_empty()
}

fn finish(core: &mut AppCore, s: &State, done: Done) -> Outcome {
    match done {
        Done::Keep => Outcome::Keep,
        Done::Cancel => Outcome::Close(None),
        Done::Set { reference, pending } => {
            if let Some(inbox) = &s.inbox {
                post(inbox, core, Commit::Set { path: s.path.clone(), value: toml::Value::String(reference) });
            }
            // Standalone, nobody polls the write later: a MCP client token is re-read
            // now (the reference change re-reads it too once it is applied).
            if pending.mcp_client {
                core.services.mcp.refresh_tokens();
            }
            Outcome::Close(None)
        }
    }
}

/// A key press or paste (standalone).
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let done = input(core, s, i);
    finish(core, s, done)
}

/// A mouse event (standalone).
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let done = mouse(core, s, m);
    finish(core, s, done)
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_reference_or_the_gui() {
        let s = state("ai.profiles[0].key", "My Claude", "");
        assert_eq!(s.name, "my-claude");
        assert!(!s.form.buttons[REMOVE].enabled, "nothing stored yet");
        let s = state("ai.profiles[0].key", "My Claude", "credential:kept");
        assert_eq!(s.name, "kept");
        assert!(s.form.buttons[REMOVE].enabled);
        let s = state("ai.mcp_clients[1].token", "Desk", "");
        assert_eq!(s.name, soyuz_mcp::trust::token_credential("Desk"));
        assert!(is_mcp_client(&s.path));
        assert!(body(&s).contains(&s.name));
    }
}
