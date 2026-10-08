//! Close confirmation while file operations are pending: closing the window cancels
//! them, so ask first ("Keep open" / "Quit when finished" / "Cancel and quit").

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, Key, Layout, RichText, Ui};

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::cmd::Cmd;
use crate::theme::{icons::ph, palette};

pub struct QuitDialog {
    title: String,
    /// Shared with `QuitState::prompting`; cleared when the dialog goes away.
    open: Arc<AtomicBool>,
}

impl QuitDialog {
    pub fn new(pending: usize, open: Arc<AtomicBool>) -> Self {
        let title = if pending == 1 {
            "A file operation is still running".to_string()
        } else {
            format!("{pending} file operations are still running")
        };
        QuitDialog { title, open }
    }
}

impl Drop for QuitDialog {
    fn drop(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

impl Dialog for QuitDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::WARNING)
    }

    fn width(&self) -> f32 {
        460.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(
            RichText::new(
                "Quitting now cancels them and may leave partially copied files behind. You can keep Soyuz open, \
                 or let it close by itself once they finish.",
            )
            .color(p.text_secondary),
        );
        ui.add_space(18.0);
        let (mut quit, mut wait, mut keep) = (false, false, false);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            quit = button(ui, "Cancel jobs and quit", Btn::Danger).clicked();
            wait = button(ui, "Quit when finished", Btn::Primary).clicked();
            keep = button(ui, "Keep open", Btn::Secondary).clicked();
        });
        // Enter picks the safe default.
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            wait = true;
        }
        if quit || wait {
            ctx.cmds.push(Cmd::Quit { when_idle: wait });
            return false;
        }
        !keep
    }
}
