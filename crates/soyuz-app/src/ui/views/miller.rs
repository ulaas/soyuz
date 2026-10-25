//! Miller columns: one column per ancestor of the current folder (from the aux listing
//! cache), the pane's own listing as the active column, and a preview column for the
//! folder under the cursor.

use std::path::{Path, PathBuf};

use egui::{Align2, CornerRadius, FontId, Id, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use soyuz_core::{EntryInfo, Location};

use super::ViewCtx;
use super::flow::{self, Style};
use super::icons;
use crate::cmd::Cmd;
use crate::services::AuxState;
use crate::services::thumbs;
use crate::state::pane::Pane;
use crate::theme::{icons as ticons, icons::ph, palette};
use crate::ui::widgets::{spinner, trunc_galley};

const COL_W: f32 = 250.0;
const ROW_H: f32 = 26.0;

pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let Location::Dir(cur) = pane.location.clone() else {
        flow::show(ui, rect, pane, v, Style::Column);
        return;
    };
    let p = palette(ui.ctx());
    let mut chain: Vec<PathBuf> = cur.ancestors().map(Path::to_path_buf).collect();
    chain.reverse();
    let preview =
        pane.cursor.as_deref().and_then(|c| pane.entry(c)).filter(|e| e.is_dir_like()).map(|e| e.path.clone());
    let n = chain.len() + usize::from(preview.is_some());
    let content_w = (n as f32 * COL_W).max(rect.width());
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::horizontal()
        .id_salt(("soyuz.miller", pane.id.0))
        .stick_to_right(true)
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let origin = ui.min_rect().min;
            ui.set_min_size(vec2(content_w, rect.height()));
            for (i, dir) in chain.iter().enumerate() {
                let col =
                    Rect::from_min_size(pos2(origin.x + i as f32 * COL_W, rect.top()), vec2(COL_W, rect.height()));
                let last = i + 1 == chain.len();
                if last {
                    flow::show(ui, col, pane, v, Style::Column);
                } else {
                    let highlight =
                        chain.get(i + 1).and_then(|c| c.file_name()).map(|n| n.to_string_lossy().into_owned());
                    aux_column(ui, col, dir, highlight.as_deref(), pane.id, v, false);
                }
                ui.painter().line_segment(
                    [pos2(col.right() - 0.5, col.top() + 6.0), pos2(col.right() - 0.5, col.bottom() - 6.0)],
                    Stroke::new(1.0, p.border_subtle),
                );
            }
            if let Some(dir) = preview {
                let col = Rect::from_min_size(
                    pos2(origin.x + chain.len() as f32 * COL_W, rect.top()),
                    vec2(COL_W, rect.height()),
                );
                aux_column(ui, col, &dir, None, pane.id, v, true);
            }
        });
}

