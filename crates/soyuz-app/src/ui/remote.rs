//! Remote locations (GUI): the "Connect to server…" dialog, the sidebar's Network section
//! (saved connections with their connection state), the "trust this host key?" prompt and
//! the new folder / rename dialog. The state, the commands and the file operations live in
//! `models::remote`.

use std::collections::BTreeSet;

use egui::{Key, Margin, RichText, TextEdit, Ui};
use soyuz_core::Location;
use soyuz_vfs::known_hosts::HostKeyStatus;
use soyuz_vfs::registry::ConnState;
use soyuz_vfs::{RemoteUri, Scheme};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::models::remote::{ConnectForm, HostKeyAsk, NameTarget, PROTOCOLS, RemotePrompt, delete_cmd, name_submit};
pub use crate::models::remote::{RemoteCmd, apply, execute, handles, open_entry, preview_target, pump, transfer};
use crate::services::remote::{self as svc, AuthKind};
use crate::state::pane::PaneId;
use crate::theme::{icons::ph, palette};
use crate::ui::dialogs::{ConfirmDialog, Dialog, DialogCtx, footer};
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, menu_separator};

/// Show a dialog the core asked for ([`crate::models::frontend::FrontendCmd::Remote`]).
pub fn prompt(app: &mut SoyuzApp, p: RemotePrompt) {
    match p {
        RemotePrompt::Connect(form) => app.gui.dialogs.push(ConnectDialog { form: *form, focus: true }),
        RemotePrompt::HostKey(ask) => app.gui.dialogs.push(HostKeyDialog { ask }),
        RemotePrompt::ConfirmDelete { message, paths } => app.gui.dialogs.push(ConfirmDialog::new(
            tr!("remote-delete-title"),
            &message,
            tr!("common-delete"),
            delete_cmd(paths),
        )),
        RemotePrompt::Name { target, name } => {
            app.gui.dialogs.push(NameDialog { target, name, error: None, focus: true })
        }
    }
}

// ---- sidebar ------------------------------------------------------------------------------

