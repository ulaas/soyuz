//! Smart folders (GUI): the sidebar section, the create / edit / delete dialogs and the
//! index status chip of the status bar. The store and the commands live in
//! `models::smart`.

use std::collections::BTreeSet;

use egui::{Align, Align2, CornerRadius, FontId, Key, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use soyuz_core::Location;

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
pub use crate::models::smart::{SmartCmd, SmartFolder, SmartPrompt, SmartStore, apply, folders, pump, status_line};
use crate::models::smart::{coverage_text, query_help, shown_name, validate};
use crate::services::index::{self as ix};
use crate::theme::{icons, icons::ph, palette};
use crate::ui::dialogs::conflict::{Btn, button};
use crate::ui::dialogs::{Dialog, DialogCtx};
use crate::ui::widgets::{NavRow, collapsible_header, menu_item, spinner};

/// Show a dialog the core asked for ([`crate::models::frontend::FrontendCmd::Smart`]).
pub fn prompt(app: &mut SoyuzApp, p: SmartPrompt) {
    match p {
        SmartPrompt::Edit { index, folder } => app.gui.dialogs.push(SmartDialog::new(index, folder)),
        SmartPrompt::ConfirmDelete { index, name } => app.gui.dialogs.push(ConfirmDelete { index, name }),
    }
}

/// Collapsed-set key of the sidebar section (persisted; the header is translated).
const SECTION: &str = "Smart folders";

/// Sidebar section (drawn when there are smart folders or indexing is on).
pub fn sidebar_section(ui: &mut Ui, collapsed: &mut BTreeSet<String>, current: &Location, cmds: &mut Vec<Cmd>) {
    let list = folders();
    if list.is_empty() && !ix::is_enabled() {
        return;
    }
    let mut open = !collapsed.contains(SECTION);
    collapsible_header(ui, tr!("views-smart-section"), &mut open);
    if open {
        collapsed.remove(SECTION);
    } else {
        collapsed.insert(SECTION.into());
        return;
    }
    let shown = shown_name(current);
    for (i, f) in list.iter().enumerate() {
        let active = shown.as_deref() == Some(f.name.as_str());
        let r = NavRow { icon: ph::FOLDER_SIMPLE_DOTTED, label: &f.name, active, icon_color: None, usage: None }
            .show(ui)
            .on_hover_text(&f.query);
        if r.clicked() {
            cmds.push(Cmd::Smart(SmartCmd::Open { index: i, new_tab: false }));
        }
        if r.middle_clicked() {
            cmds.push(Cmd::Smart(SmartCmd::Open { index: i, new_tab: true }));
        }
        crate::ui::native::popup::context_menu(&r, |ui| {
            ui.set_width(210.0);
            crate::ui::context_menu::draw_entries(ui, crate::models::context_menu::smart_folder_row(i), cmds);
        });
    }
    let r = NavRow {
        icon: ph::PLUS,
        label: tr!("views-smart-new"),
        active: false,
        icon_color: Some(palette(ui.ctx()).text_tertiary),
        usage: None,
    }
    .show(ui)
    .on_hover_text(tr!("views-smart-new-tip"));
    if r.clicked() {
        cmds.push(Cmd::Smart(SmartCmd::Edit(None)));
    }
}

/// Status bar chip: index activity; click opens the index settings.
pub fn status_chip(ui: &mut Ui, cmds: &mut Vec<Cmd>) {
    use soyuz_index::Phase;
    let Some(st) = ix::status() else { return };
    let p = palette(ui.ctx());
    let busy =
        matches!(st.phase, Phase::Starting | Phase::Crawling | Phase::Verifying | Phase::CatchingUp | Phase::Contents);
    let failed = matches!(st.phase, Phase::Failed(_));
    let label = match &st.phase {
        Phase::Crawling => match st.progress() {
            Some(pr) => tr!("views-index-chip-pct", pct = (pr * 100.0).round() as u64),
            None => tr!("views-index-chip-indexing").to_string(),
        },
        Phase::Verifying | Phase::CatchingUp => tr!("views-index-chip-updating").into(),
        Phase::Contents => tr!("views-index-chip-contents").into(),
        Phase::Starting => tr!("views-index-chip-starting").into(),
        Phase::Idle => tr!("views-index-chip-idle").into(),
        Phase::Failed(_) => tr!("views-index-chip-off").into(),
    };
    let font = FontId::proportional(11.5);
    let tw = ui.painter().layout_no_wrap(label.clone(), font.clone(), p.text_secondary).size().x;
    let (r, resp) = ui.allocate_exact_size(vec2(tw + 26.0, 18.0), Sense::click());
    crate::ui::a11y::button(&resp, &label);
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), p.bg_surface2);
    }
    let icon_c = r.left_center() + vec2(10.0, 0.0);
    let color = if failed { p.warning } else { p.text_secondary };
    if busy {
        spinner(ui, icon_c, 4.5, p.accent);
    } else {
        ui.painter().text(icon_c, Align2::CENTER_CENTER, ph::DATABASE, icons::font(12.0), color);
    }
    ui.painter().text(r.left_center() + vec2(20.0, 0.0), Align2::LEFT_CENTER, &label, font, color);
    let mut tip = status_line(&st);
    if !st.ready_roots.is_empty() {
        tip.push_str(&format!(
            "\n{}",
            st.ready_roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join("\n")
        ));
    }
    tip.push('\n');
    tip.push_str(tr!("views-index-chip-tip"));
    let resp = resp.on_hover_text(tip);
    if resp.clicked() {
        cmds.push(Cmd::Smart(SmartCmd::OpenSettings));
    }
    crate::ui::native::popup::context_menu(&resp, |ui| {
        ui.set_width(200.0);
        if menu_item(ui, Some(ph::GEAR_SIX), tr!("views-index-settings"), None, true).clicked() {
            cmds.push(Cmd::Smart(SmartCmd::OpenSettings));
            ui.close();
        }
        if menu_item(ui, Some(ph::ARROWS_CLOCKWISE), tr!("views-index-rebuild"), None, !failed).clicked() {
            cmds.push(Cmd::Smart(SmartCmd::RebuildIndex));
            ui.close();
        }
        if menu_item(ui, Some(ph::FOLDER_SIMPLE_DOTTED), tr!("views-smart-new"), None, true).clicked() {
            cmds.push(Cmd::Smart(SmartCmd::Edit(None)));
            ui.close();
        }
    });
}

