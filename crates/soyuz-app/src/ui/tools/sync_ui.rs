//! "Sync profiles" window: every saved profile with its schedule, last run
//! (status, summary, when) and next run; Run now / Dry run / Open in Compare & sync;
//! a schedule editor and the saved run reports (what changed or would change, conflicts).

use std::collections::HashMap;
use std::time::SystemTime;

use crossbeam_channel::Receiver;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke, Ui, vec2,
};
use soyuz_core::DateFormat;

use super::sync_sched::{self, ReportOp, RunReport, RunStatus, Schedule, ScheduleKind, SyncProfile, short_path};
use super::{button, tool_window};
use crate::app::SoyuzApp;
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::settings::widgets::{number, segmented};
use crate::ui::widgets::trunc_galley;

pub fn status_color(s: RunStatus, p: &Palette) -> Color32 {
    match s {
        RunStatus::InSync | RunStatus::Applied => p.success,
        RunStatus::DryRun => p.accent,
        RunStatus::Partial => p.warning,
        RunStatus::Failed => p.danger,
        RunStatus::Cancelled => p.text_tertiary,
    }
}

#[derive(Default)]
pub struct SyncProfilesWindow {
    pub open: bool,
    expanded: Option<String>,
    reports: HashMap<String, Vec<RunReport>>,
    loading: Option<(String, Receiver<Vec<RunReport>>)>,
    /// (profile, report index) shown in detail.
    detail: Option<(String, usize)>,
}

impl SyncProfilesWindow {
    pub fn new() -> Self {
        SyncProfilesWindow { open: true, ..Default::default() }
    }

