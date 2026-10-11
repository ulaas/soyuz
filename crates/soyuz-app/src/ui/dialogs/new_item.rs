//! Small input dialogs for file operations: create a symbolic link / hard link /
//! junction, and "Copy to…" / "Move to…" with a destination path. The state and the
//! answers are the core's [`LinkModel`] / [`TransferModel`]; this file draws them.

use egui::{Align, FontId, Key, Layout, Margin, RichText, TextEdit, Ui};

use super::DialogCtx;
use super::conflict::{Btn, button};
pub use crate::models::dialogs::LinkChoice;
use crate::models::dialogs::{LinkModel, TransferModel};
use crate::theme::{icons::ph, palette};

fn field(ui: &mut Ui, label: &str, text: &mut String, hint: &str, focus: &mut bool) -> egui::Response {
    let p = palette(ui.ctx());
    ui.label(RichText::new(label).font(FontId::proportional(11.5)).color(p.text_secondary));
    let r = ui
        .add(TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).margin(Margin::symmetric(10, 7)));
    if *focus {
        r.request_focus();
        *focus = false;
    }
    ui.add_space(8.0);
    r
}

/// Draw the "new link" form. Returns `false` to close.
pub(crate) fn show_link(ui: &mut Ui, m: &mut LinkModel, ctx: &mut DialogCtx<'_>) -> bool {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        for k in LinkChoice::offered() {
            ui.selectable_value(&mut m.kind, k, k.label());
        }
    });
    ui.label(RichText::new(m.kind.hint()).font(FontId::proportional(11.5)).color(p.text_tertiary));
    ui.add_space(10.0);
    let mut no_focus = false;
    let t = field(ui, tr!("dialogs-link-target"), &mut m.target, tr!("dialogs-link-target-hint"), &mut m.focus);
    if t.changed() {
        m.target_edited();
    }
    let n = field(ui, tr!("dialogs-link-name"), &mut m.name, tr!("dialogs-link-name-hint"), &mut no_focus);
    if n.changed() {
        m.name_edited();
    }
    if let Some(e) = m.error()
        && !m.target.trim().is_empty()
    {
        ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.danger));
    }
    ui.label(
        RichText::new(tr!("dialogs-link-in", dir = soyuz_fs::strip_verbatim(&m.dir).display().to_string()))
            .font(FontId::proportional(11.5))
            .color(p.text_tertiary),
    );
    ui.add_space(16.0);
    let mut ok = false;
    let mut cancel = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ok = button(ui, tr!("dialogs-link-create"), Btn::Primary).clicked();
        cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
    });
    if ui.input(|i| i.key_pressed(Key::Enter)) {
        ok = true;
    }
    if ok && let Some(cmd) = m.create() {
        ctx.cmds.push(cmd);
        return false;
    }
    !cancel
}

pub(crate) fn transfer_icon(m: &TransferModel) -> &'static str {
    if m.mv { ph::ARROW_BEND_DOWN_RIGHT } else { ph::COPY }
}

/// Draw "Copy to…" / "Move to…". Returns `false` to close.
pub(crate) fn show_transfer(ui: &mut Ui, m: &mut TransferModel, ctx: &mut DialogCtx<'_>) -> bool {
    let p = palette(ui.ctx());
    ui.label(RichText::new(m.message()).color(p.text_secondary));
    ui.add_space(6.0);
    field(ui, tr!("dialogs-transfer-dest"), &mut m.dest, tr!("dialogs-transfer-dest-hint"), &mut m.focus);
    let mut ok = false;
    let mut cancel = false;
    ui.add_space(8.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ok = button(ui, if m.mv { tr!("common-move") } else { tr!("common-copy") }, Btn::Primary).clicked();
        cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
    });
    if ui.input(|i| i.key_pressed(Key::Enter)) {
        ok = true;
    }
    if ok && let Some(cmd) = m.confirm() {
        ctx.cmds.push(cmd);
        return false;
    }
    !cancel
}
