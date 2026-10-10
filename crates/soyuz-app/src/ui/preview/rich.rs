//! Painters for the rich previews: Office / ODF documents, spreadsheets,
//! slides, EPUB, font specimens, 3D models, audio (waveform + player), video posters
//! and PDF pages. Shared by the preview pane and Quick Look (`interactive`).
//!
//! Per-view UI state (PDF page, model orientation, text/page toggle) lives in egui's
//! temporary memory keyed by the file path and the consumer, so it survives frames
//! without touching the shared `PreviewState`.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Rect, ScrollArea, Sense, Shape, Stroke, StrokeKind, Ui,
    UiBuilder, pos2, vec2,
};
use soyuz_core::EntryInfo;

use super::render::PreviewAction;
use crate::services::preview_audio;
use crate::services::preview_epub::EpubModel;
use crate::services::preview_media::format_duration;
use crate::services::preview_model::{self, View};
use crate::services::preview_office::{DocModel, Office, SheetModel, SlidesModel};
use crate::services::preview_pdf;
use crate::services::preview_rich::{PdfView, Rich, Tex, pdf_page_async};
use crate::theme::icons::{self, ph};
use crate::theme::{Palette, palette};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{pill_button, spinner, trunc_galley};

/// Paragraphs laid out for a document preview (the rest is summarized).
const DOC_PARAS: usize = 1_500;

pub fn show(ui: &mut Ui, rect: Rect, rich: &Rich, entry: &EntryInfo, interactive: bool) -> Option<PreviewAction> {
    let p = palette(ui.ctx());
    let id = Id::new(("soyuz.rich", interactive, entry.path.clone()));
    match rich {
        Rich::Office(o) => match o.as_ref() {
            Office::Doc(d) => doc(ui, rect, d, id, &p, interactive),
            Office::Sheet(s) => sheet(ui, rect, s, id, &p),
            Office::Slides(s) => slides(ui, rect, s, id, &p, interactive),
        },
        Rich::Epub { model, cover } => epub(ui, rect, model, cover.as_ref(), id, &p),
        Rich::Font { strips, .. } => font(ui, rect, strips, id, &p),
        Rich::Model { model, turntable } => model_view(ui, rect, model, *turntable, id, &p, interactive),
        Rich::Audio { info, src, ext, playback, cover } => {
            audio(ui, rect, entry, info, src, ext, *playback, cover.as_ref(), &p, interactive)
        }
        Rich::Video { poster, note } => video(ui, rect, entry, poster.as_ref(), note.as_deref(), &p),
        Rich::Pdf(v) => return pdf(ui, rect, v, id, &p, interactive),
    }
    None
}

fn child(ui: &mut Ui, r: Rect) -> Ui {
    let mut c = ui.new_child(UiBuilder::new().max_rect(r));
    c.set_clip_rect(r.intersect(ui.clip_rect()));
    c
}

fn caption(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(egui::RichText::new(text).size(10.5).color(p.text_tertiary).strong());
}

// ---- documents ----------------------------------------------------------------------

fn heading_size(level: u8, big: bool) -> f32 {
    let base = match level {
        1 => 20.0,
        2 => 17.0,
        3 => 15.0,
        _ => 13.5,
    };
    if big { base + 2.0 } else { base }
}

fn doc(ui: &mut Ui, rect: Rect, d: &DocModel, id: Id, p: &Palette, big: bool) {
    ui.painter().rect_filled(rect, 0.0, p.bg_surface1);
    let pad = if big { vec2(48.0, 18.0) } else { vec2(16.0, 10.0) };
    let mut c = child(ui, rect.shrink2(pad));
    ScrollArea::vertical().id_salt(id.with("doc")).auto_shrink([false, false]).show(&mut c, |ui| {
        let w = ui.available_width().min(if big { 820.0 } else { f32::INFINITY });
        ui.set_max_width(w);
        if d.title.is_some() || d.author.is_some() {
            let line =
                [d.title.as_deref(), d.author.as_deref()].into_iter().flatten().collect::<Vec<_>>().join("  ·  ");
            ui.label(egui::RichText::new(line).size(11.0).color(p.text_tertiary));
            ui.add_space(6.0);
        }
        let body = if big { 13.5 } else { 12.5 };
        for para in d.paras.iter().take(DOC_PARAS) {
            if para.runs.iter().all(|r| r.text.trim().is_empty()) {
                ui.add_space(body * 0.6);
                continue;
            }
            let mut job = LayoutJob::default();
            job.wrap.max_width = w;
            if para.list {
                let mut f = TextFormat::simple(FontId::proportional(body), p.text_secondary);
                f.line_height = Some(body * 1.5);
                job.append("•  ", 6.0, f);
            }
            for r in &para.runs {
                let (font, color) = if para.level > 0 {
                    (semibold(heading_size(para.level, big)), p.text_primary)
                } else if r.bold {
                    (semibold(body), p.text_primary)
                } else {
                    (FontId::proportional(body), p.text_primary)
                };
                let mut f = TextFormat::simple(font, color);
                f.italics = r.italic;
                f.line_height = Some(if para.level > 0 { heading_size(para.level, big) * 1.3 } else { body * 1.5 });
                job.append(&r.text, 0.0, f);
            }
            if para.level > 0 {
                ui.add_space(if para.level <= 2 { 8.0 } else { 4.0 });
            }
            ui.add(egui::Label::new(job).wrap().selectable(true));
            ui.add_space(if para.level > 0 { 2.0 } else { 5.0 });
        }
        if d.paras.len() > DOC_PARAS || d.truncated {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(tr!("views-rich-first-paragraphs", count = d.paras.len().min(DOC_PARAS)))
                    .size(11.0)
                    .color(p.text_tertiary),
            );
        }
        if d.paras.is_empty() {
            ui.label(egui::RichText::new(tr!("views-rich-doc-no-text")).color(p.text_tertiary));
        }
    });
}

