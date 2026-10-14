//! The view of the About dialog (`TuiDialog::About`): "Soyuz", the version line, the
//! tagline and the license, then the facts as rows (label, value), then the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::about::fact_label;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::about::State;
use crate::view::text;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let version = s.facts.first().map(|(_, v)| v.as_str()).unwrap_or(env!("CARGO_PKG_VERSION"));
    // The version is already in the header.
    let facts = s.facts.iter().skip(1);
    let label_w = facts.clone().map(|(k, _)| text::width(fact_label(k))).max().unwrap_or(0);
    DialogVm {
        body: vec![
            format!("Soyuz · {}", tr!("dialogs-about-version-line", version = version)),
            tr!("dialogs-about-tagline").into(),
            tr!("dialogs-about-license").into(),
            soyuz_appcore::models::about::ffmpeg_notice().into(),
        ],
        list: facts
            .map(|(k, v)| {
                let label = fact_label(k);
                let pad = " ".repeat(label_w - text::width(label));
                ListRow::new(format!("{label}{pad}  {}", safe(v)))
            })
            .collect(),
        list_state: s.list.clone(),
        hints: tr!("tui-workspaces-about-hints").into(),
        width: 80,
        ..DialogVm::from_form(tr!("dialogs-about-title"), &s.form)
    }
}
