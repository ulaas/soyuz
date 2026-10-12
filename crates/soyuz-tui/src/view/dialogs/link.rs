//! The views of the new-link prompt (`DialogModel::NewLink`: the kind's hint, the folder
//! the link goes in, the kind / target / name form, Cancel / Create) and of the link
//! dialog (`MetaDialog::Link`, still a stub: the title and the form).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::input::dialogs::link::State;

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

/// What the terminal UI dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm::from_form(&s.title, &s.form)
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
