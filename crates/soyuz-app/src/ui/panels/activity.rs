//! Activity panel (bottom panel tab): one card per job with icon + progress ring, title,
//! current item, progress bar, bytes / items, speed + ETA, a painter-drawn speed
//! sparkline, pause / resume / cancel / priority / reorder / start-now controls and an
//! expandable log + error list. The History tab (`activity_history.rs`) lists finished
//! operations with Undo / Redo / Retry failed. Also provides the compact status-bar
//! indicator (ring + job count).

use std::f32::consts::TAU;
use std::time::Duration;

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Mesh, Pos2, Rect, Response, ScrollArea, Sense, Shape, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use soyuz_core::config::Action;
use soyuz_core::{SizeStyle, format_eta, format_size, format_speed};
use soyuz_jobs::{JobSnapshot, JobState, LogLevel, Priority};

use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::ops::fileops::{FileOpCmd, HistoryAction, names};
use crate::services::jobs::{JobMeta, OpKind};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons as ticons, icons::ph, palette};

use crate::ui::views::flow::semibold;
use crate::ui::widgets::{icon_button, menu_check, menu_item, menu_separator, submenu, trunc_galley};

pub use crate::models::panels::{ActivityState, ActivityTab, Indicator, fraction, indicator, verify_badge};

/// Glyph and color for an operation.
pub fn op_style(op: OpKind, p: &Palette) -> (&'static str, Color32) {
    match op {
        OpKind::Copy | OpKind::Duplicate => (ph::COPY, p.accent),
        OpKind::Move => (ph::ARROW_BEND_DOWN_RIGHT, p.info),
        OpKind::Trash => (ph::TRASH, p.warning),
        OpKind::Delete | OpKind::Wipe | OpKind::EmptyTrash => (ph::TRASH_SIMPLE, p.danger),
        OpKind::Rename => (ph::PENCIL_SIMPLE, p.accent),
        OpKind::NewFolder => (ph::FOLDER_PLUS, p.glyph_folder),
        OpKind::NewFile => (ph::FILE_PLUS, p.accent),
        OpKind::Link => (ph::LINK, p.info),
        OpKind::SetMeta => (ph::SLIDERS_HORIZONTAL, p.accent),
        OpKind::Restore => (ph::ARROW_COUNTER_CLOCKWISE, p.success),
        OpKind::Undo => (ph::ARROW_U_UP_LEFT, p.text_secondary),
        OpKind::Redo => (ph::ARROW_U_UP_RIGHT, p.text_secondary),
        OpKind::Retry => (ph::ARROW_CLOCKWISE, p.accent),
        OpKind::Other => (ph::GEAR_SIX, p.text_secondary),
    }
}

/// Ring progress arc (or a spinning partial ring when unknown).
pub fn ring(ui: &Ui, c: Pos2, r: f32, width: f32, frac: Option<f32>, color: Color32, track: Color32) {
    let painter = ui.painter();
    painter.circle_stroke(c, r, Stroke::new(width, track));
    let (start, sweep) = match frac {
        Some(f) => (-TAU / 4.0, TAU * f.clamp(0.0, 1.0)),
        None => {
            let t = ui.input(|i| i.time) as f32;
            ui.ctx().request_repaint_after(Duration::from_millis(33));
            (t * 4.0, TAU * 0.28)
        }
    };
    if sweep <= 0.001 {
        return;
    }
    let n = ((sweep / TAU) * 64.0).ceil().max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = start + sweep * i as f32 / n as f32;
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    painter.add(Shape::line(pts, Stroke::new(width, color)));
}