/// A read-only column listing `dir` from the aux cache. Clicking a folder navigates the
/// pane into it; clicking a file reveals it in its folder.
fn aux_column(
    ui: &mut Ui,
    rect: Rect,
    dir: &Path,
    highlight: Option<&str>,
    pane: crate::state::pane::PaneId,
    v: &mut ViewCtx<'_>,
    dim: bool,
) {
    let p = palette(ui.ctx());
    let entries = v.services.aux_entries(dir);
    let Some(entries) = entries else {
        if matches!(v.services.aux_state(dir), Some(AuxState::Failed(_))) {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                tr!("views-miller-unreadable"),
                FontId::proportional(12.0),
                p.text_tertiary,
            );
        } else {
            spinner(ui, pos2(rect.center().x, rect.top() + 40.0), 8.0, p.text_tertiary);
        }
        return;
    };
    let shown: Vec<&EntryInfo> = entries.iter().filter(|e| v.show_hidden || !e.is_hidden()).collect();
    if shown.is_empty() {
        ui.painter().text(
            pos2(rect.center().x, rect.top() + 40.0),
            Align2::CENTER_CENTER,
            tr!("views-miller-empty"),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
        return;
    }
    // Same rhythm as the active column (flow `Column` style): 4 px top pad, no row gap.
    let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(rect.min + vec2(0.0, 4.0), rect.max)));
    child.spacing_mut().item_spacing.y = 0.0;
    let id = Id::unique(("soyuz.miller.col", pane.0, dir));
    let target_row = highlight.and_then(|h| shown.iter().position(|e| e.name == h));
    let mut area = ScrollArea::vertical().id_salt(id).auto_shrink([false, false]);
    // Bring the highlighted child into view the first time this column appears.
    let first_seen = ui.ctx().data_mut(|d| {
        let seen = d.get_temp::<bool>(id.with("seen")).unwrap_or(false);
        d.insert_temp(id.with("seen"), true);
        !seen
    });
    if first_seen && let Some(r) = target_row {
        area = area.vertical_scroll_offset((r as f32 * ROW_H - rect.height() / 2.0).max(0.0));
    }
    area.show_rows(&mut child, ROW_H, shown.len(), |ui, range| {
        for i in range {
            let e = shown[i];
            let (r, resp) = ui.allocate_exact_size(vec2(rect.width() - 8.0, ROW_H), Sense::click());
            let r = r.translate(vec2(4.0, 0.0));
            let on_path = highlight == Some(e.name.as_str());
            if on_path {
                ui.painter().rect_filled(r, CornerRadius::same(6), flow::selection_fill(&p, false));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), p.bg_surface2);
            }
            let a = if dim { 0.75 } else { 1.0 } * if e.is_hidden() { 0.6 } else { 1.0 };
            let program_icon = if thumbs::has_program_icon(&e.path) {
                let px = thumbs::icon_px(17.0, ui.ctx().pixels_per_point());
                let key = thumbs::ThumbKey { path: e.path.clone(), mtime: e.modified, size: e.size, px };
                v.services.thumbs.get_program_icon(&key)
            } else {
                None
            };
            if let Some(t) = &program_icon {
                flow::paint_program_icon(ui.painter(), pos2(r.left() + 14.0, r.center().y), 17.0, t, a);
            } else if !super::icon_pack::paint(ui.painter(), pos2(r.left() + 14.0, r.center().y), 17.0, e, &p, a) {
                let gf = if e.is_dir_like() { ticons::font_fill(15.0) } else { ticons::font(15.0) };
                ui.painter().text(
                    pos2(r.left() + 14.0, r.center().y),
                    Align2::CENTER_CENTER,
                    icons::glyph(e),
                    gf,
                    icons::color(e, &p).gamma_multiply(a),
                );
            }
            let color = p.text_primary.gamma_multiply(a);
            let hidden = soyuz_core::naming::has_hidden_controls(&e.name);
            let g = trunc_galley(
                ui.painter(),
                &super::display_name(e),
                FontId::proportional(13.0),
                color,
                r.width() - 56.0 - if hidden { 14.0 } else { 0.0 },
            );
            let gw = g.size().x;
            ui.painter().galley(pos2(r.left() + 30.0, r.center().y - g.size().y / 2.0), g, color);
            if hidden {
                crate::ui::widgets::paint_hidden_badge(
                    ui.painter(),
                    &p,
                    pos2(r.left() + 30.0 + gw, r.center().y),
                    &e.name,
                );
            }
            let resp = if hidden { resp.on_hover_text(crate::ui::widgets::hidden_controls_tip()) } else { resp };
            if e.is_dir_like() {
                ui.painter().text(
                    pos2(r.right() - 10.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    ph::CARET_RIGHT,
                    ticons::font(11.0),
                    p.text_tertiary,
                );
            }
            if resp.clicked() {
                let cmd = if e.is_dir_like() {
                    Cmd::Navigate { pane, location: Location::Dir(e.path.clone()), select: None }
                } else {
                    Cmd::Navigate { pane, location: Location::Dir(dir.to_path_buf()), select: Some(e.name.clone()) }
                };
                v.cmds.push(Cmd::FocusPane(pane));
                v.cmds.push(cmd);
            }
        }
    });
}
