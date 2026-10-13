//! The view of the dialog for editing the new names of an AI plan card
//! (`TuiDialog::PlanNames`): the hint, the items (`old → new`, the cursor row is the one
//! the field edits), the field and the buttons. Names come from the model or an MCP
//! client: untrusted, made safe.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::plan_names::State;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let list = s
        .olds
        .iter()
        .zip(&s.names)
        .map(|(old, new)| ListRow::new(format!("{} \u{2192} {}", safe(old), safe(new))))
        .collect();
    DialogVm {
        body: vec![tr!("ai-card-edit-names-hint").into()],
        list,
        list_state: s.list.clone(),
        hints: tr!("tui-ai-names-hints").into(),
        width: 76,
        ..DialogVm::from_form(tr!("tui-ai-names-title"), &s.form)
    }
}
