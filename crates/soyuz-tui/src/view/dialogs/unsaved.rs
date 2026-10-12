//! The view of "Save changes?" (`FrontendCmd::EditorUnsaved`) — Phase T4: the document
//! (or the documents, when quitting) in the title, the warning, Save / Don't save /
//! Cancel.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::unsaved::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm {
        body: vec![s.body.clone()],
        hints: tr!("tui-editor-unsaved-hints").into(),
        width: 60,
        alert: true,
        ..DialogVm::from_form(&s.title, &s.form)
    }
}
