//! Folder compare + sync window. The logic (rows, actions, presets, plans, the two-way
//! state, the scan worker and the compare state machine) lives in
//! `soyuz_appcore::models::compare` ([`CompareModel`]); this window keeps the egui state
//! and draws the model: a table of rows (only-left, only-right, newer-left, newer-right,
//! differ) with a per-row action (copy →, copy ←, delete, skip), presets (two-way update,
//! mirror left → right, mirror right → left), the plan preview and the two-way conflict
//! stage (keep left / keep right / keep both / skip, apply to all). The plan is submitted
//! as one `JobKind::Sync` job. Profiles are saved in `sync_profiles.toml` next to the
//! config and owned by the scheduler (`sync_sched`).

use std::path::PathBuf;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2,
};
use soyuz_core::{SizeStyle, format_size, format_time};
use soyuz_jobs::{SyncAction, SyncPlan};

pub use crate::models::compare::{
    Category, CompareModel, Method, PlanStats, Preset, ReviewStep, Row, RowAction, Side, Stage, allowed, two_way_reason,
};
use crate::models::compare::{RESOLUTIONS, resolution_allowed, resolution_tip, row_resolution};
use crate::models::compare_profiles::SyncProfile;
use crate::models::compare_state::Resolution;

use super::{button, filter_chip, progress_bar, text_field, tool_window};
use crate::app::SoyuzApp;
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::widgets::{menu_item, menu_separator, trunc_galley};

/// A category's colour (chips, row bars, folder glyphs).
pub trait CategoryColor {
    fn color(self, p: &Palette) -> Color32;
}

