//! The PDF engine: hayro (pure Rust, always built in) with pdfium as an optional
//! alternative (cargo feature `pdf`, loaded at run time).
//!
//! * One-shot calls for the preview pane, Quick Look, thumbnails and content search:
//!   [`open_info`], [`render_page_fit`], [`page_text`], [`document_text`]. They block, so
//!   call them on a worker; each runs under [`guard::run_guarded`] (own thread, panic
//!   catch, time budget).
//! * [`session::PdfSession`] for the viewer: per-document worker threads that render
//!   regions / page bitmaps, extract text and search, reporting through events.
//!
//! Only [`hayro_backend`], [`nav`] and the font hook in `hayro_backend` touch hayro's
//! types; everything else here is engine-neutral data (pixels, page points, text boxes).
//! Coordinates are **rendered page points**: origin top-left, y down, the page's own
//! `/Rotate` applied, 1 pt = 1/72 in.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub mod fonts;
pub mod guard;
pub(crate) mod hayro_backend;
pub mod nav;
#[cfg(feature = "pdf")]
pub mod pdfium_backend;
pub mod session;
pub mod text;
pub mod tiles;

#[cfg(test)]
pub(crate) mod fixtures;

pub use nav::{Dest, Link, LinkTarget, Outline};
pub use session::{Event, Job, PdfSession, SessionCfg, prio};
pub use text::{CharBox, Match, PageText, Query};
pub use tiles::{PageKey, TILE, TexCache, TileKey};

use super::preview_pdf::{self, PdfInfo};

/// Largest side of a page bitmap ([`render_page_fit`], session base layers).
pub const MAX_PAGE_PX: u32 = 8192;
/// Largest side of one region ([`Region`]).
pub const MAX_REGION_PX: u32 = 2048;

/// Where the PDF bytes come from.
#[derive(Clone, Debug)]
pub enum PdfSrc {
    File(PathBuf),
    Mem(Arc<[u8]>),
}

impl PdfSrc {
    /// The bytes, read whole (at most `cap` bytes for a file).
    pub(crate) fn bytes(&self, cap: u64) -> Result<Vec<u8>, String> {
        match self {
            PdfSrc::File(p) => {
                let len = std::fs::metadata(p).map_err(|e| e.to_string())?.len();
                if len > cap {
                    return Err(tr!("services-pdf-over-limit", limit = (cap >> 20).to_string()));
                }
                std::fs::read(p).map_err(|e| e.to_string())
            }
            PdfSrc::Mem(b) => Ok(b.to_vec()),
        }
    }

    /// The bytes as one shared buffer (no copy for [`PdfSrc::Mem`]). File I/O: workers only.
    pub(crate) fn shared(&self, cap: u64) -> Result<Arc<[u8]>, PdfError> {
        match self {
            PdfSrc::File(p) => {
                let len = std::fs::metadata(p).map_err(|e| PdfError::Other(e.to_string()))?.len();
                if len > cap {
                    return Err(PdfError::TooLarge);
                }
                std::fs::read(p).map(Arc::from).map_err(|e| PdfError::Other(e.to_string()))
            }
            PdfSrc::Mem(b) => {
                if b.len() as u64 > cap {
                    return Err(PdfError::TooLarge);
                }
                Ok(b.clone())
            }
        }
    }

    /// The password typed for this file this session ([`preview_pdf::password_for`]).
    pub fn password(&self) -> Option<String> {
        match self {
            PdfSrc::File(p) => preview_pdf::password_for(p),
            PdfSrc::Mem(_) => None,
        }
    }
}

/// Which renderer a document goes through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EngineKind {
    #[default]
    Hayro,
    Pdfium,
}

impl EngineKind {
    /// Short name for the status bar and the preview's "Renderer" row.
    pub fn label(self) -> &'static str {
        match self {
            EngineKind::Hayro => "hayro",
            EngineKind::Pdfium => "pdfium",
        }
    }

    /// Tag for thumbnail cache keys (`thumb_disk::cache_key_with_source`): a renderer
    /// change re-renders cached thumbnails.
    pub fn cache_tag(self) -> &'static str {
        match self {
            EngineKind::Hayro => "hayro-0.9",
            EngineKind::Pdfium => "pdfium",
        }
    }
}

