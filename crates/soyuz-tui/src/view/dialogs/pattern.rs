//! The view of select by pattern (`FrontendCmd::SelectByPattern`): the hint, the pattern
//! field, the "add" checkbox and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::pattern::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: vec![tr!("dialogs-pattern-hint").into()],
        hints: tr!("tui-requests-pattern-hints").into(),
        width: 60,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
