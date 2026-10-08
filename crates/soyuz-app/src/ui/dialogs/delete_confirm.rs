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

impl DeleteConfirmDialog {
    pub fn new(paths: Vec<PathBuf>, mode: DeleteMode) -> Self {
        let title = match paths.as_slice() {
            [one] => {
                let name = crate::ui::widgets::shown_name(&soyuz_core::entry::display_name(one));
                match mode {
                    DeleteMode::Trash => tr!("dialogs-delete-trash-one", name = name),
                    DeleteMode::Permanent => tr!("dialogs-delete-permanent-one", name = name),
                    DeleteMode::Wipe { .. } => tr!("dialogs-delete-wipe-one", name = name),
                }
            }
            many => {
                let count = many.len();
                match mode {
                    DeleteMode::Trash => tr!("dialogs-delete-trash-many", count = count),
                    DeleteMode::Permanent => tr!("dialogs-delete-permanent-many", count = count),
                    DeleteMode::Wipe { .. } => tr!("dialogs-delete-wipe-many", count = count),
                }
            }
        };
        DeleteConfirmDialog { kind: ConfirmKind::Delete { paths, mode }, title }
    }

    pub fn purge(items: Vec<TrashItem>) -> Self {
        let title = tr!("dialogs-delete-permanent-many", count = items.len());
        DeleteConfirmDialog { kind: ConfirmKind::Purge(items), title }
    }

    pub fn empty_trash(n: usize) -> Self {
        DeleteConfirmDialog { kind: ConfirmKind::EmptyTrash(n), title: tr!("dialogs-delete-empty-title").into() }
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

    fn alert(&self) -> bool {
        true
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
                (tr!("dialogs-delete-trash-body").to_string(), tr!("dialogs-delete-trash-confirm"), false)
            }
            ConfirmKind::Delete { mode: DeleteMode::Permanent, .. } => {
                (tr!("dialogs-delete-permanent-body").to_string(), tr!("common-delete"), true)
            }
            ConfirmKind::Delete { mode: DeleteMode::Wipe { .. }, .. } => {
                (tr!("dialogs-delete-wipe-body").to_string(), tr!("dialogs-delete-wipe-confirm"), true)
            }
            ConfirmKind::Purge(_) => (tr!("dialogs-delete-purge-body").to_string(), tr!("common-delete"), true),
            ConfirmKind::EmptyTrash(n) => {
                (tr!("dialogs-delete-empty-body", count = *n), tr!("dialogs-delete-empty-confirm"), true)
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
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(
                                RichText::new(crate::ui::widgets::shown_name(n))
                                    .font(FontId::proportional(12.5))
                                    .color(p.text_primary),
                            );
                            crate::ui::widgets::hidden_badge(ui, n);
                        });
                    }
                    if names.len() > 5 {
                        ui.label(
                            RichText::new(tr!("dialogs-delete-and-more", count = names.len() - 5))
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
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
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
