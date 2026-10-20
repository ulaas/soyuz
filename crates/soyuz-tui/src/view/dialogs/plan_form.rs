//! The view of the dialog for editing an AI plan card's items and destination
//! (`TuiDialog::PlanForm`): the hint, the items with check boxes, the destination field
//! and the buttons. Paths come from the model or an MCP client: untrusted, made safe.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::plan_form::State;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let list = s
        .plan
        .list
        .iter()
        .flat_map(|(_, items)| items.iter())
        .map(|(path, keep)| {
            let shown = soyuz_fs::strip_verbatim(path).to_string_lossy().into_owned();
            ListRow::new(format!("[{}] {}", if *keep { 'x' } else { ' ' }, safe(&shown)))
        })
        .collect();
    let hints = if s.form.fields.is_empty() { tr!("tui-ai-form-hints-space") } else { tr!("tui-ai-form-hints") };
    DialogVm {
        body: vec![tr!("ai-card-form-items").into()],
        list,
        list_state: s.list.clone(),
        hints: hints.into(),
        width: 80,
        ..DialogVm::from_form(tr!("tui-ai-form-title"), &s.form)
    }
}
