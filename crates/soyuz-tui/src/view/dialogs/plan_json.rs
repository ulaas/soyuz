//! The view of the dialog for the JSON arguments of an AI plan card
//! (`TuiDialog::PlanJson`): the visible part of the text (scrolled to the cursor, which
//! shows while the text has the keyboard), the validation message, the buttons and the
//! key hints. The arguments come from the model or an MCP client: untrusted, made safe.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::plan_json::{State, WIDTH, dims, hints};

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let (w, h) = dims(s.screen, s.form.error.is_some());
    let (lines, (row, col)) = s.area.view(&s.text, None, w, h);
    let mut buttons = s.form.button_vms();
    if s.in_text {
        for b in &mut buttons {
            b.focused = false;
        }
    }
    DialogVm {
        title: safe(&tr!("tui-ai-json-title", tool = s.tool.as_str())),
        text: lines.iter().map(|l| safe(l)).collect(),
        text_cursor: s.in_text.then_some((row, col)),
        buttons,
        hints: hints(),
        width: WIDTH,
        ..DialogVm::from_form("", &s.form)
    }
}
