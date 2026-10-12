//! The view of "Copy to" / "Move to" (`DialogModel::Transfer`): the message, the folder
//! completions of the typed destination (the picked one highlighted), the destination
//! field and Cancel / Copy (Move).

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::Transfer(t) = m else { return crate::dialog::fallback(m) };
    DialogVm {
        body: vec![safe(&t.message())],
        list: ui.names.iter().map(|n| ListRow::new(format!("{}{}", safe(n), std::path::MAIN_SEPARATOR))).collect(),
        list_state: ui.list.clone(),
        hints: tr!("tui-dialogs-hint-transfer").to_string(),
        ..DialogVm::from_form(m.title(), &ui.form)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use soyuz_appcore::models::dialogs::TransferModel;

    use super::*;

    #[test]
    fn message_completions_and_field() {
        let m: DialogModel = TransferModel::new(vec![PathBuf::from("a.txt")], PathBuf::from("dest"), false).into();
        let mut ui = crate::input::dialogs::transfer::ui(&m);
        ui.names = vec!["sub".into()];
        let d = vm(&m, &ui);
        assert!(d.body[0].contains("a.txt"), "{:?}", d.body);
        assert_eq!(d.list.len(), 1);
        assert_eq!(d.fields.len(), 1);
        assert_eq!(d.buttons[1].label, tr!("common-copy"));
    }
}