impl CategoryColor for Category {
    fn color(self, p: &Palette) -> Color32 {
        match self {
            Category::OnlyLeft => p.accent,
            Category::NewerLeft => p.success,
            Category::Differ => p.danger,
            Category::NewerRight => p.warning,
            Category::OnlyRight => p.glyph_video,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Window.

pub struct CompareWindow {
    pub open: bool,
    /// The compare state machine (paths, method, mode, rows, scan, job).
    m: CompareModel,
}

impl CompareWindow {
    pub fn new(left: PathBuf, right: Option<PathBuf>, profiles: Vec<SyncProfile>) -> Self {
        CompareWindow { open: true, m: CompareModel::new(left, right, profiles, super::wake()) }
    }

    /// Open with a saved profile loaded and compared (profiles window, conflict toasts).
    pub fn with_profile(pr: &SyncProfile, profiles: Vec<SyncProfile>) -> Self {
        CompareWindow { open: true, m: CompareModel::with_profile(pr, profiles, super::wake()) }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        // Scan result, end of the sync job (record the two-way state, compare again).
        self.m.poll(&mut app.core);
        let sub = match self.m.compared() {
            Some((l, r)) => {
                format!("{}  ↔  {}", soyuz_core::entry::display_name(l), soyuz_core::entry::display_name(r))
            }
            None => String::new(),
        };
        let mut submit = None;
        let keep =
            tool_window(ctx, "compare", tr!("tools-compare-title"), &sub, ph::GIT_DIFF, vec2(1040.0, 660.0), |ui| {
                submit = self.body(ui, app);
            });
        if let Some(plan) = submit {
            self.m.submit_plan(&mut app.core, plan);
        }
        if !keep {
            self.m.close();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) -> Option<SyncPlan> {
        let p = palette(ui.ctx());
        // Paths.
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 44.0) / 2.0;
            ui.label(icons::icon(ph::FOLDER_SIMPLE, 14.0).color(Category::OnlyLeft.color(&p)));
            text_field(ui, &mut self.m.left, tr!("tools-compare-left-hint"), w - 22.0);
            if crate::ui::widgets::icon_button(ui, ph::ARROWS_LEFT_RIGHT, tr!("tools-compare-swap"), true).clicked() {
                self.m.swap();
            }
            text_field(ui, &mut self.m.right, tr!("tools-compare-right-hint"), w - 22.0);
            ui.label(icons::icon(ph::FOLDER_SIMPLE, 14.0).color(Category::OnlyRight.color(&p)));
        });
        ui.add_space(4.0);
        // Method, preset, compare, profiles.
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("tools-compare-by")).font(FontId::proportional(12.0)).color(p.text_tertiary));
            for m in Method::ALL {
                let mut on = self.m.method == m;
                if super::toggle_chip(ui, m.label(), &mut on, p.accent).clicked() {
                    self.m.set_method(m);
                }
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(tr!("tools-compare-sync-mode")).font(FontId::proportional(12.0)).color(p.text_tertiary),
            );
            for pr in Preset::ALL {
                let mut on = self.m.preset == pr;
                let mut chip = super::toggle_chip(ui, pr.label(), &mut on, p.accent);
                if pr == Preset::TwoWay {
                    chip = chip.on_hover_text(tr!("tools-preset-two-way-tip"));
                }
                if chip.clicked() {
                    // Two-way plans differently: compares again when switching in or out.
                    self.m.set_preset(pr);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.m.is_scanning() {
                    if button(ui, Some(ph::STOP), tr!("common-cancel"), 0, true).clicked() {
                        self.m.cancel();
                    }
                } else if button(ui, Some(ph::ARROW_CLOCKWISE), tr!("tools-compare-start"), 1, true).clicked() {
                    self.m.start();
                }
                self.profiles_menu(ui);
            });
        });
        ui.add_space(6.0);
        match self.m.stage() {
            Stage::Setup if self.m.rows().is_empty() => {
                empty_note(ui, ph::GIT_DIFF, tr!("tools-compare-empty"), tr!("tools-compare-empty-body"));
                return None;
            }
            Stage::Scanning { started, .. } => {
                progress_bar(ui, ui.available_width(), None);
                ui.label(
                    RichText::new(tr!(
                        "tools-compare-running",
                        secs = soyuz_core::locale::format_decimal(started.elapsed().as_secs_f64(), 1)
                    ))
                    .font(FontId::proportional(12.0))
                    .color(p.text_secondary),
                );
                return None;
            }
            Stage::Submitted(job) => {
                let snap = app.core.fileops.jobs.snapshot(*job);
                let frac = snap.as_ref().and_then(|s| {
                    (s.progress.items_total > 0).then(|| s.progress.items_done as f32 / s.progress.items_total as f32)
                });
                progress_bar(ui, ui.available_width(), frac);
                ui.label(
                    RichText::new(tr!("tools-compare-syncing"))
                        .font(FontId::proportional(12.0))
                        .color(p.text_secondary),
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                return None;
            }
            Stage::Recording(_) => {
                progress_bar(ui, ui.available_width(), None);
                ui.label(
                    RichText::new(tr!("tools-compare-recording"))
                        .font(FontId::proportional(12.0))
                        .color(p.text_secondary),
                );
                return None;
            }
            Stage::Failed(e) => {
                let e = e.clone();
                empty_note(ui, ph::WARNING_CIRCLE, tr!("tools-compare-failed"), &e);
                return None;
            }
            _ => {}
        }
        if self.m.step == ReviewStep::Plan {
            return self.preview(ui);
        }
        if self.m.step == ReviewStep::Conflicts {
            self.conflicts(ui, app);
            return None;
        }
        // Filters.
        let counts = self.m.counts();
        ui.horizontal(|ui| {
            for (i, c) in Category::ALL.iter().enumerate() {
                filter_chip(ui, c.label(), counts[i], c.color(&p), &mut self.m.show[i]);
            }
            ui.label(
                RichText::new(tr!("tools-compare-identical", count = self.m.same()))
                    .font(FontId::proportional(12.0))
                    .color(p.text_tertiary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                text_field(ui, &mut self.m.search, tr!("tools-compare-filter-hint"), 180.0);
            });
        });
        ui.add_space(4.0);
        if self.m.first_run() && !self.m.rows().is_empty() {
            ui.label(
                RichText::new(tr!("tools-compare-first-run")).font(FontId::proportional(11.5)).color(p.text_tertiary),
            );
        }
        if self.m.rows().is_empty() {
            empty_note(ui, ph::CHECK_CIRCLE, tr!("tools-compare-in-sync"), tr!("tools-compare-in-sync-body"));
            return None;
        }
        let vis = self.m.visible_rows();
        let footer_h = 40.0;
        let table_h = (ui.available_height() - footer_h).max(80.0);
        let width = ui.available_width();
        self.table(ui, app, &vis, width, table_h);
        // Footer: plan summary + preview.
        let stats = self.m.stats();
        let total = stats.total();
        let (conflicts, open) = self.m.conflict_counts();
        let mut go = false;
        let mut resolve = false;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            plan_line(ui, &p, &stats);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                go = button(ui, Some(ph::ARROW_RIGHT), tr!("tools-compare-review"), 1, total > 0).clicked();
                if conflicts > 0 {
                    let label = if open > 0 {
                        tr!("tools-compare-resolve-n", count = open)
                    } else {
                        tr!("tools-compare-conflicts-button").to_string()
                    };
                    let kind = if open > 0 { 2 } else { 0 };
                    resolve = button(ui, Some(ph::GIT_MERGE), &label, kind, true).clicked();
                }
            });
        });
        if go {
            self.m.step = ReviewStep::Plan;
        }
        if resolve {
            self.m.step = ReviewStep::Conflicts;
        }
        None
    }

    fn profiles_menu(&mut self, ui: &mut Ui) {
        let resp = button(ui, Some(ph::FLOPPY_DISK), tr!("tools-profiles"), 0, true);
        crate::ui::native::popup::Menu::menu(&resp).width(260.0).show(|ui| {
            ui.set_width(260.0);
            if self.m.profiles().is_empty() {
                menu_item(ui, Some(ph::INFO), tr!("tools-profiles-none"), None, false);
            }
            let mut load = None;
            let mut delete = None;
            for (i, pr) in self.m.profiles().iter().enumerate() {
                ui.horizontal(|ui| {
                    let r = menu_item(ui, Some(ph::FOLDER_SIMPLE_STAR), &pr.name, None, true).on_hover_text(tr!(
                        "tools-profile-tip",
                        left = pr.left.display().to_string(),
                        right = pr.right.display().to_string(),
                        method = pr.method.label(),
                        mode = pr.preset.label()
                    ));
                    if r.clicked() {
                        load = Some(i);
                    }
                    if r.secondary_clicked() {
                        delete = Some(i);
                    }
                });
            }
            if let Some(i) = load {
                self.m.load_profile_at(i);
                ui.close();
            }
            if let Some(i) = delete {
                self.m.delete_profile_at(i);
            }
            menu_separator(ui);
            ui.label(
                RichText::new(tr!("tools-profiles-save-as"))
                    .font(FontId::proportional(11.0))
                    .color(palette(ui.ctx()).text_tertiary),
            );
            ui.horizontal(|ui| {
                text_field(ui, &mut self.m.profile_name, tr!("tools-profiles-name-hint"), 170.0);
                if button(ui, None, tr!("common-save"), 1, self.m.can_save_profile()).clicked() {
                    self.m.save_profile();
                    ui.close();
                }
            });
            ui.label(
                RichText::new(tr!("tools-profiles-delete-hint"))
                    .font(FontId::proportional(10.5))
                    .color(palette(ui.ctx()).text_tertiary),
            );
            menu_separator(ui);
            if menu_item(ui, Some(ph::CALENDAR_CHECK), tr!("tools-profiles-schedules"), None, true).clicked() {
                super::queue(super::ToolCmd::SyncProfiles);
                ui.close();
            }
        });
    }

    fn table(&mut self, ui: &mut Ui, app: &SoyuzApp, vis: &[usize], width: f32, height: f32) {
        let p = palette(ui.ctx());
        let row_h = 28.0;
        let meta_w = 190.0;
        let act_w = 132.0;
        let name_w = (width - 2.0 * meta_w - act_w).max(160.0);
        // Header with bulk actions.
        let (hr, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());
        let painter = ui.painter();
        let hfont = FontId::proportional(11.0);
        painter.text(
            pos2(hr.left() + 10.0, hr.center().y),
            Align2::LEFT_CENTER,
            tr!("tools-compare-col-left"),
            hfont.clone(),
            p.text_tertiary,
        );
        painter.text(
            pos2(hr.left() + meta_w + 10.0, hr.center().y),
            Align2::LEFT_CENTER,
            tr!("tools-compare-col-name"),
            hfont.clone(),
            p.text_tertiary,
        );
        painter.text(
            pos2(hr.right() - 10.0, hr.center().y),
            Align2::RIGHT_CENTER,
            tr!("tools-compare-col-right"),
            hfont,
            p.text_tertiary,
        );
        let ar = Rect::from_min_size(pos2(hr.left() + meta_w + name_w, hr.top() + 1.0), vec2(act_w, 24.0));
        if let Some(a) = action_buttons(ui, ar, None, None, tr!("tools-compare-all-visible")) {
            self.m.set_actions(vis, a);
        }
        ui.painter().line_segment(
            [pos2(hr.left(), hr.bottom()), pos2(hr.right(), hr.bottom())],
            Stroke::new(1.0, p.border_subtle),
        );
        let cfg = &app.core.config.general;
        let drag_default = app.core.config.behaviour.drag_default;
        // The folders the rows are relative to.
        let bases = self.m.compared().cloned();
        let two_way = self.m.two_way().is_some();
        let mut set: Option<(usize, RowAction)> = None;
        ScrollArea::vertical().id_salt("compare.rows").max_height(height - 28.0).auto_shrink([false, false]).show_rows(
            ui,
            row_h,
            vis.len(),
            |ui, range| {
                for k in range {
                    let i = vis[k];
                    let (r, resp) = ui.allocate_exact_size(vec2(width, row_h), Sense::drag());
                    let row = &self.m.rows()[i];
                    // Dragging the left / right column takes that side's copy; the name
                    // column takes the left one when it exists.
                    if let Some((lb, rb)) = &bases {
                        super::drag_source(&resp, drag_default, || {
                            let x = ui.input(|i| i.pointer.press_origin()).map_or(r.center().x, |p| p.x);
                            let left = if x < r.left() + meta_w {
                                true
                            } else if x > r.right() - meta_w {
                                false
                            } else {
                                row.left.is_some()
                            };
                            let (side, base) = if left { (&row.left, lb) } else { (&row.right, rb) };
                            side.as_ref().map(|_| vec![base.join(&row.rel)]).unwrap_or_default()
                        });
                    }
                    let painter = ui.painter();
                    if resp.hovered() {
                        painter.rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                    }
                    let color = row.cat.color(&p);
                    // Left meta.
                    let meta = |s: &Option<Side>| -> String {
                        match s {
                            Some(m) if m.dir => {
                                tr!(
                                    "tools-compare-meta-folder",
                                    modified = m.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default()
                                )
                            }
                            Some(m) => format!(
                                "{}  ·  {}",
                                format_size(m.size, cfg.size_style),
                                m.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default()
                            ),
                            None => "—".into(),
                        }
                    };
                    let lc = if matches!(row.cat, Category::NewerLeft | Category::OnlyLeft) {
                        p.text_primary
                    } else {
                        p.text_secondary
                    };
                    let rc = if matches!(row.cat, Category::NewerRight | Category::OnlyRight) {
                        p.text_primary
                    } else {
                        p.text_secondary
                    };
                    let g = trunc_galley(painter, &meta(&row.left), FontId::proportional(11.5), lc, meta_w - 16.0);
                    painter.galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, lc);
                    let g = trunc_galley(painter, &meta(&row.right), FontId::proportional(11.5), rc, meta_w - 16.0);
                    painter.galley(pos2(r.right() - 10.0 - g.size().x, r.center().y - g.size().y / 2.0), g, rc);
                    // Name with category bar.
                    let nx = r.left() + meta_w;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(nx, r.top() + 6.0), vec2(3.0, row_h - 12.0)),
                        CornerRadius::same(1),
                        color,
                    );
                    let is_dir = row.is_dir();
                    let glyph = if is_dir { ph::FOLDER_SIMPLE } else { ph::FILE };
                    painter.text(
                        pos2(nx + 18.0, r.center().y),
                        Align2::CENTER_CENTER,
                        glyph,
                        icons::font(14.0),
                        if is_dir { p.glyph_folder } else { p.text_secondary },
                    );
                    let name = row.rel.to_string_lossy();
                    let skipped = row.action == RowAction::Skip;
                    let ncolor = if skipped { p.text_secondary } else { p.text_primary };
                    let g = trunc_galley(painter, &name, FontId::proportional(12.5), ncolor, name_w - 40.0 - 80.0);
                    let gw = g.size().x;
                    painter.galley(pos2(nx + 30.0, r.center().y - g.size().y / 2.0), g, ncolor);
                    let (tag_text, tag_color) = match row.conflict {
                        Some(_) if row.action == RowAction::Skip => (tr!("tools-git-conflict"), p.danger),
                        Some(_) => (tr!("tools-compare-resolved"), p.success),
                        None if two_way => (two_way_reason(row), color),
                        None => (row.cat.label(), color),
                    };
                    let tag = trunc_galley(painter, tag_text, FontId::proportional(10.5), tag_color, 96.0);
                    let tag_rect =
                        Rect::from_min_size(pos2(nx + 36.0 + gw, r.center().y - tag.size().y / 2.0), tag.size());
                    painter.galley(tag_rect.min, tag, tag_color);
                    let conflict = row.conflict;
                    if let Some(k) = conflict {
                        ui.interact(tag_rect, ui.id().with(("compare.conflict", i)), Sense::hover())
                            .on_hover_text(k.label());
                    }
                    // Actions.
                    let ar = Rect::from_min_size(pos2(nx + name_w, r.top() + 2.0), vec2(act_w, row_h - 4.0));
                    let cat = row.cat;
                    let current = row.action;
                    if let Some(a) = action_buttons(ui, ar, Some(cat), Some(current), "") {
                        set = Some((i, a));
                    }
                }
            },
        );
        // A hand edit turns the mode into "custom" (two-way keeps its mode).
        if let Some((i, a)) = set {
            self.m.set_action(i, a);
        }
    }

    fn preview(&mut self, ui: &mut Ui) -> Option<SyncPlan> {
        let p = palette(ui.ctx());
        let (Some(plan), stats) = (self.m.plan(), self.m.stats()) else { return None };
        let (_, open) = self.m.conflict_counts();
        ui.label(
            RichText::new(tr!("tools-compare-plan-title"))
                .font(crate::ui::views::flow::semibold(14.0))
                .color(p.text_primary),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| plan_line(ui, &p, &stats));
        if stats.deletes() {
            ui.label(
                RichText::new(tr!("tools-compare-plan-trash-note")).font(FontId::proportional(12.0)).color(p.warning),
            );
        }
        if open > 0 {
            ui.label(
                RichText::new(tr!("tools-compare-plan-open-conflicts", count = open))
                    .font(FontId::proportional(12.0))
                    .color(p.warning),
            );
        }
        ui.add_space(6.0);
        let h = (ui.available_height() - 44.0).max(80.0);
        ScrollArea::vertical().id_salt("compare.plan").max_height(h).auto_shrink([false, false]).show_rows(
            ui,
            24.0,
            plan.actions.len(),
            |ui, range| {
                for a in &plan.actions[range] {
                    let (glyph, verb, rel, color) = match a {
                        SyncAction::CopyLR(x) => (ph::ARROW_RIGHT, tr!("tools-compare-copy-right"), x, p.accent),
                        SyncAction::CopyRL(x) => (ph::ARROW_LEFT, tr!("tools-compare-copy-left"), x, p.accent),
                        SyncAction::DeleteL(x) => (ph::TRASH, tr!("tools-compare-delete-left"), x, p.danger),
                        SyncAction::DeleteR(x) => (ph::TRASH, tr!("tools-compare-delete-right"), x, p.danger),
                        SyncAction::CopyAs { dst_left, to, .. } => {
                            let verb = if *dst_left {
                                tr!("tools-compare-conflict-copy-left")
                            } else {
                                tr!("tools-compare-conflict-copy-right")
                            };
                            (ph::COPY, verb, to, p.warning)
                        }
                    };
                    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
                    let painter = ui.painter();
                    painter.text(
                        pos2(row.left() + 12.0, row.center().y),
                        Align2::CENTER_CENTER,
                        glyph,
                        icons::font(13.0),
                        color,
                    );
                    let verb_r = painter.text(
                        pos2(row.left() + 28.0, row.center().y),
                        Align2::LEFT_CENTER,
                        verb,
                        FontId::proportional(12.0),
                        p.text_secondary,
                    );
                    // Paths line up at 140 px unless a (translated) verb is wider.
                    let rel_x = (row.left() + 140.0).max(verb_r.right() + 12.0);
                    let g = trunc_galley(
                        painter,
                        &rel.to_string_lossy(),
                        FontId::proportional(12.5),
                        p.text_primary,
                        row.right() - rel_x - 20.0,
                    );
                    painter.galley(pos2(rel_x, row.center().y - g.size().y / 2.0), g, p.text_primary);
                }
            },
        );
        let mut out = None;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if button(ui, Some(ph::ARROW_LEFT), tr!("tools-back"), 0, true).clicked() {
                self.m.step = ReviewStep::Table;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let danger = stats.deletes();
                let label = tr!("tools-compare-start-sync", count = plan.actions.len());
                if button(ui, Some(ph::PLAY), &label, if danger { 2 } else { 1 }, !plan.actions.is_empty()).clicked() {
                    out = Some(plan.clone());
                }
            });
        });
        out
    }
}

