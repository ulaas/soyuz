//! Toolkit-neutral graphics made by the preview and thumbnail workers: uploaded
//! textures behind a type-erased handle, colours as premultiplied sRGBA bytes, styled
//! text lines and flat-shaded triangles. The frontend turns them into its own types
//! (the egui side lives in `ui::gfx`); nothing here depends on a GUI toolkit.
//!
//! Textures are still uploaded on the worker threads: the frontend injects an
//! [`Uploader`] that makes its native texture and returns it as an opaque handle, so the
//! UI thread keeps only looking textures up (same frame budget as before).

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

/// A colour as premultiplied sRGBA bytes (the same layout and meaning as egui's
/// `Color32`, so conversions are bit-identical).
pub type Rgba = [u8; 4];

/// An opaque colour from sRGB components.
pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    [r, g, b, 255]
}

/// Opaque white.
pub const WHITE: Rgba = [255, 255, 255, 255];
/// Opaque black.
pub const BLACK: Rgba = [0, 0, 0, 255];

/// RGBA8 pixels handed to an [`Uploader`].
#[derive(Clone, Copy, Debug)]
pub struct Pixels<'a> {
    /// `[width, height]`.
    pub size: [usize; 2],
    /// `width * height * 4` bytes, row-major.
    pub rgba: &'a [u8],
    /// The colour channels are already multiplied by alpha.
    pub premultiplied: bool,
}

/// The frontend's texture maker: `(debug name, pixels) -> native texture handle`. Called
/// on worker threads.
pub type Uploader = Arc<dyn Fn(String, Pixels<'_>) -> Arc<dyn Any + Send + Sync> + Send + Sync>;

/// An uploader that keeps nothing (tests and frontends without textures).
pub fn no_upload() -> Uploader {
    Arc::new(|_, _| Arc::new(()))
}

/// An uploaded texture: the frontend's handle plus the pixel size. Dropping the last
/// clone frees it (the handle owns the native texture).
#[derive(Clone)]
pub struct Tex {
    handle: Arc<dyn Any + Send + Sync>,
    /// `[width, height]` in pixels.
    pub size: [usize; 2],
}

impl Tex {
    /// Upload `pixels` through `up`.
    pub fn upload(up: &Uploader, name: String, pixels: Pixels<'_>) -> Tex {
        let size = pixels.size;
        Tex { handle: up(name, pixels), size }
    }

    /// Upload straight (not premultiplied) RGBA pixels.
    pub fn upload_rgba(up: &Uploader, name: String, img: &image::RgbaImage) -> Tex {
        let size = [img.width() as usize, img.height() as usize];
        Tex::upload(up, name, Pixels { size, rgba: img.as_raw(), premultiplied: false })
    }

    /// The frontend's native handle, if it is a `T`.
    pub fn handle<T: Any>(&self) -> Option<&T> {
        self.handle.downcast_ref::<T>()
    }

    /// Texture memory in bytes (RGBA8).
    pub fn bytes(&self) -> u64 {
        (self.size[0] * self.size[1] * 4) as u64
    }
}

impl std::fmt::Debug for Tex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tex").field("size", &self.size).finish_non_exhaustive()
    }
}

// ---- styled text --------------------------------------------------------------------

/// Which font family a run wants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontHint {
    #[default]
    Proportional,
    Monospace,
}

/// A font request: family and size in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFont {
    pub size: f32,
    pub family: FontHint,
}

impl TextFont {
    pub const fn monospace(size: f32) -> Self {
        TextFont { size, family: FontHint::Monospace }
    }
    pub const fn proportional(size: f32) -> Self {
        TextFont { size, family: FontHint::Proportional }
    }
}

/// How one run of a [`StyledLine`] looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunStyle {
    pub font: TextFont,
    pub color: Rgba,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl RunStyle {
    /// Plain text in `font` and `color`.
    pub const fn plain(font: TextFont, color: Rgba) -> Self {
        RunStyle { font, color, bold: false, italic: false, underline: false }
    }
}

/// One styled run: a byte range of the line's text.
#[derive(Clone, Debug, PartialEq)]
pub struct StyledRun {
    pub range: Range<usize>,
    pub style: RunStyle,
}

/// One line of styled text (a single row, never wrapped).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyledLine {
    pub text: String,
    pub runs: Vec<StyledRun>,
}

impl StyledLine {
    /// Append `text` in `style`: it extends the last run when the style is the same,
    /// else starts a new one (an empty `text` still adds an empty run, which sets the
    /// font of an empty line).
    pub fn append(&mut self, text: &str, style: RunStyle) {
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        match self.runs.last_mut() {
            Some(last) if last.style == style => last.range.end = end,
            _ => self.runs.push(StyledRun { range: start..end, style }),
        }
    }
}

// ---- flat-shaded triangles ----------------------------------------------------------

/// Receives projected, shaded triangles back to front (3D model previews). The GUI
/// writes them straight into its mesh type; [`TriList`] collects them neutrally.
pub trait TriSink {
    /// About `tris` triangles follow.
    fn reserve(&mut self, tris: usize);
    /// One triangle in screen coordinates (points) with a flat colour.
    fn tri(&mut self, pts: [[f32; 2]; 3], color: Rgba);
}

/// A plain list of triangles.
#[derive(Clone, Debug, Default)]
pub struct TriList(pub Vec<([[f32; 2]; 3], Rgba)>);

impl TriSink for TriList {
    fn reserve(&mut self, tris: usize) {
        self.0.reserve(tris);
    }
    fn tri(&mut self, pts: [[f32; 2]; 3], color: Rgba) {
        self.0.push((pts, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styled_lines_track_byte_ranges() {
        let mut l = StyledLine::default();
        let s = RunStyle::plain(TextFont::monospace(12.0), WHITE);
        l.append("", s);
        assert_eq!(l.runs.len(), 1, "an empty line keeps its font");
        l.append("fn", s);
        l.append(" ünï", RunStyle { italic: true, ..s });
        l.append("!", RunStyle { italic: true, ..s });
        assert_eq!(l.text, "fn ünï!");
        let ranges: Vec<_> = l.runs.iter().map(|r| r.range.clone()).collect();
        assert_eq!(ranges, vec![0..2, 2..l.text.len()], "same-style runs merge");
    }

    #[test]
    fn textures_keep_size_and_handle() {
        let up: Uploader = Arc::new(|name, px| Arc::new((name, px.size, px.premultiplied)));
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let t = Tex::upload_rgba(&up, "x".into(), &img);
        assert_eq!(t.size, [3, 2]);
        assert_eq!(t.bytes(), 24);
        assert_eq!(t.handle::<(String, [usize; 2], bool)>(), Some(&("x".to_string(), [3, 2], false)));
        assert!(t.handle::<u32>().is_none());
        assert_eq!(Tex::upload_rgba(&no_upload(), "y".into(), &img).size, [3, 2]);
    }
}
