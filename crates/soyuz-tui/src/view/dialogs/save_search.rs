//! The view of the dialog for the name of a saved search (`TuiDialog::SaveSearch`): what
//! saving does, the name field and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::save_search::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: vec![tr!("tui-search-save-body").into()],
        hints: tr!("tui-search-save-hints").into(),
        width: 60,
        ..DialogVm::from_form(tr!("views-search-save-tip"), &s.form)
    }
}
