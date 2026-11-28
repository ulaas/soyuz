//! Saved process filters in the GUI: the menu behind the funnel button next to the filter
//! box, and the manager dialog (the list on the left, an editor for the chosen entry on the
//! right: name, query with live checking, regex mode, examples; Save / Apply below).
//!
//! Edits go to the core as [`MonitorCmd`]s. The dialog keeps a mirror of the list, changed
//! with the same functions the core uses (`models::monitor::saved_filters`), so both agree.

use egui::{Margin, RichText, TextEdit, Ui};
use soyuz_core::config::monitor::SavedProcFilter;

use crate::cmd::Cmd;
use crate::models::monitor::{MonitorCmd, saved_filters};
use crate::state::pane::PaneId;
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::dialogs::{Dialog, DialogCtx};
use crate::ui::widgets::{Btn, menu_caption, menu_item, menu_separator, pill_button, pill_button_on};

/// Queries the editor offers as starting points.
const EXAMPLES: [&str; 6] = ["cpu>5", "mem>500M", "user:root", "threads>100", "chrome|firefox", "-svchost"];

/// The funnel menu's rows: apply a saved filter, save the filter box, open the manager.
/// `current` is the filter box (text, regex mode, whether it parses).
pub fn menu(ui: &mut Ui, list: &[SavedProcFilter], current: (&str, bool, bool), pending: &mut Vec<MonitorCmd>) {
    let (text, regex, parses) = current;
    ui.set_width(300.0);
    menu_caption(ui, tr!("monitor-saved-filters"));
    let active = saved_filters::active(list, text, regex).map(|f| f.name.as_str());
    if list.is_empty() {
        menu_item(ui, Some(ph::INFO), tr!("monitor-saved-filter-none"), None, false);
    }
    for f in list {
        let on = active == Some(f.name.as_str());
        let icon = if on { ph::CHECK } else { ph::FUNNEL_SIMPLE };
        let hint: String = if f.query.chars().count() > 22 {
            f.query.chars().take(21).chain(['\u{2026}']).collect()
        } else {
            f.query.clone()
        };
        let r = menu_item(ui, Some(icon), &f.name, Some(&hint), true);
        if r.on_hover_text(&f.query).clicked() {
            pending.push(MonitorCmd::ApplySavedFilter(f.name.clone()));
            ui.close();
        }
    }
    menu_separator(ui);
    let can_save = !text.trim().is_empty() && parses && active.is_none();
    if menu_item(ui, Some(ph::FLOPPY_DISK), tr!("monitor-saved-filter-save-current"), None, can_save).clicked() {
        pending.push(MonitorCmd::SaveCurrentFilter);
        ui.close();
    }
    if menu_item(ui, Some(ph::SLIDERS_HORIZONTAL), tr!("monitor-saved-filter-manage"), None, true).clicked() {
        pending.push(MonitorCmd::ManageFilters);
        ui.close();
    }
}

pub struct MonitorFiltersDialog {
    pane: PaneId,
    list: Vec<SavedProcFilter>,
    /// Index of the entry in the editor; `None` while a new entry is being written.
    selected: Option<usize>,
    draft: SavedProcFilter,
    /// What the editor started from, to tell whether it has unsaved changes.
    pristine: SavedProcFilter,
    focus_name: bool,
    /// Why the last save was refused (shown under the editor).
    error: Option<String>,
}

impl MonitorFiltersDialog {
    /// Open on the first entry, or with `current` (the filter box) as a new entry.
    pub fn new(pane: PaneId, list: Vec<SavedProcFilter>, current: Option<(String, bool)>) -> Self {
        let mut d = MonitorFiltersDialog {
            pane,
            list,
            selected: None,
            draft: SavedProcFilter::default(),
            pristine: SavedProcFilter::default(),
            focus_name: false,
            error: None,
        };
        match current {
            Some((query, regex)) => d.start_new(query, regex),
            None if d.list.is_empty() => d.start_new(String::new(), false),
            None => d.select(0),
        }
        d
    }

    fn select(&mut self, i: usize) {
        if let Some(f) = self.list.get(i) {
            self.selected = Some(i);
            self.draft = f.clone();
            self.pristine = f.clone();
            self.error = None;
        }
    }

