//! The view of the split / join dialog (`TuiDialog::Split`): the file and its size (split)
//! or the parts and whether a checksum file will verify the result (join), the plan, the
//! progress while it runs and the last run's message; then the part size presets (split),
//! the custom size and output folder fields and the buttons. A bad custom size or a
//! failed run shows as the validation message. While the output folder completes, the
//! list shows its subfolders instead of the presets.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::split::{CUSTOM, Mode, PRESETS, preset_label};
use soyuz_appcore::ops::fileops::names::show_path;
use soyuz_appcore::tr;
use soyuz_core::{SizeStyle, format_size};

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::split::{State, completing};
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let Some(m) = &s.model else { return DialogVm::default() };
    let title = if m.mode == Mode::Split { tr!("tools-split-title") } else { tr!("tools-join-title") };
    let name = soyuz_core::entry::display_name(&m.path);
    let mut d = DialogVm { width: 72, ..DialogVm::from_form(&format!("{title} — {name}"), &s.form) };
    match m.mode {
        Mode::Split => {
            let size = match m.size {
                Some(n) => format_size(n, SizeStyle::Binary),
                None if m.probing() => "…".into(),
                None => tr!("tools-split-unknown-size").into(),
            };
            d.body.push(tr!("tui-split-file", path = safe(&show_path(&m.path)), size = size));
            if let Some(plan) = m.plan_text() {
                d.body.push(format!("→ {plan}"));
            }
            d.list = (0..PRESETS.len())
                .map(|i| {
                    let mark = if i == m.preset { "(•) " } else { "( ) " };
                    ListRow { text: format!("{mark}{}", preset_label(i)), right: String::new(), role: None }
                })
                .collect();
            d.list_state = s.list.clone();
            if m.preset == CUSTOM && m.part_size().is_none() && !m.custom.trim().is_empty() {
                d.error = Some(tr!("tools-split-err-invalid-size").into());
            }
        }
        Mode::Join => match &m.parts {
            Some(Ok((base, parts))) => {
                let name = safe(&soyuz_core::entry::display_name(base));
                d.body.push(tr!("tools-join-plan", count = parts.len(), name = name));
                d.body.push(
                    if m.has_checksum { tr!("tools-join-has-checksum") } else { tr!("tools-join-no-checksum") }.into(),
                );
            }
            Some(Err(e)) => d.error = Some(safe(e)),
            None => d.body.push("…".into()),
        },
    }
    if let (Some((done, total, _)), Some(text)) = (m.progress(), m.progress_text()) {
        let pct =
            (done.saturating_mul(100)).checked_div(total).map(|p| format!("{}% · ", p.min(100))).unwrap_or_default();
        d.body.push(format!("{pct}{text}"));
    }
    match &m.result {
        Some(Ok(msg)) => d.body.push(safe(msg)),
        Some(Err(e)) => d.error = Some(safe(e)),
        None => {}
    }
    if completing(s) {
        d.list = s.names.iter().map(|n| ListRow::new(format!("{}{}", safe(n), std::path::MAIN_SEPARATOR))).collect();
        d.list_state = s.names_list.clone();
    }
    d.hints = if completing(s) {
        tr!("tui-split-complete-hints")
    } else if m.running() {
        tr!("tui-split-running-hints")
    } else if m.finished_ok() {
        tr!("tui-split-done-hints")
    } else if m.mode == Mode::Split {
        tr!("tui-split-hints")
    } else {
        tr!("tui-split-join-hints")
    }
    .into();
    d
}
