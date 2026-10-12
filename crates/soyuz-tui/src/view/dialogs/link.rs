//! The views of the new-link prompt (`DialogModel::NewLink`: the kind's hint, the folder
//! the link goes in, the kind / target / name form, Cancel / Create) and of the link
//! dialog (`MetaDialog::Link`: the kind's hint, the target's state, the folder the link
//! goes in, the target picker, the kind / target / relative / name form, Cancel / Create).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use soyuz_appcore::models::link_dialog::TargetState;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::input::dialogs::link::State;
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the core prompt shows.
pub fn model_vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::NewLink(l) = m else { return crate::dialog::fallback(m) };
    let dir = safe(&soyuz_fs::strip_verbatim(&l.dir).display().to_string());
    DialogVm {
        body: vec![l.kind.hint().to_string(), tr!("dialogs-link-in", dir = dir)],
        hints: tr!("tui-dialogs-hint-link").to_string(),
        ..DialogVm::from_form(m.title(), &ui.form)
    }
}

/// Preferred width of the link dialog (cells).
pub const WIDTH: u16 = 84;

/// What the terminal UI dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = WIDTH;
    d.hints = tr!("tui-dialogs-hint-links").to_string();
    let Some(f) = &s.link else { return d };
    let (state, _) = s.target_state();
    let status = match state {
        TargetState::Empty => tr!("dialogs-link-choose-target"),
        TargetState::Checking => tr!("dialogs-link-checking"),
        TargetState::Missing => tr!("dialogs-link-missing"),
        TargetState::File => tr!("dialogs-link-file"),
        TargetState::Dir => tr!("dialogs-conflict-folder"),
    };
    let dir = safe(&soyuz_fs::strip_verbatim(&f.dir).display().to_string());
    let browse = safe(&soyuz_fs::strip_verbatim(&s.browse).display().to_string());
    d.body = vec![
        f.kind.hint().to_string(),
        format!("{}: {status}", tr!("dialogs-link-target")),
        tr!("dialogs-link-created-in", dir = dir),
        format!("{}: {browse}", tr!("dialogs-link-browse")),
    ];
    let rows = s.rows();
    d.list = rows
        .iter()
        .map(|r| ListRow { text: r.label.clone(), right: String::new(), role: r.dir.then_some(Role::Folder) })
        .collect();
    let loading = s.listing.as_ref().map(|(_, bg)| bg.with(|l| l.map(|r| r.as_ref().err().cloned())));
    match loading {
        Some(Some(Some(e))) => d.list.push(ListRow { text: safe(&e), right: String::new(), role: Some(Role::Danger) }),
        Some(None) | None => d.list.push(ListRow::new(tr!("common-loading"))),
        Some(Some(None)) if rows.len() <= 1 => d.list.push(ListRow {
            text: tr!("dialogs-link-empty-folder").into(),
            right: String::new(),
            role: Some(Role::Muted),
        }),
        Some(Some(None)) => {}
    }
    d.list_state = s.list.clone();
    d
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::models::dialogs::{LinkChoice, LinkModel};

    use super::*;

    #[test]
    fn hint_folder_and_form() {
        let m: DialogModel = LinkModel::new(std::env::temp_dir(), None, LinkChoice::Hardlink, Vec::new()).into();
        let d = model_vm(&m, &crate::input::dialogs::link::ui(&m));
        assert_eq!(d.title, tr!("dialogs-link-title"));
        assert_eq!(d.body[0], LinkChoice::Hardlink.hint());
        assert_eq!(d.fields.len(), 3);
        assert_eq!(d.focus, crate::input::dialogs::link::TARGET);
    }
}
