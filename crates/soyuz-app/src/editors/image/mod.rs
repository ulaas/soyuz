//! The image editor: zoom / pan / fit, rotate 90°, flip and crop with undo. Decoding,
//! the display copy and encoding all run on worker threads; edits are replayed on a
//! downscaled display copy for the screen and on the full image when saving. PNG, BMP,
//! TIFF, WebP (lossless), GIF and ICO are re-encoded without loss; JPEG has no pure-Rust
//! lossless transform, so it is re-encoded at `[editor] jpeg_quality` with a warning.

pub mod ops;

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{Receiver, unbounded};
use egui::{
    Align2, Color32, ColorImage, CornerRadius, CursorIcon, Event, FontId, Id, Key, Rect, Sense, Stroke, TextureHandle,
    TextureOptions, Ui, Vec2, pos2, vec2,
};
use image::{DynamicImage, ImageFormat};

use ops::{ImgOp, NormRect, SaveMode};

use super::widgets;
use crate::services::mailbox::Wake;
use crate::theme::{Palette, icons::ph};

const TOOLBAR_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;
/// Longest side of the on-screen copy (GPU texture limit).
const DISPLAY_MAX: u32 = 8192;

#[derive(Default)]
pub struct ImageOutput {
    pub save: bool,
}

/// A decoded image ready for the editor.
pub struct Loaded {
    pub format: Option<ImageFormat>,
    pub full: Arc<DynamicImage>,
    pub display: Arc<DynamicImage>,
}

/// Decode (EXIF orientation applied) and build the display copy. Worker threads only.
pub fn load(path: &std::path::Path, max_mp: u32) -> Result<Loaded, String> {
    let reader =
        image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let format = reader.format();
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if u64::from(w) * u64::from(h) > u64::from(max_mp.max(1)) * 1_000_000 {
        return Err(tr!(
            "editors-image-too-large",
            width = w.to_string(),
            height = h.to_string(),
            limit = max_mp.to_string()
        ));
    }
    let reader =
        image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(u64::from(max_mp.max(1)) * 1_000_000 * 16);
    let mut decoder = {
        let mut r = reader;
        r.limits(limits);
        r.into_decoder().map_err(|e| e.to_string())?
    };
    use image::ImageDecoder;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    img.apply_orientation(orientation);
    let display = display_copy(&img);
    Ok(Loaded { format, full: Arc::new(img), display: Arc::new(display) })
}

fn display_copy(img: &DynamicImage) -> DynamicImage {
    if img.width() > DISPLAY_MAX || img.height() > DISPLAY_MAX {
        img.resize(DISPLAY_MAX, DISPLAY_MAX, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    }
}

/// Encode `img` for `mode` (worker threads only).
pub fn encode(img: &DynamicImage, mode: SaveMode, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    match mode {
        SaveMode::Lossless(f) => {
            let img = match f {
                // BMP / ICO / GIF encoders take 8-bit RGB(A).
                ImageFormat::Bmp | ImageFormat::Ico | ImageFormat::Gif => DynamicImage::ImageRgba8(img.to_rgba8()),
                ImageFormat::WebP => DynamicImage::ImageRgba8(img.to_rgba8()),
                _ => img.clone(),
            };
            img.write_to(&mut out, f).map_err(|e| e.to_string())?;
        }
        SaveMode::Lossy(_) => {
            let rgb = img.to_rgb8();
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality.clamp(1, 100));
            rgb.write_with_encoder(enc).map_err(|e| e.to_string())?;
        }
        SaveMode::Unsupported => return Err(tr!("editors-image-cant-write").into()),
    }
    Ok(out.into_inner())
}

pub struct ImageDoc {
    pub path: PathBuf,
    pub format: Option<ImageFormat>,
    pub full: Arc<DynamicImage>,
    base_display: Arc<DynamicImage>,
    pub ops: Vec<ImgOp>,
    redo: Vec<ImgOp>,
    texture: Option<TextureHandle>,
    /// Display copy with the current ops, built on a worker.
    pending: Option<(u64, Receiver<ColorImage>)>,
    generation: u64,
    shown_generation: u64,
    wake: Wake,
    /// `None` = fit to the view.
    zoom: Option<f32>,
    pan: Vec2,
    crop_mode: bool,
    crop: Option<NormRect>,
    crop_drag: Option<egui::Pos2>,
    pub focus_request: bool,
    last_scale: f32,
}

