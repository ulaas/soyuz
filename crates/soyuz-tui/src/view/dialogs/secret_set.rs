//! The view of "Set secret…" (`TuiDialog::SecretSet`, also drawn inside the table
//! editor's element form): where the secret goes, the masked field and the buttons; after
//! "Generate token" (MCP clients) the token, shown once, and the client snippet.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::secret_set::{State, body, snippet};

/// What the dialog shows.
pub fn vm(core: &AppCore, s: &State) -> DialogVm {
    if let Some(token) = &s.shown {
        let mut lines = vec![tr!("settings-mcp-token-once").to_string()];
        lines.extend(snippet(core.config.ai.mcp.http_port, token).lines().map(safe));
        return DialogVm {
            body: lines,
            hints: tr!("tui-tables-token-hint").into(),
            width: 72,
            ..DialogVm::from_form(&s.title, &s.form)
        };
    }
    let hint = if s.form.buttons.len() > 3 { tr!("tui-tables-secret-hint-mcp") } else { tr!("tui-tables-secret-hint") };
    DialogVm { body: vec![body(s)], hints: hint.into(), width: 64, ..DialogVm::from_form(&s.title, &s.form) }
}