/// The sidebar's "Network" section: saved connections (dot = connection state) and
/// "Connect to server…".
pub fn sidebar_section(
    ui: &mut Ui,
    collapsed: &mut BTreeSet<String>,
    current: &Location,
    pane: PaneId,
    cmds: &mut Vec<Cmd>,
) {
    // `name` keys the collapsed state (settings); the header shows the translation.
    let name = "Network";
    let mut open = !collapsed.contains(name);
    collapsible_header(ui, tr!("remote-network"), &mut open);
    if open {
        collapsed.remove(name);
    } else {
        collapsed.insert(name.to_string());
        return;
    }
    let p = palette(ui.ctx());
    let here = match current {
        Location::Remote(u) => RemoteUri::parse(u).ok().map(|u| u.conn_key()),
        _ => None,
    };
    for c in svc::connections() {
        let Some(uri) = c.parsed() else { continue };
        let state = svc::conn_state(&uri);
        let (color, state_text) = match &state {
            ConnState::Connected => (Some(p.success), tr!("remote-state-connected").to_string()),
            ConnState::Connecting => (Some(p.warning), tr!("remote-state-connecting").to_string()),
            ConnState::Failed(m) => (Some(p.danger), tr!("remote-state-failed", error = m)),
            ConnState::Idle => (None, tr!("remote-state-idle").to_string()),
        };
        let icon = match uri.scheme {
            Scheme::S3 => ph::CLOUD,
            Scheme::WebDav | Scheme::WebDavs => ph::GLOBE,
            Scheme::Fake => ph::FLASK,
            _ => ph::HARD_DRIVES,
        };
        let active = here.as_deref() == Some(uri.conn_key().as_str());
        let r = NavRow { icon, label: &c.name, active, icon_color: color, usage: None }.show(ui);
        let r = r.on_hover_text(format!("{}\n{} · {}", uri.display(), uri.scheme.label(), state_text));
        if r.clicked() {
            cmds.push(Cmd::FocusPane(pane));
            cmds.push(Cmd::Navigate { pane, location: Location::Remote(uri.display()), select: None });
        }
        if r.middle_clicked() {
            cmds.push(Cmd::OpenInNewTab { location: Location::Remote(uri.display()), activate: false });
        }
        let saved = svc::is_saved(&c.id);
        crate::ui::native::popup::context_menu(&r, |ui| {
            ui.set_width(220.0);
            if menu_item(ui, Some(ph::BROWSERS), tr!("remote-open-new-tab"), None, true).clicked() {
                cmds.push(Cmd::Remote(RemoteCmd::Open { uri: uri.display(), new_tab: true }));
                ui.close();
            }
            if menu_item(ui, Some(ph::PENCIL_SIMPLE), tr!("remote-edit-connection"), None, saved).clicked() {
                cmds.push(Cmd::Remote(RemoteCmd::ConnectDialog { uri: None, edit: Some(c.id.clone()), note: None }));
                ui.close();
            }
            let connected = matches!(state, ConnState::Connected | ConnState::Failed(_));
            if menu_item(ui, Some(ph::PLUGS), tr!("remote-disconnect"), None, connected).clicked() {
                cmds.push(Cmd::Remote(RemoteCmd::Disconnect(uri.display())));
                ui.close();
            }
            if menu_item(ui, Some(ph::CLIPBOARD_TEXT), tr!("remote-copy-address"), None, true).clicked() {
                cmds.push(Cmd::CopyText(uri.display()));
                ui.close();
            }
            if saved {
                menu_separator(ui);
                if menu_item(ui, Some(ph::TRASH), tr!("remote-remove"), None, true).clicked() {
                    cmds.push(Cmd::Remote(RemoteCmd::Remove(c.id.clone())));
                    ui.close();
                }
            }
        });
    }
    let r =
        NavRow { icon: ph::PLUS, label: tr!("remote-connect"), active: false, icon_color: Some(p.accent), usage: None }
            .show(ui)
            .on_hover_text(tr!("remote-connect-tip"));
    if r.clicked() {
        cmds.push(Cmd::Remote(RemoteCmd::ConnectDialog { uri: None, edit: None, note: None }));
    }
}

// ---- dialogs ------------------------------------------------------------------------------

fn field_label(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(11.5).color(p.text_tertiary));
    ui.add_space(2.0);
}

fn text_field(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> egui::Response {
    ui.add(
        TextEdit::singleline(value)
            .password(password)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 7)),
    )
}

/// "Connect to server…" over a [`ConnectForm`].
pub struct ConnectDialog {
    form: ConnectForm,
    focus: bool,
}

