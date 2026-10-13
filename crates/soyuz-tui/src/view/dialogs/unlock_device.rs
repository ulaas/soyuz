//! The view of the unlock dialog (`TuiDialog::UnlockDevice`, agent B): the intro, the
//! passphrase field (bullets unless revealed), then "Unlocking…" while the worker runs or
//! (BitLocker) the recovery key note; a wrong passphrase is the form's error.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::devices::is_bitlocker;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::unlock_device::State;

/// Preferred width (cells).
pub const WIDTH: u16 = 64;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    view(s)
}

fn view(s: &State) -> DialogVm {
    let bitlocker = is_bitlocker(&s.kind);
    let mut d = DialogVm::from_form(&s.title, &s.form);
    d.width = WIDTH;
    let intro = if bitlocker { tr!("dialogs-unlock-intro-bitlocker") } else { tr!("dialogs-unlock-intro") };
    d.body = vec![intro.to_string()];
    if s.working() {
        d.body.push(tr!("dialogs-unlock-working").to_string());
    } else if bitlocker && s.form.error.is_none() {
        d.body.push(tr!("dialogs-unlock-recovery-note").to_string());
    }
    d.hints = tr!("tui-devices-unlock-hints").to_string();
    d
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::state::pane::PaneId;

    use super::*;
    use crate::widgets::form::FieldVmKind;

    #[test]
    fn bitlocker_shows_the_recovery_note() {
        let s = State::new("o".into(), "Backup".into(), "BitLocker".into(), PaneId(1));
        let d = view(&s);
        assert!(d.title.contains("Backup"));
        assert_eq!(d.body.len(), 2);
        assert!(d.body[1].contains("48-digit"));
        assert_eq!(d.buttons.len(), 3);
        let s = State::new("o".into(), "Stick".into(), "crypto_LUKS".into(), PaneId(1));
        let d = view(&s);
        assert_eq!(d.body.len(), 1);
        assert!(matches!(&d.fields[0].kind, FieldVmKind::Text { text, .. } if text.is_empty()));
    }
}
