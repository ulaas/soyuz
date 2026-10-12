//! Helpers shared by the dialogs of the terminal UI (`crate::view::dialogs` and
//! `crate::input::dialogs`): [`safe`] makes untrusted text (names, messages) safe to put
//! on the terminal, [`titled`] is the bare view of a core prompt, and [`stale`] tells
//! when a core prompt went away by itself.

use soyuz_appcore::models::dialogs::DialogModel;

use crate::view::dialogs::DialogVm;

/// Text safe to put on the terminal: control characters made visible (names are
/// untrusted and must not smuggle escape sequences).
pub fn safe(s: &str) -> String {
    soyuz_core::naming::reveal_controls(s).chars().map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect()
}

/// Just the title of `m`: what a kind's view shows for a model of another kind (which
/// `crate::view::dialogs::from_core` never hands it).
pub fn titled(m: &DialogModel) -> DialogVm {
    DialogVm { title: safe(m.title()), alert: m.alert(), ..DialogVm::default() }
}

/// Models that went away by themselves (a job prompt answered elsewhere, an MCP request
/// withdrawn or timed out) should close without a key.
pub fn stale(m: &mut DialogModel) -> bool {
    match m {
        DialogModel::Prompt(p) => !p.poll(),
        DialogModel::McpApproval(a) => !a.is_pending(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::models::dialogs::DeleteConfirmModel;
    use soyuz_jobs::DeleteMode;

    use super::*;

    #[test]
    fn safe_hides_escape_sequences() {
        let s = safe("evil\x1b]52;c;x\x07.txt");
        assert!(!s.chars().any(|c| c.is_control()), "{s:?}");
    }

    #[test]
    fn titled_keeps_the_title_and_alert() {
        let m: DialogModel = DeleteConfirmModel::new(vec!["/tmp/a.txt".into()], DeleteMode::Trash).into();
        let v = titled(&m);
        assert!(v.title.contains("a.txt"), "{v:?}");
        assert_eq!(v.alert, m.alert());
    }
}
