//! The view of the dialog for the search bar's size filter (`TuiDialog::SearchSize`): the
//! minimum and maximum fields, the presets and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::search_size::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: vec![tr!("tui-search-size-body").into()],
        hints: tr!("tui-search-size-hints").into(),
        width: 64,
        ..DialogVm::from_form(tr!("views-search-size-tip"), &s.form)
    }
}
