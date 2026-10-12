//! History tab of the activity panel: finished operations (newest first) with Undo /
//! Redo / Retry failed.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_jobs::HistoryEntry;

use super::activity::op_style;
use crate::app::SoyuzApp;
use crate::models::jobs_rows::{self, HistoryRow, RowButton};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons as ticons, icons::ph};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::trunc_galley;

pub use crate::models::panels::{describe, is_verified};

fn small_button(ui: &mut Ui, r: Rect, id: Id, label: &str, enabled: bool, p: &Palette) -> bool {
    let resp = ui.interact(r, id, if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        Color32::TRANSPARENT
    } else if resp.hovered() {
        p.bg_surface3
    } else {
        p.bg_surface2
    };
    ui.painter().rect_filled(r, CornerRadius::same(radius::ROW), fill);
    if enabled {
        ui.painter().rect_stroke(
            r,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
    }
    let c = if enabled { p.text_primary } else { p.text_tertiary.gamma_multiply(0.7) };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), c);
    enabled && resp.clicked()
}

/// Width of a [`small_button`] for `label`: at least `min`, wider for longer translations.
fn button_width(ui: &Ui, label: &str, min: f32) -> f32 {
    let w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), Color32::WHITE).size().x;
    (w + 20.0).max(min)
}

pub fn show(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette) {
    let entries: Vec<HistoryEntry> = app.core.fileops.jobs.history().iter().rev().cloned().collect();
    let now = std::time::SystemTime::now();
    if entries.is_empty() {
        let r = ui.available_rect_before_wrap();
        let c = r.center() - vec2(0.0, 12.0);
        ui.painter().text(
            c - vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            ph::CLOCK_COUNTER_CLOCKWISE,
            ticons::font_fill(26.0),
            p.text_tertiary,
        );
        ui.painter().text(
            c + vec2(0.0, 12.0),
            Align2::CENTER_CENTER,
            tr!("panels-history-empty"),
            semibold(13.0),
            p.text_secondary,
        );
        ui.painter().text(
            c + vec2(0.0, 30.0),
            Align2::CENTER_CENTER,
            tr!("panels-history-empty-hint"),
            FontId::proportional(11.5),
            p.text_tertiary,
        );
        ui.allocate_rect(r, Sense::hover());
        return;
    }
    let mut todo: Option<(soyuz_jobs::HistoryId, RowButton)> = None;
    ScrollArea::vertical().id_salt("soyuz.activity.history").auto_shrink([false, false]).show_rows(
        ui,
        50.0,
        entries.len(),
        |ui, range| {
            for e in &entries[range] {
                let h = &HistoryRow::new(e, now);
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
                let row = r.shrink2(vec2(0.0, 2.0));
                if resp.hovered() {
                    ui.painter().rect_filled(row, CornerRadius::same(radius::CARD), p.bg_surface2);
                }
                let (op, title) = (h.op, &h.title);
                let (glyph, color) = op_style(op, p);
                let dim = if h.undone { 0.5 } else { 1.0 };
                let ic = pos2(row.left() + 22.0, row.center().y);
                ui.painter().circle_filled(ic, 14.0, color.gamma_multiply(0.12 * dim));
                ui.painter().text(ic, Align2::CENTER_CENTER, glyph, ticons::font(14.0), color.gamma_multiply(dim));
                let tx = row.left() + 46.0;
                let tw = row.width() - 46.0 - 240.0;
                let verified = h.verified;
                let tag_w = if verified {
                    ui.painter()
                        .layout_no_wrap(tr!("panels-verified").to_string(), FontId::proportional(10.5), p.success)
                        .size()
                        .x
                        + 20.0
                } else {
                    0.0
                };
                let tg = trunc_galley(
                    ui.painter(),
                    title,
                    semibold(12.5),
                    p.text_primary.gamma_multiply(dim),
                    (tw - tag_w).max(40.0),
                );
                let tgw = tg.size().x;
                ui.painter().galley(pos2(tx, row.top() + 8.0), tg, p.text_primary);
                if h.undone {
                    crate::ui::dialogs::conflict::tag(
                        ui,
                        pos2(tx + tgw + 8.0, row.top() + 16.0),
                        tr!("panels-history-undone"),
                        p.text_tertiary,
                    );
                } else if verified {
                    let at = pos2(tx + tgw + 8.0, row.top() + 16.0);
                    let w = crate::ui::dialogs::conflict::tag(ui, at, tr!("panels-verified"), p.success);
                    ui.interact(
                        Rect::from_min_size(pos2(at.x, at.y - 8.0), vec2(w, 16.0)),
                        Id::new(("soyuz.hist.verified", h.id.0)),
                        Sense::hover(),
                    )
                    .on_hover_text(tr!("panels-history-verified-tip"));
                }
                let sc = if h.failed { p.warning } else { p.text_tertiary };
                let sg = trunc_galley(ui.painter(), &h.sub, FontId::proportional(11.5), sc, tw);
                ui.painter().galley(pos2(tx, row.top() + 27.0), sg, sc);
                // Buttons.
                let mut bx = row.right() - 8.0;
                let by = row.center().y - 12.0;
                let mut place = |w: f32| {
                    let rr = Rect::from_min_size(pos2(bx - w, by), vec2(w, 24.0));
                    bx -= w + 6.0;
                    rr
                };
                if h.undone {
                    let label = tr!("common-redo");
                    let rr = place(button_width(ui, label, 64.0));
                    if small_button(ui, rr, Id::new(("soyuz.hist.redo", h.id.0)), label, true, p) {
                        todo = Some((h.id, RowButton::Redo));
                    }
                } else {
                    let label = tr!("common-undo");
                    let rr = place(button_width(ui, label, 64.0));
                    if small_button(ui, rr, Id::new(("soyuz.hist.undo", h.id.0)), label, h.enabled(RowButton::Undo), p)
                    {
                        todo = Some((h.id, RowButton::Undo));
                    }
                    if !h.undoable {
                        ui.interact(rr, Id::new(("soyuz.hist.undo.tip", h.id.0)), Sense::hover())
                            .on_hover_text(tr!("panels-history-not-undoable-tip"));
                    }
                }
                if h.enabled(RowButton::RetryFailed) {
                    let label = tr!("panels-retry-failed");
                    let rr = place(button_width(ui, label, 92.0));
                    if small_button(ui, rr, Id::new(("soyuz.hist.retry", h.id.0)), label, true, p) {
                        todo = Some((h.id, RowButton::RetryFailed));
                    }
                }
            }
        },
    );
    if let Some((h, b)) = todo {
        jobs_rows::apply_history(&mut app.core, h, b);
    }
}