    fn load_reports(&mut self, name: &str) {
        self.loading = Some((name.to_string(), sync_sched::load_reports_async(name, 50)));
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        if let Some((name, rx)) = &self.loading
            && let Ok(v) = rx.try_recv()
        {
            self.reports.insert(name.clone(), v);
            self.loading = None;
        }
        let mut profiles = sync_sched::profiles();
        let n = profiles.len();
        let sub = tr!("tools-profiles-count", count = n);
        let mut changed = false;
        let mut open_compare: Option<SyncProfile> = None;
        let mut cancel: Option<String> = None;
        let enabled = app.core.config.sync.scheduler;
        let date = app.core.config.general.date_format.clone();
        let keep = tool_window(
            ctx,
            "sync_profiles",
            tr!("tools-profiles-title"),
            &sub,
            ph::CALENDAR_CHECK,
            vec2(880.0, 640.0),
            |ui| {
                let p = palette(ui.ctx());
                ui.horizontal(|ui| {
                    let (t, c) = if enabled {
                        (tr!("tools-profiles-scheduler-on"), p.text_tertiary)
                    } else {
                        (tr!("tools-profiles-scheduler-off"), p.warning)
                    };
                    ui.label(RichText::new(t).font(FontId::proportional(11.5)).color(c));
                });
                ui.add_space(6.0);
                if profiles.is_empty() {
                    super::compare::empty_note(
                        ui,
                        ph::FOLDER_SIMPLE_STAR,
                        tr!("tools-profiles-empty"),
                        tr!("tools-profiles-empty-body"),
                    );
                    return;
                }
                ScrollArea::vertical().id_salt("sync.profiles").auto_shrink([false, false]).show(ui, |ui| {
                    for pr in profiles.iter_mut() {
                        let (c, open, cancel_it) = self.card(ui, pr, enabled, &date);
                        changed |= c;
                        if open {
                            open_compare = Some(pr.clone());
                        }
                        if cancel_it {
                            cancel = Some(pr.name.clone());
                        }
                        ui.add_space(8.0);
                    }
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            },
        );
        if changed {
            sync_sched::set_profiles(profiles);
        }
        if let Some(name) = cancel {
            sync_sched::cancel(&app.core, &name);
        }
        if let Some(pr) = open_compare {
            super::queue(super::ToolCmd::CompareProfile(pr.name));
        }
        if !keep {
            self.open = false;
        }
    }

    /// One profile card. Returns (schedule changed, open in compare, cancel run).
    fn card(&mut self, ui: &mut Ui, pr: &mut SyncProfile, enabled: bool, date: &DateFormat) -> (bool, bool, bool) {
        let p = palette(ui.ctx());
        let mut changed = false;
        let mut open = false;
        let mut cancel = false;
        let last = sync_sched::last_report(&pr.name);
        let running = sync_sched::activity(&pr.name);
        let expanded = self.expanded.as_deref() == Some(pr.name.as_str());
        Frame::new()
            .fill(p.bg_window)
            .stroke(Stroke::new(1.0, p.border_subtle))
            .corner_radius(CornerRadius::same(radius::CARD))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(icons::icon(ph::FOLDER_SIMPLE_STAR, 18.0).color(p.accent));
                    ui.label(
                        RichText::new(&pr.name).font(crate::ui::views::flow::semibold(13.5)).color(p.text_primary),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let glyph = if expanded { ph::CARET_UP } else { ph::CARET_DOWN };
                        if crate::ui::widgets::icon_button(ui, glyph, tr!("tools-profiles-expand"), true).clicked() {
                            self.expanded = if expanded { None } else { Some(pr.name.clone()) };
                            self.detail = None;
                            if !expanded {
                                self.load_reports(&pr.name);
                            }
                        }
                        if running.is_some() {
                            cancel = button(ui, Some(ph::STOP), tr!("common-cancel"), 0, true).clicked();
                        } else {
                            if button(ui, Some(ph::PLAY), tr!("tools-profiles-run-now"), 1, true).clicked() {
                                sync_sched::run_now(&pr.name, false);
                            }
                            if button(ui, Some(ph::LIST_CHECKS), tr!("tools-status-dry-run"), 0, true)
                                .on_hover_text(tr!("tools-profiles-dry-run-tip"))
                                .clicked()
                            {
                                sync_sched::run_now(&pr.name, true);
                            }
                        }
                        if button(ui, Some(ph::GIT_DIFF), tr!("tools-open"), 0, true)
                            .on_hover_text(tr!("tools-profiles-open-tip"))
                            .clicked()
                        {
                            open = true;
                        }
                    });
                });
                let paths = tr!(
                    "tools-profiles-card-paths",
                    left = short_path(&pr.left),
                    right = short_path(&pr.right),
                    mode = pr.preset.label(),
                    method = pr.method.label()
                );
                let g = trunc_galley(
                    ui.painter(),
                    &paths,
                    FontId::proportional(11.5),
                    p.text_secondary,
                    ui.available_width(),
                );
                let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                ui.painter().galley(r.min, g, p.text_secondary);
                ui.interact(r, ui.id().with(("sync.paths", &pr.name)), Sense::hover()).on_hover_text(tr!(
                    "tools-profiles-card-paths-tip",
                    left = pr.left.display().to_string(),
                    right = pr.right.display().to_string()
                ));
                ui.add_space(4.0);
                // Status line.
                ui.horizontal(|ui| {
                    match (running, &last) {
                        (Some(what), _) => {
                            ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                            ui.label(RichText::new(what).font(FontId::proportional(12.0)).color(p.accent));
                        }
                        (None, Some(r)) => {
                            status_pill(ui, r.status, &p);
                            let when = soyuz_core::time::format_time_at(
                                r.finished.unwrap_or(r.started),
                                &DateFormat::Named(soyuz_core::DateFormatName::Relative),
                                SystemTime::now(),
                            );
                            let text = tr!(
                                "tools-profiles-last-run",
                                when = when,
                                trigger = r.trigger.label(),
                                summary = r.summary()
                            );
                            let g = trunc_galley(
                                ui.painter(),
                                &text,
                                FontId::proportional(12.0),
                                p.text_secondary,
                                ui.available_width() - 260.0,
                            );
                            let (rr, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                            ui.painter().galley(rr.min, g, p.text_secondary);
                        }
                        (None, None) => {
                            ui.label(
                                RichText::new(tr!("tools-profiles-never-run"))
                                    .font(FontId::proportional(12.0))
                                    .color(p.text_tertiary),
                            );
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(sync_sched::next_label(pr, enabled))
                                .font(FontId::proportional(12.0))
                                .color(p.text_secondary),
                        );
                        ui.label(icons::icon(ph::CLOCK, 13.0).color(p.text_tertiary));
                        ui.label(
                            RichText::new(pr.schedule.label()).font(FontId::proportional(12.0)).color(p.text_primary),
                        );
                    });
                });
                if expanded {
                    ui.add_space(8.0);
                    changed |= schedule_editor(ui, &mut pr.schedule);
                    ui.add_space(8.0);
                    self.reports_view(ui, &pr.name, date);
                }
            });
        (changed, open, cancel)
    }

    fn reports_view(&mut self, ui: &mut Ui, name: &str, date: &DateFormat) {
        let p = palette(ui.ctx());
        ui.label(
            RichText::new(tr!("tools-profiles-reports"))
                .font(crate::ui::views::flow::semibold(12.5))
                .color(p.text_primary),
        );
        let Some(list) = self.reports.get(name).cloned() else {
            ui.label(RichText::new(tr!("common-loading")).font(FontId::proportional(12.0)).color(p.text_tertiary));
            return;
        };
        // Refresh when a newer run finished meanwhile.
        if let Some(last) = sync_sched::last_report(name)
            && list.first().is_none_or(|f| f.started < last.started)
            && self.loading.is_none()
        {
            self.load_reports(name);
        }
        if list.is_empty() {
            ui.label(
                RichText::new(tr!("tools-profiles-no-runs")).font(FontId::proportional(12.0)).color(p.text_tertiary),
            );
            return;
        }
        ui.horizontal_top(|ui| {
            let lw = 300.0_f32.min(ui.available_width() * 0.45);
            ui.vertical(|ui| {
                ui.set_width(lw);
                ScrollArea::vertical().id_salt(("sync.reports", name)).max_height(220.0).show(ui, |ui| {
                    for (i, r) in list.iter().enumerate() {
                        let sel = self.detail.as_ref().is_some_and(|(n, k)| n == name && *k == i);
                        let (rect, resp) = ui.allocate_exact_size(vec2(lw, 34.0), Sense::click());
                        if sel {
                            ui.painter().rect_filled(rect, CornerRadius::same(6), p.accent.gamma_multiply(0.16));
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(6), p.bg_surface2);
                        }
                        let c = status_color(r.status, &p);
                        ui.painter().circle_filled(rect.left_center() + vec2(10.0, 0.0), 4.0, c);
                        let t = soyuz_core::format_time(r.started, date);
                        ui.painter().text(
                            rect.min + vec2(22.0, 4.0),
                            Align2::LEFT_TOP,
                            &t,
                            FontId::proportional(12.0),
                            p.text_primary,
                        );
                        let s =
                            tr!("tools-profiles-report-row", status = r.status.label(), trigger = r.trigger.label());
                        ui.painter().text(
                            rect.min + vec2(22.0, 19.0),
                            Align2::LEFT_TOP,
                            s,
                            FontId::proportional(10.5),
                            p.text_tertiary,
                        );
                        if resp.clicked() {
                            self.detail = Some((name.to_string(), i));
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                let sel =
                    self.detail.as_ref().filter(|(n, _)| n == name).and_then(|(_, k)| list.get(*k)).or(list.first());
                if let Some(r) = sel {
                    report_detail(ui, r);
                }
            });
        });
    }
}

fn status_pill(ui: &mut Ui, s: RunStatus, p: &Palette) {
    let c = status_color(s, p);
    let font = FontId::proportional(11.0);
    let g = ui.painter().layout_no_wrap(s.label().to_string(), font, c);
    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 16.0, 18.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(9), c.gamma_multiply(0.16));
    ui.painter().galley(r.center() - g.size() / 2.0, g, c);
}

/// Schedule controls; returns `true` when the schedule changed.
fn schedule_editor(ui: &mut Ui, s: &mut Schedule) -> bool {
    let p = palette(ui.ctx());
    let before = s.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("tools-sched-label")).font(FontId::proportional(12.0)).color(p.text_tertiary));
        let mut k = s.kind();
        let options = ScheduleKind::ALL.map(|k| (k, k.label()));
        if segmented(ui, &mut k, &options) {
            *s = k.default_schedule();
        }
    });
    match s {
        Schedule::Interval { minutes } => {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tr!("tools-sched-every")).font(FontId::proportional(12.0)).color(p.text_secondary),
                );
                number(ui, minutes, sync_sched::INTERVAL_MINUTES, &format!(" {}", tr!("tools-unit-min")));
            });
        }
        Schedule::Daily { hour, minute } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr!("tools-sched-at")).font(FontId::proportional(12.0)).color(p.text_secondary));
                number(ui, hour, sync_sched::DAILY_HOURS, &format!(" {}", tr!("tools-unit-hour")));
                number(ui, minute, sync_sched::DAILY_MINUTES, &format!(" {}", tr!("tools-unit-min")));
            });
            if let Some(note) = ScheduleKind::Daily.note() {
                ui.label(RichText::new(note).font(FontId::proportional(11.0)).color(p.text_tertiary));
            }
        }
        Schedule::OnChange => {
            if let Some(note) = ScheduleKind::OnChange.note() {
                ui.label(RichText::new(note).font(FontId::proportional(11.0)).color(p.text_tertiary));
            }
        }
        _ => {}
    }
    *s != before
}