fn col_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.iter().rev().collect()
}

fn sheet(ui: &mut Ui, rect: Rect, s: &SheetModel, id: Id, p: &Palette) {
    ui.painter().rect_filled(rect, 0.0, p.bg_sunken);
    // Sheet tabs: the first is shown.
    let tabs_h = 30.0;
    let tabs = Rect::from_min_size(rect.min, vec2(rect.width(), tabs_h));
    let mut x = tabs.left() + 10.0;
    for (i, name) in s.sheets.iter().enumerate().take(12) {
        let g = trunc_galley(ui.painter(), name, FontId::proportional(11.5), p.text_primary, 140.0);
        let r = Rect::from_min_size(pos2(x, tabs.top() + 5.0), vec2(g.size().x + 18.0, 20.0));
        if r.right() > tabs.right() - 8.0 {
            break;
        }
        let (bg, fg) = if i == 0 { (p.bg_surface1, p.text_primary) } else { (Color32::TRANSPARENT, p.text_tertiary) };
        ui.painter().rect_filled(r, CornerRadius::same(5), bg);
        if i == 0 {
            ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
        }
        ui.painter().galley(pos2(r.left() + 9.0, r.center().y - g.size().y / 2.0), g, fg);
        let resp = ui.interact(r, id.with(("tab", i)), Sense::hover());
        if i > 0 {
            resp.on_hover_text(tr!("views-rich-first-sheet-only"));
        }
        x = r.right() + 4.0;
    }
    let body = Rect::from_min_max(pos2(rect.left(), tabs.bottom()), rect.max);
    if s.rows.is_empty() {
        ui.painter().text(
            body.center(),
            Align2::CENTER_CENTER,
            tr!("views-rich-sheet-empty"),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
        return;
    }
    let font = FontId::proportional(12.0);
    // Column widths from content (bounded).
    let ncols = s.ncols.max(1);
    let mut widths = vec![44.0f32; ncols];
    ui.ctx().fonts_mut(|f| {
        for row in s.rows.iter().take(200) {
            for (c, v) in row.iter().enumerate() {
                if v.is_empty() {
                    continue;
                }
                let w =
                    f.layout_no_wrap(v.lines().next().unwrap_or("").to_string(), font.clone(), p.text_primary).size().x;
                widths[c] = widths[c].max((w + 16.0).min(240.0));
            }
        }
    });
    let gutter = 40.0;
    let row_h = 22.0;
    let total_w = gutter + widths.iter().sum::<f32>();
    let mut c = child(ui, body);
    ScrollArea::both().id_salt(id.with("sheet")).auto_shrink([false, false]).show_rows(
        &mut c,
        row_h,
        s.rows.len() + 1 + usize::from(s.truncated),
        |ui, range| {
            let w = total_w.max(ui.available_width());
            for i in range {
                let (r, _) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
                let painter = ui.painter();
                if i == 0 {
                    painter.rect_filled(r, 0.0, p.bg_surface2);
                    let mut x = r.left() + gutter;
                    for (c, cw) in widths.iter().enumerate() {
                        painter.text(
                            pos2(x + cw / 2.0, r.center().y),
                            Align2::CENTER_CENTER,
                            col_name(c),
                            FontId::proportional(10.5),
                            p.text_tertiary,
                        );
                        x += cw;
                        painter
                            .line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, p.border_subtle));
                    }
                    continue;
                }
                let ri = i - 1;
                if ri >= s.rows.len() {
                    painter.text(
                        pos2(r.left() + gutter + 8.0, r.center().y),
                        Align2::LEFT_CENTER,
                        tr!("views-rich-more-rows"),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                    continue;
                }
                if ri % 2 == 1 {
                    painter.rect_filled(r, 0.0, p.bg_surface1.gamma_multiply(0.5));
                }
                painter.text(
                    pos2(r.left() + gutter - 8.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    (ri + 1).to_string(),
                    FontId::monospace(10.5),
                    p.text_tertiary,
                );
                let mut x = r.left() + gutter;
                let row = &s.rows[ri];
                for (c, cw) in widths.iter().enumerate() {
                    if let Some(v) = row.get(c).filter(|v| !v.is_empty()) {
                        let first = v.lines().next().unwrap_or("");
                        let numeric = first.parse::<f64>().is_ok();
                        let g = trunc_galley(painter, first, font.clone(), p.text_primary, cw - 12.0);
                        let gx = if numeric { x + cw - 6.0 - g.size().x } else { x + 6.0 };
                        painter.galley(pos2(gx, r.center().y - g.size().y / 2.0), g, p.text_primary);
                    }
                    x += cw;
                    painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, p.border_subtle));
                }
                painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, p.border_subtle));
            }
        },
    );
    ui.painter().line_segment(
        [pos2(body.left() + gutter, body.top()), pos2(body.left() + gutter, body.bottom())],
        Stroke::new(1.0, p.border_strong),
    );
}

