//! "Open with…" chooser: the applications registered for the file's type (looked up off
//! the UI thread, shared with the context menu's cache), the programs picked earlier, and
//! any other program: Browse opens a picker on programs, or type its path. It starts on
//! the extension's default or last-used application (`[open]`, `model::start`); "Always
//! use this app" makes the choice the default. Keys: ↑ / ↓ choose, Enter opens, Tab reaches
//! the field, Browse (also Alt+B) and the checkbox (Space ticks it). When nothing is
//! listed, Browse has the keyboard, so Enter goes straight to the programs.

use std::path::PathBuf;

use egui::{Align, Layout, Margin, RichText, ScrollArea, TextEdit, Ui};
use soyuz_fs::AppInfo;

use super::{Dialog, DialogCtx, footer};
use crate::models::requests::open_with as model;
use crate::theme::{icons::ph, palette};
use crate::ui::path_picker::{PathPicker, PickResult};
use crate::ui::widgets::{pill_button, trunc_galley};

pub struct OpenWithDialog {
    path: PathBuf,
    title: String,
    /// What the list shows (`None` while the lookup runs): registered, then remembered.
    listed: Option<Vec<AppInfo>>,
    /// Index into `listed`.
    selected: Option<usize>,
    /// "Another program" path typed by the user.
    custom: String,
    /// "Always use this app for .ext files".
    always: bool,
    /// `[open]` when the dialog opened (the default / last-used application).
    open_cfg: soyuz_core::config::OpenSection,
    /// The program browser, on top of the dialog.
    picker: Option<PathPicker>,
    /// Browse was given the keyboard (once, when nothing is listed).
    focused_browse: bool,
    error: Option<String>,
}

impl OpenWithDialog {
    pub fn new(path: PathBuf, open_cfg: soyuz_core::config::OpenSection) -> Self {
        let title = model::title(&path);
        OpenWithDialog {
            path,
            title,
            listed: None,
            selected: None,
            custom: String::new(),
            always: false,
            open_cfg,
            picker: None,
            focused_browse: false,
            error: None,
        }
    }

    /// A program came back from the browser: list it and select it.
    fn picked(&mut self, exe: PathBuf, name: Option<String>) {
        self.error = None;
        match &mut self.listed {
            Some(list) => {
                self.selected = Some(model::adopt(list, model::picked_app(&exe, name)));
                self.custom.clear();
            }
            // The lookup is still running: it would replace the list.
            None => self.custom = exe.display().to_string(),
        }
    }
}

/// Draw a row: the application's name, and dimmed the folder of a program picked by path
/// (two tools of the same name stay apart). One line, cut to fit.
fn app_row(ui: &mut Ui, app: &AppInfo, on: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    let label = model::app_label(app);
    crate::ui::a11y::toggle(&resp, &label, on);
    let corner = egui::CornerRadius::same(crate::theme::tokens::radius::ROW);
    if on {
        ui.painter().rect_filled(rect, corner, p.accent_subtle);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, corner, p.bg_surface2);
    }
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect.shrink(1.0));
    }
    let font = egui::FontId::proportional(13.0);
    let name = trunc_galley(ui.painter(), &label, font.clone(), p.text_primary, rect.width() - 20.0);
    let name_w = name.size().x;
    let left = rect.left() + 10.0;
    ui.painter().galley(egui::pos2(left, rect.center().y - name.size().y / 2.0), name, p.text_primary);
    let room = rect.width() - 20.0 - name_w - 10.0;
    if room > 40.0
        && model::is_by_path(app)
        && let Some(dir) = app.exec.parent()
    {
        let path = trunc_galley(ui.painter(), &dir.display().to_string(), font, p.text_tertiary, room);
        let y = rect.center().y - path.size().y / 2.0;
        ui.painter().galley(egui::pos2(left + name_w + 10.0, y), path, p.text_tertiary);
    }
    resp
}

impl Dialog for OpenWithDialog {
    fn title(&self) -> &str {
        &self.title
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::APP_WINDOW)
    }

    fn width(&self) -> f32 {
        500.0
    }

    fn overlay(&mut self, ctx: &egui::Context) {
        let Some(picker) = &mut self.picker else { return };
        match picker.show(ctx) {
            PickResult::Pending => {}
            PickResult::Cancelled => self.picker = None,
            PickResult::Picked(exe) => {
                let name = picker.picked_name();
                self.picker = None;
                self.picked(exe, name);
            }
        }
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        if self.listed.is_none()
            && let Some(registered) = crate::ui::context_menu::apps_for_cached(ui.ctx(), &self.path).as_deref()
        {
            let (list, pre) = model::start(&self.open_cfg, &self.path, registered);
            self.listed = Some(list);
            self.selected = pre.selected;
            self.always = pre.always;
            if self.custom.trim().is_empty() {
                self.custom = pre.custom;
            }
        }
        let browsing = self.picker.is_some();
        if browsing {
            // The browser has the keyboard; the dialog waits beneath it.
            ui.disable();
        }
        // ↑ / ↓ choose from anywhere (the program field has no use for them).
        let n = self.listed.as_ref().map_or(0, Vec::len);
        if n > 0 && !browsing {
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
                ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| match &self.listed {
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
                            let on = self.selected == Some(i);
                            let r = app_row(ui, app, on);
                            let r = if model::is_by_path(app) {
                                r.on_hover_text(app.exec.display().to_string())
                            } else {
                                r
                            };
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
        let mut submit = false;
        let row = egui::vec2(ui.available_width(), 32.0);
        ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
            let browse = pill_button(ui, Some(ph::FOLDER_OPEN), tr!("dialogs-openwith-browse"), n == 0)
                .on_hover_text(tr!("dialogs-openwith-browse-tip"));
            let shortcut = !browsing && ui.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::B));
            if browse.clicked() || shortcut {
                self.picker = Some(PathPicker::programs(
                    tr!("dialogs-openwith-browse-title"),
                    model::browse_start(&self.open_cfg),
                ));
            }
            if n == 0 && self.listed.is_some() && !self.focused_browse {
                self.focused_browse = true;
                browse.request_focus();
            }
            ui.add_space(6.0);
            let r = ui.add(
                TextEdit::singleline(&mut self.custom)
                    .hint_text(model::program_hint())
                    .desired_width(f32::INFINITY)
                    .margin(Margin::symmetric(10, 7)),
            );
            if r.changed() && !self.custom.trim().is_empty() {
                self.selected = None;
                self.error = None;
            }
            submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        if let Some(label) = model::always_label(&self.path) {
            ui.add_space(8.0);
            let chosen = (self.custom.trim().is_empty())
                .then(|| self.selected.and_then(|i| self.listed.as_ref().and_then(|l| l.get(i))))
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
        if browsing {
            return true;
        }
        if cancel {
            return false;
        }
        // Enter on the list (no widget has the keyboard: a focused button takes its own).
        let enter = ui.memory(|m| m.focused().is_none()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if !(ok || open_now || submit || enter) {
            return true;
        }
        match model::choose(&self.custom, self.selected, self.listed.as_deref()) {
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