/// Compact status-bar indicator; returns the response when shown.
pub fn status_indicator(ui: &mut Ui, ind: &Indicator) -> Option<Response> {
    if ind.pending == 0 {
        return None;
    }
    let p = palette(ui.ctx());
    let color = if ind.waiting {
        p.warning
    } else if ind.paused {
        p.text_tertiary
    } else {
        p.accent
    };
    let text =
        if ind.waiting { tr!("panels-needs-input").to_string() } else { tr!("panels-jobs-count", count = ind.pending) };
    let g = ui.painter().layout_no_wrap(text.clone(), FontId::proportional(11.5), p.text_secondary);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 28.0, 18.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r.expand2(vec2(2.0, 1.0)), CornerRadius::same(4), p.bg_surface2);
    }
    let frac = if ind.paused || ind.waiting { ind.progress.or(Some(0.0)) } else { ind.progress };
    ring(ui, pos2(r.left() + 9.0, r.center().y), 5.5, 2.0, frac, color, p.border_strong);
    ui.painter().galley(pos2(r.left() + 20.0, r.center().y - g.size().y / 2.0), g, p.text_secondary);
    let tip = tr!("panels-show-activity-tip");
    crate::ui::a11y::button(&resp, &format!("{text}, {tip}"));
    Some(resp.on_hover_text(tip))
}

/// Speed sparkline: gradient area + line, last two minutes.
pub fn sparkline(ui: &Ui, rect: Rect, samples: &[(f32, f64)], color: Color32) {
    let painter = ui.painter_at(rect.expand(1.0));
    painter.line_segment(
        [pos2(rect.left(), rect.bottom() - 0.5), pos2(rect.right(), rect.bottom() - 0.5)],
        Stroke::new(1.0, color.gamma_multiply(0.18)),
    );
    if samples.len() < 2 {
        return;
    }
    let max = samples.iter().map(|s| s.1).fold(0.0f64, f64::max).max(1.0);
    let t1 = samples.last().map_or(0.0, |s| s.0);
    let t0 = samples.first().map_or(0.0, |s| s.0).max(t1 - 60.0);
    let span = (t1 - t0).max(1.0);
    let pts: Vec<Pos2> = samples
        .iter()
        .filter(|s| s.0 >= t0)
        .map(|s| {
            let x = rect.left() + (s.0 - t0) / span * rect.width();
            let y = rect.bottom() - (s.1 / max) as f32 * (rect.height() - 2.0);
            pos2(x, y)
        })
        .collect();
    if pts.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    let top = color.gamma_multiply(0.30);
    let bottom = color.gamma_multiply(0.02);
    for (i, pt) in pts.iter().enumerate() {
        mesh.colored_vertex(*pt, top);
        mesh.colored_vertex(pos2(pt.x, rect.bottom()), bottom);
        if i > 0 {
            let b = (i as u32) * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b + 1, b);
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(pts.clone(), Stroke::new(1.5, color)));
    if let Some(last) = pts.last() {
        painter.circle_filled(*last, 2.5, color);
    }
}