fn slides(ui: &mut Ui, rect: Rect, s: &SlidesModel, id: Id, p: &Palette, big: bool) {
    ui.painter().rect_filled(rect, 0.0, p.bg_sunken);
    let mut c = child(ui, rect.shrink2(vec2(if big { 40.0 } else { 12.0 }, 10.0)));
    ScrollArea::vertical().id_salt(id.with("slides")).auto_shrink([false, false]).show(&mut c, |ui| {
        if s.slides.is_empty() {
            ui.label(egui::RichText::new(tr!("views-rich-no-slides")).color(p.text_tertiary));
        }
        let w = ui.available_width().min(if big { 900.0 } else { f32::INFINITY });
        for (i, slide) in s.slides.iter().enumerate() {
            egui::Frame::new()
                .fill(p.bg_surface1)
                .stroke(Stroke::new(1.0, p.border_subtle))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(w - 26.0);
                    ui.horizontal_top(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(26.0, 20.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(5), p.accent_subtle);
                        ui.painter().text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            (i + 1).to_string(),
                            semibold(11.0),
                            p.accent,
                        );
                        ui.vertical(|ui| {
                            let title = if slide.title.is_empty() {
                                tr!("views-rich-untitled-slide")
                            } else {
                                slide.title.as_str()
                            };
                            ui.add(
                                egui::Label::new(egui::RichText::new(title).font(semibold(13.5)).color(p.text_primary))
                                    .wrap(),
                            );
                            for b in slide.body.iter().take(12) {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!("•  {b}")).size(12.0).color(p.text_secondary),
                                    )
                                    .wrap(),
                                );
                            }
                            if slide.body.len() > 12 {
                                ui.label(egui::RichText::new("…").color(p.text_tertiary));
                            }
                        });
                    });
                });
            ui.add_space(8.0);
        }
    });
}

// ---- e-books / fonts ----------------------------------------------------------------

fn image_fit(tex: &Tex, max: egui::Vec2, ppp: f32) -> egui::Vec2 {
    let natural = vec2(tex.size[0] as f32, tex.size[1] as f32) / ppp;
    let s = (max.x / natural.x).min(max.y / natural.y).min(1.0);
    natural * s
}