impl Dialog for ConnectDialog {
    fn title(&self) -> &str {
        if self.form.edit.is_some() { tr!("remote-dialog-edit-title") } else { tr!("remote-dialog-connect-title") }
    }
    fn icon(&self) -> Option<&'static str> {
        Some(ph::GLOBE)
    }
    fn width(&self) -> f32 {
        480.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let focus = &mut self.focus;
        let f = &mut self.form;
        let p = palette(ui.ctx());
        if let Some(n) = &f.note {
            ui.label(RichText::new(n).color(p.warning));
            ui.add_space(4.0);
        }
        // Protocol.
        field_label(ui, tr!("remote-protocol"));
        ui.horizontal(|ui| {
            let mut proto = f.proto;
            crate::ui::settings::widgets::segmented(ui, &mut proto, PROTOCOLS);
            if proto != f.proto {
                f.proto = proto;
                if !f.auth_options().iter().any(|(a, _)| *a == f.auth) {
                    f.auth = AuthKind::Password;
                }
            }
            if matches!(f.proto, Scheme::WebDav | Scheme::Ftp) {
                ui.add_space(10.0);
                ui.checkbox(&mut f.tls, if f.proto == Scheme::WebDav { "HTTPS" } else { "TLS (FTPS)" });
            }
        });
        let s3 = f.proto == Scheme::S3;
        // Server / bucket and port.
        ui.columns(2, |cols| {
            let (a, b) = cols.split_at_mut(1);
            let ui = &mut a[0];
            field_label(ui, if s3 { tr!("remote-bucket") } else { tr!("remote-server") });
            let r = text_field(ui, &mut f.host, if s3 { "my-bucket" } else { "example.com" }, false);
            if *focus {
                r.request_focus();
                *focus = false;
            }
            let ui = &mut b[0];
            if s3 {
                field_label(ui, tr!("remote-region"));
                text_field(ui, &mut f.region, "us-east-1", false);
            } else {
                field_label(ui, tr!("remote-port"));
                let def = f.scheme().default_port().to_string();
                text_field(ui, &mut f.port, &def, false);
            }
        });
        if s3 {
            field_label(ui, tr!("remote-endpoint"));
            text_field(ui, &mut f.endpoint, "https://s3.amazonaws.com, http://127.0.0.1:9000 …", false);
        }
        // Sign-in.
        field_label(ui, tr!("remote-sign-in-with"));
        let mut auth = f.auth;
        crate::ui::settings::widgets::segmented(ui, &mut auth, &f.auth_options());
        f.auth = auth;
        if f.auth != AuthKind::Anonymous {
            ui.columns(2, |cols| {
                let (a, b) = cols.split_at_mut(1);
                let ui = &mut a[0];
                field_label(ui, if s3 { tr!("remote-access-key") } else { tr!("remote-user") });
                let hint = match f.proto {
                    Scheme::S3 => "AKIA…",
                    Scheme::Smb => tr!("remote-user-hint-smb"),
                    _ => tr!("remote-user-hint"),
                };
                text_field(ui, &mut f.user, hint, false);
                let ui = &mut b[0];
                let (label, hint) = match (f.auth, s3) {
                    (_, true) => (tr!("remote-secret-key"), ""),
                    (AuthKind::KeyFile, _) => (tr!("remote-key-passphrase"), tr!("remote-key-passphrase-hint")),
                    (AuthKind::Agent, _) => ("", ""),
                    _ => (tr!("remote-password"), ""),
                };
                if !label.is_empty() {
                    field_label(ui, label);
                    let keep = f.edit.as_ref().is_some_and(|c| c.has_secret);
                    let hint = if keep && f.secret.is_empty() { tr!("remote-secret-unchanged") } else { hint };
                    text_field(ui, &mut f.secret, hint, true);
                }
            });
            if f.auth == AuthKind::KeyFile {
                field_label(ui, tr!("remote-key-file-label"));
                text_field(ui, &mut f.key_file, r"C:\Users\me\.ssh\id_ed25519", false);
            }
            if f.auth == AuthKind::Agent {
                ui.add_space(4.0);
                let note = if cfg!(windows) { tr!("remote-agent-note") } else { tr!("remote-agent-note-unix") };
                ui.label(RichText::new(note).size(11.5).color(p.text_tertiary));
            }
        }
        let (label, hint) = match f.proto {
            Scheme::S3 => (tr!("remote-prefix"), "/"),
            Scheme::Smb => (tr!("remote-share-folder"), "/share/folder"),
            _ => (tr!("remote-folder"), "/"),
        };
        field_label(ui, label);
        text_field(ui, &mut f.path, hint, false);
        // Save.
        ui.add_space(10.0);
        ui.checkbox(&mut f.save, tr!("remote-save"));
        if f.save {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.add(
                    TextEdit::singleline(&mut f.name)
                        .hint_text(tr!("remote-name-hint"))
                        .desired_width(220.0)
                        .margin(Margin::symmetric(8, 5)),
                );
            });
            if f.auth != AuthKind::Anonymous && f.auth != AuthKind::Agent {
                ui.horizontal(|ui| {
                    ui.add_space(24.0);
                    ui.checkbox(
                        &mut f.remember,
                        if cfg!(target_os = "linux") { tr!("remote-remember-linux") } else { tr!("remote-remember") },
                    );
                });
            }
        }
        if let Some(e) = &f.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(p.danger));
        }
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("remote-connect-button"), tr!("common-cancel"));
        if ok || enter {
            match f.build() {
                Ok(uri) => {
                    ctx.cmds.push(f.connect(uri));
                    return false;
                }
                Err(e) => f.error = Some(e),
            }
        }
        !cancel
    }
}