/// Body of the Activity tab (below the bottom panel's tab header).
pub fn show(ui: &mut Ui, body: Rect, app: &mut SoyuzApp) {
    let p = palette(ui.ctx());
    let mut child = ui.new_child(UiBuilder::new().max_rect(body.shrink2(vec2(10.0, 4.0))));
    let ui = &mut child;
    // ---- toolbar ----
    let pending = app.core.fileops.jobs.pending().count();
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let mut x = bar.left();
    for (tab, label) in [
        (
            ActivityTab::Jobs,
            if pending > 0 {
                tr!("panels-tab-jobs-pending", count = pending)
            } else {
                tr!("panels-tab-jobs").to_string()
            },
        ),
        (ActivityTab::History, tr!("panels-tab-history").to_string()),
    ] {
        let g = ui.painter().layout_no_wrap(label, FontId::proportional(12.0), p.text_primary);
        let r = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(g.size().x + 20.0, 24.0));
        let resp = ui.interact(r, Id::new(("soyuz.activity.tab", tab as u8)), Sense::click());
        let active = app.core.fileops.activity.tab == tab;
        crate::ui::a11y::tab(&resp, g.text(), active);
        if active {
            ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.accent_subtle);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), p.bg_surface2);
        }
        let c = if active { p.accent } else { p.text_secondary };
        let g = ui.painter().layout_no_wrap(g.text().to_string(), FontId::proportional(12.0), c);
        ui.painter().galley(r.center() - g.size() / 2.0, g, c);
        if resp.clicked() {
            app.core.fileops.activity.tab = tab;
        }
        x = r.right() + 4.0;
    }
    if app.core.fileops.activity.tab == ActivityTab::Jobs {
        let mut rx = bar.right();
        let mut btn = |ui: &mut Ui, glyph: &str, tip: &str, enabled: bool| {
            let r = Rect::from_min_size(pos2(rx - 28.0, bar.top() + 1.0), vec2(28.0, 28.0));
            rx -= 30.0;
            icon_button(
                &mut ui.new_child(UiBuilder::new().max_rect(r).id_salt(("soyuz.act.bar", glyph))),
                glyph,
                tip,
                enabled,
            )
        };
        let any_done = app.core.fileops.jobs.snapshots().iter().any(|s| s.state.is_terminal());
        if btn(ui, ph::BROOM, tr!("panels-clear-finished"), any_done).clicked() {
            app.core.fileops.jobs.engine().clear_finished();
        }
        if btn(ui, ph::X_CIRCLE, tr!("panels-cancel-all"), pending > 0).clicked() {
            app.core.fileops.jobs.engine().cancel_all();
        }
        if btn(ui, ph::PLAY, tr!("panels-resume-all"), pending > 0).clicked() {
            app.core.fileops.jobs.engine().resume_all();
        }
        if btn(ui, ph::PAUSE, tr!("panels-pause-all"), pending > 0).clicked() {
            app.core.fileops.jobs.engine().pause_all();
        }
    }
    ui.add_space(4.0);
    match app.core.fileops.activity.tab {
        ActivityTab::Jobs => jobs_list(ui, app, &p),
        ActivityTab::History => super::activity_history::show(ui, app, &p),
    }
}

fn empty(ui: &mut Ui, p: &Palette, glyph: &str, title: &str, sub: &str) {
    let r = ui.available_rect_before_wrap();
    let c = r.center() - vec2(0.0, 12.0);
    ui.painter().text(c - vec2(0.0, 14.0), Align2::CENTER_CENTER, glyph, ticons::font_fill(26.0), p.text_tertiary);
    ui.painter().text(c + vec2(0.0, 12.0), Align2::CENTER_CENTER, title, semibold(13.0), p.text_secondary);
    ui.painter().text(c + vec2(0.0, 30.0), Align2::CENTER_CENTER, sub, FontId::proportional(11.5), p.text_tertiary);
    ui.allocate_rect(r, Sense::hover());
}

fn jobs_list(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette) {
    let mut snaps: Vec<JobSnapshot> = app.core.fileops.jobs.snapshots().to_vec();
    if snaps.is_empty() {
        empty(ui, p, ph::PULSE, tr!("panels-jobs-empty"), tr!("panels-jobs-empty-hint"));
        return;
    }
    // Active first (by queue position), finished last (newest first).
    snaps.sort_by_key(|s| {
        (
            s.state.is_terminal(),
            if s.state.is_terminal() {
                u64::MAX - s.id.0
            } else {
                s.queue_pos.map_or(0, |q| q as u64 + 1) * 1_000_000 + s.id.0
            },
        )
    });
    ScrollArea::vertical().id_salt("soyuz.activity.jobs").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        for s in &snaps {
            job_card(ui, app, s, p);
        }
    });
}

fn state_line(s: &JobSnapshot, meta: Option<&JobMeta>) -> (String, Option<Color32>) {
    let _ = meta;
    match &s.state {
        JobState::Queued => (
            match s.queue_pos {
                Some(q) => tr!("panels-state-queued-at", position = (q + 1).to_string()),
                None => tr!("panels-state-queued").into(),
            },
            None,
        ),
        JobState::Planning => (tr!("panels-state-planning").into(), None),
        JobState::Running => (
            s.progress
                .current
                .as_ref()
                .map(|c| soyuz_core::entry::display_name(c))
                .unwrap_or_else(|| tr!("panels-state-working").into()),
            None,
        ),
        JobState::Paused => (tr!("panels-state-paused").into(), None),
        JobState::WaitingForUser => (tr!("panels-state-waiting").into(), Some(Color32::YELLOW)),
        JobState::Finishing => (tr!("panels-state-finishing").into(), None),
        JobState::Done(_) | JobState::Failed(_) | JobState::Cancelled => (names::outcome(&s.state), None),
    }
}