fn epub(ui: &mut Ui, rect: Rect, m: &EpubModel, cover: Option<&Tex>, id: Id, p: &Palette) {
    ui.painter().rect_filled(rect, 0.0, p.bg_surface1);
    let ppp = ui.ctx().pixels_per_point();
    let mut c = child(ui, rect.shrink2(vec2(16.0, 12.0)));
    ScrollArea::vertical().id_salt(id.with("epub")).auto_shrink([false, false]).show(&mut c, |ui| {
        let wide = ui.available_width() > 560.0;
        let head = |ui: &mut Ui| {
            ui.vertical(|ui| {
                let title = m.title.as_deref().unwrap_or(tr!("views-rich-untitled"));
                ui.add(egui::Label::new(egui::RichText::new(title).font(semibold(19.0)).color(p.text_primary)).wrap());
                if !m.authors.is_empty() {
                    ui.label(egui::RichText::new(m.authors.join(", ")).size(13.0).color(p.text_secondary));
                }
                let sub: Vec<&str> =
                    [m.publisher.as_deref(), m.date.as_deref(), m.language.as_deref()].into_iter().flatten().collect();
                if !sub.is_empty() {
                    ui.label(egui::RichText::new(sub.join("  ·  ")).size(11.0).color(p.text_tertiary));
                }
                if let Some(d) = &m.description {
                    ui.add_space(6.0);
                    let short: String = d.chars().take(600).collect();
                    ui.add(egui::Label::new(egui::RichText::new(short).size(12.0).color(p.text_secondary)).wrap());
                }
            });
        };
        let cover_box = |ui: &mut Ui, max: egui::Vec2| {
            if let Some(t) = cover {
                let size = image_fit(t, max, ppp);
                let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().add(
                    egui::epaint::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(70) }
                        .as_shape(r, CornerRadius::same(3)),
                );
                ui.painter().image(t.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
        };
        if wide {
            ui.horizontal_top(|ui| {
                cover_box(ui, vec2(220.0, 320.0));
                ui.add_space(16.0);
                head(ui);
            });
        } else {
            ui.vertical_centered(|ui| cover_box(ui, vec2(ui.available_width().min(200.0), 280.0)));
            ui.add_space(10.0);
            head(ui);
        }
        ui.add_space(14.0);
        if !m.toc.is_empty() {
            caption(ui, tr!("views-rich-contents"), p);
            ui.add_space(4.0);
            for e in m.toc.iter().take(600) {
                ui.horizontal(|ui| {
                    ui.add_space(e.depth as f32 * 16.0);
                    let (size, color) = if e.depth == 0 { (12.5, p.text_primary) } else { (12.0, p.text_secondary) };
                    ui.add(egui::Label::new(egui::RichText::new(&e.title).size(size).color(color)).truncate());
                });
            }
        }
    });
}

fn font(ui: &mut Ui, rect: Rect, strips: &Arc<Vec<(String, Tex)>>, id: Id, p: &Palette) {
    ui.painter().rect_filled(rect, 0.0, p.bg_surface1);
    let ppp = ui.ctx().pixels_per_point();
    let mut c = child(ui, rect.shrink2(vec2(16.0, 12.0)));
    ScrollArea::vertical().id_salt(id.with("font")).auto_shrink([false, false]).show(&mut c, |ui| {
        let w = ui.available_width();
        for (cap, t) in strips.iter() {
            if !cap.is_empty() {
                ui.add_space(6.0);
                caption(ui, &cap.to_uppercase(), p);
            }
            let natural = vec2(t.size[0] as f32, t.size[1] as f32) / ppp;
            let s = (w / natural.x).min(1.0);
            let (r, _) = ui.allocate_exact_size(natural * s, Sense::hover());
            ui.painter().image(t.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), p.text_primary);
            ui.add_space(4.0);
        }
    });
}

// ---- 3D -----------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ModelState {
    view: View,
    /// Last user interaction (turntable pauses after it).
    touched: f64,
}

