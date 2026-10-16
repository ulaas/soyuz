//! Shared preview state and renderers used by both the preview pane and Quick Look:
//! request management (one in-flight load per consumer, stale requests dropped), hex
//! paging, folder-size streaming, and painters for every content kind plus the info
//! section.

use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};
use soyuz_appcore::models::preview_load::PreviewLoader;
use soyuz_core::config::Config;
use soyuz_core::{EntryInfo, format_size, format_time};

use super::hex;
use super::markdown;
use crate::services::preview::{Content, Kind, Limits};
use crate::services::preview_gfx::StyledLine;
use crate::theme::icons::{self, ph};
use crate::theme::{Palette, palette};
use crate::ui::gfx::{self, TexExt};
use crate::ui::views::flow::semibold;
use crate::ui::widgets::{pill_button, spinner, trunc_galley};

pub use soyuz_appcore::models::preview_info::{group_digits, info_label, info_rows};
pub use soyuz_appcore::models::preview_load::PreviewAction;

/// One preview consumer's state: the core's [`PreviewLoader`] plus Quick Look's image
/// zoom and pan.
pub struct PreviewState {
    loader: PreviewLoader,
    /// Quick Look image zoom (1 = fit) and pan offset.
    pub zoom: f32,
    pub pan: Vec2,
}

impl std::ops::Deref for PreviewState {
    type Target = PreviewLoader;
    fn deref(&self) -> &PreviewLoader {
        &self.loader
    }
}

impl std::ops::DerefMut for PreviewState {
    fn deref_mut(&mut self) -> &mut PreviewLoader {
        &mut self.loader
    }
}

impl PreviewState {
    pub fn new(ctx: &egui::Context, big: bool) -> Self {
        PreviewState {
            loader: PreviewLoader::new(
                crate::host::egui_wake(ctx),
                gfx::egui_uploader(ctx),
                big,
                ctx.pixels_per_point(),
            ),
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }

    /// Request limits from the config and palette.
    pub fn limits(&self, cfg: &Config, p: &Palette) -> Limits {
        self.loader.limits(cfg, p.dark, gfx::rgba(p.text_primary))
    }

    /// Show `entry` (or nothing). Issues a new request only when the target changed.
    pub fn set_target(&mut self, entry: Option<&EntryInfo>, limits: Limits) {
        if self.loader.set_target(entry, limits) {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
    }

    /// Drain worker results (call once per frame).
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.loader.pixels_per_point = ctx.pixels_per_point();
        self.loader.poll();
    }
}

/// Two-column info grid.
pub fn info_grid(ui: &mut Ui, rows: &[(String, String)], label_w: f32) {
    let p = palette(ui.ctx());
    let font = FontId::proportional(11.5);
    // Translated labels can be longer: widen the label column up to a limit.
    let widest = rows
        .iter()
        .map(|(k, _)| ui.painter().layout_no_wrap(info_label(k).to_string(), font.clone(), p.text_tertiary).size().x)
        .fold(0.0_f32, f32::max);
    let label_w = label_w.max(widest + 4.0).min(label_w * 1.6);
    for (k, v) in rows {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (r, _) = ui.allocate_exact_size(vec2(label_w, 18.0), Sense::hover());
            let label = info_label(k);
            ui.painter().text(r.left_center(), Align2::LEFT_CENTER, label, font.clone(), p.text_tertiary);
            ui.add(egui::Label::new(egui::RichText::new(v).size(12.0).color(p.text_primary)).wrap());
        });
    }
}

// ---- content ------------------------------------------------------------------------

fn centered_message(ui: &Ui, rect: Rect, icon: &str, color: Color32, title: &str, body: &str) {
    let p = palette(ui.ctx());
    let c = rect.center();
    ui.painter().text(c - vec2(0.0, 30.0), Align2::CENTER_CENTER, icon, icons::font_fill(34.0), color);
    if !title.is_empty() {
        ui.painter().text(c + vec2(0.0, 8.0), Align2::CENTER_CENTER, title, semibold(13.5), p.text_primary);
    }
    if !body.is_empty() {
        let g = ui.painter().layout(
            body.to_string(),
            FontId::proportional(12.0),
            p.text_tertiary,
            (rect.width() - 40.0).clamp(80.0, 360.0),
        );
        let pos = pos2(c.x - g.size().x / 2.0, c.y + 22.0);
        ui.painter().galley(pos, g, p.text_tertiary);
    }
}