    fn start_new(&mut self, query: String, regex: bool) {
        let base: String = query.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
        self.draft = SavedProcFilter { name: saved_filters::suggest_name(&self.list, &base), query, regex };
        self.pristine = SavedProcFilter::default();
        self.selected = None;
        self.focus_name = true;
        self.error = None;
    }

    fn dirty(&self) -> bool {
        self.draft != self.pristine
    }

    fn old_name(&self) -> Option<String> {
        self.selected.and_then(|i| self.list.get(i)).map(|f| f.name.clone())
    }

    /// Save the draft; `true` when it was valid.
    fn save(&mut self, cmds: &mut Vec<Cmd>) -> bool {
        let old = self.old_name();
        match saved_filters::save(&mut self.list, old.as_deref(), self.draft.clone()) {
            Ok(i) => {
                cmds.push(Cmd::Monitor(self.pane, MonitorCmd::SaveFilter { old, entry: self.draft.clone() }));
                self.select(i);
                true
            }
            Err(e) => {
                self.error = Some(e.message());
                false
            }
        }
    }

    fn delete(&mut self, cmds: &mut Vec<Cmd>) {
        let Some(name) = self.old_name() else {
            // An unsaved new entry: drop it.
            if !self.list.is_empty() {
                self.select(0);
            }
            return;
        };
        saved_filters::remove(&mut self.list, &name);
        cmds.push(Cmd::Monitor(self.pane, MonitorCmd::DeleteSavedFilter(name)));
        match self.selected {
            Some(i) if !self.list.is_empty() => self.select(i.min(self.list.len() - 1)),
            _ => self.start_new(String::new(), false),
        }
    }

    fn shift(&mut self, cmds: &mut Vec<Cmd>, down: bool) {
        let (Some(i), Some(name)) = (self.selected, self.old_name()) else { return };
        let Some(to) = (if down { i.checked_add(1) } else { i.checked_sub(1) }).filter(|t| *t < self.list.len()) else {
            return;
        };
        saved_filters::move_to(&mut self.list, &name, to);
        cmds.push(Cmd::Monitor(self.pane, MonitorCmd::MoveSavedFilter(name, to)));
        self.selected = Some(to);
    }

