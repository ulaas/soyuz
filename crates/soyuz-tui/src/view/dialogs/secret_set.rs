//! The view of "Set secret…" (`TuiDialog::SecretSet`, also drawn inside the table
//! editor's element form): where the secret goes, the masked field and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::secret_set::{State, body};

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: vec![body(s)],
        hints: tr!("tui-tables-secret-hint").into(),
        width: 64,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