impl CompareWindow {
    /// Conflict stage (two-way): per item keep left / right / both / skip, plus apply to all.
    fn conflicts(&mut self, ui: &mut Ui, app: &SoyuzApp) {
        let p = palette(ui.ctx());
        let cfg = &app.core.config.general;
        let idx = self.m.conflict_rows();
        let (_, open) = self.m.conflict_counts();
        ui.label(
            RichText::new(tr!("tools-compare-conflicts-title"))
                .font(crate::ui::views::flow::semibold(14.0))
                .color(p.text_primary),
        );
        ui.label(
            RichText::new(tr!("tools-compare-conflicts-body", count = idx.len()))
                .font(FontId::proportional(12.0))
                .color(p.text_secondary),
        );
        ui.add_space(4.0);
        // Apply to all.
        let mut all: Option<Option<Resolution>> = None;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tr!("tools-compare-apply-all")).font(FontId::proportional(12.0)).color(p.text_tertiary),
            );
            for r in RESOLUTIONS {
                if button(ui, None, r.label(), 0, true).clicked() {
                    all = Some(Some(r));
                }
            }
            if button(ui, Some(ph::CLOCK), tr!("tools-compare-keep-newer"), 0, true)
                .on_hover_text(tr!("tools-compare-keep-newer-tip"))
                .clicked()
            {
                all = Some(None);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (t, c) = if open == 0 {
                    (tr!("tools-compare-all-decided").to_string(), p.success)
                } else {
                    (tr!("tools-compare-n-open", count = open), p.warning)
                };
                ui.label(RichText::new(t).font(FontId::proportional(12.0)).color(c));
            });
        });
        if let Some(choice) = all {
            // `None` keeps the newer side.
            self.m.resolve_all(choice);
        }
        ui.add_space(6.0);
        let row_h = 52.0;
        let h = (ui.available_height() - 44.0).max(80.0);
        let meta = |s: &Option<Side>| -> String {
            match s {
                None => tr!("tools-compare-meta-deleted").into(),
                Some(m) if m.dir => tr!("tools-compare-meta-folder-only").into(),
                Some(m) => format!(
                    "{}  ·  {}",
                    format_size(m.size, cfg.size_style),
                    m.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default()
                ),
            }
        };
        let mut set: Option<(usize, Resolution)> = None;
        ScrollArea::vertical().id_salt("compare.conflicts").max_height(h).auto_shrink([false, false]).show_rows(
            ui,
            row_h,
            idx.len(),
            |ui, range| {
                for k in range {
                    let i = idx[k];
                    let row = &self.m.rows()[i];
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface2);
                    }
                    let painter = ui.painter();
                    let current = row_resolution(row);
                    let decided = current != Resolution::Skip;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(r.left() + 2.0, r.top() + 8.0), vec2(3.0, row_h - 16.0)),
                        CornerRadius::same(1),
                        if decided { p.success } else { p.danger },
                    );
                    // Choice buttons: at least 78 px, wider for longer (translated) labels.
                    let choice_w = |res: Resolution| {
                        let g = painter.layout_no_wrap(
                            res.label().to_string(),
                            FontId::proportional(11.5),
                            p.text_secondary,
                        );
                        (g.size().x + 16.0).max(78.0)
                    };
                    let choices = RESOLUTIONS;
                    let widths = choices.map(choice_w);
                    let choices_w = widths.iter().sum::<f32>() + 4.0 * (choices.len() as f32 - 1.0) + 6.0;
                    let text_w = (r.width() - choices_w - 24.0).max(120.0);
                    let name = row.rel.to_string_lossy();
                    let g =
                        trunc_galley(painter, &name, crate::ui::views::flow::semibold(12.5), p.text_primary, text_w);
                    let gw = g.size().x;
                    painter.galley(pos2(r.left() + 14.0, r.top() + 7.0), g, p.text_primary);
                    if let Some(kind) = row.conflict {
                        let t = trunc_galley(painter, kind.label(), FontId::proportional(10.5), p.danger, 200.0);
                        painter.galley(pos2(r.left() + 22.0 + gw, r.top() + 9.0), t, p.danger);
                    }
                    let lm = tr!("tools-compare-meta-left", meta = meta(&row.left));
                    let rm = tr!("tools-compare-meta-right", meta = meta(&row.right));
                    let half = (text_w / 2.0 - 6.0).max(60.0);
                    let lg = trunc_galley(painter, &lm, FontId::proportional(11.5), p.text_secondary, half);
                    painter.galley(pos2(r.left() + 14.0, r.top() + 28.0), lg, p.text_secondary);
                    let rg = trunc_galley(painter, &rm, FontId::proportional(11.5), p.text_secondary, half);
                    painter.galley(pos2(r.left() + 14.0 + half + 12.0, r.top() + 28.0), rg, p.text_secondary);
                    // Choices.
                    let mut x = r.right() - choices_w;
                    for (res, w) in choices.into_iter().zip(widths) {
                        let cr = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(w, 26.0));
                        x += w + 4.0;
                        let ok = resolution_allowed(row, res);
                        let id = ui.id().with(("compare.res", i, res as u8));
                        let resp = ui.interact(cr, id, if ok { Sense::click() } else { Sense::hover() });
                        let on = current == res;
                        let accent = match res {
                            Resolution::Skip => p.text_secondary,
                            Resolution::KeepBoth => p.warning,
                            _ => p.accent,
                        };
                        let fill = if on {
                            accent.gamma_multiply(0.22)
                        } else if resp.hovered() && ok {
                            p.bg_surface3
                        } else {
                            p.bg_surface2
                        };
                        ui.painter().rect_filled(cr, CornerRadius::same(6), fill);
                        let fg = if !ok {
                            p.text_tertiary.gamma_multiply(0.4)
                        } else if on {
                            accent
                        } else {
                            p.text_secondary
                        };
                        ui.painter().text(
                            cr.center(),
                            Align2::CENTER_CENTER,
                            res.label(),
                            FontId::proportional(11.5),
                            fg,
                        );
                        let tip = resolution_tip(row, res);
                        if ok && resp.on_hover_text(tip).clicked() {
                            set = Some((i, res));
                        }
                    }
                }
            },
        );
        if let Some((i, res)) = set {
            self.m.resolve(i, res);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if button(ui, Some(ph::ARROW_LEFT), tr!("tools-back"), 0, true).clicked() {
                self.m.step = ReviewStep::Table;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, Some(ph::ARROW_RIGHT), tr!("tools-compare-review"), 1, true).clicked() {
                    self.m.step = ReviewStep::Plan;
                }
            });
        });
    }
}

