//! The view of the remote location dialogs (`TuiDialog::Remote`): the connect form (the
//! sign-in note above it, the SSH agent note), the host-key question (an alert when the
//! key changed: the fingerprint and the key trusted before) and the new folder / rename
//! prompt.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::remote::NameTarget;
use soyuz_appcore::services::remote::AuthKind;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::remote::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    match s {
        State::Connect(c) => {
            let f = &c.form;
            let mut body = Vec::new();
            if let Some(n) = &f.note {
                body.push(safe(n));
            }
            if f.auth == AuthKind::Agent {
                body.push(if cfg!(windows) { tr!("remote-agent-note") } else { tr!("remote-agent-note-unix") }.into());
            }
            let title =
                if f.edit.is_some() { tr!("remote-dialog-edit-title") } else { tr!("remote-dialog-connect-title") };
            DialogVm {
                body,
                hints: tr!("tui-locations-connect-hints").into(),
                width: 72,
                ..DialogVm::from_form(title, &c.ui)
            }
        }
        State::HostKey(h) => {
            let p = &h.ask.prompt;
            let host = h.ask.host_label();
            let changed = h.ask.previous();
            let mut body = vec![if changed.is_some() {
                tr!("remote-hostkey-changed-body", host = &host)
            } else {
                tr!("remote-hostkey-first-body", host = &host)
            }];
            body.push(format!("{}: {}", safe(&p.algorithm), safe(&p.fingerprint)));
            if let Some(prev) = changed {
                body.push(format!("{}: {}", tr!("remote-hostkey-previous"), safe(prev)));
            }
            let title =
                if changed.is_some() { tr!("remote-hostkey-changed-title") } else { tr!("remote-hostkey-trust-title") };
            DialogVm {
                body,
                hints: tr!("tui-locations-hostkey-hints").into(),
                alert: changed.is_some(),
                width: 76,
                ..DialogVm::from_form(title, &h.form)
            }
        }
        State::Name { target, form } => {
            let title = match target {
                NameTarget::NewFolder(_) => tr!("remote-new-folder-title"),
                NameTarget::Rename(_) => tr!("common-rename"),
            };
            DialogVm { hints: tr!("tui-requests-confirm-hints").into(), width: 56, ..DialogVm::from_form(title, form) }
        }
    }
}
