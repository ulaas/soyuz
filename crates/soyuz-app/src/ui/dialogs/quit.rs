//! Close confirmation while file operations are pending: closing the window cancels
//! them, so ask first ("Keep open" / "Quit when finished" / "Cancel and quit"). The
//! choices are the core's [`QuitModel`]; this file draws it.

use egui::{Align, Layout, RichText, Ui};

use super::DialogCtx;
use crate::models::dialogs::{QuitChoice, QuitModel};
use crate::ui::widgets::{Btn, pill_button};

/// Draw the quit confirmation. Returns `false` to close.
pub(crate) fn show(ui: &mut Ui, m: &mut QuitModel, ctx: &mut DialogCtx<'_>) -> bool {
    let p = crate::theme::palette(ui.ctx());
    ui.label(RichText::new(tr!("dialogs-quit-body")).color(p.text_secondary));
    ui.add_space(18.0);
    let mut choice = None;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if pill_button(ui, None, tr!("dialogs-quit-cancel-jobs"), Btn::Danger).clicked() {
            choice = Some(QuitChoice::CancelAndQuit);
        }
        if pill_button(ui, None, tr!("dialogs-quit-when-finished"), Btn::Primary).clicked() {
            choice = Some(QuitChoice::QuitWhenFinished);
        }
        if pill_button(ui, None, tr!("dialogs-quit-keep-open"), Btn::Secondary).clicked() {
            choice.get_or_insert(QuitChoice::KeepOpen);
        }
    });
    // Enter picks the safe default.
    if super::enter_default(ui) {
        choice = Some(QuitChoice::QuitWhenFinished);
    }
    match choice {
        Some(c) => {
            if let Some(cmd) = m.choose(c) {
                ctx.cmds.push(cmd);
            }
            false
        }
        None => true,
    }
}