/// A picked engine plus what the one-shot calls need to run it.
#[derive(Clone, Debug)]
pub struct Engine {
    pub kind: EngineKind,
    /// Folders searched for the pdfium library ([`preview_pdf::pdfium_dirs`]).
    pub pdfium_dirs: Vec<PathBuf>,
    /// File size cap in bytes (`[preview] max_document_mib`).
    pub max_bytes: u64,
    /// Time budget of one call.
    pub timeout: Duration,
}

impl Engine {
    /// hayro with the given limits.
    pub fn hayro(max_bytes: u64, timeout: Duration) -> Engine {
        Engine { kind: EngineKind::Hayro, pdfium_dirs: Vec::new(), max_bytes, timeout }
    }
}

/// The engine to use: pdfium only when `want` is pdfium, the `pdf` feature is built and
/// the library is found in `pdfium_dirs`; hayro otherwise. Probes the file system:
/// call on a worker, never on the UI thread.
pub fn pick(want: EngineKind, pdfium_dirs: Vec<PathBuf>, max_bytes: u64, timeout: Duration) -> Engine {
    let kind = if want == EngineKind::Pdfium && cfg!(feature = "pdf") && preview_pdf::pdfium_available(&pdfium_dirs) {
        EngineKind::Pdfium
    } else {
        EngineKind::Hayro
    };
    Engine { kind, pdfium_dirs, max_bytes, timeout }
}

/// Why a PDF (or one of its pages) could not be read or rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// Encrypted: no password typed (`wrong == false`) or a wrong one.
    Password {
        wrong: bool,
    },
    /// Not a PDF, or damaged beyond repair.
    Invalid,
    /// Over the size cap.
    TooLarge,
    /// The render / extraction ran past its time budget and was abandoned.
    TimedOut,
    /// The engine panicked (caught).
    Panicked,
    /// The caller stopped waiting.
    Stale,
    Other(String),
}

impl PdfError {
    /// Needs a (different) password.
    pub fn is_password(&self) -> bool {
        matches!(self, PdfError::Password { .. })
    }
}

impl std::fmt::Display for PdfError {
    /// Same texts as the existing PDF messages, so [`preview_pdf::is_password_error`]
    /// recognizes the password cases.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfError::Password { wrong: false } => f.write_str(tr!("services-pdf-password")),
            PdfError::Password { wrong: true } => f.write_str(tr!("services-pdf-password-wrong")),
            PdfError::Invalid => f.write_str(tr!("services-pdf-parser-failed")),
            PdfError::TooLarge => f.write_str(tr!("services-pdf-too-large")),
            PdfError::TimedOut => f.write_str(tr!("services-pdf-timed-out")),
            PdfError::Panicked => f.write_str(tr!("services-pdf-renderer-failed")),
            PdfError::Stale => f.write_str("stale"),
            PdfError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for PdfError {}

/// Size of one page in rendered points (`/Rotate` applied, crop box).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageGeom {
    pub w_pt: f32,
    pub h_pt: f32,
    /// The page's own `/Rotate` in degrees (0, 90, 180, 270), already in `w_pt` / `h_pt`.
    pub rotation: u16,
}

impl PageGeom {
    /// Size in points with an extra view rotation of `rot_quarter` × 90° clockwise.
    pub fn rotated(&self, rot_quarter: u8) -> (f32, f32) {
        if rot_quarter % 2 == 1 { (self.h_pt, self.w_pt) } else { (self.w_pt, self.h_pt) }
    }

    /// Pixel size at `scale` (pixels per point) and view rotation: truncated like hayro's
    /// own full-page render (with a little slack for float error), at least 1.
    pub fn px(&self, scale: f32, rot_quarter: u8) -> (u32, u32) {
        let (w, h) = self.rotated(rot_quarter);
        let side = |v: f32| ((v * scale + 0.01) as u32).max(1);
        (side(w), side(h))
    }

    /// The scale that fits the page (rotated) into `max_w` × `max_h` pixels, capped so
    /// neither side passes [`MAX_PAGE_PX`].
    pub fn fit_scale(&self, max_w: u32, max_h: u32, rot_quarter: u8) -> f32 {
        let (w, h) = self.rotated(rot_quarter);
        let (w, h) = (w.max(1.0), h.max(1.0));
        let s = (max_w.max(1) as f32 / w).min(max_h.max(1) as f32 / h);
        s.min(MAX_PAGE_PX as f32 / w).min(MAX_PAGE_PX as f32 / h).max(0.001)
    }
}

