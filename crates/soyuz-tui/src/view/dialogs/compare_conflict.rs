//! The view of the dialog for the two-way sync conflicts of the compare screen
//! (`TuiDialog::CompareConflict`): "Resolve conflicts (2 / 5)", the item and why it
//! conflicts, both sides (size and date, or "Deleted"), the focused choice's explanation,
//! the "Apply to all" box and the choices.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::compare::{RESOLUTIONS, resolution_tip};
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::compare_conflict::{KEEP_NEWER, State};
use crate::view::compare::meta;

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    let title = format!(
        "{} ({} / {})",
        tr!("tools-compare-conflicts-title"),
        (s.pos + 1).min(s.rows.len().max(1)),
        s.rows.len()
    );
    let mut body = Vec::new();
    if let (Some(model), Some(i)) = (&s.model, s.row()) {
        let m = model.borrow();
        if let Some(row) = m.rows().get(i) {
            let kind = row.conflict.map(|k| k.label()).unwrap_or_default();
            body.push(safe(&format!("{} · {kind}", row.rel.to_string_lossy())));
            let deleted = tr!("tools-compare-meta-deleted");
            body.push(safe(&tr!("tools-compare-meta-left", meta = meta(core, row.left.as_ref(), deleted))));
            body.push(safe(&tr!("tools-compare-meta-right", meta = meta(core, row.right.as_ref(), deleted))));
            let tip = match s.form.focused_button() {
                Some(KEEP_NEWER) => Some(tr!("tools-compare-keep-newer-tip")),
                Some(b) => RESOLUTIONS.get(b).map(|r| resolution_tip(row, *r)),
                None => None,
            };
            if let Some(t) = tip {
                body.push(safe(t));
            }
        }
    }
    DialogVm {
        body,
        hints: tr!("tui-compare-conflict-hints").into(),
        width: 72,
        ..DialogVm::from_form(&title, &s.form)
    }
}