fn model_view(
    ui: &mut Ui,
    rect: Rect,
    m: &Arc<preview_model::Model>,
    turntable: bool,
    id: Id,
    p: &Palette,
    interactive: bool,
) {
    // Soft vertical gradient backdrop.
    let top = if p.dark { Color32::from_gray(44) } else { Color32::from_gray(246) };
    let bottom = if p.dark { Color32::from_gray(22) } else { Color32::from_gray(214) };
    let mut bg = egui::epaint::Mesh::default();
    bg.colored_vertex(rect.left_top(), top);
    bg.colored_vertex(rect.right_top(), top);
    bg.colored_vertex(rect.right_bottom(), bottom);
    bg.colored_vertex(rect.left_bottom(), bottom);
    bg.add_triangle(0, 1, 2);
    bg.add_triangle(0, 2, 3);
    ui.painter().add(Shape::mesh(bg));

    let now = ui.input(|i| i.time);
    let mut st =
        ui.data(|d| d.get_temp::<ModelState>(id)).unwrap_or(ModelState { view: View::default(), touched: -10.0 });
    let resp = ui.interact(rect, id.with("drag"), Sense::click_and_drag());
    if resp.dragged() {
        let d = resp.drag_delta();
        st.view.yaw += d.x * 0.01;
        st.view.pitch = (st.view.pitch + d.y * 0.01).clamp(-1.55, 1.55);
        st.touched = now;
    }
    if resp.hovered() && interactive {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
        if scroll != 0.0 {
            st.view.zoom = (st.view.zoom * (scroll / 300.0).exp()).clamp(0.3, 6.0);
            st.touched = now;
        }
    }
    if resp.double_clicked() {
        st.view = View::default();
        st.touched = now;
    }
    let reduced = ui.style().animation_time <= 0.0;
    if turntable && !reduced && !resp.dragged() && now - st.touched > 2.5 {
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        st.view.yaw += dt * 0.45;
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(id, st));

    let radius = rect.width().min(rect.height()) * 0.36;
    let center = rect.center() - vec2(0.0, rect.height() * 0.02);
    // Ground shadow.
    // Soft ground shadow: stacked ellipses fading outwards.
    let sc = center + vec2(0.0, radius * 0.95);
    for k in 0..6 {
        let f = 1.0 - k as f32 * 0.12;
        ui.painter().add(Shape::ellipse_filled(
            sc,
            vec2(radius * 0.75, radius * 0.1) * f * 1.25,
            Color32::from_black_alpha(if p.dark { 22 } else { 12 }),
        ));
    }
    let tint = [p.accent.r() as f32 / 255.0, p.accent.g() as f32 / 255.0, p.accent.b() as f32 / 255.0];
    // Mostly neutral with a hint of the accent.
    let tint = [0.75 + 0.25 * tint[0], 0.75 + 0.25 * tint[1], 0.75 + 0.25 * tint[2]];
    let mesh = preview_model::project(m, st.view, center, radius, tint);
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).add(Shape::mesh(mesh));
    let hint = if interactive { tr!("views-rich-model-hint-full") } else { tr!("views-rich-model-hint") };
    ui.painter().text(
        rect.left_bottom() + vec2(12.0, -12.0),
        Align2::LEFT_BOTTOM,
        hint,
        FontId::proportional(11.0),
        p.text_tertiary,
    );
}