/// Document facts for the preview, the viewer's status bar and Properties.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub pages: u32,
    /// `1.7`.
    pub version: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    /// `2024-01-31 12:00`.
    pub created: Option<String>,
    pub modified: Option<String>,
    pub encrypted: bool,
    pub first_page: Option<PageGeom>,
    pub engine: EngineKind,
}

impl DocInfo {
    /// The preview's [`PdfInfo`] shape, with `text` as its text.
    pub fn to_pdf_info(&self, text: String) -> PdfInfo {
        PdfInfo {
            pages: self.pages,
            version: self.version.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            producer: self.producer.clone().or_else(|| self.creator.clone()),
            created: self.created.clone(),
            encrypted: self.encrypted,
            page_size: self.first_page.map(|g| (g.w_pt, g.h_pt)),
            text,
        }
    }
}

/// Rendering switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderOpts {
    /// Draw annotation appearances (form fields, stamps, highlights).
    pub annotations: bool,
    /// Invert lightness (dark pages), keeping hues.
    pub invert: bool,
    /// Use installed fonts for fonts the PDF does not embed (CJK etc.).
    pub system_fonts: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts { annotations: true, invert: false, system_fonts: true }
    }
}

/// A pixel rectangle of a page rendered at `scale` pixels per point and turned
/// `rot_quarter` × 90° clockwise on top of the page's own `/Rotate`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub page: u32,
    pub scale: f32,
    pub rot_quarter: u8,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Rendered pixels: `w * h * 4` bytes of premultiplied RGBA8 (opaque: pages render on
/// white, so premultiplied and straight are the same bytes).
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bitmap").field("w", &self.w).field("h", &self.h).finish_non_exhaustive()
    }
}

/// Invert lightness in place (HSL `L → 1 − L`), keeping hue and saturation: black text
/// on white turns light on dark while colours stay recognisable. Alpha is kept.
pub fn invert_lightness(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
        let shift = 255 - r.max(g).max(b) - r.min(g).min(b);
        px[0] = (r + shift).clamp(0, 255) as u8;
        px[1] = (g + shift).clamp(0, 255) as u8;
        px[2] = (b + shift).clamp(0, 255) as u8;
    }
}

/// Page count, metadata and first page size. pdfium when `engine` says so (falling back
/// to hayro when pdfium fails for a reason other than a password).
pub fn open_info(src: &PdfSrc, engine: &Engine, stale: &dyn Fn() -> bool) -> Result<DocInfo, PdfError> {
    #[cfg(feature = "pdf")]
    if engine.kind == EngineKind::Pdfium {
        match pdfium_backend::info_pages(src, &engine.pdfium_dirs, engine.max_bytes, stale, 0, 0) {
            Ok(i) => {
                return Ok(DocInfo {
                    pages: i.pages,
                    version: i.version,
                    title: i.title,
                    author: i.author,
                    producer: i.producer,
                    created: i.created,
                    encrypted: i.encrypted,
                    first_page: i.page_size.map(|(w, h)| PageGeom { w_pt: w, h_pt: h, rotation: 0 }),
                    engine: EngineKind::Pdfium,
                    ..Default::default()
                });
            }
            Err(e) if preview_pdf::is_password_error(&e) => return Err(pdfium_error(&e)),
            Err(e) if e == "stale" => return Err(PdfError::Stale),
            Err(e) => log::debug!("pdfium info failed, using hayro: {e}"),
        }
    }
    let (src, cap) = (src.clone(), engine.max_bytes);
    guard::run_guarded(engine.timeout, stale, move || {
        let bytes = src.shared(cap)?;
        let pdf = hayro_backend::load(bytes.clone(), src.password().as_deref())?;
        Ok(hayro_backend::info(&pdf, &bytes))
    })?
}

