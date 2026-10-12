//! The view of script approval cards (`DialogModel::ScriptApproval`): what happens, every
//! planned change as a list (trash / delete in the danger colour; summaries come from
//! disk: `crate::dialog::safe`), the note and Cancel / Run. A destructive plan gets the
//! warning border.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;
use soyuz_jobs::JobKindBrief;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::theme::Role;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::ScriptApproval(s) = m else { return crate::dialog::fallback(m) };
    let list = s
        .jobs
        .iter()
        .map(|j| {
            let danger = matches!(j.kind.brief(), JobKindBrief::Trash | JobKindBrief::Delete);
            ListRow { role: Some(if danger { Role::Danger } else { Role::Text }), ..ListRow::new(safe(&j.summary)) }
        })
        .collect();
    DialogVm {
        body: vec![tr!("automation-approval-intro").to_string(), tr!("automation-approval-note").to_string()],
        list,
        list_state: ui.list.clone(),
        hints: tr!("tui-dialogs-hint-script").to_string(),
        alert: s.destructive,
        ..DialogVm::from_form(m.title(), &ui.form)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::dialogs::approval_script;

    #[test]
    fn lists_every_change() {
        let m = approval_script::tests::model(3);
        let d = vm(&m, &approval_script::ui(&m));
        assert!(d.title.contains("Tidy"), "{}", d.title);
        assert_eq!(d.list.len(), 3);
        assert_eq!(d.list[0].role, Some(Role::Danger));
        assert!(d.alert);
        assert!(d.buttons[1].focused && d.buttons[1].default);
    }
}
