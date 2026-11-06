//! The column chooser of the process table: every column with its description, a search
//! box, and a reset.

use egui::{Margin, RichText, TextEdit, Ui};
use soyuz_core::config::monitor::ProcColumn;

use crate::cmd::Cmd;
use crate::models::monitor::{MonitorCmd, labels};
use crate::state::pane::PaneId;
use crate::theme::icons::ph;
use crate::theme::palette;
use crate::ui::dialogs::{Dialog, DialogCtx};
use crate::ui::widgets::{Btn, pill_button};

pub struct MonitorColumnsDialog {
    pane: PaneId,
    shown: Vec<ProcColumn>,
    query: String,
    focus: bool,
}

impl MonitorColumnsDialog {
    pub fn new(pane: PaneId, shown: Vec<ProcColumn>) -> Self {
        MonitorColumnsDialog { pane, shown, query: String::new(), focus: true }
    }
}

impl Dialog for MonitorColumnsDialog {
    fn title(&self) -> &str {
        tr!("monitor-gui-columns")
    }

    fn icon(&self) -> Option<&'static str> {
        Some(ph::COLUMNS)
    }

    fn width(&self) -> f32 {
        460.0
    }

    fn show(&mut self, ui: &mut Ui, ctx: &mut DialogCtx<'_>) -> bool {
        let p = palette(ui.ctx());
        let r = ui.add(
            TextEdit::singleline(&mut self.query)
                .hint_text(tr!("monitor-gui-search-columns"))
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 6)),
        );
        crate::ui::a11y::name(&r, tr!("monitor-gui-search-columns"));
        if self.focus {
            r.request_focus();
            self.focus = false;
        }
        ui.add_space(8.0);
        let q = self.query.trim().to_lowercase();
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for col in ProcColumn::ALL {
                let title = labels::column_title(col);
                let desc = labels::column_desc(col);
                if !q.is_empty()
                    && !title.to_lowercase().contains(&q)
                    && !desc.to_lowercase().contains(&q)
                    && !col.id().contains(&q)
                {
                    continue;
                }
                let mut on = self.shown.contains(&col);
                ui.horizontal(|ui| {
                    let r = ui.add_enabled(
                        col != ProcColumn::Name,
                        egui::Checkbox::new(&mut on, RichText::new(title).color(p.text_primary)),
                    );
                    if r.changed() {
                        if on {
                            self.shown.push(col);
                        } else {
                            self.shown.retain(|x| *x != col);
                        }
                        ctx.cmds.push(Cmd::Monitor(self.pane, MonitorCmd::ToggleColumn(col)));
                    }
                });
                ui.add(
                    egui::Label::new(RichText::new(desc).size(crate::theme::font::px(11.0)).color(p.text_tertiary))
                        .wrap(),
                );
                ui.add_space(4.0);
            }
        });
        ui.add_space(16.0);
        let mut close = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pill_button(ui, None, tr!("dialogs-done"), Btn::Primary).clicked() {
                close = true;
            }
            ui.add_space(6.0);
            if pill_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("common-reset"), Btn::Secondary).clicked() {
                self.shown = soyuz_core::config::default_proc_columns().iter().map(|c| c.id).collect();
                ctx.cmds.push(Cmd::Monitor(self.pane, MonitorCmd::ResetColumns));
            }
        });
        !close && !crate::ui::dialogs::enter_default(ui)
    }
}
