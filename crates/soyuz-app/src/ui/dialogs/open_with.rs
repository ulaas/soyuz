//! "Open with…" chooser: the applications registered for the file's type (looked up off
//! the UI thread, shared with the context menu's cache) plus any program by path. It
//! starts on the extension's default or last-used application (`[open]`,
//! `model::preselect`); "Always use this app" makes the choice the default. Keys: ↑ / ↓
//! choose, Enter opens, Tab reaches the field and the checkbox (Space ticks it).

use std::path::PathBuf;

use egui::{Margin, RichText, ScrollArea, TextEdit, Ui};

use super::{Dialog, DialogCtx, footer};
use crate::models::requests::open_with as model;
use crate::theme::{icons::ph, palette};

pub struct OpenWithDialog {
    path: PathBuf,
    title: String,
    /// Index into the registered apps.
    selected: Option<usize>,
    /// "Another program" path typed by the user.
    custom: String,
    /// "Always use this app for .ext files".
    always: bool,
    /// `[open]` when the dialog opened (the default / last-used application).
    open_cfg: soyuz_core::config::OpenSection,
    /// The starting choice was applied (once the applications are known).
    preselected: bool,
    error: Option<String>,
}

impl OpenWithDialog {
    pub fn new(path: PathBuf, open_cfg: soyuz_core::config::OpenSection) -> Self {
        let title = model::title(&path);
        OpenWithDialog {
            path,
            title,
            selected: None,
            custom: String::new(),
            always: false,
            open_cfg,
            preselected: false,
            error: None,
        }
    }
}

impl Dialog for OpenWithDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::APP_WINDOW)
    }

    fn width(&self) -> f32 {
        460.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let apps = crate::ui::context_menu::apps_for_cached(ui.ctx(), &self.path);
        if let Some(list) = apps.as_deref().filter(|_| !self.preselected) {
            self.preselected = true;
            let pre = model::preselect(&self.open_cfg, &self.path, list);
            self.selected = pre.selected;
            self.custom = pre.custom;
            self.always = pre.always;
        }
        // ↑ / ↓ choose from anywhere (the program field has no use for them).
        let n = apps.as_ref().map_or(0, |l| l.len());
        if n > 0 {
            let (up, down) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                )
            });
            if up || down {
                let cur = self.selected.unwrap_or(if up { n } else { usize::MAX });
                self.selected = Some(if up { cur.saturating_sub(1) } else { cur.wrapping_add(1).min(n - 1) });
                self.custom.clear();
                self.error = None;
            }
        }
        let mut open_now = false;
        egui::Frame::new()
            .fill(p.bg_sunken)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(Margin::symmetric(6, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| match &apps {
                    None => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(14.0));
                            ui.label(RichText::new(tr!("dialogs-openwith-looking")).color(p.text_secondary));
                        });
                    }
                    Some(list) if list.is_empty() => {
                        ui.label(RichText::new(tr!("dialogs-openwith-none")).color(p.text_tertiary));
                    }
                    Some(list) => {
                        for (i, app) in list.iter().enumerate() {
                            let label = model::app_label(app);
                            let on = self.selected == Some(i);
                            let r = ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(on, label));
                            if on && self.custom.trim().is_empty() {
                                r.scroll_to_me(None);
                            }
                            if r.clicked() {
                                self.selected = Some(i);
                                self.custom.clear();
                                self.error = None;
                            }
                            if r.double_clicked() {
                                open_now = true;
                            }
                        }
                    }
                });
            });
        ui.add_space(12.0);
        ui.label(RichText::new(tr!("dialogs-openwith-other")).color(p.text_secondary));
        ui.add_space(4.0);
        let hint = model::program_hint();
        let r = ui.add(
            TextEdit::singleline(&mut self.custom)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if r.changed() && !self.custom.trim().is_empty() {
            self.selected = None;
            self.error = None;
        }
        let submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some(label) = model::always_label(&self.path) {
            ui.add_space(8.0);
            let chosen = (self.custom.trim().is_empty())
                .then(|| self.selected.and_then(|i| apps.as_ref().and_then(|l| l.get(i))))
                .flatten();
            let can = chosen.is_none_or(model::can_be_default);
            let r = ui.add_enabled(can, egui::Checkbox::new(&mut self.always, label));
            if !can {
                r.on_disabled_hover_text(tr!("dialogs-openwith-always-system"));
            }
        }
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(p.danger));
        }
        let (ok, cancel) = footer(ui, tr!("dialogs-openwith-open"), tr!("common-cancel"));
        if cancel {
            return false;
        }
        if !(ok || open_now || submit) {
            return true;
        }
        match model::choose(&self.custom, self.selected, apps.as_deref()) {
            Ok(app) => {
                ctx.cmds.extend(model::choice_cmds(&self.path, app, self.always));
                false
            }
            Err(e) => {
                self.error = Some(e);
                true
            }
        }
    }
}
