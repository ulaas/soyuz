//! Workspace dialogs: "Save workspace as…", "Manage workspaces" and the prompt shown
//! when a window with several tabs is closed.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, FontId, Key, Layout, Margin, RichText, TextEdit, Ui};

use super::WsCmd;
use crate::cmd::Cmd;
use crate::state::workspace_windows::WindowId;
use crate::theme::{icons::ph, palette};
use crate::ui::dialogs::conflict::{Btn, button};
use crate::ui::dialogs::{Dialog, DialogCtx, footer};

/// Name a new workspace (or overwrite one with the same name).
pub struct SaveWorkspaceDialog {
    name: String,
    focus: bool,
}

impl SaveWorkspaceDialog {
    pub fn new(name: String) -> Self {
        SaveWorkspaceDialog { name, focus: true }
    }
}

impl Dialog for SaveWorkspaceDialog {
    fn title(&self) -> &str {
        tr!("workspaces-save-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::FLOPPY_DISK)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(tr!("workspaces-save-intro")).color(p.text_secondary));
        ui.add_space(10.0);
        let r = ui.add(
            TextEdit::singleline(&mut self.name)
                .hint_text(tr!("workspaces-save-name-hint"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        let tabs = ctx.ws.tabs.len();
        let groups = ctx.ws.groups.len();
        ui.add_space(6.0);
        ui.label(
            RichText::new(if groups == 0 {
                tr!("workspaces-tabs", count = tabs)
            } else {
                tr!("workspaces-tabs-and-groups", tabs = tabs, groups = groups)
            })
            .font(FontId::proportional(11.5))
            .color(p.text_tertiary),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let (ok, cancel) = footer(ui, tr!("common-save"), tr!("common-cancel"));
        if (ok || enter) && !self.name.trim().is_empty() {
            ctx.cmds.push(Cmd::Workspace(WsCmd::Save(self.name.trim().to_string())));
            return false;
        }
        !cancel
    }
}

/// Saved workspaces: open, rename, hotkey, reorder, delete.
#[derive(Default)]
pub struct ManageWorkspacesDialog {
    /// Row being edited: `(index, name, hotkey)`.
    editing: Option<(usize, String, String)>,
}

impl Dialog for ManageWorkspacesDialog {
    fn title(&self) -> &str {
        tr!("workspaces-manage-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::STACK)
    }

    fn width(&self) -> f32 {
        560.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let list = crate::ui::workspaces::store_snapshot();
        if list.is_empty() {
            ui.label(RichText::new(tr!("workspaces-manage-empty")).color(p.text_secondary));
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for (i, (name, hotkey, tabs)) in list.iter().enumerate() {
                let editing = self.editing.as_ref().is_some_and(|(k, _, _)| *k == i);
                ui.horizontal(|ui| {
                    ui.set_min_height(30.0);
                    if editing {
                        let (_, n, h) = self.editing.as_mut().expect("editing");
                        ui.add(TextEdit::singleline(n).desired_width(200.0).margin(Margin::symmetric(8, 5)));
                        ui.add(
                            TextEdit::singleline(h)
                                .hint_text(tr!("workspaces-manage-hotkey-hint"))
                                .desired_width(150.0)
                                .margin(Margin::symmetric(8, 5)),
                        );
                    } else {
                        ui.label(RichText::new(name).color(p.text_primary).font(FontId::proportional(13.0)));
                        ui.label(
                            RichText::new(tr!("workspaces-tabs", count = *tabs))
                                .font(FontId::proportional(11.5))
                                .color(p.text_tertiary),
                        );
                        if !hotkey.is_empty() {
                            crate::ui::widgets::kbd(ui, hotkey);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if editing {
                            if button(ui, tr!("workspaces-manage-done"), Btn::Primary).clicked()
                                && let Some((k, n, h)) = self.editing.take()
                            {
                                ctx.cmds.push(Cmd::Workspace(WsCmd::Rename(k, n)));
                                ctx.cmds.push(Cmd::Workspace(WsCmd::SetHotkey(k, h)));
                            }
                            return;
                        }
                        if button(ui, tr!("common-delete"), Btn::Ghost).clicked() {
                            ctx.cmds.push(Cmd::Workspace(WsCmd::Delete(i)));
                        }
                        if button(ui, tr!("workspaces-manage-edit"), Btn::Ghost).clicked() {
                            self.editing = Some((i, name.clone(), hotkey.clone()));
                        }
                        if i > 0
                            && button(ui, tr!("workspaces-manage-up"), Btn::Ghost)
                                .on_hover_text(tr!("workspaces-manage-up-tip"))
                                .clicked()
                        {
                            ctx.cmds.push(Cmd::Workspace(WsCmd::MoveUp(i)));
                        }
                        if button(ui, tr!("workspaces-manage-new-window"), Btn::Secondary).clicked() {
                            ctx.cmds.push(Cmd::Workspace(WsCmd::OpenInNewWindow(i)));
                        }
                        if button(ui, tr!("workspaces-manage-open"), Btn::Primary).clicked() {
                            ctx.cmds.push(Cmd::Workspace(WsCmd::Open(i)));
                        }
                    });
                });
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new(tr!("workspaces-manage-slots-note")).font(FontId::proportional(11.5)).color(p.text_tertiary),
        );
        !crate::ui::dialogs::close_footer(ui, tr!("common-close"))
    }
}

/// Closing a window that has several tabs.
pub struct CloseWindowDialog {
    id: WindowId,
    title: String,
    open: Arc<AtomicBool>,
}

impl CloseWindowDialog {
    pub fn new(id: WindowId, tabs: usize, open: Arc<AtomicBool>) -> Self {
        CloseWindowDialog { id, title: tr!("workspaces-close-title", count = tabs), open }
    }
}

impl Drop for CloseWindowDialog {
    fn drop(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

impl Dialog for CloseWindowDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::BROWSERS)
    }

    fn width(&self) -> f32 {
        470.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let body =
            if self.id.is_main() { tr!("workspaces-close-body-next") } else { tr!("workspaces-close-body-main") };
        ui.label(RichText::new(body).color(p.text_secondary));
        ui.add_space(18.0);
        let (mut merge, mut close, mut cancel) = (false, false, false);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            merge = button(ui, tr!("workspaces-close-move"), Btn::Primary).clicked();
            close = button(ui, tr!("workspaces-close-close"), Btn::Danger).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            merge = true;
        }
        if merge || close {
            ctx.cmds.push(super::windows::close_cmd(self.id, merge));
            return false;
        }
        !cancel
    }
}
