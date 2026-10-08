//! History tab of the activity panel: finished operations (newest first) with Undo /
//! Redo / Retry failed.

use std::path::PathBuf;

use egui::{Align2, Color32, CornerRadius, FontId, Id, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::DateFormat;
use soyuz_jobs::{CreateKind, DeleteMode, HistoryEntry, JobKind};

use super::activity::op_style;
use crate::app::SoyuzApp;
use crate::ops::fileops::{HistoryAction, history_action, names};
use crate::services::jobs::{JobMeta, OpKind};
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons as ticons, icons::ph};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::trunc_galley;

/// Operation kind and past-tense title of a history entry.
pub fn describe(job: &JobKind) -> (OpKind, String) {
    let (op, meta) = match job {
        JobKind::Copy { sources, dest_dir, .. } => {
            let same = sources.iter().all(|s| s.parent() == Some(dest_dir.as_path()));
            let op = if same { OpKind::Duplicate } else { OpKind::Copy };
            (op, JobMeta::new(op, sources, Some(dest_dir.clone())))
        }
        JobKind::Move { sources, dest_dir, .. } => {
            (OpKind::Move, JobMeta::new(OpKind::Move, sources, Some(dest_dir.clone())))
        }
        JobKind::Delete { paths, mode } => {
            let op = match mode {
                DeleteMode::Trash => OpKind::Trash,
                DeleteMode::Permanent => OpKind::Delete,
                DeleteMode::Wipe { .. } => OpKind::Wipe,
            };
            (op, JobMeta::new(op, paths, None))
        }
        JobKind::Rename { pairs } => {
            let from: Vec<PathBuf> = pairs.iter().map(|p| p.0.clone()).collect();
            let dest = (pairs.len() == 1).then(|| PathBuf::from(soyuz_core::entry::display_name(&pairs[0].1)));
            (OpKind::Rename, JobMeta::new(OpKind::Rename, &from, dest))
        }
        JobKind::Create { path, kind } => {
            let op = match kind {
                CreateKind::Dir => OpKind::NewFolder,
                CreateKind::File { .. } => OpKind::NewFile,
                _ => OpKind::Link,
            };
            (op, JobMeta::new(op, std::slice::from_ref(path), None))
        }
        JobKind::RestoreTrash { items, .. } => {
            let paths: Vec<PathBuf> = items.iter().map(|i| i.original.clone()).collect();
            (OpKind::Restore, JobMeta::new(OpKind::Restore, &paths, None))
        }
        JobKind::EmptyTrash => (OpKind::EmptyTrash, JobMeta::new(OpKind::EmptyTrash, &[], None)),
        other => (OpKind::Other, JobMeta::new(OpKind::Other, &other.touched_roots(), None)),
    };
    (op, names::job_title(&meta, false, None, ""))
}

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

pub fn show(ui: &mut Ui, app: &mut SoyuzApp, p: &Palette) {
    let entries: Vec<HistoryEntry> = app.fileops.jobs.history().iter().rev().cloned().collect();
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
            "No history yet",
            semibold(13.0),
            p.text_secondary,
        );
        ui.painter().text(
            c + vec2(0.0, 30.0),
            Align2::CENTER_CENTER,
            "Finished operations appear here and can be undone.",
            FontId::proportional(11.5),
            p.text_tertiary,
        );
        ui.allocate_rect(r, Sense::hover());
        return;
    }
    let mut todo: Option<(soyuz_jobs::HistoryId, HistoryAction)> = None;
    let rel = DateFormat::Named(soyuz_core::DateFormatName::Relative);
    ScrollArea::vertical().id_salt("soyuz.activity.history").auto_shrink([false, false]).show_rows(
        ui,
        50.0,
        entries.len(),
        |ui, range| {
            for h in &entries[range] {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
                let row = r.shrink2(vec2(0.0, 2.0));
                if resp.hovered() {
                    ui.painter().rect_filled(row, CornerRadius::same(radius::CARD), p.bg_surface2);
                }
                let (op, title) = describe(&h.job);
                let (glyph, color) = op_style(op, p);
                let dim = if h.undone { 0.5 } else { 1.0 };
                let ic = pos2(row.left() + 22.0, row.center().y);
                ui.painter().circle_filled(ic, 14.0, color.gamma_multiply(0.12 * dim));
                ui.painter().text(ic, Align2::CENTER_CENTER, glyph, ticons::font(14.0), color.gamma_multiply(dim));
                let tx = row.left() + 46.0;
                let tw = row.width() - 46.0 - 240.0;
                let tg = trunc_galley(ui.painter(), &title, semibold(12.5), p.text_primary.gamma_multiply(dim), tw);
                let tgw = tg.size().x;
                ui.painter().galley(pos2(tx, row.top() + 8.0), tg, p.text_primary);
                if h.undone {
                    crate::ui::dialogs::conflict::tag(
                        ui,
                        pos2(tx + tgw + 8.0, row.top() + 16.0),
                        "Undone",
                        p.text_tertiary,
                    );
                }
                let mut sub = format!(
                    "{} \u{b7} {}",
                    soyuz_core::time::format_time_at(h.when, &rel, std::time::SystemTime::now()),
                    names::summary_line(&h.summary)
                );
                if !h.undoable && !h.undone {
                    sub.push_str(" \u{b7} can't be undone");
                }
                let sc = if h.summary.items_failed > 0 { p.warning } else { p.text_tertiary };
                let sg = trunc_galley(ui.painter(), &sub, FontId::proportional(11.5), sc, tw);
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
                    if small_button(ui, place(64.0), Id::new(("soyuz.hist.redo", h.id.0)), "Redo", true, p) {
                        todo = Some((h.id, HistoryAction::Redo));
                    }
                } else {
                    let rr = place(64.0);
                    if small_button(ui, rr, Id::new(("soyuz.hist.undo", h.id.0)), "Undo", h.undoable, p) {
                        todo = Some((h.id, HistoryAction::Undo));
                    }
                    if !h.undoable {
                        ui.interact(rr, Id::new(("soyuz.hist.undo.tip", h.id.0)), Sense::hover())
                            .on_hover_text("This operation can't be undone (e.g. a permanent delete).");
                    }
                }
                if h.summary.items_failed > 0
                    && small_button(ui, place(92.0), Id::new(("soyuz.hist.retry", h.id.0)), "Retry failed", true, p)
                {
                    todo = Some((h.id, HistoryAction::Retry));
                }
            }
        },
    );
    if let Some((h, a)) = todo {
        history_action(app, h, a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_history_entries() {
        let (op, t) = describe(&JobKind::Move { sources: vec!["/a/x".into()], dest_dir: "/b".into(), rename_to: None });
        assert_eq!(op, OpKind::Move);
        assert_eq!(t, "Moved \u{201c}x\u{201d} to /b");
        let (op, _) = describe(&JobKind::Copy { sources: vec!["/a/x".into()], dest_dir: "/a".into(), rename_to: None });
        assert_eq!(op, OpKind::Duplicate);
        let (op, t) = describe(&JobKind::Rename { pairs: vec![("/a/x".into(), "/a/y".into())] });
        assert_eq!(op, OpKind::Rename);
        assert_eq!(t, "Renamed \u{201c}x\u{201d} to \u{201c}y\u{201d}");
        let (op, _) = describe(&JobKind::Delete { paths: vec!["/a".into()], mode: DeleteMode::Permanent });
        assert_eq!(op, OpKind::Delete);
    }
}
