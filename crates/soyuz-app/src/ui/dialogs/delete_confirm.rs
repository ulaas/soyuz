//! Delete confirmation: move to trash (when `general.confirm_delete`), permanent delete
//! (Shift+Delete), secure wipe, permanent delete of trashed items and "Empty trash".

use std::path::PathBuf;

use egui::{Align, CornerRadius, FontId, Key, Layout, Margin, RichText, Ui};
use soyuz_fs::TrashItem;
use soyuz_jobs::DeleteMode;

use super::conflict::{Btn, button};
use super::{Dialog, DialogCtx};
use crate::cmd::Cmd;
use crate::ops::fileops::FileOpCmd;
use crate::theme::{icons::ph, palette};

pub enum ConfirmKind {
    Delete { paths: Vec<PathBuf>, mode: DeleteMode },
    Purge(Vec<TrashItem>),
    EmptyTrash(usize),
}

pub struct DeleteConfirmDialog {
    kind: ConfirmKind,
    title: String,
}

fn count(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{} items", crate::ui::statusbar::group_thousands(n)) }
}

impl DeleteConfirmDialog {
    pub fn new(paths: Vec<PathBuf>, mode: DeleteMode) -> Self {
        let what = match paths.as_slice() {
            [one] => format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(one)),
            many => count(many.len()),
        };
        let title = match mode {
            DeleteMode::Trash => format!("Move {what} to the trash?"),
            DeleteMode::Permanent => format!("Delete {what} permanently?"),
            DeleteMode::Wipe { .. } => format!("Securely wipe {what}?"),
        };
        DeleteConfirmDialog { kind: ConfirmKind::Delete { paths, mode }, title }
    }

    pub fn purge(items: Vec<TrashItem>) -> Self {
        let title = format!("Delete {} permanently?", count(items.len()));
        DeleteConfirmDialog { kind: ConfirmKind::Purge(items), title }
    }

    pub fn empty_trash(n: usize) -> Self {
        DeleteConfirmDialog { kind: ConfirmKind::EmptyTrash(n), title: "Empty the trash?".into() }
    }

    fn names(&self) -> Vec<String> {
        match &self.kind {
            ConfirmKind::Delete { paths, .. } => paths.iter().map(|p| soyuz_core::entry::display_name(p)).collect(),
            ConfirmKind::Purge(items) => items.iter().map(|i| soyuz_core::entry::display_name(&i.original)).collect(),
            ConfirmKind::EmptyTrash(_) => Vec::new(),
        }
    }
}

impl Dialog for DeleteConfirmDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(match &self.kind {
            ConfirmKind::Delete { mode: DeleteMode::Trash, .. } => ph::TRASH,
            _ => ph::WARNING,
        })
    }

    fn width(&self) -> f32 {
        440.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let (msg, confirm, danger) = match &self.kind {
            ConfirmKind::Delete { mode: DeleteMode::Trash, .. } => {
                ("You can restore items from the trash later.".to_string(), "Move to trash", false)
            }
            ConfirmKind::Delete { mode: DeleteMode::Permanent, .. } => {
                ("Permanently deleted items can't be restored.".to_string(), "Delete", true)
            }
            ConfirmKind::Delete { mode: DeleteMode::Wipe { .. }, .. } => (
                "The data is overwritten before deletion. On SSDs and copy-on-write filesystems this cannot guarantee \
                 the old blocks are gone."
                    .to_string(),
                "Wipe",
                true,
            ),
            ConfirmKind::Purge(_) => {
                ("These items will be removed from the trash for good.".to_string(), "Delete", true)
            }
            ConfirmKind::EmptyTrash(n) => {
                (format!("All {} in the trash will be deleted permanently.", count(*n)), "Empty trash", true)
            }
        };
        ui.label(RichText::new(msg).color(p.text_secondary));
        let names = self.names();
        if !names.is_empty() {
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(p.bg_sunken)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for n in names.iter().take(5) {
                        ui.label(RichText::new(n).font(FontId::proportional(12.5)).color(p.text_primary));
                    }
                    if names.len() > 5 {
                        ui.label(
                            RichText::new(format!("and {} more", names.len() - 5))
                                .font(FontId::proportional(12.0))
                                .color(p.text_tertiary),
                        );
                    }
                });
        }
        ui.add_space(18.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, confirm, if danger { Btn::Danger } else { Btn::Primary }).clicked();
            cancel = button(ui, "Cancel", Btn::Secondary).clicked();
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if ok {
            let cmd = match std::mem::replace(&mut self.kind, ConfirmKind::EmptyTrash(0)) {
                ConfirmKind::Delete { paths, mode } => FileOpCmd::Delete { paths, mode },
                ConfirmKind::Purge(items) => FileOpCmd::TrashPurge(items),
                ConfirmKind::EmptyTrash(_) => FileOpCmd::EmptyTrash,
            };
            ctx.cmds.push(Cmd::FileOp(cmd));
            return false;
        }
        !cancel
    }
}