/// Subtle checkerboard behind images (transparency).
fn checker(ui: &Ui, r: Rect, p: &Palette) {
    let cell = 8.0;
    let (a, b) = if p.dark {
        (Color32::from_gray(38), Color32::from_gray(30))
    } else {
        (Color32::from_gray(238), Color32::from_gray(226))
    };
    ui.painter().rect_filled(r, 0.0, a);
    let cols = (r.width() / cell).ceil() as i32;
    let rows = (r.height() / cell).ceil() as i32;
    if cols * rows > 40_000 {
        return;
    }
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                let cr =
                    Rect::from_min_size(pos2(r.left() + x as f32 * cell, r.top() + y as f32 * cell), vec2(cell, cell))
                        .intersect(r);
                ui.painter().rect_filled(cr, 0.0, b);
            }
        }
    }
}

/// Draw the preview content into `rect`. `interactive` enables zoom/pan (Quick Look).
pub fn content(ui: &mut Ui, rect: Rect, st: &mut PreviewState, interactive: bool) -> Option<PreviewAction> {
    let p = palette(ui.ctx());
    let mut action = None;
    let Some(entry) = st.entry.clone() else {
        centered_message(
            ui,
            rect,
            ph::EYE,
            p.text_tertiary,
            tr!("views-preview-nothing-selected"),
            tr!("views-preview-nothing-selected-sub"),
        );
        return None;
    };
    let Some(loaded) = st.loaded.clone() else {
        if st.loading_since.is_some_and(|t| t.elapsed().as_secs_f32() > 0.12) {
            spinner(ui, rect.center(), 11.0, p.accent);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        return None;
    };
    // A newer request is still loading: dim the old content slightly.
    let stale = st.loading_since.is_some();
    match &loaded.content {
        Content::Image(img) => {
            let tex_size = img.tex.size_vec2();
            let ppp = ui.ctx().pixels_per_point();
            let natural = vec2(img.width as f32, img.height as f32) / ppp;
            let avail = rect.shrink(if interactive { 0.0 } else { 12.0 });
            let fit = (avail.width() / natural.x).min(avail.height() / natural.y).clamp(0.0001, 1.0);
            let mut scale = fit;
            if interactive {
                let resp = ui.interact(rect, ui.scope_id().with("ql.image"), Sense::click_and_drag());
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
                    if scroll != 0.0 {
                        let old = st.zoom;
                        st.zoom = (st.zoom * (scroll / 300.0).exp()).clamp(0.1, 32.0);
                        if let Some(ptr) = resp.hover_pos() {
                            let c = rect.center() + st.pan;
                            st.pan += (ptr - c) * (1.0 - st.zoom / old);
                        }
                    }
                }
                if resp.dragged() {
                    st.pan += resp.drag_delta();
                }
                if resp.double_clicked() {
                    if (st.zoom - 1.0).abs() < 0.01 {
                        st.zoom = 1.0 / fit; // 100%
                    } else {
                        st.zoom = 1.0;
                        st.pan = Vec2::ZERO;
                    }
                }
                scale = fit * st.zoom;
            }
            let size = natural * scale;
            let r = Rect::from_center_size(avail.center() + if interactive { st.pan } else { Vec2::ZERO }, size);
            let clip = ui.clip_rect().intersect(rect);
            let painter = ui.painter().with_clip_rect(clip);
            if !interactive {
                checker(ui, r, &p);
            }
            let tint = if stale { Color32::from_white_alpha(200) } else { Color32::WHITE };
            painter.image(img.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
            if !interactive {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, p.border_subtle), StrokeKind::Outside);
            }
            let _ = tex_size;
            if interactive {
                let pct = format!("{:.0}%", scale * ppp * 100.0);
                let badge = Rect::from_min_size(rect.left_bottom() + vec2(12.0, -34.0), vec2(56.0, 22.0));
                painter.rect_filled(badge, CornerRadius::same(6), p.bg_surface3.gamma_multiply(0.9));
                painter.text(badge.center(), Align2::CENTER_CENTER, pct, FontId::proportional(11.5), p.text_secondary);
            }
        }
        Content::Svg { uri, bytes } => {
            let avail = rect.shrink(16.0);
            let img = egui::Image::from_bytes(uri.clone(), egui::load::Bytes::Shared(bytes.clone()))
                .max_size(avail.size() * if interactive { st.zoom } else { 1.0 })
                .maintain_aspect_ratio(true);
            let mut child = ui.new_child(
                UiBuilder::new().max_rect(avail).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
            );
            child.add(img);
        }
        Content::Text(t) => text_view(ui, rect, &t.lines, t.widest, t.truncated),
        Content::Markdown { blocks, truncated } => {
            let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 8.0))));
            ScrollArea::vertical().id_salt(("md", entry.path.clone())).auto_shrink([false, false]).show(
                &mut child,
                |ui| {
                    markdown::show(ui, blocks, &p);
                    if *truncated {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(tr!("views-preview-truncated")).size(11.0).color(p.text_tertiary));
                    }
                },
            );
        }
        Content::Hex(_) => {
            if let Some((doc, client)) = st.hex_doc() {
                hex::show(ui, rect, doc, client, &p, if interactive { 13.0 } else { 11.5 });
            }
        }
        Content::Archive(l) => {
            let header = Rect::from_min_size(rect.min, vec2(rect.width(), 40.0));
            ui.painter().text(
                header.left_center() + vec2(14.0, -7.0),
                Align2::LEFT_CENTER,
                tr!("views-preview-archive-entries", count = l.total_entries),
                semibold(12.5),
                p.text_primary,
            );
            ui.painter().text(
                header.left_center() + vec2(14.0, 9.0),
                Align2::LEFT_CENTER,
                tr!(
                    "views-preview-archive-unpacked",
                    format = &l.format,
                    size = format_size(l.total_size, soyuz_core::SizeStyle::Binary)
                ),
                FontId::proportional(11.0),
                p.text_tertiary,
            );
            let br = Rect::from_min_size(pos2(header.right() - 84.0, header.top() + 8.0), vec2(74.0, 24.0));
            let mut bchild = ui.new_child(UiBuilder::new().max_rect(br));
            if pill_button(&mut bchild, Some(ph::FOLDER_OPEN), tr!("views-preview-browse"), false).clicked() {
                action = Some(PreviewAction::Open(entry.path.clone()));
            }
            let list = Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max);
            let mut child = ui.new_child(UiBuilder::new().max_rect(list));
            child.set_clip_rect(list.intersect(ui.clip_rect()));
            let row_h = 22.0;
            ScrollArea::vertical().id_salt(("arch", entry.path.clone())).auto_shrink([false, false]).show_rows(
                &mut child,
                row_h,
                l.entries.len(),
                |ui, range| {
                    for i in range {
                        let (path, is_dir, size) = &l.entries[i];
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                        if resp.hovered() {
                            ui.painter().rect_filled(r.shrink2(vec2(6.0, 0.0)), CornerRadius::same(4), p.bg_surface2);
                        }
                        let depth = path.trim_end_matches('/').matches('/').count() as f32;
                        let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
                        let x = r.left() + 14.0 + depth.min(8.0) * 12.0;
                        let (g, c) =
                            if *is_dir { (ph::FOLDER_SIMPLE, p.glyph_folder) } else { (ph::FILE, p.text_tertiary) };
                        ui.painter().text(pos2(x + 7.0, r.center().y), Align2::CENTER_CENTER, g, icons::font(13.0), c);
                        let tg = trunc_galley(
                            ui.painter(),
                            name,
                            FontId::proportional(12.0),
                            p.text_primary,
                            r.right() - x - 100.0,
                        );
                        ui.painter().galley(pos2(x + 20.0, r.center().y - tg.size().y / 2.0), tg, p.text_primary);
                        if !is_dir {
                            ui.painter().text(
                                pos2(r.right() - 14.0, r.center().y),
                                Align2::RIGHT_CENTER,
                                format_size(*size, soyuz_core::SizeStyle::Binary),
                                FontId::proportional(11.0),
                                p.text_tertiary,
                            );
                        }
                    }
                },
            );
        }
        Content::Folder(s) => {
            let c = rect.center() - vec2(0.0, rect.height() * 0.12);
            ui.painter().text(c, Align2::CENTER_CENTER, ph::FOLDER_SIMPLE, icons::font_fill(54.0), p.glyph_folder);
            let mut y = c.y + 48.0;
            let line = |ui: &Ui, y: &mut f32, text: String, font: FontId, color: Color32| {
                ui.painter().text(pos2(rect.center().x, *y), Align2::CENTER_CENTER, text, font, color);
                *y += 20.0;
            };
            if let Some(err) = &s.error {
                line(ui, &mut y, tr!("views-preview-folder-unreadable").into(), semibold(13.0), p.text_primary);
                line(ui, &mut y, err.clone(), FontId::proportional(11.5), p.danger);
            } else {
                line(
                    ui,
                    &mut y,
                    tr!("views-preview-folder-counts", dirs = s.dirs, files = s.files),
                    semibold(13.5),
                    p.text_primary,
                );
                match st.folder_size() {
                    Some((bytes, files, _, done)) => {
                        let size = format_size(bytes, soyuz_core::SizeStyle::Binary);
                        let text = if done {
                            tr!("views-preview-folder-size", size = size, files = files)
                        } else {
                            tr!("views-preview-folder-size-running", size = size, files = files)
                        };
                        line(
                            ui,
                            &mut y,
                            text,
                            FontId::proportional(12.0),
                            if done { p.text_secondary } else { p.accent },
                        );
                        if !done {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                        }
                    }
                    None => line(
                        ui,
                        &mut y,
                        tr!(
                            "views-preview-folder-direct",
                            size = format_size(s.direct_bytes, soyuz_core::SizeStyle::Binary)
                        ),
                        FontId::proportional(12.0),
                        p.text_secondary,
                    ),
                }
                if s.hidden > 0 {
                    line(
                        ui,
                        &mut y,
                        tr!("views-preview-folder-hidden", count = s.hidden),
                        FontId::proportional(11.5),
                        p.text_tertiary,
                    );
                }
                if !s.top_ext.is_empty() {
                    y += 6.0;
                    let chips: Vec<String> = s.top_ext.iter().map(|(e, n)| format!(".{e} {n}")).collect();
                    let font = FontId::proportional(11.0);
                    let widths: Vec<f32> = chips
                        .iter()
                        .map(|c| ui.painter().layout_no_wrap(c.clone(), font.clone(), p.text_secondary).size().x + 16.0)
                        .collect();
                    let total: f32 = widths.iter().sum::<f32>() + 6.0 * (chips.len() as f32 - 1.0);
                    let mut x = rect.center().x - total.min(rect.width() - 20.0) / 2.0;
                    for (chip, w) in chips.iter().zip(widths) {
                        if x + w > rect.right() - 8.0 {
                            break;
                        }
                        let r = Rect::from_min_size(pos2(x, y - 10.0), vec2(w, 20.0));
                        ui.painter().rect_filled(r, CornerRadius::same(10), p.bg_surface2);
                        ui.painter().text(r.center(), Align2::CENTER_CENTER, chip, font.clone(), p.text_secondary);
                        x += w + 6.0;
                    }
                    y += 22.0;
                }
                if let Some((name, t)) = &s.newest {
                    let rel = format_time(*t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Relative));
                    line(
                        ui,
                        &mut y,
                        tr!("views-preview-folder-newest", name = name, when = rel),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                }
            }
            let _ = y;
        }
        Content::Media { video } => {
            let c = rect.center() - vec2(0.0, rect.height() * 0.18);
            let (g, col) = if *video { (ph::FILM_SLATE, p.glyph_video) } else { (ph::WAVEFORM, p.glyph_audio) };
            let disc = Rect::from_center_size(c, vec2(96.0, 96.0));
            ui.painter().rect_filled(disc, CornerRadius::same(22), col.gamma_multiply(0.14));
            ui.painter().text(c, Align2::CENTER_CENTER, g, icons::font_fill(46.0), col);
            let mut y = disc.bottom() + 22.0;
            let title = loaded.facts.iter().find(|(k, _)| k == "Title").map(|(_, v)| v.clone());
            let artist = loaded.facts.iter().find(|(k, _)| k == "Artist").map(|(_, v)| v.clone());
            let headline = title.unwrap_or_else(|| entry.name.clone());
            let tg = trunc_galley(ui.painter(), &headline, semibold(14.0), p.text_primary, rect.width() - 32.0);
            ui.painter().galley(pos2(rect.center().x - tg.size().x / 2.0, y - tg.size().y / 2.0), tg, p.text_primary);
            y += 20.0;
            if let Some(a) = artist {
                ui.painter().text(
                    pos2(rect.center().x, y),
                    Align2::CENTER_CENTER,
                    a,
                    FontId::proportional(12.0),
                    p.text_secondary,
                );
                y += 20.0;
            }
            let summary: Vec<String> = ["Duration", "Dimensions", "Format", "Container"]
                .iter()
                .filter_map(|k| loaded.facts.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.clone()))
                .collect();
            if !summary.is_empty() {
                ui.painter().text(
                    pos2(rect.center().x, y),
                    Align2::CENTER_CENTER,
                    summary.join(" · "),
                    FontId::proportional(11.5),
                    p.text_tertiary,
                );
            }
        }
        Content::Rich(r) => {
            if let Some(a) = super::rich::show(ui, rect, r, &entry, interactive) {
                action = Some(a);
            }
        }
        Content::Message { icon, title, body } => {
            centered_message(ui, rect, crate::ui::icons::glyph(*icon), p.text_tertiary, title, body);
            if loaded.kind == Kind::Pdf || loaded.kind == Kind::Image {
                let br = Rect::from_center_size(rect.center() + vec2(0.0, 74.0), vec2(120.0, 26.0));
                let mut child = ui.new_child(UiBuilder::new().max_rect(br));
                if pill_button(&mut child, Some(ph::BINARY), tr!("views-preview-show-bytes"), false).clicked() {
                    action = Some(PreviewAction::ToggleHex);
                }
            }
        }
    }
    action
}