fn report_detail(ui: &mut Ui, r: &RunReport) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        status_pill(ui, r.status, &p);
        let head = if r.dry_run {
            tr!("tools-report-would-change", summary = r.summary())
        } else {
            tr!("tools-report-changed", summary = r.summary())
        };
        ui.label(RichText::new(head).font(FontId::proportional(12.0)).color(p.text_secondary));
    });
    if !r.message.is_empty() {
        ui.label(RichText::new(&r.message).font(FontId::proportional(11.5)).color(p.warning));
    }
    let rows: Vec<(&str, Color32, String, &str)> = r
        .conflicts
        .iter()
        .map(|c| (ph::GIT_MERGE, p.danger, c.path.clone(), c.kind.label()))
        .chain(r.actions.iter().map(|a| {
            let (g, col) = match a.op {
                ReportOp::CopyToRight => (ph::ARROW_RIGHT, p.accent),
                ReportOp::CopyToLeft => (ph::ARROW_LEFT, p.accent),
                ReportOp::DeleteLeft | ReportOp::DeleteRight => (ph::TRASH, p.danger),
                ReportOp::KeepCopy => (ph::COPY, p.warning),
            };
            (g, col, a.path.clone(), a.op.label())
        }))
        .collect();
    if rows.is_empty() {
        ui.label(
            RichText::new(tr!("tools-report-nothing-to-change"))
                .font(FontId::proportional(12.0))
                .color(p.text_tertiary),
        );
        return;
    }
    ScrollArea::vertical().id_salt(("sync.report", r.started)).max_height(196.0).show_rows(
        ui,
        22.0,
        rows.len(),
        |ui, range| {
            for (g, col, path, what) in &rows[range] {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
                ui.painter().text(
                    rect.left_center() + vec2(9.0, 0.0),
                    Align2::CENTER_CENTER,
                    *g,
                    icons::font(12.5),
                    *col,
                );
                let what_r = ui.painter().text(
                    rect.left_center() + vec2(22.0, 0.0),
                    Align2::LEFT_CENTER,
                    *what,
                    FontId::proportional(11.0),
                    p.text_tertiary,
                );
                // Paths line up at 140 px unless a (translated) label is wider.
                let x = (rect.left() + 140.0).max(what_r.right() + 10.0);
                let gal = trunc_galley(
                    ui.painter(),
                    path,
                    FontId::proportional(12.0),
                    p.text_primary,
                    rect.right() - x - 10.0,
                );
                ui.painter().galley(egui::pos2(x, rect.center().y - gal.size().y / 2.0), gal, p.text_primary);
            }
        },
    );
    if r.total_actions > r.actions.len() {
        ui.label(
            RichText::new(tr!("tools-report-more", count = r.total_actions - r.actions.len()))
                .font(FontId::proportional(11.0))
                .color(p.text_tertiary),
        );
    }
}