// ---- audio / video ------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn audio(
    ui: &mut Ui,
    rect: Rect,
    entry: &EntryInfo,
    info: &preview_audio::AudioInfo,
    src: &preview_audio::AudioSrc,
    ext: &str,
    playback: bool,
    cover: Option<&Tex>,
    p: &Palette,
    big: bool,
) {
    let ppp = ui.ctx().pixels_per_point();
    let key = src.key();
    let ps = preview_audio::state();
    let mine = ps.key.as_deref() == Some(key.as_str());
    let playing = mine && ps.playing;
    if mine {
        preview_audio::heartbeat();
        if playing {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    let inner = rect.shrink2(vec2(if big { 48.0 } else { 16.0 }, 16.0));
    let art = if big { 180.0 } else { 96.0 };
    let wave_h = if big { 150.0 } else { 70.0 };
    let block_h = art + 24.0 + wave_h + 54.0;
    let top = (inner.center().y - block_h / 2.0).max(inner.top());
    // Cover art or a glyph disc.
    let art_r = Rect::from_center_size(pos2(inner.center().x, top + art / 2.0), vec2(art, art));
    match cover {
        Some(t) => {
            let size = image_fit(t, vec2(art, art), ppp);
            let r = Rect::from_center_size(art_r.center(), size);
            ui.painter().add(
                egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(80) }
                    .as_shape(r, CornerRadius::same(8)),
            );
            ui.painter().image(t.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(art_r, CornerRadius::same(20), p.glyph_audio.gamma_multiply(0.14));
            ui.painter().text(
                art_r.center(),
                Align2::CENTER_CENTER,
                ph::WAVEFORM,
                icons::font_fill(art * 0.45),
                p.glyph_audio,
            );
        }
    }
    let tag = |k: &str| info.tags.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
    let title = tag("Title").unwrap_or_else(|| entry.name.clone());
    let sub = [tag("Artist"), tag("Album")].into_iter().flatten().collect::<Vec<_>>().join("  ·  ");
    let mut y = art_r.bottom() + 16.0;
    let tg = trunc_galley(ui.painter(), &title, semibold(14.0), p.text_primary, inner.width());
    ui.painter().galley(pos2(inner.center().x - tg.size().x / 2.0, y), tg, p.text_primary);
    y += 20.0;
    if !sub.is_empty() {
        let sg = trunc_galley(ui.painter(), &sub, FontId::proportional(12.0), p.text_secondary, inner.width());
        ui.painter().galley(pos2(inner.center().x - sg.size().x / 2.0, y), sg, p.text_secondary);
        y += 18.0;
    }
    y += 8.0;
    // Waveform.
    let wave = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), wave_h));
    let dur = info.duration.unwrap_or(0.0);
    let pos = if mine { ps.pos } else { 0.0 };
    let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    ui.painter().rect_filled(wave, CornerRadius::same(6), p.bg_sunken);
    let mid = wave.center().y;
    if info.peaks.is_empty() {
        ui.painter().line_segment(
            [pos2(wave.left() + 8.0, mid), pos2(wave.right() - 8.0, mid)],
            Stroke::new(1.0, p.text_tertiary),
        );
    } else {
        let cols = ((wave.width() - 12.0) / 3.0).max(1.0) as usize;
        let span = (wave.width() - 12.0) * info.covered.max(0.02);
        let n = info.peaks.len();
        for c in 0..cols {
            let x = wave.left() + 6.0 + c as f32 * 3.0;
            if x > wave.left() + 6.0 + span {
                break;
            }
            let a = c * n / cols;
            let b = ((c + 1) * n / cols).max(a + 1).min(n);
            let (lo, hi) = info.peaks[a..b].iter().fold((0f32, 0f32), |(l, h), &(pl, ph)| (l.min(pl), h.max(ph)));
            let half = wave_h / 2.0 - 6.0;
            let top = mid - hi.max(0.01) * half;
            let bot = mid - lo.min(-0.01) * half;
            let played = (x - wave.left()) / wave.width() <= frac && (mine && pos > 0.0);
            let col = if played { p.accent } else { p.text_tertiary.gamma_multiply(0.85) };
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(x, top), pos2(x + 2.0, bot.max(top + 1.0))),
                CornerRadius::same(1),
                col,
            );
        }
    }
    if mine && dur > 0.0 {
        let x = wave.left() + wave.width() * frac;
        ui.painter()
            .line_segment([pos2(x, wave.top() + 3.0), pos2(x, wave.bottom() - 3.0)], Stroke::new(1.5, p.accent));
    }
    let wresp = ui.interact(wave, Id::new(("soyuz.audio.wave", &key, big)), Sense::click_and_drag());
    if playback
        && dur > 0.0
        && (wresp.clicked() || wresp.dragged())
        && let Some(ptr) = wresp.interact_pointer_pos()
    {
        let t = (((ptr.x - wave.left()) / wave.width()).clamp(0.0, 1.0) as f64) * dur;
        if mine {
            preview_audio::seek(t);
        } else {
            preview_audio::play(src.clone(), ext, t);
        }
    }
    if playback && dur > 0.0 {
        wresp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    y = wave.bottom() + 12.0;
    // Transport.
    let btn = Rect::from_center_size(pos2(inner.center().x, y + 18.0), vec2(36.0, 36.0));
    if playback {
        let bresp = ui.interact(btn, Id::new(("soyuz.audio.play", &key, big)), Sense::click());
        let fill = if bresp.hovered() { p.accent_hover } else { p.accent };
        ui.painter().circle_filled(btn.center(), 18.0, fill);
        let glyph = if playing { ph::PAUSE } else { ph::PLAY };
        ui.painter().text(btn.center(), Align2::CENTER_CENTER, glyph, icons::font_fill(16.0), Color32::WHITE);
        crate::ui::a11y::button(&bresp, if playing { tr!("views-rich-pause") } else { tr!("views-rich-play") });
        if bresp.clicked() {
            if playing {
                preview_audio::pause();
            } else if mine {
                preview_audio::resume();
            } else {
                preview_audio::play(src.clone(), ext, 0.0);
            }
        }
    }
    let time =
        if dur > 0.0 { format!("{} / {}", format_duration(pos), format_duration(dur)) } else { "—".to_string() };
    let tx = if playback { btn.right() + 14.0 } else { inner.center().x };
    let align = if playback { Align2::LEFT_CENTER } else { Align2::CENTER_CENTER };
    ui.painter().text(pos2(tx, btn.center().y), align, time, FontId::monospace(11.5), p.text_secondary);
    if let Some(e) = ps.error.as_ref().filter(|_| mine || ps.key.is_none()) {
        ui.painter().text(
            pos2(inner.center().x, btn.bottom() + 14.0),
            Align2::CENTER_CENTER,
            e,
            FontId::proportional(11.0),
            p.danger,
        );
    }
}