/// Four action buttons (← · → ✕) in `rect`; returns the clicked action.
fn action_buttons(
    ui: &mut Ui,
    rect: Rect,
    cat: Option<Category>,
    current: Option<RowAction>,
    tip: &str,
) -> Option<RowAction> {
    let p = palette(ui.ctx());
    let items = [
        (RowAction::CopyLeft, ph::ARROW_LEFT, tr!("tools-compare-copy-left")),
        (RowAction::Skip, ph::MINUS, tr!("common-skip")),
        (RowAction::CopyRight, ph::ARROW_RIGHT, tr!("tools-compare-copy-right")),
        (RowAction::Delete, ph::TRASH, tr!("common-delete")),
    ];
    let w = rect.width() / items.len() as f32;
    let mut out = None;
    if current.is_some() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.bg_surface2.gamma_multiply(0.6));
    }
    for (k, (a, glyph, label)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + k as f32 * w, rect.top()), vec2(w, rect.height())).shrink(1.0);
        let ok = cat.is_none_or(|c| allowed(c, *a));
        let id = ui.id().with(("compare.act", rect.min.x as i32, rect.min.y as i32, k));
        let resp = ui.interact(r, id, if ok { Sense::click() } else { Sense::hover() });
        let on = current == Some(*a);
        let color = match a {
            RowAction::Delete => p.danger,
            RowAction::Skip => p.text_secondary,
            _ => p.accent,
        };
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(5), color.gamma_multiply(0.22));
        } else if resp.hovered() && ok {
            ui.painter().rect_filled(r, CornerRadius::same(5), p.bg_surface3);
        }
        let fg = if !ok {
            p.text_tertiary.gamma_multiply(0.35)
        } else if on {
            color
        } else {
            p.text_tertiary
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *glyph, icons::font(13.0), fg);
        let hint = if tip.is_empty() { label.to_string() } else { format!("{label} ({tip})") };
        if ok && resp.on_hover_text(hint).clicked() {
            out = Some(*a);
        }
    }
    out
}

