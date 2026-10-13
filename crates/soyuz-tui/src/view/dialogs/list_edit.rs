//! The view of the list editor (`TuiDialog::ListEdit`): the items with the cursor, the
//! item field, the buttons and the keys.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::list_edit::State;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let list = if s.items.is_empty() {
        vec![ListRow { role: Some(crate::theme::Role::Muted), ..ListRow::new(tr!("tui-settings-list-empty")) }]
    } else {
        s.items.iter().map(|i| ListRow::new(crate::dialog::safe(i))).collect()
    };
    DialogVm {
        list,
        list_state: s.list.clone(),
        hints: tr!("tui-settings-list-hint").into(),
        width: 64,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
