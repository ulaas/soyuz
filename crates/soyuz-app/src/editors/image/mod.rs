//! The image editor: zoom / pan / fit, rotate 90°, flip and crop with undo. Decoding,
//! the display copy and encoding all run on worker threads; edits are replayed on a
//! downscaled display copy for the screen and on the full image when saving. PNG, BMP,
//! TIFF, WebP (lossless), GIF and ICO are re-encoded without loss; JPEG has no pure-Rust
//! lossless transform, so it is re-encoded at `[editor] jpeg_quality` with a warning.
//! This is the document model (images, edits, zoom / pan / crop state); the display
//! copy is uploaded through the frontend's [`Uploader`] and `ui::editors::image` draws it.

pub mod ops;

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{Receiver, unbounded};
use image::{DynamicImage, ImageFormat};
use soyuz_core::config::KeyName;

use ops::{ImgOp, NormRect, SaveMode};

use super::KeyMods;
use crate::services::mailbox::Wake;
use crate::services::preview_gfx::{Tex, Uploader};

/// Longest side of the on-screen copy (GPU texture limit).
const DISPLAY_MAX: u32 = 8192;

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
    texture: Option<Tex>,
    /// Display copy with the current ops, built (and uploaded) on a worker.
    pending: Option<(u64, Receiver<Tex>)>,
    generation: u64,
    shown_generation: u64,
    wake: Wake,
    upload: Uploader,
    /// `None` = fit to the view.
    pub zoom: Option<f32>,
    /// Offset of the image centre from the view centre, in points.
    pub pan: [f32; 2],
    pub crop_mode: bool,
    pub crop: Option<NormRect>,
    /// Where the crop drag started (view coordinates).
    pub crop_drag: Option<[f32; 2]>,
    /// The frontend should give the image view keyboard focus.
    pub focus_request: bool,
    /// The scale the image was last drawn at (zoom buttons step from it).
    pub last_scale: f32,
}

impl ImageDoc {
    /// `upload` makes the display copy's texture (on the worker).
    pub fn new(path: PathBuf, loaded: Loaded, wake: Wake, upload: Uploader) -> Self {
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
            upload,
            zoom: None,
            pan: [0.0; 2],
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
        let upload = self.upload.clone();
        let name = format!("soyuz-editor-{}", self.path.display());
        let _ = std::thread::Builder::new().name("soyuz-image-edit".into()).spawn(move || {
            let img = ops::replay(&base, &ops).to_rgba8();
            let _ = tx.send(Tex::upload_rgba(&upload, name, &img));
            wake();
        });
        self.pending = Some((self.generation, rx));
    }

    pub fn push(&mut self, op: ImgOp) {
        self.ops.push(op);
        self.redo.clear();
        self.crop = None;
        if matches!(op, ImgOp::RotateCw | ImgOp::RotateCcw | ImgOp::Crop(_)) {
            self.pan = [0.0; 2];
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

    /// Pixel size after the edits.
    pub fn size(&self) -> (u32, u32) {
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

    /// Take the edited display copy when the worker has it (call every frame).
    pub fn pump(&mut self) {
        if let Some((generation, rx)) = &self.pending
            && let Ok(tex) = rx.try_recv()
        {
            self.shown_generation = *generation;
            self.pending = None;
            self.texture = Some(tex);
        }
    }

    /// The display texture and whether it shows the current edits (else the previous
    /// copy is shown dimmed while the new one is built).
    pub fn texture(&self) -> Option<(&Tex, bool)> {
        let current = self.shown_generation == self.generation || self.pending.is_none();
        self.texture.as_ref().map(|t| (t, current))
    }

    pub fn can_undo(&self) -> bool {
        !self.ops.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// A key press while the view has focus, drawn at `scale` (`fit` = the fit-to-view
    /// scale); returns the scale to draw at after it.
    pub fn key(&mut self, key: Option<KeyName>, m: KeyMods, scale: f32, fit: f32) -> f32 {
        let cmd = match key {
            Some(KeyName::Z) if m.command && m.shift => "redo",
            Some(KeyName::Z) if m.command => "undo",
            Some(KeyName::Y) if m.command => "redo",
            _ if m.command || m.alt => "",
            Some(KeyName::Plus | KeyName::Equals) => "in",
            Some(KeyName::Minus) => "out",
            Some(KeyName::Num0) => "fit",
            Some(KeyName::Num1) => "100",
            Some(KeyName::R) if m.shift => "rl",
            Some(KeyName::R) => "rr",
            Some(KeyName::H) => "fh",
            Some(KeyName::V) => "fv",
            Some(KeyName::C) => "crop",
            Some(KeyName::Enter) => "apply",
            _ => "",
        };
        self.last_scale = scale;
        self.command(cmd);
        self.zoom.unwrap_or(fit)
    }

    /// Double click: toggle between fit and 100 %.
    pub fn toggle_fit(&mut self) {
        self.zoom = if self.zoom.is_none() { Some(1.0) } else { None };
        self.pan = [0.0; 2];
    }

    /// A toolbar command: `undo`, `redo`, `fit`, `100`, `out`, `in`, `rl`, `rr`, `fh`,
    /// `fv`, `crop`, `apply` (the crop) or `cancel` (the crop).
    pub fn command(&mut self, key: &str) {
        let scale = self.last_scale;
        match key {
            "undo" => self.undo(),
            "redo" => self.redo(),
            "fit" => {
                self.zoom = None;
                self.pan = [0.0; 2];
            }
            "100" => {
                self.zoom = Some(1.0);
                self.pan = [0.0; 2];
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
}