    /// The left column: one row per entry (name over query), the unsaved new entry, "New".
    fn list_ui(&mut self, ui: &mut Ui, max_h: f32) {
        let p = palette(ui.ctx());
        let mut pick = None;
        let new_index = self.list.len();
        egui::ScrollArea::vertical().id_salt("soyuz.monitor.filters.list").max_height(max_h).show(ui, |ui| {
            let new_row = self.selected.is_none().then_some(&self.draft);
            for (i, f) in self.list.iter().chain(new_row).enumerate() {
                let on = self.selected.map_or(i == new_index, |s| s == i);
                let w = ui.available_width();
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(w, crate::theme::font::grow(40.0)), egui::Sense::click());
                let shown_name =
                    if f.name.trim().is_empty() { tr!("monitor-saved-filter-untitled") } else { f.name.as_str() };
                crate::ui::a11y::name(&resp, shown_name);
                let cr = crate::theme::tokens::radius::same(crate::theme::tokens::radius::ROW);
                if on {
                    ui.painter().rect_filled(rect, cr, p.accent_subtle);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, cr, p.bg_surface2);
                }
                let title = if i == new_index {
                    format!("{shown_name} \u{2022} {}", tr!("monitor-saved-filter-new-badge"))
                } else {
                    shown_name.to_string()
                };
                let fit = |s: String, font: egui::FontId, color| {
                    let mut job = egui::text::LayoutJob::simple_singleline(s, font, color);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 20.0);
                    ui.painter().layout_job(job)
                };
                let left = rect.left() + 10.0;
                let g = fit(title, crate::theme::font::ui(13.0), if on { p.accent } else { p.text_primary });
                ui.painter().galley(egui::pos2(left, rect.top() + 4.0), g, p.text_primary);
                let q = if f.regex { format!(".*  {}", f.query) } else { f.query.clone() };
                let g = fit(q, crate::theme::font::mono(11.0), p.text_tertiary);
                ui.painter().galley(egui::pos2(left, rect.top() + 22.0), g, p.text_tertiary);
                if resp.clicked() && i < new_index {
                    pick = Some(i);
                }
            }
        });
        if let Some(i) = pick {
            self.select(i);
        }
        ui.add_space(8.0);
        if pill_button(ui, Some(ph::PLUS), tr!("monitor-saved-filter-new"), Btn::Secondary).clicked() {
            self.start_new(String::new(), false);
        }
    }

    /// The right column: the editor of the draft. Returns `true` when Enter was pressed in a field.
    fn editor_ui(&mut self, ui: &mut Ui) -> bool {
        let p = palette(ui.ctx());
        let mut enter = false;
        let caption = |ui: &mut Ui, t: &str| {
            ui.label(RichText::new(t).size(crate::theme::font::px(11.5)).color(p.text_secondary));
        };
        caption(ui, tr!("monitor-saved-filter-name"));
        let r = ui.add(
            TextEdit::singleline(&mut self.draft.name)
                .hint_text(tr!("monitor-saved-filter-name-hint"))
                .char_limit(saved_filters::NAME_MAX)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        crate::ui::a11y::name(&r, tr!("monitor-saved-filter-name"));
        if self.focus_name {
            r.request_focus();
            self.focus_name = false;
        }
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if r.changed() {
            self.error = None;
        }
        ui.add_space(8.0);
        caption(ui, tr!("monitor-saved-filter-query"));
        let hint = if self.draft.regex { tr!("monitor-gui-filter-regex-hint") } else { tr!("monitor-gui-filter-hint") };
        let r = ui.add(
            TextEdit::singleline(&mut self.draft.query)
                .hint_text(hint)
                .font(crate::theme::font::mono(12.5))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        crate::ui::a11y::name(&r, tr!("monitor-saved-filter-query"));
        let r = r.on_hover_text(tr!("monitor-gui-filter-help"));
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if r.changed() {
            self.error = None;
        }
        ui.add_space(4.0);
        // Live check of the query.
        let q = self.draft.query.trim();
        let (icon, text, color) = if q.is_empty() {
            (ph::INFO, tr!("monitor-saved-filter-empty-query").to_string(), p.text_tertiary)
        } else {
            match soyuz_sysmon::Filter::parse(q, self.draft.regex) {
                Ok(_) => (ph::CHECK_CIRCLE, tr!("monitor-saved-filter-valid").to_string(), p.success),
                Err(e) => (ph::WARNING_CIRCLE, tr!("monitor-gui-filter-error", message = e.message.as_str()), p.danger),
            }
        };
        ui.horizontal(|ui| {
            ui.label(crate::theme::icons::icon(icon, 13.0).color(color));
            ui.add(egui::Label::new(RichText::new(text).size(crate::theme::font::px(11.5)).color(color)).wrap());
        });
        ui.add_space(6.0);
        let r = ui.checkbox(&mut self.draft.regex, RichText::new(tr!("monitor-gui-regex")).color(p.text_primary));
        if r.changed() {
            self.error = None;
        }
        ui.add_space(10.0);
        caption(ui, tr!("monitor-saved-filter-examples"));
        ui.horizontal_wrapped(|ui| {
            for ex in EXAMPLES {
                let r = crate::ui::widgets::chip(ui, None, ex, self.draft.query.trim() == ex);
                if r.on_hover_text(tr!("monitor-saved-filter-example-tip")).clicked() {
                    self.draft.query = ex.to_string();
                    self.draft.regex = false;
                    if self.selected.is_none() || self.draft.name.trim().is_empty() {
                        self.draft.name = saved_filters::suggest_name(&self.list, ex);
                    }
                    self.error = None;
                }
            }
        });
        enter
    }

    /// Move up / down, duplicate, delete; then the error or the unsaved note.
    fn actions_ui(&mut self, ui: &mut Ui, cmds: &mut Vec<Cmd>) {
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            let saved = self.selected.is_some();
            let i = self.selected.unwrap_or(0);
            let r = pill_button_on(ui, Some(ph::ARROW_UP), "", Btn::Secondary, saved && i > 0);
            crate::ui::a11y::name(&r, tr!("monitor-saved-filter-up"));
            if r.on_hover_text(tr!("monitor-saved-filter-up")).clicked() {
                self.shift(cmds, false);
            }
            let r = pill_button_on(ui, Some(ph::ARROW_DOWN), "", Btn::Secondary, saved && i + 1 < self.list.len());
            crate::ui::a11y::name(&r, tr!("monitor-saved-filter-down"));
            if r.on_hover_text(tr!("monitor-saved-filter-down")).clicked() {
                self.shift(cmds, true);
            }
            if pill_button_on(ui, Some(ph::COPY), tr!("monitor-saved-filter-duplicate"), Btn::Secondary, saved)
                .clicked()
            {
                let (name, query, regex) = (self.draft.name.clone(), self.draft.query.clone(), self.draft.regex);
                self.start_new(query, regex);
                self.draft.name = saved_filters::suggest_name(&self.list, &name);
            }
            let can_delete = saved || !self.list.is_empty();
            if pill_button_on(ui, Some(ph::TRASH), tr!("common-delete"), Btn::DangerSoft, can_delete).clicked() {
                self.delete(cmds);
            }
        });
        let note = match &self.error {
            Some(e) => Some((e.clone(), p.danger)),
            None if self.dirty() && self.selected.is_some() => {
                Some((tr!("monitor-saved-filter-unsaved").to_string(), p.warning))
            }
            None => None,
        };
        if let Some((text, color)) = note {
            ui.add_space(6.0);
            ui.label(RichText::new(text).size(crate::theme::font::px(11.5)).color(color));
        }
    }
}

