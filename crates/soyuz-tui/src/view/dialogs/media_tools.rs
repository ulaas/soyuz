//! The view of the media tools prompt (`DialogModel::MediaTools`, Phase V): the source,
//! the probed input (container, length, size, one line per stream), the dry-run plan
//! (what happens to each stream, estimated size, warnings) above the form; for Convert
//! the presets as a list with the picked one marked.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::models::media_tools::{MediaTool, PlanTone, preset_label};
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::theme::Role;
use crate::widgets::ListRow;

/// Preferred width (cells).
pub const WIDTH: u16 = 84;

fn toned(lines: &[(PlanTone, String)]) -> impl Iterator<Item = String> + '_ {
    lines.iter().map(|(tone, l)| match tone {
        PlanTone::Warning | PlanTone::Danger => format!("! {}", safe(l)),
        PlanTone::Text | PlanTone::Muted => format!("  {}", safe(l)),
    })
}

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::MediaTools(t) = m else { return crate::dialog::titled(m) };
    let mut body = vec![safe(&t.source_line()), format!("{}:", tr!("dialogs-media-input"))];
    body.extend(toned(&t.input_lines()));
    let plan = t.plan_lines();
    if !plan.is_empty() {
        body.push(format!("{}:", tr!("dialogs-media-plan")));
        body.extend(toned(&plan));
    }
    let list = if t.tool == MediaTool::Convert {
        t.presets
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mark = if i == t.preset { "\u{25cf}" } else { " " };
                ListRow {
                    role: (i == t.preset).then_some(Role::Accent),
                    ..ListRow::new(format!("{mark} {} (.{})", preset_label(p.id), p.ext))
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    DialogVm {
        body,
        list,
        list_state: ui.list.clone(),
        hints: tr!("tui-dialogs-hint-media").to_string(),
        width: WIDTH,
        ..DialogVm::from_form(m.title(), &ui.form)
    }
}
