//! The view of the shortcut recorder (`TuiDialog::Keybind`): what is recorded so far, what
//! those keys are already used for, the buttons for the mouse and the rules.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;
use soyuz_core::config::format_sequence;

use super::DialogVm;
use crate::input::dialogs::keybind::{State, used_by};

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let keys = if s.chords.is_empty() {
        tr!("tui-settings-record-waiting").to_string()
    } else {
        tr!("tui-settings-record-keys", keys = format_sequence(&s.chords))
    };
    let used = used_by(core, s);
    DialogVm {
        body: vec![keys],
        error: (!used.is_empty()).then(|| tr!("tui-settings-record-used", actions = used.join(", "))),
        hints: tr!("tui-settings-record-hint").into(),
        width: 60,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
