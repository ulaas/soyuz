//! The view of a confirmation before a command (`FrontendCmd::Confirm`): the message,
//! the confirm and Cancel buttons, drawn as an alert.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::confirm::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: s.model.as_ref().map(|m| vec![safe(&m.message)]).unwrap_or_default(),
        hints: tr!("tui-requests-confirm-hints").into(),
        alert: true,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