/// "Trust this host key?" — shown when an SSH server presents an unknown or changed key.
pub struct HostKeyDialog {
    ask: HostKeyAsk,
}

impl Drop for HostKeyDialog {
    fn drop(&mut self) {
        self.ask.answer(false); // closed with Esc / click outside: do not trust
    }
}

impl Dialog for HostKeyDialog {
    fn title(&self) -> &str {
        match self.ask.prompt.status {
            HostKeyStatus::Changed { .. } => tr!("remote-hostkey-changed-title"),
            _ => tr!("remote-hostkey-trust-title"),
        }
    }
    fn icon(&self) -> Option<&'static str> {
        Some(match self.ask.prompt.status {
            HostKeyStatus::Changed { .. } => ph::SHIELD_WARNING,
            _ => ph::FINGERPRINT,
        })
    }
    fn width(&self) -> f32 {
        500.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let host = if self.ask.prompt.port == 22 {
            self.ask.prompt.host.clone()
        } else {
            format!("{}:{}", self.ask.prompt.host, self.ask.prompt.port)
        };
        let changed = matches!(self.ask.prompt.status, HostKeyStatus::Changed { .. });
        if changed {
            ui.label(RichText::new(tr!("remote-hostkey-changed-body", host = &host)).color(p.danger));
        } else {
            ui.label(RichText::new(tr!("remote-hostkey-first-body", host = &host)).color(p.text_secondary));
        }
        ui.add_space(10.0);
        let card = egui::Frame::new()
            .fill(p.bg_surface2)
            .corner_radius(egui::CornerRadius::same(crate::theme::tokens::radius::CARD))
            .inner_margin(Margin::symmetric(12, 10));
        card.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&self.ask.prompt.algorithm).size(11.5).color(p.text_tertiary));
            ui.add_space(2.0);
            ui.label(
                RichText::new(&self.ask.prompt.fingerprint).font(egui::FontId::monospace(12.5)).color(p.text_primary),
            );
            if let HostKeyStatus::Changed { previous } = &self.ask.prompt.status {
                ui.add_space(8.0);
                ui.label(RichText::new(tr!("remote-hostkey-previous")).size(11.5).color(p.text_tertiary));
                ui.label(RichText::new(previous).font(egui::FontId::monospace(12.0)).color(p.text_secondary));
            }
        });
        let (ok, cancel) = footer(
            ui,
            if changed { tr!("remote-hostkey-trust-new") } else { tr!("remote-hostkey-trust") },
            tr!("common-cancel"),
        );
        if ok {
            self.ask.answer(true);
            return false;
        }
        if cancel {
            self.ask.answer(false);
            return false;
        }
        true
    }
}

/// New folder / rename on a server.
pub struct NameDialog {
    target: NameTarget,
    name: String,
    error: Option<String>,
    focus: bool,
}

impl Dialog for NameDialog {
    fn title(&self) -> &str {
        if self.is_new() { tr!("remote-new-folder-title") } else { tr!("common-rename") }
    }
    fn icon(&self) -> Option<&'static str> {
        Some(if self.is_new() { ph::FOLDER_PLUS } else { ph::PENCIL_SIMPLE })
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let r = text_field(ui, &mut self.name, tr!("remote-name"), false);
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        if let Some(e) = &self.error {
            ui.add_space(4.0);
            ui.label(RichText::new(e).color(p.danger));
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) =
            footer(ui, if self.is_new() { tr!("remote-create") } else { tr!("common-rename") }, tr!("common-cancel"));
        if ok || enter {
            match name_submit(&self.target, &self.name) {
                Err(e) => {
                    self.error = Some(e);
                    return true;
                }
                Ok(cmd) => ctx.cmds.extend(cmd),
            }
            return false;
        }
        !cancel
    }
}

impl NameDialog {
    fn is_new(&self) -> bool {
        matches!(self.target, NameTarget::NewFolder(_))
    }
}