impl ImageDoc {
    pub fn new(path: PathBuf, loaded: Loaded, wake: Wake) -> Self {
        let mut d = ImageDoc {
            path,
            format: loaded.format,
            full: loaded.full,
            base_display: loaded.display,
            ops: Vec::new(),
            redo: Vec::new(),
            texture: None,
            pending: None,
            generation: 0,
            shown_generation: u64::MAX,
            wake,
            zoom: None,
            pan: Vec2::ZERO,
            crop_mode: false,
            crop: None,
            crop_drag: None,
            focus_request: true,
            last_scale: 1.0,
        };
        d.rebuild();
        d
    }

    pub fn is_dirty(&self) -> bool {
        !self.ops.is_empty()
    }

    pub fn save_mode(&self) -> SaveMode {
        ops::save_mode(self.format)
    }

    /// Replace with a fresh load (external change or after a save).
    pub fn reload(&mut self, loaded: Loaded) {
        self.format = loaded.format.or(self.format);
        self.full = loaded.full;
        self.base_display = loaded.display;
        self.ops.clear();
        self.redo.clear();
        self.crop = None;
        self.crop_mode = false;
        self.rebuild();
    }

    /// The image after the edits (for saving; call on a worker).
    pub fn edited(full: &DynamicImage, ops: &[ImgOp]) -> DynamicImage {
        ops::replay(full, ops)
    }

    fn rebuild(&mut self) {
        self.generation += 1;
        let (tx, rx) = unbounded();
        let base = self.base_display.clone();
        let ops = self.ops.clone();
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new().name("soyuz-image-edit".into()).spawn(move || {
            let img = ops::replay(&base, &ops).to_rgba8();
            let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            let _ = tx.send(ci);
            wake();
        });
        self.pending = Some((self.generation, rx));
    }

    pub fn push(&mut self, op: ImgOp) {
        self.ops.push(op);
        self.redo.clear();
        self.crop = None;
        if matches!(op, ImgOp::RotateCw | ImgOp::RotateCcw | ImgOp::Crop(_)) {
            self.pan = Vec2::ZERO;
        }
        self.rebuild();
    }

    pub fn undo(&mut self) {
        if let Some(op) = self.ops.pop() {
            self.redo.push(op);
            self.rebuild();
        }
    }

    pub fn redo(&mut self) {
        if let Some(op) = self.redo.pop() {
            self.ops.push(op);
            self.rebuild();
        }
    }

    /// Esc: leave crop mode; `false` when nothing to close.
    pub fn close_bars(&mut self) -> bool {
        if self.crop_mode || self.crop.is_some() {
            self.crop_mode = false;
            self.crop = None;
            return true;
        }
        false
    }

    fn size(&self) -> (u32, u32) {
        ops::size_after(self.full.width(), self.full.height(), &self.ops)
    }