/// Render page `page` (0-based) as large as fits `max_w` × `max_h` pixels (each side at
/// most [`MAX_PAGE_PX`]), on white. Falls back from pdfium to hayro like [`open_info`].
pub fn render_page_fit(
    src: &PdfSrc,
    engine: &Engine,
    page: u32,
    max_w: u32,
    max_h: u32,
    opts: RenderOpts,
    stale: &dyn Fn() -> bool,
) -> Result<Bitmap, PdfError> {
    #[cfg(feature = "pdf")]
    if engine.kind == EngineKind::Pdfium {
        let (w, h) = (max_w.min(MAX_PAGE_PX), max_h.min(MAX_PAGE_PX));
        match pdfium_backend::render(src, &engine.pdfium_dirs, engine.max_bytes, page, w, h, stale) {
            Ok(img) => {
                let mut rgba = img.rgba;
                if opts.invert {
                    invert_lightness(&mut rgba);
                }
                return Ok(Bitmap { w: img.width as u32, h: img.height as u32, rgba });
            }
            Err(e) if preview_pdf::is_password_error(&e) => return Err(pdfium_error(&e)),
            Err(e) if e == "stale" => return Err(PdfError::Stale),
            Err(e) => log::debug!("pdfium render failed, using hayro: {e}"),
        }
    }
    let (src, cap) = (src.clone(), engine.max_bytes);
    guard::run_guarded(engine.timeout, stale, move || {
        let bytes = src.shared(cap)?;
        let pdf = hayro_backend::load(bytes, src.password().as_deref())?;
        hayro_backend::render_fit(&pdf, page, max_w, max_h, 0, &opts)
    })?
}

/// Positioned text of one page (always hayro: pdfium's text has no char boxes here).
pub fn page_text(
    src: &PdfSrc,
    engine: &Engine,
    page: u32,
    stale: &dyn Fn() -> bool,
) -> Result<Arc<PageText>, PdfError> {
    let (src, cap) = (src.clone(), engine.max_bytes);
    guard::run_guarded(engine.timeout, stale, move || {
        let bytes = src.shared(cap)?;
        let pdf = hayro_backend::load(bytes, src.password().as_deref())?;
        hayro_backend::page_text(&pdf, page).map(Arc::new)
    })?
}

/// Plain text of the first `max_pages` pages, at most `max_chars` characters (blank-line
/// runs collapsed), for content search and the index. pdfium when picked and loadable,
/// else hayro; `budget` caps the whole call ([`PdfError::TimedOut`] past it, so the
/// caller can fall back to lopdf).
pub fn document_text(
    src: &PdfSrc,
    engine: &Engine,
    max_pages: u32,
    max_chars: usize,
    budget: Duration,
) -> Result<String, PdfError> {
    #[cfg(feature = "pdf")]
    if engine.kind == EngineKind::Pdfium {
        match pdfium_backend::info_pages(src, &engine.pdfium_dirs, engine.max_bytes, &|| false, max_pages, max_chars) {
            Ok(i) => return Ok(i.text),
            Err(e) if preview_pdf::is_password_error(&e) => return Err(pdfium_error(&e)),
            Err(e) => log::debug!("pdfium text failed, using hayro: {e}"),
        }
    }
    let (src, cap) = (src.clone(), engine.max_bytes);
    guard::run_guarded(budget, &|| false, move || {
        let bytes = src.shared(cap)?;
        let password = src.password();
        hayro_backend::document_text(&bytes, password.as_deref(), max_pages, max_chars)
    })?
}

