//! The view of batch rename on the `rename` planner (`FrontendCmd::MetaDialog(MetaDialog::BatchRename)`) — Phase T3.
//!
//! The summary (to rename, problems, cycles), the live old → new preview (unchanged rows
//! muted, conflicts in the danger / warning colours with a short reason on the right),
//! the mode, the rule page's fields and the buttons; on the Presets page the list shows
//! the presets, in the "one name per line" mode a hint.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::rename::Status;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use soyuz_appcore::models::batch_rename::Mode;

use crate::input::dialogs::batch_rename::{Page, State, status_label};
use crate::theme::Role;
use crate::widgets::ListRow;

/// Preferred width (cells).
pub const WIDTH: u16 = 100;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = WIDTH;
    d.hints = tr!("tui-ops-batch-hints").to_string();
    let Some(m) = &s.model else { return d };
    let (ok, bad) = m.counts();
    let mut summary = vec![tr!("dialogs-rename-to-rename", count = ok)];
    if bad > 0 {
        summary.push(tr!("dialogs-rename-problems", count = bad));
    }
    if m.cycles > 0 {
        summary.push(tr!("dialogs-rename-cycles", count = m.cycles));
    }
    d.body = vec![summary.join(" · ")];
    if m.mode == Mode::Editor {
        d.body.push(tr!("dialogs-rename-editor-hint").to_string());
    } else if s.page == Page::Presets {
        d.list = s.presets().into_iter().map(|(label, _, _)| ListRow::new(safe(&label))).collect();
        d.list_state = s.list.clone();
        return d;
    }
    d.list = m
        .items
        .iter()
        .zip(&m.rows)
        .map(|(it, r)| {
            let new = if r.new_name.is_empty() { "\u{2014}".to_string() } else { safe(&r.new_name) };
            ListRow {
                text: format!("{} \u{2192} {new}", safe(&it.name())),
                right: status_label(&r.status).to_string(),
                role: match r.status {
                    Status::Ok => None,
                    Status::Unchanged => Some(Role::Muted),
                    Status::Exists => Some(Role::Warning),
                    Status::Invalid(_) | Status::Duplicate | Status::Error(_) => Some(Role::Danger),
                },
            }
        })
        .collect();
    d.list_state = s.list.clone();
    d
}