    fn apply_crop(&mut self) {
        if let Some(c) = self.crop.take() {
            let c = c.normalized();
            if c.x1 - c.x0 > 0.001 && c.y1 - c.y0 > 0.001 {
                self.push(ImgOp::Crop(c));
                self.crop_mode = false;
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        jpeg_quality: u8,
        focus: &mut Vec<Id>,
        id: Id,
    ) -> ImageOutput {
        if let Some((generation, rx)) = &self.pending
            && let Ok(ci) = rx.try_recv()
        {
            let generation = *generation;
            self.pending = None;
            match &mut self.texture {
                Some(t) => t.set(ci, TextureOptions::LINEAR),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        format!("soyuz-editor-{}", self.path.display()),
                        ci,
                        TextureOptions::LINEAR,
                    ))
                }
            }
            self.shown_generation = generation;
        }
        let mut out = ImageOutput::default();
        let tb = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
        self.toolbar(ui, tb, p, id, &mut out);
        let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STATUS_H), rect.max);
        let mut top = tb.bottom();
        if self.is_dirty() && matches!(self.save_mode(), SaveMode::Lossy(_) | SaveMode::Unsupported) {
            let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
            let text = match self.save_mode() {
                SaveMode::Lossy(_) => tr!("editors-image-lossy-warning", quality = jpeg_quality.to_string()),
                _ => tr!("editors-image-unsupported-warning").to_string(),
            };
            widgets::banner(ui, r, p, ph::WARNING, p.warning, &text, &[]);
            top += 36.0;
        }
        let area = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
        self.view(ui, area, p, focus, id);
        self.status(ui, status, p);
        out
    }

    fn toolbar(&mut self, ui: &mut Ui, r: Rect, p: &Palette, id: Id, out: &mut ImageOutput) {
        let mut x = r.left() + 6.0;
        let can_save = self.is_dirty() && self.save_mode() != SaveMode::Unsupported;
        let (can_undo, can_redo) = (!self.ops.is_empty(), !self.redo.is_empty());
        let crop_mode = self.crop_mode;
        let fit = self.zoom.is_none();
        let mut clicked: Vec<&'static str> = Vec::new();
        let items: [(&'static str, &'static str, &'static str, bool, bool); 12] = [
            ("save", ph::FLOPPY_DISK, tr!("editors-tb-save"), can_save, false),
            ("undo", ph::ARROW_COUNTER_CLOCKWISE, tr!("editors-tb-undo"), can_undo, false),
            ("redo", ph::ARROW_CLOCKWISE, tr!("editors-tb-redo"), can_redo, false),
            ("fit", ph::ARROWS_IN, tr!("editors-image-fit"), true, fit),
            ("100", ph::MAGNIFYING_GLASS, tr!("editors-image-actual-size"), true, false),
            ("out", ph::MAGNIFYING_GLASS_MINUS, tr!("editors-image-zoom-out"), true, false),
            ("in", ph::MAGNIFYING_GLASS_PLUS, tr!("editors-image-zoom-in"), true, false),
            ("rl", ph::ARROW_COUNTER_CLOCKWISE, tr!("editors-image-rotate-left"), true, false),
            ("rr", ph::ARROW_CLOCKWISE, tr!("editors-image-rotate-right"), true, false),
            ("fh", ph::FLIP_HORIZONTAL, tr!("editors-image-flip-h"), true, false),
            ("fv", ph::FLIP_VERTICAL, tr!("editors-image-flip-v"), true, false),
            ("crop", ph::CROP, tr!("editors-image-crop"), true, crop_mode),
        ];
        for (i, (key, glyph, tip, enabled, on)) in items.into_iter().enumerate() {
            if matches!(i, 3 | 7 | 11) {
                x += 8.0;
            }
            let b = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(26.0, 26.0));
            x += 28.0;
            if on {
                ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
            }
            if widgets::icon_at(ui, b, id.with(("im", key)), glyph, tip, enabled, p) {
                clicked.push(key);
            }
        }
        if self.crop.is_some() {
            for (label, key) in [(tr!("editors-image-apply-crop"), "apply"), (tr!("common-cancel"), "cancel")] {
                let g = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), p.text_primary);
                let b = Rect::from_min_size(pos2(x + 4.0, r.top() + 5.0), vec2(g.size().x + 18.0, 24.0));
                x = b.right() + 2.0;
                let resp = ui.interact(b, id.with(("imb", key)), Sense::click());
                crate::ui::a11y::button(&resp, label);
                let primary = key == "apply";
                let fill = match (primary, resp.hovered()) {
                    (true, true) => p.accent_hover,
                    (true, false) => p.accent,
                    (false, true) => p.bg_surface3,
                    (false, false) => p.bg_surface2,
                };
                ui.painter().rect_filled(b, CornerRadius::same(4), fill);
                let fg = if primary { Color32::WHITE } else { p.text_primary };
                ui.painter().galley(pos2(b.left() + 9.0, b.center().y - g.size().y / 2.0), g, fg);
                if resp.clicked() {
                    clicked.push(key);
                }
            }
        }
        out.save |= clicked.contains(&"save");
        for k in clicked {
            self.command(k);
        }
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }

    fn command(&mut self, key: &str) {
        let scale = self.last_scale;
        match key {
            "undo" => self.undo(),
            "redo" => self.redo(),
            "fit" => {
                self.zoom = None;
                self.pan = Vec2::ZERO;
            }
            "100" => {
                self.zoom = Some(1.0);
                self.pan = Vec2::ZERO;
            }
            "out" => self.zoom = Some((scale / 1.25).max(0.02)),
            "in" => self.zoom = Some((scale * 1.25).min(32.0)),
            "rl" => self.push(ImgOp::RotateCcw),
            "rr" => self.push(ImgOp::RotateCw),
            "fh" => self.push(ImgOp::FlipH),
            "fv" => self.push(ImgOp::FlipV),
            "crop" => {
                self.crop_mode = !self.crop_mode;
                if !self.crop_mode {
                    self.crop = None;
                }
            }
            "apply" => self.apply_crop(),
            "cancel" => {
                self.crop = None;
                self.crop_mode = false;
            }
            _ => {}
        }
    }

    fn status(&self, ui: &mut Ui, r: Rect, p: &Palette) {
        ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
        let (w, h) = self.size();
        let fmt =
            self.format.map(|f| format!("{f:?}").to_uppercase()).unwrap_or_else(|| tr!("editors-image-kind").into());
        let mut s = tr!(
            "editors-image-status",
            width = w.to_string(),
            height = h.to_string(),
            format = fmt,
            zoom = format!("{:.0}", self.last_scale * 100.0)
        );
        if let Some(c) = self.crop {
            let (_, _, cw, ch) = c.to_pixels(w, h);
            s.push_str("  ·  ");
            s.push_str(&tr!("editors-image-status-crop", width = cw.to_string(), height = ch.to_string()));
        }
        if !self.ops.is_empty() {
            s.push_str("  ·  ");
            s.push_str(&tr!("editors-image-status-edits", count = self.ops.len()));
        }
        ui.painter().text(
            pos2(r.left() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            s,
            FontId::proportional(11.5),
            p.text_secondary,
        );
        let mode = match self.save_mode() {
            SaveMode::Lossless(_) => tr!("editors-image-mode-lossless"),
            SaveMode::Lossy(_) => tr!("editors-image-mode-lossy"),
            SaveMode::Unsupported => tr!("editors-image-mode-readonly"),
        };
        ui.painter().text(
            pos2(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            mode,
            FontId::proportional(11.5),
            p.text_tertiary,
        );
    }

    fn view(&mut self, ui: &mut Ui, area: Rect, p: &Palette, focus: &mut Vec<Id>, id: Id) {
        let vid = id.with("image.view");
        focus.push(vid);
        let resp = ui.interact(area, vid, Sense::click_and_drag());
        let just_focused = std::mem::take(&mut self.focus_request) && !resp.has_focus();
        if just_focused {
            resp.request_focus();
        }
        let (w, h) = self.size();
        let (w, h) = (w.max(1) as f32, h.max(1) as f32);
        let fit = ((area.width() - 24.0) / w).min((area.height() - 24.0) / h).clamp(0.01, 1.0);
        let mut scale = self.zoom.unwrap_or(fit);
        // Keys.
        if resp.has_focus() && !just_focused {
            let events = ui.input(|i| i.events.clone());
            for ev in events {
                if let Event::Key { key, pressed: true, modifiers: m, .. } = ev {
                    let cmd = match key {
                        Key::Z if m.command && m.shift => "redo",
                        Key::Z if m.command => "undo",
                        Key::Y if m.command => "redo",
                        _ if m.command || m.alt => "",
                        Key::Plus | Key::Equals => "in",
                        Key::Minus => "out",
                        Key::Num0 => "fit",
                        Key::Num1 => "100",
                        Key::R if m.shift => "rl",
                        Key::R => "rr",
                        Key::H => "fh",
                        Key::V => "fv",
                        Key::C => "crop",
                        Key::Enter => "apply",
                        _ => "",
                    };
                    self.last_scale = scale;
                    self.command(cmd);
                    scale = self.zoom.unwrap_or(fit);
                }
            }
        }
        // Wheel zoom around the pointer.
        if resp.hovered() {
            let (d, ptr) = ui.input_mut(|i| (std::mem::take(&mut i.smooth_scroll_delta), i.pointer.hover_pos()));
            if d.y != 0.0 {
                let factor = (d.y / 200.0).exp();
                let new = (scale * factor).clamp(0.02, 32.0);
                if let Some(ptr) = ptr {
                    let c = area.center() + self.pan;
                    let rel = ptr - c;
                    self.pan += rel - rel * (new / scale);
                }
                scale = new;
                self.zoom = Some(new);
            }
        }
        self.last_scale = scale;
        let size = vec2(w * scale, h * scale);
        let mut center = area.center() + self.pan;
        // Pan (drag) or crop.
        let img_rect = Rect::from_center_size(center, size);
        let to_norm = |pt: egui::Pos2| ((pt.x - img_rect.left()) / size.x, (pt.y - img_rect.top()) / size.y);
        if self.crop_mode {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
            if resp.drag_started()
                && let Some(pt) = resp.interact_pointer_pos()
            {
                self.crop_drag = Some(pt);
            }
            if resp.dragged()
                && let (Some(a), Some(b)) = (self.crop_drag, resp.interact_pointer_pos())
            {
                let (x0, y0) = to_norm(a);
                let (x1, y1) = to_norm(b);
                self.crop = Some(NormRect { x0, y0, x1, y1 }.normalized());
            }
            if !resp.dragged() {
                self.crop_drag = None;
            }
        } else if resp.dragged() {
            self.pan += resp.drag_delta();
            center = area.center() + self.pan;
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if resp.hovered() && (size.x > area.width() || size.y > area.height()) {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if resp.double_clicked() && !self.crop_mode {
            self.zoom = if self.zoom.is_none() { Some(1.0) } else { None };
            self.pan = Vec2::ZERO;
        }
        let img_rect = Rect::from_center_size(center, size);
        // Paint.
        let painter = ui.painter_at(area);
        painter.rect_filled(area, CornerRadius::ZERO, p.bg_sunken.gamma_multiply(if p.dark { 0.55 } else { 0.35 }));
        let vis = img_rect.intersect(area);
        if vis.is_positive() {
            // Checkerboard under transparent pixels (visible part only).
            let cell = 12.0;
            let (c1, c2) = if p.dark {
                (Color32::from_gray(52), Color32::from_gray(64))
            } else {
                (Color32::from_gray(236), Color32::from_gray(250))
            };
            painter.rect_filled(vis, CornerRadius::ZERO, c1);
            let x0 = ((vis.left() - img_rect.left()) / cell).floor() as i64;
            let y0 = ((vis.top() - img_rect.top()) / cell).floor() as i64;
            let x1 = ((vis.right() - img_rect.left()) / cell).ceil() as i64;
            let y1 = ((vis.bottom() - img_rect.top()) / cell).ceil() as i64;
            if (x1 - x0) * (y1 - y0) < 20_000 {
                for gy in y0..y1 {
                    for gx in x0..x1 {
                        if (gx + gy) % 2 == 0 {
                            let r = Rect::from_min_size(
                                pos2(img_rect.left() + gx as f32 * cell, img_rect.top() + gy as f32 * cell),
                                vec2(cell, cell),
                            )
                            .intersect(vis);
                            if r.is_positive() {
                                painter.rect_filled(r, CornerRadius::ZERO, c2);
                            }
                        }
                    }
                }
            }
        }
        match &self.texture {
            Some(t) if self.shown_generation == self.generation || self.pending.is_none() => {
                painter.image(t.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            Some(t) => {
                // The edited copy is on its way: keep the previous one, dimmed.
                painter.image(
                    t.id(),
                    img_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::from_gray(160),
                );
                crate::ui::widgets::spinner(ui, area.center(), 10.0, p.accent);
            }
            None => crate::ui::widgets::spinner(ui, area.center(), 12.0, p.accent),
        }
        painter.rect_stroke(img_rect, CornerRadius::ZERO, Stroke::new(1.0, p.border_subtle), egui::StrokeKind::Outside);
        if let Some(c) = self.crop {
            let r = Rect::from_min_max(
                pos2(img_rect.left() + c.x0 * size.x, img_rect.top() + c.y0 * size.y),
                pos2(img_rect.left() + c.x1 * size.x, img_rect.top() + c.y1 * size.y),
            );
            let shade = Color32::from_black_alpha(140);
            for s in [
                Rect::from_min_max(img_rect.min, pos2(img_rect.right(), r.top())),
                Rect::from_min_max(pos2(img_rect.left(), r.bottom()), img_rect.max),
                Rect::from_min_max(pos2(img_rect.left(), r.top()), pos2(r.left(), r.bottom())),
                Rect::from_min_max(pos2(r.right(), r.top()), pos2(img_rect.right(), r.bottom())),
            ] {
                painter.rect_filled(s, CornerRadius::ZERO, shade);
            }
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, p.accent), egui::StrokeKind::Outside);
            let (iw, ih) = self.size();
            let (_, _, cw, ch) = c.to_pixels(iw, ih);
            painter.text(
                pos2(r.left() + 6.0, r.top() + 6.0),
                Align2::LEFT_TOP,
                format!("{cw} × {ch}"),
                FontId::proportional(11.5),
                Color32::WHITE,
            );
        } else if self.crop_mode {
            painter.text(
                pos2(area.center().x, area.top() + 16.0),
                Align2::CENTER_CENTER,
                tr!("editors-image-crop-hint"),
                FontId::proportional(12.0),
                p.text_secondary,
            );
        }
    }
}
