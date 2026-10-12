//! The view of bulk attributes and times (`FrontendCmd::MetaDialog(MetaDialog::Attributes)`) — Phase T3.
//!
//! The items' current permissions (when they share them) and how times are typed, the
//! fields (attribute flags, octal mode, times, recursive), the validation message, the
//! buttons; "Collecting folder contents…" while the recursive walk runs.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::attributes::State;

/// Preferred width (cells).
pub const WIDTH: u16 = 76;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = WIDTH;
    if s.collecting.is_some() {
        d.body = vec![tr!("dialogs-attrs-collecting").to_string()];
    } else {
        if let Some(m) = s.mode_line() {
            d.body.push(format!("{}: {m}", tr!("dialogs-mode-permissions")));
        }
        d.body.push(tr!("tui-ops-time-hint").to_string());
        d.body.push(tr!("dialogs-props-undo-note").to_string());
    }
    d.hints = tr!("tui-ops-attrs-hints").to_string();
    d
}