fn video(ui: &mut Ui, rect: Rect, entry: &EntryInfo, poster: Option<&Tex>, note: Option<&str>, p: &Palette) {
    let ppp = ui.ctx().pixels_per_point();
    match poster {
        Some(t) => {
            let avail = rect.shrink(12.0);
            let size = image_fit(t, avail.size() - vec2(0.0, 26.0), ppp);
            let r = Rect::from_center_size(avail.center() - vec2(0.0, 12.0), size);
            ui.painter().rect_filled(rect, 0.0, Color32::from_gray(12));
            ui.painter().image(t.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let badge = Rect::from_min_size(r.left_top() + vec2(10.0, 10.0), vec2(30.0, 30.0));
            ui.painter().circle_filled(badge.center(), 15.0, Color32::from_black_alpha(150));
            ui.painter().text(
                badge.center(),
                Align2::CENTER_CENTER,
                ph::FILM_SLATE,
                icons::font_fill(15.0),
                Color32::WHITE,
            );
            ui.painter().text(
                pos2(rect.center().x, rect.bottom() - 14.0),
                Align2::CENTER_CENTER,
                tr!("views-rich-poster-note"),
                FontId::proportional(11.0),
                Color32::from_gray(170),
            );
        }
        None => {
            let c = rect.center() - vec2(0.0, rect.height() * 0.12);
            let disc = Rect::from_center_size(c, vec2(96.0, 96.0));
            ui.painter().rect_filled(disc, CornerRadius::same(22), p.glyph_video.gamma_multiply(0.14));
            ui.painter().text(c, Align2::CENTER_CENTER, ph::FILM_SLATE, icons::font_fill(46.0), p.glyph_video);
            let tg = trunc_galley(ui.painter(), &entry.name, semibold(14.0), p.text_primary, rect.width() - 32.0);
            ui.painter().galley(pos2(rect.center().x - tg.size().x / 2.0, disc.bottom() + 14.0), tg, p.text_primary);
            if let Some(n) = note {
                let g = ui.painter().layout(
                    n.to_string(),
                    FontId::proportional(11.5),
                    p.text_tertiary,
                    (rect.width() - 40.0).clamp(80.0, 380.0),
                );
                ui.painter().galley(pos2(rect.center().x - g.size().x / 2.0, disc.bottom() + 40.0), g, p.text_tertiary);
            }
        }
    }
}

// ---- PDF ----------------------------------------------------------------------------

type PageRx = crossbeam_channel::Receiver<Result<(u32, Tex), String>>;

#[derive(Clone, Default)]
struct PdfNav {
    page: u32,
    shown: Option<(u32, Tex)>,
    pending: Option<(u32, PageRx)>,
    error: Option<String>,
    text: bool,
}

fn pdf(ui: &mut Ui, rect: Rect, v: &PdfView, id: Id, p: &Palette, interactive: bool) -> Option<PreviewAction> {
    let mut nav = ui.data(|d| d.get_temp::<PdfNav>(id)).unwrap_or_default();
    if nav.shown.is_none()
        && let Some(f) = &v.first
    {
        nav.shown = Some((0, f.clone()));
    }
    let pages = v.info.pages.max(1);
    let can_render = v.no_render.is_none();
    // Keys (Quick Look): PageUp / PageDown flip pages.
    if interactive && can_render {
        let (up, down) = ui.input_mut(|i| {
            (i.consume_key(Modifiers::NONE, Key::PageUp), i.consume_key(Modifiers::NONE, Key::PageDown))
        });
        if up {
            nav.page = nav.page.saturating_sub(1);
        }
        if down {
            nav.page = (nav.page + 1).min(pages - 1);
        }
    }
    // Toolbar.
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 38.0));
    ui.painter().rect_filled(bar, 0.0, p.bg_surface1);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, p.border_subtle));
    let mut bc = ui.new_child(
        UiBuilder::new()
            .max_rect(bar.shrink2(vec2(10.0, 5.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if can_render {
        if crate::ui::widgets::icon_toggle(&mut bc, ph::CARET_LEFT, tr!("views-rich-prev-page"), nav.page > 0, false)
            .clicked()
        {
            nav.page = nav.page.saturating_sub(1);
        }
        bc.label(
            egui::RichText::new(tr!("views-rich-page-of", page = nav.page + 1, pages = pages))
                .size(12.0)
                .color(p.text_secondary),
        );
        if crate::ui::widgets::icon_toggle(
            &mut bc,
            ph::CARET_RIGHT,
            tr!("views-rich-next-page"),
            nav.page + 1 < pages,
            false,
        )
        .clicked()
        {
            nav.page = (nav.page + 1).min(pages - 1);
        }
    } else {
        bc.label(
            egui::RichText::new(tr!("views-rich-pages-text-only", pages = pages)).size(12.0).color(p.text_secondary),
        );
    }
    let mut toggle_r = bar;
    toggle_r.min.x = bar.right() - 120.0;
    if can_render && !v.info.text.is_empty() {
        let mut tc = ui.new_child(
            UiBuilder::new()
                .max_rect(toggle_r.shrink2(vec2(10.0, 5.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if crate::ui::widgets::segmented(
            &mut tc,
            &[(ph::FILE, tr!("views-rich-page")), (ph::TEXT_ALIGN_LEFT, tr!("views-rich-text"))],
            usize::from(nav.text),
            vec2(30.0, 24.0),
        )
        .is_some_and(|i| (i == 1) != nav.text)
        {
            nav.text = !nav.text;
        }
    }
    let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
    // Fetch the wanted page.
    if can_render && nav.shown.as_ref().is_none_or(|(pg, _)| *pg != nav.page) {
        if nav.pending.as_ref().is_none_or(|(pg, _)| *pg != nav.page) {
            nav.pending = Some((nav.page, pdf_page_async(v, nav.page, ui.ctx())));
            nav.error = None;
        }
        if let Some((_, rx)) = &nav.pending
            && let Ok(r) = rx.try_recv()
        {
            match r {
                Ok((pg, t)) => nav.shown = Some((pg, t)),
                Err(e) => nav.error = Some(e),
            }
            nav.pending = None;
        }
    }
    let mut action = None;
    if !can_render || nav.text {
        ui.painter().rect_filled(body, 0.0, p.bg_surface1);
        let mut c = child(ui, body.shrink2(vec2(16.0, 10.0)));
        ScrollArea::vertical().id_salt(id.with("pdftext")).auto_shrink([false, false]).show(&mut c, |ui| {
            if let Some(why) = &v.no_render {
                pdfium_card(ui, why, &v.dirs, p);
                ui.add_space(10.0);
            }
            if v.info.text.is_empty() {
                ui.label(egui::RichText::new(tr!("views-rich-pdf-no-text")).color(p.text_tertiary));
            } else {
                caption(ui, tr!("views-rich-pdf-text-caption"), p);
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(&v.info.text).size(12.0).color(p.text_primary))
                        .wrap()
                        .selectable(true),
                );
            }
        });
    } else {
        ui.painter().rect_filled(body, 0.0, p.bg_sunken);
        if let Some((pg, t)) = &nav.shown {
            let ppp = ui.ctx().pixels_per_point();
            let avail = body.shrink(if interactive { 14.0 } else { 10.0 });
            let natural = vec2(t.size[0] as f32, t.size[1] as f32) / ppp;
            // Pane: fit width and scroll; Quick Look: fit the whole page.
            let s = if interactive {
                (avail.width() / natural.x).min(avail.height() / natural.y)
            } else {
                avail.width() / natural.x
            };
            let size = natural * s;
            let dim = *pg != nav.page;
            let tint = if dim { Color32::from_white_alpha(150) } else { Color32::WHITE };
            let mut c = child(ui, body);
            ScrollArea::vertical().id_salt(id.with(("pdfpage", *pg))).auto_shrink([false, false]).show(&mut c, |ui| {
                ui.add_space((avail.top() - body.top()).max(0.0));
                let (r0, _) = ui.allocate_exact_size(vec2(body.width(), size.y), Sense::hover());
                let r = Rect::from_center_size(r0.center(), size);
                ui.painter().add(
                    egui::epaint::Shadow { offset: [0, 3], blur: 12, spread: 0, color: Color32::from_black_alpha(60) }
                        .as_shape(r, CornerRadius::ZERO),
                );
                ui.painter().image(t.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
                ui.add_space(10.0);
            });
            if dim {
                spinner(ui, body.center(), 11.0, p.accent);
            }
        } else if nav.error.is_none() {
            spinner(ui, body.center(), 11.0, p.accent);
        }
        if let Some(e) = &nav.error {
            ui.painter().text(body.center(), Align2::CENTER_CENTER, e, FontId::proportional(12.0), p.danger);
            let br = Rect::from_center_size(body.center() + vec2(0.0, 30.0), vec2(120.0, 26.0));
            let mut c = ui.new_child(UiBuilder::new().max_rect(br));
            if pill_button(&mut c, Some(ph::BINARY), tr!("views-preview-show-bytes"), false).clicked() {
                action = Some(PreviewAction::ToggleHex);
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(id, nav));
    action
}

fn pdfium_card(ui: &mut Ui, why: &str, dirs: &[std::path::PathBuf], p: &Palette) {
    egui::Frame::new()
        .fill(p.accent_subtle)
        .stroke(Stroke::new(1.0, p.accent.gamma_multiply(0.4)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(ph::FILE_PDF).font(icons::font_fill(16.0)).color(p.accent));
                ui.label(
                    egui::RichText::new(tr!("views-rich-pdfium-needed")).font(semibold(13.0)).color(p.text_primary),
                );
            });
            ui.add_space(4.0);
            let text = if why.is_empty() { preview_pdf::missing_message(dirs) } else { why.to_string() };
            ui.add(
                egui::Label::new(egui::RichText::new(text).size(11.5).color(p.text_secondary)).wrap().selectable(true),
            );
        });
}