// ---- dialogs ------------------------------------------------------------------------

/// Create / edit a smart folder.
pub struct SmartDialog {
    index: Option<usize>,
    folder: SmartFolder,
    focus: bool,
}

impl SmartDialog {
    pub fn new(index: Option<usize>, folder: SmartFolder) -> Self {
        SmartDialog { index, folder, focus: true }
    }
}

impl Dialog for SmartDialog {
    fn title(&self) -> &str {
        if self.index.is_some() { tr!("views-smart-dialog-edit") } else { tr!("views-smart-dialog-new") }
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::FOLDER_SIMPLE_DOTTED)
    }

    fn width(&self) -> f32 {
        520.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let label = |ui: &mut Ui, s: &str| {
            ui.label(RichText::new(s).font(FontId::proportional(11.5)).color(p.text_secondary));
        };
        label(ui, tr!("views-smart-name"));
        let n = ui.add(
            TextEdit::singleline(&mut self.folder.name)
                .hint_text(tr!("views-smart-name-hint"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if self.focus {
            n.request_focus();
            self.focus = false;
        }
        ui.add_space(8.0);
        label(ui, tr!("views-smart-query"));
        ui.add(
            TextEdit::singleline(&mut self.folder.query)
                .hint_text("kind:image modified:<30d -screenshot")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        let parsed = soyuz_index::parse(&self.folder.query);
        ui.add_space(4.0);
        match (&parsed, self.folder.query.trim().is_empty()) {
            (_, true) => {}
            (Err(e), false) => {
                ui.label(RichText::new(&e.msg).font(FontId::proportional(11.5)).color(p.danger));
            }
            (Ok(_), false) => {
                if let Some(text) = coverage_text(&self.folder.query, self.folder.include_hidden) {
                    ui.label(RichText::new(text).font(FontId::proportional(11.5)).color(p.text_tertiary));
                }
            }
        }
        ui.add_space(6.0);
        ui.checkbox(&mut self.folder.include_hidden, tr!("views-smart-include-hidden"));
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(p.bg_surface2)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                for (example, what) in query_help() {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(vec2(250.0, 16.0), Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_width(250.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(example).font(FontId::monospace(11.0)).color(p.text_primary),
                                )
                                .truncate(),
                            );
                        });
                        ui.label(RichText::new(what).font(FontId::proportional(11.0)).color(p.text_tertiary));
                    });
                }
            });
        let err = validate(&self.folder, self.index);
        ui.add_space(14.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, tr!("common-save"), Btn::Primary).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
            if let Some(e) = &err
                && !self.folder.name.is_empty()
            {
                ui.label(RichText::new(e).font(FontId::proportional(11.5)).color(p.text_tertiary));
            }
        });
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            ok = true;
        }
        if ok && err.is_none() {
            ctx.cmds.push(Cmd::Smart(SmartCmd::Save { index: self.index, folder: self.folder.clone() }));
            return false;
        }
        !cancel
    }
}

struct ConfirmDelete {
    index: usize,
    name: String,
}

impl Dialog for ConfirmDelete {
    fn title(&self) -> &str {
        tr!("views-smart-delete-title")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::TRASH)
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        ui.label(RichText::new(tr!("views-smart-delete-confirm", name = &self.name)).color(p.text_primary));
        ui.label(RichText::new(tr!("views-smart-delete-body")).font(FontId::proportional(11.5)).color(p.text_tertiary));
        ui.add_space(14.0);
        let mut ok = false;
        let mut cancel = false;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ok = button(ui, tr!("common-delete"), Btn::Danger).clicked();
            cancel = button(ui, tr!("common-cancel"), Btn::Secondary).clicked();
        });
        if ok {
            ctx.cmds.push(Cmd::Smart(SmartCmd::ConfirmDelete(self.index)));
            return false;
        }
        !cancel
    }
}