#[allow(clippy::too_many_lines)]
fn job_card(ui: &mut Ui, app: &mut SoyuzApp, s: &JobSnapshot, p: &Palette) {
    let meta = app
        .core
        .fileops
        .jobs
        .meta(s.id)
        .cloned()
        .unwrap_or_else(|| JobMeta::new(OpKind::from_brief(s.kind), &[], None));
    let active = !s.state.is_terminal();
    let total_items = s.progress.totals_known.then_some(s.progress.items_total);
    let title = names::job_title(&meta, !matches!(s.state, JobState::Done(_)), total_items, &s.label);
    let expanded = app.core.fileops.activity.expanded.contains(&s.id);
    let summary = match &s.state {
        JobState::Done(sum) => Some(sum.clone()),
        _ => None,
    };
    let failed = summary.as_ref().is_some_and(|x| x.items_failed > 0);
    let w = ui.available_width();
    let base_h = 86.0;
    let extra = if expanded { 168.0 } else { 0.0 };
    let (card, _) = ui.allocate_exact_size(vec2(w, base_h + extra), Sense::hover());
    let painter = ui.painter_at(card);
    painter.rect_filled(card, CornerRadius::same(radius::CARD), p.bg_window);
    let waiting = s.pending_prompt.is_some();
    painter.rect_stroke(
        card,
        CornerRadius::same(radius::CARD),
        Stroke::new(1.0, if waiting { p.warning.gamma_multiply(0.6) } else { p.border_subtle }),
        StrokeKind::Inside,
    );

    // Status color.
    let (glyph, op_color) = op_style(meta.op, p);
    let status = match &s.state {
        JobState::Failed(_) => p.danger,
        JobState::Cancelled => p.text_tertiary,
        JobState::Done(_) if failed => p.warning,
        JobState::Done(_) => p.success,
        JobState::Paused => p.text_tertiary,
        JobState::WaitingForUser => p.warning,
        _ => p.accent,
    };
    let frac = fraction(s);

    // Icon with ring.
    let ic = pos2(card.left() + 30.0, card.top() + 30.0);
    painter.circle_filled(ic, 17.0, op_color.gamma_multiply(0.12));
    let done_glyph = match &s.state {
        JobState::Done(_) if !failed => Some(ph::CHECK),
        JobState::Failed(_) => Some(ph::WARNING),
        JobState::Cancelled => Some(ph::PROHIBIT),
        _ => None,
    };
    painter.text(
        ic,
        Align2::CENTER_CENTER,
        done_glyph.unwrap_or(glyph),
        ticons::font(16.0),
        done_glyph.map_or(op_color, |_| status),
    );
    if active {
        let ring_frac = if matches!(s.state, JobState::Paused | JobState::WaitingForUser | JobState::Queued) {
            frac.or(Some(0.0))
        } else {
            frac
        };
        ring(ui, ic, 20.0, 2.5, ring_frac, status, p.border_subtle);
    }
    if meta.ai_task.is_some() {
        // Started by the in-app assistant (its task can be undone as a whole).
        let b = pos2(ic.x + 14.0, ic.y + 14.0);
        painter.circle_filled(b, 8.5, p.bg_window);
        painter.circle_filled(b, 7.0, p.accent);
        painter.text(b, Align2::CENTER_CENTER, ph::SPARKLE, ticons::font_fill(8.5), Color32::WHITE);
    } else if meta.mcp_client.is_some() {
        // Started by an external MCP client (named next to the title).
        let b = pos2(ic.x + 14.0, ic.y + 14.0);
        painter.circle_filled(b, 8.5, p.bg_window);
        painter.circle_filled(b, 7.0, p.warning);
        painter.text(b, Align2::CENTER_CENTER, ph::PLUGS_CONNECTED, ticons::font(8.5), Color32::WHITE);
    } else if let Some(origin) = meta.origin {
        // Started by an automation rule, a script or the CLI.
        let glyph = crate::ui::icons::glyph(crate::automation::origin_icon(origin));
        let b = pos2(ic.x + 14.0, ic.y + 14.0);
        painter.circle_filled(b, 8.5, p.bg_window);
        painter.circle_filled(b, 7.0, p.info);
        painter.text(b, Align2::CENTER_CENTER, glyph, ticons::font_fill(8.5), Color32::WHITE);
        let hit = egui::Rect::from_center_size(b, vec2(18.0, 18.0));
        ui.interact(hit, ui.id().with(("origin", s.id)), egui::Sense::hover())
            .on_hover_text(crate::automation::origin_label(origin));
    }

    // Text block.
    let tx = card.left() + 60.0;
    let spark_w = if card.width() > 640.0 { 150.0 } else { 0.0 };
    let buttons_w = 128.0;
    let text_w = card.right() - tx - spark_w - buttons_w - 16.0;
    let badge = verify_badge(s.kind, s.verify.is_some(), &s.state);
    let badge_w = badge
        .map_or(0.0, |b| painter.layout_no_wrap(b.to_string(), FontId::proportional(10.5), p.accent).size().x + 20.0);
    // MCP jobs: a "MCP · <client>" tag after the title.
    let tag = meta.mcp_client.as_ref().map(|c| {
        let g = painter.layout_no_wrap(format!("MCP \u{00b7} {c}"), FontId::proportional(10.5), p.warning);
        let w = g.size().x + 14.0;
        (g, w)
    });
    let tag_w = tag.as_ref().map_or(0.0, |(_, w)| w + 8.0);
    let tg = trunc_galley(&painter, &title, semibold(13.0), p.text_primary, (text_w - tag_w).max(40.0));
    let title_w = tg.size().x;
    painter.galley(pos2(tx, card.top() + 12.0), tg, p.text_primary);
    if let Some((g, w)) = tag {
        let r = Rect::from_min_size(pos2(tx + title_w + 8.0, card.top() + 12.0), vec2(w, 17.0));
        painter.rect_filled(r, CornerRadius::same(8), p.warning.gamma_multiply(0.14));
        painter.galley(pos2(r.left() + 7.0, r.center().y - g.size().y / 2.0), g, p.warning);
    }
    let (line, _) = state_line(s, Some(&meta));
    let line_color = match &s.state {
        JobState::WaitingForUser => p.warning,
        JobState::Failed(_) => p.danger,
        JobState::Done(_) if failed => p.warning,
        _ => p.text_secondary,
    };
    let lg = trunc_galley(&painter, &line, FontId::proportional(11.5), line_color, text_w - badge_w);
    let lgw = lg.size().x;
    painter.galley(pos2(tx, card.top() + 31.0), lg, line_color);
    if let Some(b) = badge {
        // Verified copies.
        let c = if matches!(s.state, JobState::Done(_)) { p.success } else { p.accent };
        let at = pos2(tx + lgw + 8.0, card.top() + 39.0);
        let w = crate::ui::dialogs::conflict::tag(ui, at, b, c);
        ui.interact(
            Rect::from_min_size(pos2(at.x, at.y - 8.0), vec2(w, 16.0)),
            Id::new(("soyuz.activity.verified", s.id.0)),
            Sense::hover(),
        )
        .on_hover_text(tr!("panels-verified-tip"));
    }

    // Progress bar.
    let bar = Rect::from_min_size(pos2(tx, card.top() + 52.0), vec2(card.right() - tx - 14.0, 5.0));
    painter.rect_filled(bar, CornerRadius::same(3), p.bg_surface2);
    match frac {
        Some(f) => {
            let mut fill = bar;
            fill.set_width((bar.width() * f).max(if f > 0.0 { 5.0 } else { 0.0 }));
            if fill.width() > 0.0 {
                painter.rect_filled(fill, CornerRadius::same(3), status);
            }
        }
        None if active
            && s.state != JobState::Paused
            && s.state != JobState::Queued
            && crate::theme::reduced_motion(ui.ctx()) =>
        {
            // Reduced motion: a static, dimmed bar instead of the sliding segment.
            painter.rect_filled(bar, CornerRadius::same(3), status.gamma_multiply(0.35));
        }
        None if active && s.state != JobState::Paused && s.state != JobState::Queued => {
            // Indeterminate: a sliding segment.
            let t = ui.input(|i| i.time) as f32;
            let seg = bar.width() * 0.25;
            let x0 = bar.left() + ((t * 0.6).fract() * (bar.width() + seg)) - seg;
            let r = Rect::from_min_max(
                pos2(x0.max(bar.left()), bar.top()),
                pos2((x0 + seg).min(bar.right()), bar.bottom()),
            );
            if r.width() > 0.0 {
                painter.rect_filled(r, CornerRadius::same(3), status);
            }
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        None => {}
    }

    crate::ui::a11y::progress(
        ui,
        Id::new(("soyuz.activity.progress", s.id.0)),
        card,
        &format!("{title}: {line}"),
        frac.or(if matches!(s.state, JobState::Done(_)) { Some(1.0) } else { None }),
    );

    // Stats row.
    let pr = &s.progress;
    let mut left = Vec::new();
    if pr.bytes_total > 0 {
        left.push(tr!(
            "panels-progress-of",
            done = format_size(pr.bytes_done, SizeStyle::Binary),
            total = format_size(pr.bytes_total, SizeStyle::Binary)
        ));
    } else if let Some(sum) = &summary
        && sum.bytes > 0
    {
        left.push(format_size(sum.bytes, SizeStyle::Binary));
    }
    if pr.items_total > 0 {
        left.push(tr!("panels-items-of", done = pr.items_done, total = pr.items_total));
    }
    let sy = card.top() + 70.0;
    let num_font = crate::theme::fonts::numbers(ui.ctx(), 11.5);
    let right = if matches!(s.state, JobState::Running) {
        match s.eta {
            Some(eta) => {
                tr!("panels-speed-eta", speed = format_speed(s.speed_bps), eta = format_eta(eta.as_secs_f64()))
            }
            None => format_speed(s.speed_bps),
        }
    } else if let Some(sum) = &summary {
        let secs = sum.elapsed.as_secs_f64();
        if sum.bytes > 0 && secs > 0.5 {
            tr!("panels-speed-avg", speed = format_speed(sum.bytes as f64 / secs))
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    // Speed / ETA on the right; the byte / item counts on the left are cut with "…"
    // before they reach it (wider fonts or narrow panels used to overlap the two).
    let right_w = painter.text(pos2(bar.right(), sy), Align2::RIGHT_CENTER, right, num_font.clone(), p.text_secondary);
    let mut job = egui::text::LayoutJob::simple_singleline(left.join(" \u{b7} "), num_font, p.text_tertiary);
    job.wrap = egui::text::TextWrapping::truncate_at_width((right_w.left() - 10.0 - tx).max(0.0));
    let left_g = painter.layout_job(job);
    painter.galley(pos2(tx, sy - left_g.size().y / 2.0), left_g, p.text_tertiary);

    // Sparkline.
    if spark_w > 0.0 && (s.samples.len() > 1) {
        let sr =
            Rect::from_min_size(pos2(card.right() - buttons_w - spark_w - 8.0, card.top() + 10.0), vec2(spark_w, 34.0));
        sparkline(ui, sr, &s.samples, if active { p.accent } else { p.text_tertiary });
    }

    // Buttons (right to left).
    let engine_id = s.id;
    let mut bx = card.right() - 8.0;
    let by = card.top() + 8.0;
    let mut btn = |ui: &mut Ui, glyph: &str, tip: &str| {
        let r = Rect::from_min_size(pos2(bx - 30.0, by - 1.0), vec2(30.0, 30.0));
        bx -= 32.0;
        icon_button(
            &mut ui.new_child(UiBuilder::new().max_rect(r).id_salt(("soyuz.act.btn", engine_id.0, glyph))),
            glyph,
            tip,
            true,
        )
    };
    let exp = btn(
        ui,
        if expanded { ph::CARET_UP } else { ph::CARET_DOWN },
        if expanded { tr!("panels-hide-details") } else { tr!("panels-show-log") },
    );
    if exp.clicked() {
        if expanded {
            app.core.fileops.activity.expanded.remove(&engine_id);
        } else {
            app.core.fileops.activity.expanded.insert(engine_id);
        }
    }
    if active {
        let more = btn(ui, ph::DOTS_THREE, tr!("panels-more"));
        let queued = matches!(s.state, JobState::Queued) || (s.state == JobState::Paused && s.started.is_none());
        let prio = s.priority;
        let mut action: Option<u8> = None;
        crate::ui::native::popup::Menu::menu(&more).show(|ui| {
            ui.set_min_width(200.0);
            submenu(ui, Some(ph::FLAG), tr!("panels-priority"), |ui| {
                ui.set_min_width(150.0);
                for (pr, label, code) in [
                    (Priority::High, tr!("panels-priority-high"), 1u8),
                    (Priority::Normal, tr!("panels-priority-normal"), 2),
                    (Priority::Low, tr!("panels-priority-low"), 3),
                ] {
                    if menu_check(ui, prio == pr, label, None).clicked() {
                        action = Some(code);
                        ui.close();
                    }
                }
            });
            if menu_item(ui, Some(ph::ARROW_UP), tr!("panels-move-up"), None, queued).clicked() {
                action = Some(4);
            }
            if menu_item(ui, Some(ph::ARROW_DOWN), tr!("panels-move-down"), None, queued).clicked() {
                action = Some(5);
            }
            if menu_item(ui, Some(ph::LIGHTNING), tr!("panels-start-now"), None, queued).clicked() {
                action = Some(6);
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::X), tr!("common-cancel"), None, true).clicked() {
                action = Some(7);
            }
        });
        let eng = app.core.fileops.jobs.engine();
        match action {
            Some(1) => eng.set_priority(engine_id, Priority::High),
            Some(2) => eng.set_priority(engine_id, Priority::Normal),
            Some(3) => eng.set_priority(engine_id, Priority::Low),
            Some(4) => eng.move_in_queue(engine_id, -1),
            Some(5) => eng.move_in_queue(engine_id, 1),
            Some(6) => eng.start_now(engine_id),
            Some(7) => eng.cancel(engine_id),
            _ => {}
        }
        if btn(ui, ph::X, tr!("common-cancel")).clicked() {
            app.core.fileops.jobs.engine().cancel(engine_id);
        }
        if s.state == JobState::Paused {
            if btn(ui, ph::PLAY, tr!("panels-resume")).clicked() {
                app.core.fileops.jobs.engine().resume(engine_id);
            }
        } else if s.state != JobState::WaitingForUser && btn(ui, ph::PAUSE, tr!("panels-pause")).clicked() {
            app.core.fileops.jobs.engine().pause(engine_id);
        }
        if waiting && app.core.fileops.prompts.is_deferred(engine_id) {
            let label = tr!("panels-resolve");
            let lw =
                ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), Color32::BLACK).size().x;
            let w = (lw + 16.0).max(76.0);
            // On the stats row (free while waiting: no speed / ETA), so the title and the
            // state line keep their full width in narrow panels and longer translations.
            let r = Rect::from_min_size(pos2(card.right() - 14.0 - w, card.top() + 60.0), vec2(w, 20.0));
            let resp = ui.interact(r, Id::new(("soyuz.activity.resolve", engine_id.0)), Sense::click());
            crate::ui::a11y::button(&resp, tr!("panels-resolve-conflict"));
            if resp.has_focus() {
                crate::ui::widgets::focus_ring(ui, r);
            }
            ui.painter().rect_filled(
                r,
                CornerRadius::same(radius::ROW),
                if resp.hovered() { p.warning } else { p.warning.gamma_multiply(0.85) },
            );
            ui.painter().text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), Color32::BLACK);
            if resp.clicked() {
                app.core.cmds.push(Cmd::FileOp(FileOpCmd::Resolve(engine_id)));
            }
        }
    } else {
        if btn(ui, ph::X, tr!("panels-remove-from-list")).clicked() {
            app.core.fileops.jobs.engine().remove(engine_id);
        }
        if failed {
            let label = tr!("panels-retry-failed");
            let lw =
                ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), p.text_primary).size().x;
            let w = (lw + 16.0).max(90.0);
            // On the stats row (free while waiting: no speed / ETA), so the title and the
            // state line keep their full width in narrow panels and longer translations.
            let r = Rect::from_min_size(pos2(card.right() - 14.0 - w, card.top() + 60.0), vec2(w, 20.0));
            let resp = ui.interact(r, Id::new(("soyuz.activity.retry", engine_id.0)), Sense::click());
            crate::ui::a11y::button(&resp, tr!("panels-retry-failed-items"));
            if resp.has_focus() {
                crate::ui::widgets::focus_ring(ui, r);
            }
            ui.painter().rect_filled(
                r,
                CornerRadius::same(radius::ROW),
                if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 },
            );
            ui.painter().text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), p.text_primary);
            if resp.clicked() {
                let h = summary.as_ref().and_then(|sum| {
                    app.core.fileops.jobs.history().iter().rev().find(|h| &h.summary == sum).map(|h| h.id)
                });
                match h {
                    Some(h) => crate::ops::fileops::history_action(&mut app.core, h, HistoryAction::Retry),
                    None => app.core.cmds.push(Cmd::Action(Action::JobsShowHistory)),
                }
            }
        }
    }

    // Expanded details: errors + log.
    if expanded {
        let dr = Rect::from_min_max(
            pos2(card.left() + 12.0, card.top() + base_h),
            pos2(card.right() - 12.0, card.bottom() - 10.0),
        );
        ui.painter().rect_filled(dr, CornerRadius::same(6), p.bg_sunken);
        let mut lines: Vec<(Color32, String)> = Vec::new();
        if let Some(sum) = &summary {
            for (path, e) in sum.errors.iter().take(200) {
                lines.push((p.danger, format!("\u{2715} {}  \u{2014}  {e}", soyuz_fs::strip_verbatim(path).display())));
            }
            for n in &sum.notes {
                lines.push((p.text_tertiary, format!("\u{2139} {n}")));
            }
        }
        if let JobState::Failed(e) = &s.state {
            lines.push((p.danger, format!("\u{2715} {e}")));
        }
        for l in app.core.fileops.jobs.log(s.id).iter().rev().take(300) {
            let c = match l.level {
                LogLevel::Info => p.text_secondary,
                LogLevel::Warn => p.warning,
                LogLevel::Error => p.danger,
            };
            let path =
                l.path.as_ref().map(|p| format!("  {}", soyuz_fs::strip_verbatim(p).display())).unwrap_or_default();
            lines.push((c, format!("{:>6.1}s  {}{path}", l.at.as_secs_f32(), l.msg)));
        }
        if lines.is_empty() {
            lines.push((p.text_tertiary, tr!("panels-log-empty").into()));
        }
        let mut child = ui.new_child(UiBuilder::new().max_rect(dr.shrink(8.0)));
        ScrollArea::vertical().id_salt(("soyuz.activity.log", s.id.0)).auto_shrink([false, false]).show(
            &mut child,
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (c, t) in lines {
                    ui.label(egui::RichText::new(t).font(FontId::monospace(11.0)).color(c));
                }
            },
        );
    }
}