fn plan_line(ui: &mut Ui, p: &Palette, s: &PlanStats) {
    let small = |t: String, c: Color32| RichText::new(t).font(FontId::proportional(12.0)).color(c);
    ui.label(small(
        tr!("tools-plan-to-right", count = s.to_right),
        if s.to_right > 0 { p.accent } else { p.text_tertiary },
    ));
    ui.label(small("·".into(), p.text_tertiary));
    ui.label(small(
        tr!("tools-plan-to-left", count = s.to_left),
        if s.to_left > 0 { p.accent } else { p.text_tertiary },
    ));
    ui.label(small("·".into(), p.text_tertiary));
    let del = s.delete_left + s.delete_right;
    ui.label(small(tr!("tools-plan-to-delete", count = del), if del > 0 { p.danger } else { p.text_tertiary }));
    if s.bytes > 0 {
        ui.label(small("·".into(), p.text_tertiary));
        ui.label(small(tr!("tools-plan-to-copy", size = format_size(s.bytes, SizeStyle::Binary)), p.text_secondary));
    }
}

/// Centered glyph + title + subtitle filling the remaining space.
pub fn empty_note(ui: &mut Ui, glyph: &str, title: &str, sub: &str) {
    let p = palette(ui.ctx());
    let (r, _) = ui.allocate_exact_size(ui.available_size().max(vec2(10.0, 120.0)), Sense::hover());
    let c = r.center();
    ui.painter().text(c - vec2(0.0, 28.0), Align2::CENTER_CENTER, glyph, icons::font(34.0), p.text_tertiary);
    ui.painter().text(
        c + vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        title,
        crate::ui::views::flow::semibold(14.0),
        p.text_primary,
    );
    let g = ui.painter().layout(sub.to_string(), FontId::proportional(12.0), p.text_secondary, r.width() * 0.7);
    ui.painter().galley(pos2(c.x - g.size().x / 2.0, c.y + 22.0), g, p.text_secondary);
}
