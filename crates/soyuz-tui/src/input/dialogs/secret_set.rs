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
//! | Alt+G | MCP client tokens: generate (rotate) a token, like the GUI's Connections page |
//! | Esc | cancel |
//!
//! Generate token (`settings_draft::generate_mcp_token`, the GUI's generator) stores the new
//! token through `settings_draft::store_secret` like Save, puts the reference into the
//! field and keeps the dialog open to show the token once with the client snippet (Copy
//! token, Copy config, Close); the token never reaches `config.toml`.

use crossbeam_channel::Receiver;
use soyuz_appcore::cmd::Cmd;
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
/// MCP client tokens only (after Cancel, so the other indices stay).
const GENERATE: usize = 3;
/// While a generated token is shown.
const COPY_TOKEN: usize = 0;
const COPY_CONFIG: usize = 1;

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
    /// A token just generated, shown once (buttons Copy token, Copy config, Close).
    pub shown: Option<String>,
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
    /// [`Done::Set`], but the dialog stays open to show the generated token.
    Generated {
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
    let mut buttons = vec![Button::new(tr!("common-save")).accel('s'), remove, Button::new(tr!("common-cancel"))];
    if is_mcp_client(path) {
        let label = if has { tr!("settings-mcp-rotate") } else { tr!("settings-mcp-generate") };
        buttons.push(Button::new(label).accel('g'));
    }
    let form = Form::new(vec![Field::secret(tr!("tui-tables-secret-field"))], buttons);
    State {
        title: tr!("tui-tables-secret-title", name = crate::dialog::safe(element_name)),
        path: path.to_string(),
        name,
        current: current.to_string(),
        form,
        inbox: None,
        shown: None,
    }
}

/// The body text: where the secret goes.
pub fn body(s: &State) -> String {
    tr!("tui-tables-secret-body", name = s.name.as_str())
}

/// What an MCP client puts in its config to use `token` (the GUI's snippet).
pub fn snippet(port: u16, token: &str) -> String {
    format!(
        "{{\n  \"type\": \"http\",\n  \"url\": \"http://127.0.0.1:{port}/mcp\",\n  \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n}}"
    )
}

/// Generate a new MCP client token, store it and show it once.
fn generate(core: &AppCore, s: &mut State) -> Done {
    let token = settings_draft::generate_mcp_token();
    let (reference, rx) =
        settings_draft::store_secret_text(&s.current, s.name.clone(), token.clone(), core.wake.clone());
    s.current = reference.clone();
    s.shown = Some(token);
    s.form = Form::new(
        Vec::new(),
        vec![
            Button::new(tr!("settings-mcp-copy-token")).accel('t'),
            Button::new(tr!("settings-mcp-copy-config")).accel('c'),
            Button::new(tr!("common-close")),
        ],
    );
    Done::Generated { reference, pending: Pending { rx, stored: true, mcp_client: true } }
}

/// A button while a generated token is shown: what to copy (`None`: close).
fn copy_cmd(core: &AppCore, token: &str, b: usize) -> Option<Cmd> {
    match b {
        COPY_TOKEN => Some(Cmd::CopyText(token.to_string())),
        COPY_CONFIG => Some(Cmd::CopyText(snippet(core.config.ai.mcp.http_port, token))),
        _ => None,
    }
}

fn is_mcp_client(path: &str) -> bool {
    soyuz_appcore::models::settings_schema::normalize(path) == "ai.mcp_clients[].token"
}

fn pressed(core: &mut AppCore, s: &mut State, b: usize) -> Done {
    if let Some(token) = &s.shown {
        return match copy_cmd(core, token, b) {
            Some(cmd) => {
                core.cmds.push(cmd);
                Done::Keep
            }
            None => Done::Cancel,
        };
    }
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
        GENERATE if is_mcp_client(&s.path) => generate(core, s),
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
pub fn input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Done {
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
pub fn mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Done {
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

/// Standalone: post the reference to the settings editor.
fn set_standalone(core: &mut AppCore, s: &State, reference: String, pending: &Pending) {
    if let Some(inbox) = &s.inbox {
        post(inbox, core, Commit::Set { path: s.path.clone(), value: toml::Value::String(reference) });
    }
    // Standalone, nobody polls the write later: a MCP client token is re-read now (the
    // reference change re-reads it too once it is applied).
    if pending.mcp_client {
        core.services.mcp.refresh_tokens();
    }
}

fn finish(core: &mut AppCore, s: &State, done: Done) -> Outcome {
    match done {
        Done::Keep => Outcome::Keep,
        Done::Cancel => Outcome::Close(None),
        Done::Set { reference, pending } => {
            set_standalone(core, s, reference, &pending);
            Outcome::Close(None)
        }
        Done::Generated { reference, pending } => {
            set_standalone(core, s, reference, &pending);
            Outcome::Keep
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
        // Only MCP client tokens can be generated.
        assert_eq!(s.form.buttons.len(), 4);
        assert_eq!(s.form.buttons[GENERATE].label, tr!("settings-mcp-generate"));
        let rotate = state("ai.mcp_clients[1].token", "Desk", "credential:x");
        assert_eq!(rotate.form.buttons[GENERATE].label, tr!("settings-mcp-rotate"));
        assert_eq!(state("ai.profiles[0].key", "My Claude", "").form.buttons.len(), 3);
    }

    #[test]
    fn the_snippet_is_the_guis() {
        let s = snippet(8765, "soyuz_abc");
        assert!(s.contains("\"url\": \"http://127.0.0.1:8765/mcp\""), "{s}");
        assert!(s.contains("\"Authorization\": \"Bearer soyuz_abc\""), "{s}");
    }
}