impl Dialog for MonitorFiltersDialog {
    fn title(&self) -> &str {
        tr!("monitor-saved-filters")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::FUNNEL)
    }

    fn width(&self) -> f32 {
        680.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.add(egui::Label::new(RichText::new(tr!("monitor-saved-filters-intro")).color(p.text_secondary)).wrap());
        ui.add_space(12.0);
        let mut enter = false;
        let list_w = (ui.available_width() * 0.36).clamp(180.0, 260.0);
        // Both columns get a fixed height: a separator in a row would grow to the screen's.
        let h = crate::theme::font::grow(330.0);
        let top_down = egui::Layout::top_down(egui::Align::Min);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(list_w, h), top_down, |ui| {
                ui.set_width(list_w);
                ui.set_min_height(h);
                self.list_ui(ui, h - crate::theme::font::grow(40.0));
            });
            let (line, _) = ui.allocate_exact_size(egui::vec2(17.0, h), egui::Sense::hover());
            ui.painter().vline(line.center().x, line.y_range(), egui::Stroke::new(1.0, p.border_subtle));
            let w = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(w, h), top_down, |ui| {
                ui.set_width(w);
                enter = self.editor_ui(ui);
                ui.add_space(14.0);
                self.actions_ui(ui, ctx.cmds);
            });
        });
        ui.add_space(16.0);
        let valid = saved_filters::validate(&self.list, self.old_name().as_deref(), &self.draft).is_ok();
        let applicable = if self.dirty() { valid } else { self.selected.is_some() };
        let mut close = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = pill_button_on(
                ui,
                Some(ph::FUNNEL_SIMPLE),
                tr!("monitor-saved-filter-apply"),
                Btn::Primary,
                applicable,
            );
            if r.on_hover_text(tr!("monitor-saved-filter-apply-tip")).clicked()
                && (!self.dirty() || self.save(ctx.cmds))
            {
                ctx.cmds.push(Cmd::Monitor(self.pane, MonitorCmd::ApplySavedFilter(self.draft.name.clone())));
                close = true;
            }
            ui.add_space(6.0);
            if pill_button_on(ui, Some(ph::FLOPPY_DISK), tr!("common-save"), Btn::Secondary, valid && self.dirty())
                .clicked()
            {
                self.save(ctx.cmds);
            }
            ui.add_space(6.0);
            if pill_button(ui, None, tr!("common-close"), Btn::Ghost).clicked() {
                close = true;
            }
        });
        if enter && self.dirty() {
            self.save(ctx.cmds);
        }
        !close
    }
}