#[cfg(feature = "pdf")]
fn pdfium_error(msg: &str) -> PdfError {
    let msg = msg.trim_start_matches("pdfium: ");
    if msg == tr!("services-pdf-password-wrong") {
        PdfError::Password { wrong: true }
    } else if msg == tr!("services-pdf-password") {
        PdfError::Password { wrong: false }
    } else {
        PdfError::Other(msg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_geom_maths() {
        let g = PageGeom { w_pt: 612.0, h_pt: 792.0, rotation: 0 };
        assert_eq!(g.px(1.0, 0), (612, 792));
        assert_eq!(g.px(2.0, 1), (1584, 1224));
        assert_eq!(g.rotated(3), (792.0, 612.0));
        let s = g.fit_scale(306, 10_000, 0);
        assert!((s - 0.5).abs() < 1e-6);
        assert!(g.fit_scale(100_000, 100_000, 0) * 792.0 <= MAX_PAGE_PX as f32 + 0.5, "capped");
    }

    #[test]
    fn lightness_inversion_keeps_hue() {
        let mut px = [255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 0, 255, 200, 220, 255, 255];
        invert_lightness(&mut px);
        assert_eq!(&px[0..4], &[0, 0, 0, 255], "white → black");
        assert_eq!(&px[4..8], &[255, 255, 255, 255], "black → white");
        assert_eq!(&px[8..12], &[255, 0, 0, 255], "pure red stays red (L = 0.5)");
        assert!(px[12] < px[14] && px[13] < px[14], "light blue stays blue: {:?}", &px[12..16]);
        assert!(px[14] < 120, "and turns dark: {:?}", &px[12..16]);
    }

    #[test]
    fn errors_read_like_the_preview_messages() {
        assert!(preview_pdf::is_password_error(&PdfError::Password { wrong: false }.to_string()));
        assert!(preview_pdf::is_password_error(&PdfError::Password { wrong: true }.to_string()));
        assert!(!preview_pdf::is_password_error(&PdfError::Invalid.to_string()));
        assert_eq!(EngineKind::Hayro.cache_tag(), "hayro-0.9");
    }

    #[test]
    fn pick_falls_back_to_hayro() {
        let e = pick(EngineKind::Pdfium, vec![PathBuf::from("Z:/no/pdfium/here")], 1 << 20, Duration::from_secs(5));
        if !cfg!(target_os = "linux") {
            assert_eq!(e.kind, EngineKind::Hayro);
        }
        assert_eq!(pick(EngineKind::Hayro, Vec::new(), 1, Duration::from_secs(1)).kind, EngineKind::Hayro);
    }

    #[test]
    fn one_shot_calls_render_and_read_text() {
        let src = PdfSrc::Mem(fixtures::text_pages().into());
        let eng = Engine::hayro(1 << 24, Duration::from_secs(20));
        let info = open_info(&src, &eng, &|| false).unwrap();
        assert_eq!(info.pages, 3);
        assert_eq!(info.title.as_deref(), Some("Fixture doc"));
        assert_eq!(info.created.as_deref(), Some("2024-01-31 12:00"));
        assert_eq!(info.first_page.map(|g| (g.w_pt, g.h_pt)), Some((612.0, 792.0)));
        assert_eq!(info.to_pdf_info(String::new()).page_size, Some((612.0, 792.0)));
        let bmp = render_page_fit(&src, &eng, 0, 306, 1000, RenderOpts::default(), &|| false).unwrap();
        assert_eq!((bmp.w, bmp.h), (306, 396));
        assert_eq!(bmp.rgba.len(), 306 * 396 * 4);
        let t = page_text(&src, &eng, 1, &|| false).unwrap();
        assert!(t.text.contains("Second page"), "{:?}", t.text);
        let all = document_text(&src, &eng, 10, 10_000, Duration::from_secs(20)).unwrap();
        assert!(all.contains("Hello world") && all.contains("Third page"), "{all:?}");
        assert!(render_page_fit(&src, &eng, 99, 100, 100, RenderOpts::default(), &|| false).is_err());
        let bad = PdfSrc::Mem(b"%PDF-1.4 nothing".to_vec().into());
        assert_eq!(open_info(&bad, &eng, &|| false).unwrap_err(), PdfError::Invalid);
        assert_eq!(
            open_info(&src, &Engine::hayro(10, Duration::from_secs(5)), &|| false).unwrap_err(),
            PdfError::TooLarge
        );
    }

    #[test]
    fn encrypted_none_wrong_right() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("locked.pdf");
        std::fs::write(&path, fixtures::encrypted("secret")).unwrap();
        let src = PdfSrc::File(path.clone());
        let eng = Engine::hayro(1 << 24, Duration::from_secs(20));
        assert_eq!(open_info(&src, &eng, &|| false).unwrap_err(), PdfError::Password { wrong: false });
        preview_pdf::set_password(&path, "nope".into());
        assert_eq!(open_info(&src, &eng, &|| false).unwrap_err(), PdfError::Password { wrong: true });
        preview_pdf::set_password(&path, "secret".into());
        let info = open_info(&src, &eng, &|| false).unwrap();
        assert_eq!(info.pages, 2);
        assert!(info.encrypted);
        let t = page_text(&src, &eng, 0, &|| false).unwrap();
        assert!(t.text.contains("Hello from page one"), "{:?}", t.text);
    }
}
