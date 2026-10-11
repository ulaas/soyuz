//! "Unlock volume": asks for the passphrase of a locked BitLocker / LUKS container in
//! the sidebar's Devices section (Linux, UDisks2 `Encrypted.Unlock`). The unlock runs on
//! a worker thread ([`crate::ui::sidebar::unlock_device`]); a wrong passphrase keeps the
//! dialog open with an error, success closes it and the volume is mounted and opened.
//! The passphrase is never logged; its buffers are zeroized after use.

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{Align, FontId, Key, Layout, Margin, RichText, TextEdit, Ui};
use zeroize::{Zeroize, Zeroizing};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::state::pane::PaneId;
use crate::theme::{icons::ph, palette};

pub struct UnlockVolumeDialog {
    object: String,
    name: String,
    /// `BitLocker` / `crypto_LUKS`, for the hint.
    kind: String,
    pane: PaneId,
    title: String,
    pass: String,
    reveal: bool,
    focus: bool,
    error: Option<String>,
    pending: Option<Receiver<Result<(), String>>>,
}

impl UnlockVolumeDialog {
    pub fn new(object: String, name: String, kind: String, pane: PaneId) -> Self {
        let title = tr!("dialogs-unlock-title", name = name.clone());
        UnlockVolumeDialog {
            object,
            name,
            kind,
            pane,
            title,
            pass: String::new(),
            reveal: false,
            focus: true,
            error: None,
            pending: None,
        }
    }

    fn bitlocker(&self) -> bool {
        is_bitlocker(&self.kind)
    }
}

fn is_bitlocker(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("bitlocker") || kind.eq_ignore_ascii_case("crypto_bitlk")
}

/// A BitLocker recovery key as typed: 48 digits in eight dash-separated groups (spaces
/// and missing dashes are tolerated); `None` when the text is not one.
fn recovery_key(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    if digits.len() != 48 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let groups: Vec<&str> = (0..8).map(|i| &digits[i * 6..i * 6 + 6]).collect();
    Some(groups.join("-"))
}

impl Drop for UnlockVolumeDialog {
    fn drop(&mut self) {
        self.pass.zeroize();
    }
}

impl Dialog for UnlockVolumeDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::LOCK_KEY)
    }

    fn width(&self) -> f32 {
        440.0
    }

    fn show(&mut self, ui: &mut Ui, _ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(())) => return false,
                Ok(Err(e)) => {
                    self.error = Some(e);
                    self.pending = None;
                    self.focus = true;
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
                Err(TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        let working = self.pending.is_some();
        let intro = if self.bitlocker() { tr!("dialogs-unlock-intro-bitlocker") } else { tr!("dialogs-unlock-intro") };
        ui.label(RichText::new(intro).color(p.text_secondary));
        ui.add_space(10.0);
        let id = ui.make_persistent_id(("soyuz.unlock.pass", &self.object));
        let hint = if self.bitlocker() { tr!("dialogs-unlock-hint-bitlocker") } else { tr!("dialogs-unlock-hint") };
        let mut submit = false;
        ui.horizontal(|ui| {
            let w = ui.available_width() - 36.0;
            let r = ui.add_enabled(
                !working,
                TextEdit::singleline(&mut self.pass)
                    .id(id)
                    .password(!self.reveal)
                    .hint_text(hint)
                    .desired_width(w)
                    .margin(Margin::symmetric(10, 7)),
            );
            if self.focus && !working {
                r.request_focus();
                self.focus = false;
            }
            if r.changed() {
                self.error = None;
            }
            submit = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let (glyph, tip) = if self.reveal {
                (ph::EYE_SLASH, tr!("dialogs-unlock-hide"))
            } else {
                (ph::EYE, tr!("dialogs-unlock-show"))
            };
            if crate::ui::widgets::icon_toggle(ui, glyph, tip, true, self.reveal).clicked() {
                self.reveal = !self.reveal;
            }
        });
        ui.add_space(6.0);
        if let Some(e) = &self.error {
            ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
        } else if working {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                crate::ui::widgets::spinner(ui, r.center(), 5.0, p.accent);
                ui.label(
                    RichText::new(tr!("dialogs-unlock-working"))
                        .font(FontId::proportional(11.5))
                        .color(p.text_secondary),
                );
            });
        } else if self.bitlocker() {
            ui.label(
                RichText::new(tr!("dialogs-unlock-recovery-note"))
                    .font(FontId::proportional(11.5))
                    .color(p.text_tertiary),
            );
        }
        ui.add_space(16.0);
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let ok = ui.add_enabled_ui(!working && !self.pass.is_empty(), |ui| {
                button(ui, tr!("dialogs-unlock-button"), Btn::Primary)
            });
            submit |= ok.inner.clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
        });
        if submit && !working && !self.pass.is_empty() {
            // Move the buffer out (no copy); a recovery key is normalized to its dashed form.
            let mut typed = Zeroizing::new(std::mem::take(&mut self.pass));
            let secret = if self.bitlocker()
                && let Some(k) = recovery_key(&typed)
            {
                typed.zeroize();
                Zeroizing::new(k)
            } else {
                typed
            };
            // Drop egui's copy of the text (undo history) too.
            ui.ctx().data_mut(|d| d.remove::<egui::text_edit::TextEditState>(id));
            let (tx, rx) = channel();
            crate::ui::sidebar::unlock_device(&self.object, self.name.clone(), secret, self.pane, tx);
            self.pending = Some(rx);
            self.error = None;
        }
        !cancel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_keys_are_normalized() {
        let k = "123456-234567-345678-456789-567890-678901-789012-890123";
        assert_eq!(recovery_key(k).as_deref(), Some(k));
        assert_eq!(recovery_key(&k.replace('-', "")).as_deref(), Some(k));
        assert_eq!(recovery_key(&k.replace('-', " ")).as_deref(), Some(k));
        assert_eq!(recovery_key("hunter2"), None);
        assert_eq!(recovery_key(&"1".repeat(47)), None);
        assert!(is_bitlocker("BitLocker") && is_bitlocker("crypto_BITLK") && !is_bitlocker("crypto_LUKS"));
    }
}