/// Virtualized, highlighted text with a line-number gutter.
fn text_view(ui: &mut Ui, rect: Rect, lines: &Arc<Vec<StyledLine>>, widest: usize, truncated: bool) {
    let p = palette(ui.ctx());
    let font = FontId::monospace(12.0);
    let row_h = 18.0;
    let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
    let n = lines.len() + usize::from(truncated);
    let digits = n.to_string().len().max(2);
    let gutter = char_w * digits as f32 + 18.0;
    let content_w = gutter + 12.0 + char_w * widest.min(crate::services::preview_text::MAX_LINE_CHARS) as f32 + 24.0;
    ui.painter().rect_filled(rect, 0.0, p.bg_sunken);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    ScrollArea::both().id_salt(("text", Arc::as_ptr(lines) as usize)).auto_shrink([false, false]).show_rows(
        &mut child,
        row_h,
        n,
        |ui, range| {
            let w = content_w.max(ui.available_width());
            for i in range {
                let (r, _) = ui.allocate_exact_size(vec2(w, row_h), Sense::hover());
                if i >= lines.len() {
                    ui.painter().text(
                        pos2(r.left() + gutter + 12.0, r.center().y),
                        Align2::LEFT_CENTER,
                        tr!("views-preview-text-truncated"),
                        FontId::proportional(11.0),
                        p.text_tertiary,
                    );
                    continue;
                }
                ui.painter().text(
                    pos2(r.left() + gutter - 8.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    (i + 1).to_string(),
                    font.clone(),
                    p.text_tertiary.gamma_multiply(0.8),
                );
                let galley = ui.painter().layout_job(gfx::layout_job(&lines[i]));
                ui.painter().galley(
                    pos2(r.left() + gutter + 12.0, r.center().y - galley.size().y / 2.0),
                    galley,
                    p.text_primary,
                );
            }
        },
    );
    // Gutter separator.
    ui.painter().line_segment(
        [pos2(rect.left() + gutter + 2.0, rect.top()), pos2(rect.left() + gutter + 2.0, rect.bottom())],
        Stroke::new(1.0, p.border_subtle),
    );
}
