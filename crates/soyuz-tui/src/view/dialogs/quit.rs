//! The view of quit with running jobs (`DialogModel::Quit`): why quitting now is a bad
//! idea and the three choices, drawn as an alert.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::ModelUi;

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    DialogVm {
        body: vec![tr!("dialogs-quit-body").to_string()],
        hints: tr!("tui-dialogs-hint-quit").to_string(),
        alert: true,
        ..DialogVm::from_form(m.title(), &ui.form)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use soyuz_appcore::models::dialogs::QuitModel;

    use super::*;

    #[test]
    fn shows_the_three_choices() {
        let m: DialogModel = QuitModel::new(1, Arc::new(AtomicBool::new(true))).into();
        let d = vm(&m, &crate::input::dialogs::quit::ui(&m));
        assert_eq!(d.buttons.len(), 3);
        assert!(d.buttons[1].focused && d.buttons[1].default);
        assert!(d.alert && !d.body.is_empty() && !d.hints.is_empty());
    }
}
