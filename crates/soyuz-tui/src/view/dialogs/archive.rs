//! The view of the archive dialogs (`TuiDialog::Archive`): the password prompt, Extract
//! to…, Compress… and the rename of a zip entry, with the GUI's titles and wording.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::archive::{Kind, State, names_encrypted};
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let (title, body, hints) = match &s.kind {
        Kind::Password { archive, .. } => {
            let name = soyuz_core::entry::display_name(archive);
            (
                tr!("views-archive-password-title"),
                vec![tr!("views-archive-password-body", name = &name)],
                tr!("tui-locations-password-hints"),
            )
        }
        Kind::Extract(f) => (tr!("views-archive-extract"), vec![f.summary()], tr!("tui-locations-extract-hints")),
        Kind::Create(f) => {
            let n = f.sources.len();
            let what =
                if n == 1 { soyuz_core::entry::display_name(&f.sources[0]) } else { tr!("common-items", count = n) };
            let mut body = vec![what];
            if names_encrypted(f) {
                body.push(tr!("views-archive-names-encrypted").to_string());
            }
            (tr!("views-archive-compress"), body, tr!("tui-locations-compress-hints"))
        }
        Kind::Rename { archive, from, is_dir } => {
            let kind = if *is_dir { "folder" } else { "file" };
            let body = tr!(
                "views-archive-rename-body",
                kind = kind,
                name = from.as_str(),
                archive = soyuz_core::entry::display_name(archive)
            );
            (tr!("views-archive-rename-title"), vec![body], tr!("tui-requests-confirm-hints"))
        }
    };
    DialogVm {
        body: body.iter().map(|l| safe(l)).collect(),
        hints: hints.into(),
        width: 64,
        // Extract to…: the subfolders completing the destination (the picked one marked).
        list: s.names.iter().map(|n| ListRow::new(format!("{}{}", safe(n), std::path::MAIN_SEPARATOR))).collect(),
        list_state: s.list.clone(),
        ..DialogVm::from_form(title, &s.form)
    }
}
