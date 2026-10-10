//! Rich previews: the dispatch between the generic preview loader
//! ([`super::preview`]) and the format modules — Office / ODF documents, EPUB, fonts,
//! 3D models, audio (waveform + player), video poster frames (ffmpeg) and PDF (pdfium,
//! with a pure-Rust text fallback).
//!
//! Everything here runs on the preview worker threads: inputs are size-capped by the
//! `[preview]` limits, long loops check the request's staleness, and the caller's
//! `catch_unwind` turns a parser panic into an error card. Results are ready to draw
//! (textures uploaded, models normalized).

use std::io::{Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use soyuz_core::config::Preview as PreviewCfg;

use super::preview::{Content, Kind, Limits, Loaded, ReadSeek, Source};
use super::preview_audio::{self, AudioInfo, AudioSrc};
use super::preview_epub::{self, EpubModel};
use super::preview_font::{self, FontMeta};
use super::preview_media::{self, format_duration};
use super::preview_model::{self, Model};
use super::preview_office::{self, Office, OfficeKind};
use super::preview_pdf::{self, PdfInfo, PdfSrc};
use super::preview_text;
use super::preview_video;
use crate::theme::icons::ph;

/// Per-request switches and caps (from `[preview]`).
#[derive(Clone, Debug, PartialEq)]
pub struct RichLimits {
    pub pdf: bool,
    pub pdfium_dir: String,
    pub office: bool,
    pub epub: bool,
    pub fonts: bool,
    pub models: bool,
    pub waveform: bool,
    pub playback: bool,
    pub ffmpeg: bool,
    pub ffmpeg_path: String,
    pub max_document: u64,
    pub sheet_rows: usize,
    pub max_model: u64,
    pub max_audio: u64,
    pub turntable: bool,
    /// Quick Look (large renders) vs the preview pane.
    pub big: bool,
}

impl RichLimits {
    pub fn from_config(c: &PreviewCfg, big: bool) -> Self {
        RichLimits {
            pdf: c.pdf,
            pdfium_dir: c.pdfium_dir.clone(),
            office: c.office,
            epub: c.epub,
            fonts: c.fonts,
            models: c.models,
            waveform: c.audio_waveform,
            playback: c.audio_playback,
            ffmpeg: c.video_ffmpeg,
            ffmpeg_path: c.ffmpeg_path.clone(),
            max_document: c.max_document_mib.max(1).saturating_mul(1 << 20),
            sheet_rows: c.sheet_rows.clamp(1, 100_000) as usize,
            max_model: c.max_model_mib.max(1).saturating_mul(1 << 20),
            max_audio: c.max_audio_mib.saturating_mul(1 << 20),
            turntable: c.model_turntable,
            big,
        }
    }

    /// Render width in points for page-like content.
    fn width_pt(&self) -> f32 {
        if self.big { 1000.0 } else { 340.0 }
    }
}

impl Default for RichLimits {
    fn default() -> Self {
        RichLimits::from_config(&PreviewCfg::default(), false)
    }
}

/// Which rich previewer handles a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RichKind {
    Office(OfficeKind),
    Epub,
    Font,
    Model,
    Audio,
    Video,
    Pdf,
}

/// An uploaded texture with its pixel size.
#[derive(Clone)]
pub struct Tex {
    pub tex: TextureHandle,
    pub size: [usize; 2],
}

#[derive(Clone)]
pub struct PdfView {
    pub src: PdfSrc,
    pub info: Arc<PdfInfo>,
    /// First page (pdfium only).
    pub first: Option<Tex>,
    /// Why pages can't be rendered (`None` = pdfium works).
    pub no_render: Option<String>,
    pub dirs: Vec<PathBuf>,
    pub cap: u64,
    pub width_px: u32,
}

#[derive(Clone)]
pub enum Rich {
    Office(Arc<Office>),
    Epub { model: Arc<EpubModel>, cover: Option<Tex> },
    Font { meta: Arc<FontMeta>, strips: Arc<Vec<(String, Tex)>> },
    Model { model: Arc<Model>, turntable: bool },
    Audio { info: Arc<AudioInfo>, src: AudioSrc, ext: String, playback: bool, cover: Option<Tex> },
    Video { poster: Option<Tex>, note: Option<String> },
    Pdf(PdfView),
}

impl Rich {
    /// Text-like content that wants most of the pane height.
    pub fn is_textual(&self) -> bool {
        matches!(self, Rich::Office(_) | Rich::Epub { .. } | Rich::Font { .. } | Rich::Pdf(_))
    }
}

const FONT_EXT: &[&str] = &["ttf", "otf", "ttc", "otc", "woff", "woff2"];
const EPUB_EXT: &[&str] = &["epub"];

fn ext_of(name: &str) -> String {
    preview_text::name_ext(&name.to_ascii_lowercase()).map(str::to_string).unwrap_or_default()
}

/// Pick the rich previewer for a file (`None` = the generic preview handles it).
pub fn route(name: &str, head: &[u8], detected: Kind, lim: &RichLimits) -> Option<RichKind> {
    let ext = ext_of(name);
    let e = ext.as_str();
    if let Some(k) = preview_office::kind_for_ext(e) {
        return lim.office.then_some(RichKind::Office(k));
    }
    if EPUB_EXT.contains(&e) {
        return lim.epub.then_some(RichKind::Epub);
    }
    if FONT_EXT.contains(&e) || (detected == Kind::Binary && preview_font::format_of(head).is_some()) {
        return lim.fonts.then_some(RichKind::Font);
    }
    if preview_model::kind_for_ext(e).is_some() {
        return lim.models.then_some(RichKind::Model);
    }
    match detected {
        Kind::Audio if lim.waveform || lim.playback => Some(RichKind::Audio),
        Kind::Video if lim.ffmpeg => Some(RichKind::Video),
        Kind::Pdf if lim.pdf => Some(RichKind::Pdf),
        _ => None,
    }
}

fn card(kind: Kind, icon: &'static str, title: impl Into<String>, body: impl Into<String>) -> Loaded {
    Loaded {
        kind,
        content: Content::Message { icon, title: title.into(), body: body.into() },
        facts: Vec::new(),
        meta: None,
    }
}

fn loaded(kind: Kind, rich: Rich, facts: Vec<(String, String)>) -> Loaded {
    Loaded { kind, content: Content::Rich(rich), facts, meta: None }
}

fn fact(facts: &mut Vec<(String, String)>, k: &str, v: impl Into<String>) {
    let v = v.into();
    if !v.trim().is_empty() && !facts.iter().any(|(fk, _)| fk == k) {
        facts.push((k.to_string(), v.trim().to_string()));
    }
}

fn read_all(r: &mut dyn ReadSeek, cap: u64) -> Result<Vec<u8>, String> {
    r.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.take(cap).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn upload(ctx: &Context, name: String, rgba: &[u8], w: usize, h: usize, premultiplied: bool) -> Tex {
    let img = if premultiplied {
        ColorImage::from_rgba_premultiplied([w, h], rgba)
    } else {
        ColorImage::from_rgba_unmultiplied([w, h], rgba)
    };
    Tex { tex: ctx.load_texture(name, img, TextureOptions::LINEAR), size: [w, h] }
}

/// Decode an embedded image (cover art, poster) downscaled to `max` px.
fn decode_image(ctx: &Context, name: String, bytes: &[u8], max: u32) -> Option<Tex> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(256 << 20);
    r.limits(lim);
    let mut img = r.decode().ok()?;
    if img.width() > max || img.height() > max {
        img = img.resize(max, max, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    Some(upload(ctx, name, rgba.as_raw(), rgba.width() as usize, rgba.height() as usize, false))
}

/// Load a rich preview. `path` is the file on disk (`None` for archive entries, which
/// arrive in memory through `reader`).
#[allow(clippy::too_many_arguments)]
pub fn load(
    rk: RichKind,
    source: &Source,
    reader: &mut dyn ReadSeek,
    len: u64,
    name: &str,
    ctx: &Context,
    id: u64,
    limits: &Limits,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let lim = &limits.rich;
    let path = match source {
        Source::File(p) => Some(p.as_path()),
        Source::Entry { .. } => None,
    };
    let ext = ext_of(name);
    let ppp = ctx.pixels_per_point().clamp(1.0, 4.0);
    match rk {
        RichKind::Office(k) => office(k, reader, len, lim),
        RichKind::Epub => epub(reader, len, lim, ctx, id, ppp),
        RichKind::Font => font(reader, len, lim, ctx, id, ppp),
        RichKind::Model => model(reader, len, &ext, path, lim),
        RichKind::Audio => audio(reader, len, &ext, path, lim, ctx, id, stale),
        RichKind::Video => video(reader, len, &ext, path, lim, ctx, id, ppp, stale),
        RichKind::Pdf => pdf(reader, len, path, lim, ctx, id, ppp, stale),
    }
}

fn too_large(kind: Kind, icon: &'static str, len: u64, cap: u64, key: &str) -> Loaded {
    card(
        kind,
        icon,
        tr!("services-preview-too-large"),
        tr!(
            "services-preview-too-large-body",
            size = (len >> 20).to_string(),
            limit = (cap >> 20).to_string(),
            key = key
        ),
    )
}

fn office(k: OfficeKind, reader: &mut dyn ReadSeek, len: u64, lim: &RichLimits) -> Loaded {
    let (kind, icon) = match k {
        OfficeKind::Docx | OfficeKind::Odt => (Kind::Document, ph::FILE_DOC),
        OfficeKind::Xlsx | OfficeKind::Ods => (Kind::Spreadsheet, ph::FILE_XLS),
        OfficeKind::Pptx | OfficeKind::Odp => (Kind::Presentation, ph::FILE_PPT),
    };
    // The container may be large (embedded media); each part is capped on its own.
    if len > lim.max_document.saturating_mul(8) {
        return too_large(kind, icon, len, lim.max_document * 8, "max_document_mib");
    }
    let _ = reader.seek(SeekFrom::Start(0));
    match preview_office::parse(&mut *reader, k, lim.max_document, lim.sheet_rows) {
        Ok(o) => {
            let mut facts = Vec::new();
            match &o {
                Office::Doc(d) => {
                    fact(&mut facts, "Format", d.format);
                    if let Some(t) = &d.title {
                        fact(&mut facts, "Title", t.clone());
                    }
                    if let Some(a) = &d.author {
                        fact(&mut facts, "Author", a.clone());
                    }
                    fact(&mut facts, "Words", d.words.to_string());
                    fact(&mut facts, "Paragraphs", d.paras.len().to_string());
                }
                Office::Sheet(s) => {
                    fact(&mut facts, "Format", s.format);
                    fact(&mut facts, "Sheets", s.sheets.len().to_string());
                    fact(
                        &mut facts,
                        "Shown",
                        tr!(
                            "services-preview-sheet-shown",
                            rows = s.rows.len().to_string(),
                            cols = s.ncols.to_string()
                        ),
                    );
                }
                Office::Slides(s) => {
                    fact(&mut facts, "Format", s.format);
                    if let Some(t) = &s.title {
                        fact(&mut facts, "Title", t.clone());
                    }
                    fact(&mut facts, "Slides", s.slides.len().to_string());
                }
            }
            loaded(kind, Rich::Office(Arc::new(o)), facts)
        }
        Err(e) => card(kind, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-document"), e),
    }
}

fn epub(reader: &mut dyn ReadSeek, len: u64, lim: &RichLimits, ctx: &Context, id: u64, ppp: f32) -> Loaded {
    if len > lim.max_document.saturating_mul(8) {
        return too_large(Kind::Ebook, ph::BOOK_OPEN, len, lim.max_document * 8, "max_document_mib");
    }
    let _ = reader.seek(SeekFrom::Start(0));
    match preview_epub::parse(&mut *reader, lim.max_document) {
        Ok(mut m) => {
            let max = (if lim.big { 900.0 } else { 420.0 } * ppp) as u32;
            let cover = m.cover.take().and_then(|b| decode_image(ctx, format!("preview:epub-cover:{id}"), &b, max));
            let mut facts = Vec::new();
            if let Some(t) = &m.title {
                fact(&mut facts, "Title", t.clone());
            }
            if !m.authors.is_empty() {
                fact(&mut facts, "Author", m.authors.join(", "));
            }
            for (k, v) in [("Language", &m.language), ("Publisher", &m.publisher), ("Published", &m.date)] {
                if let Some(v) = v {
                    fact(&mut facts, k, v.clone());
                }
            }
            fact(&mut facts, "Chapters", m.chapters.to_string());
            if let Some(v) = &m.version {
                fact(&mut facts, "EPUB version", v.clone());
            }
            loaded(Kind::Ebook, Rich::Epub { model: Arc::new(m), cover }, facts)
        }
        Err(e) => card(Kind::Ebook, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-ebook"), e),
    }
}

fn font(reader: &mut dyn ReadSeek, len: u64, lim: &RichLimits, ctx: &Context, id: u64, ppp: f32) -> Loaded {
    if len > lim.max_document {
        return too_large(Kind::Font, ph::TEXT_AA, len, lim.max_document, "max_document_mib");
    }
    let data = match read_all(reader, lim.max_document) {
        Ok(d) => d,
        Err(e) => return card(Kind::Font, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-font"), e),
    };
    let data = match preview_font::format_of(&data) {
        Some("WOFF") => match preview_font::woff_to_sfnt(&data, lim.max_document as usize) {
            Ok(d) => d,
            Err(e) => return card(Kind::Font, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-font"), e),
        },
        Some("WOFF 2") => {
            return card(Kind::Font, ph::TEXT_AA, tr!("services-preview-woff2"), tr!("services-preview-woff2-body"));
        }
        _ => data,
    };
    let meta = match preview_font::meta(&data) {
        Ok(m) => m,
        Err(e) => return card(Kind::Font, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-font"), e),
    };
    let width = (lim.width_pt() * ppp) as usize;
    let strips = match preview_font::specimen(&data, &meta.family, width, ppp) {
        Ok(s) => s,
        Err(e) => return card(Kind::Font, ph::WARNING_CIRCLE, tr!("services-preview-cant-render-font"), e),
    };
    let strips: Vec<(String, Tex)> = strips
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let t = upload(ctx, format!("preview:font:{id}:{i}"), &s.rgba, s.width, s.height, true);
            (s.caption, t)
        })
        .collect();
    let mut facts = Vec::new();
    fact(&mut facts, "Family", meta.family.clone());
    fact(&mut facts, "Style", meta.subfamily.clone());
    fact(&mut facts, "Format", meta.format);
    if let Some(v) = &meta.version {
        fact(&mut facts, "Version", v.clone());
    }
    fact(&mut facts, "Glyphs", meta.glyphs.to_string());
    fact(&mut facts, "Weight", meta.weight.to_string());
    if meta.monospaced {
        fact(&mut facts, "Spacing", tr!("services-preview-monospaced"));
    }
    if !meta.axes.is_empty() {
        let axes: Vec<String> = meta.axes.iter().map(|(t, lo, _, hi)| format!("{t} {lo:.0}–{hi:.0}")).collect();
        fact(&mut facts, "Variable axes", axes.join(", "));
    }
    if meta.faces > 1 {
        fact(&mut facts, "Faces", tr!("services-preview-faces", count = meta.faces));
    }
    if let Some(d) = meta.designer.as_ref().or(meta.manufacturer.as_ref()) {
        fact(&mut facts, "Designer", d.clone());
    }
    if let Some(c) = &meta.copyright {
        fact(&mut facts, "Copyright", c.chars().take(160).collect::<String>());
    }
    loaded(Kind::Font, Rich::Font { meta: Arc::new(meta), strips: Arc::new(strips) }, facts)
}

fn model(reader: &mut dyn ReadSeek, len: u64, ext: &str, path: Option<&Path>, lim: &RichLimits) -> Loaded {
    if len > lim.max_model {
        return too_large(Kind::Model, ph::CUBE, len, lim.max_model, "max_model_mib");
    }
    let data = match read_all(reader, lim.max_model) {
        Ok(d) => d,
        Err(e) => return card(Kind::Model, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-model"), e),
    };
    let dir = path.and_then(Path::parent);
    match preview_model::parse(&data, ext, dir, lim.max_model) {
        Ok(m) => {
            let mut facts = Vec::new();
            fact(&mut facts, "Format", m.format);
            fact(&mut facts, "Triangles", group(m.total_tris as u64));
            if m.tris.len() < m.total_tris {
                fact(&mut facts, "Drawn", tr!("services-preview-decimated", count = m.tris.len()));
            }
            fact(&mut facts, "Vertices", group(m.positions.len() as u64));
            if m.meshes > 1 {
                fact(&mut facts, "Meshes", m.meshes.to_string());
            }
            let [x, y, z] = m.extent;
            fact(&mut facts, "Bounds", format!("{} × {} × {}", fmt_len(x), fmt_len(y), fmt_len(z)));
            loaded(Kind::Model, Rich::Model { model: Arc::new(m), turntable: lim.turntable }, facts)
        }
        Err(e) => card(Kind::Model, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-model"), e),
    }
}

/// `1234567` → `1,234,567` (`1.234.567` in Turkish).
fn group(n: u64) -> String {
    soyuz_core::locale::group_int(n)
}

fn fmt_len(v: f32) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.4}")
    }
}

#[allow(clippy::too_many_arguments)]
fn audio(
    reader: &mut dyn ReadSeek,
    len: u64,
    ext: &str,
    path: Option<&Path>,
    lim: &RichLimits,
    ctx: &Context,
    id: u64,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let _ = reader.seek(SeekFrom::Start(0));
    let mut facts = preview_media::probe(&mut *reader, len, ext);
    let src = match path {
        Some(p) => AudioSrc::File(p.to_path_buf()),
        None => match read_all(reader, len) {
            Ok(b) => AudioSrc::Mem(b.into()),
            Err(e) => return card(Kind::Audio, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-file"), e),
        },
    };
    let waveform = lim.waveform && len <= lim.max_audio;
    let mut info = match preview_audio::analyze(&src, ext, waveform, stale) {
        Ok(i) => i,
        Err(e) if e == "stale" => return card(Kind::Audio, ph::WAVEFORM, "", ""),
        Err(e) => {
            // Unsupported codec (e.g. Opus, WMA): keep the header facts, no waveform.
            fact(&mut facts, "Waveform", e);
            AudioInfo::default()
        }
    };
    for (k, v) in info.tags.clone() {
        fact(&mut facts, &k, v);
    }
    if let Some(c) = &info.codec {
        fact(&mut facts, "Codec", c.clone());
    }
    if let Some(d) = info.duration {
        fact(&mut facts, "Duration", format_duration(d));
        if d > 0.5 {
            fact(&mut facts, "Bitrate", format!("{:.0} kbps", len as f64 * 8.0 / d / 1000.0));
        }
    }
    if let Some(sr) = info.sample_rate {
        fact(&mut facts, "Sample rate", format!("{sr} Hz"));
    }
    if lim.waveform && len > lim.max_audio {
        fact(
            &mut facts,
            "Waveform",
            tr!("services-preview-waveform-skipped", size = (lim.max_audio >> 20).to_string()),
        );
    }
    let cover = info.cover.take().and_then(|b| decode_image(ctx, format!("preview:audio-cover:{id}"), &b, 512));
    let rich = Rich::Audio { info: Arc::new(info), src, ext: ext.to_string(), playback: lim.playback, cover };
    loaded(Kind::Audio, rich, facts)
}

#[allow(clippy::too_many_arguments)]
fn video(
    reader: &mut dyn ReadSeek,
    len: u64,
    ext: &str,
    path: Option<&Path>,
    lim: &RichLimits,
    ctx: &Context,
    id: u64,
    ppp: f32,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let _ = reader.seek(SeekFrom::Start(0));
    let facts = preview_media::probe(&mut *reader, len, ext);
    let (poster, note) = match (path, preview_video::find_ffmpeg(&lim.ffmpeg_path)) {
        (None, _) => (None, Some(tr!("services-preview-poster-disk-only").to_string())),
        (Some(_), None) => (None, Some(tr!("services-preview-no-ffmpeg", path = &lim.ffmpeg_path))),
        (Some(p), Some(ff)) => {
            let dur = facts.iter().find(|(k, _)| k == "Duration").and_then(|(_, v)| preview_video::parse_duration(v));
            let w = ((if lim.big { 1280.0 } else { 640.0 }) * ppp).min(1920.0) as u32;
            match preview_video::poster(&ff, p, preview_video::poster_time(dur), w, stale) {
                Ok(png) => (decode_image(ctx, format!("preview:poster:{id}"), &png, w), None),
                Err(e) if e == "stale" => return card(Kind::Video, ph::FILM_SLATE, "", ""),
                Err(e) => (None, Some(e)),
            }
        }
    };
    loaded(Kind::Video, Rich::Video { poster, note }, facts)
}

#[allow(clippy::too_many_arguments)]
fn pdf(
    reader: &mut dyn ReadSeek,
    len: u64,
    path: Option<&Path>,
    lim: &RichLimits,
    ctx: &Context,
    id: u64,
    ppp: f32,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let src = match path {
        Some(p) => PdfSrc::File(p.to_path_buf()),
        None => match read_all(reader, len) {
            Ok(b) => PdfSrc::Mem(b.into()),
            Err(e) => return card(Kind::Pdf, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-file"), e),
        },
    };
    let dirs = preview_pdf::pdfium_dirs(&lim.pdfium_dir);
    let width_px = ((lim.width_pt() * ppp) as u32).min(2600);
    let mut no_render: Option<String>;
    let mut info: Option<PdfInfo> = None;
    #[cfg_attr(not(feature = "pdf"), allow(unused_mut))]
    let mut first = None;
    #[cfg(feature = "pdf")]
    {
        match preview_pdf::pdfium::info(&src, &dirs, lim.max_document.max(1 << 30), stale) {
            Ok(i) => {
                no_render = None;
                if i.pages > 0 {
                    match preview_pdf::pdfium::render(&src, &dirs, u64::MAX, 0, width_px, width_px * 3, stale) {
                        Ok(pg) => {
                            first =
                                Some(upload(ctx, format!("preview:pdf:{id}:0"), &pg.rgba, pg.width, pg.height, false))
                        }
                        Err(e) if e == "stale" => return card(Kind::Pdf, ph::FILE_PDF, "", ""),
                        Err(e) => no_render = Some(e),
                    }
                }
                info = Some(i);
            }
            Err(e) if e == "stale" => return card(Kind::Pdf, ph::FILE_PDF, "", ""),
            Err(e) => no_render = Some(e),
        }
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (ctx, id, stale, width_px);
        no_render = Some(tr!("services-preview-no-pdf-renderer").into());
    }
    if info.is_none() {
        if len > lim.max_document {
            let mut l = too_large(Kind::Pdf, ph::FILE_PDF, len, lim.max_document, "max_document_mib");
            if let Content::Message { body, .. } = &mut l.content
                && let Some(why) = &no_render
            {
                body.push_str("\n\n");
                body.push_str(why);
            }
            return l;
        }
        match preview_pdf::info_lopdf(&src, lim.max_document) {
            Ok(i) => info = Some(i),
            Err(e) => {
                let mut body = e;
                if let Some(why) = &no_render {
                    body.push_str("\n\n");
                    body.push_str(why);
                }
                return card(Kind::Pdf, ph::FILE_PDF, tr!("services-preview-cant-read-pdf"), body);
            }
        }
    }
    let info = info.unwrap_or_default();
    let mut facts = Vec::new();
    fact(&mut facts, "Pages", info.pages.to_string());
    if let Some(v) = &info.version {
        fact(&mut facts, "PDF version", v.clone());
    }
    for (k, v) in
        [("Title", &info.title), ("Author", &info.author), ("Producer", &info.producer), ("Created", &info.created)]
    {
        if let Some(v) = v {
            fact(&mut facts, k, v.clone());
        }
    }
    if let Some((w, h)) = info.page_size {
        fact(&mut facts, "Page size", format!("{:.0} × {:.0} mm", w / 72.0 * 25.4, h / 72.0 * 25.4));
    }
    if info.encrypted {
        fact(&mut facts, "Encrypted", tr!("common-yes"));
    }
    fact(&mut facts, "Renderer", if no_render.is_none() { "pdfium" } else { tr!("services-preview-text-only") });
    if let Some(why) = &mut no_render {
        *why = why.trim_start_matches("pdfium: ").to_string();
    }
    let view =
        PdfView { src, info: Arc::new(info), first, no_render, dirs, cap: lim.max_document.max(1 << 30), width_px };
    loaded(Kind::Pdf, Rich::Pdf(view), facts)
}

/// Render another PDF page off the UI thread; the texture arrives on the receiver.
pub fn pdf_page_async(
    view: &PdfView,
    page: u32,
    ctx: &Context,
) -> crossbeam_channel::Receiver<Result<(u32, Tex), String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    #[cfg(feature = "pdf")]
    {
        let (src, dirs, cap, w, ctx) = (view.src.clone(), view.dirs.clone(), view.cap, view.width_px, ctx.clone());
        let _ = std::thread::Builder::new().name("soyuz-pdf-page".into()).spawn(move || {
            let r = preview_pdf::pdfium::render(&src, &dirs, cap, page, w, w * 3, &|| false).map(|pg| {
                let t = upload(
                    &ctx,
                    format!("preview:pdfpage:{page}:{}", src_key(&src)),
                    &pg.rgba,
                    pg.width,
                    pg.height,
                    false,
                );
                (page, t)
            });
            let _ = tx.send(r);
            ctx.request_repaint();
        });
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (view, page, ctx);
        let _ = tx.send(Err(tr!("services-preview-no-pdf-renderer-short").into()));
    }
    rx
}

#[cfg(feature = "pdf")]
fn src_key(src: &PdfSrc) -> String {
    match src {
        PdfSrc::File(p) => p.display().to_string(),
        PdfSrc::Mem(b) => format!("{:p}", b.as_ptr()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::preview::{Msg, PreviewClient};
    use crate::services::preview_office::fixtures;
    use std::time::{Duration, Instant};

    #[test]
    fn routing_respects_toggles() {
        let on = RichLimits::default();
        assert_eq!(route("a.DOCX", b"PK", Kind::Archive, &on), Some(RichKind::Office(OfficeKind::Docx)));
        assert_eq!(route("b.epub", b"PK", Kind::Archive, &on), Some(RichKind::Epub));
        assert_eq!(route("c.ttf", &[0, 1, 0, 0], Kind::Binary, &on), Some(RichKind::Font));
        assert_eq!(route("noext", b"OTTO", Kind::Binary, &on), Some(RichKind::Font));
        assert_eq!(route("m.stl", b"solid", Kind::Text, &on), Some(RichKind::Model));
        assert_eq!(route("s.mp3", b"ID3", Kind::Audio, &on), Some(RichKind::Audio));
        assert_eq!(route("d.pdf", b"%PDF", Kind::Pdf, &on), Some(RichKind::Pdf));
        assert_eq!(route("x.txt", b"hi", Kind::Text, &on), None);
        let off = RichLimits {
            office: false,
            fonts: false,
            models: false,
            waveform: false,
            playback: false,
            pdf: false,
            ..RichLimits::default()
        };
        assert_eq!(route("a.docx", b"PK", Kind::Archive, &off), None);
        assert_eq!(route("c.ttf", b"", Kind::Binary, &off), None);
        assert_eq!(route("m.obj", b"", Kind::Text, &off), None);
        assert_eq!(route("s.mp3", b"", Kind::Audio, &off), None);
        assert_eq!(route("d.pdf", b"", Kind::Pdf, &off), None);
    }

    fn limits() -> Limits {
        Limits {
            max_image_px: 256,
            max_image_mp: 40,
            max_text_bytes: 512 * 1024,
            dark: true,
            mono: egui::FontId::monospace(12.0),
            fg: egui::Color32::WHITE,
            rich: RichLimits { ffmpeg_path: "definitely-missing-ffmpeg".into(), ..RichLimits::default() },
        }
    }

    fn wait(client: &PreviewClient) -> Loaded {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            if let Some(Msg::Loaded { loaded, .. }) = client.try_recv() {
                return *loaded;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("preview timed out");
    }

    #[test]
    fn end_to_end_through_the_preview_worker() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::default();
        let client = PreviewClient::new(&ctx);
        let docx = dir.path().join("r.docx");
        std::fs::write(&docx, fixtures::docx()).unwrap();
        client.request(Source::File(docx), false, false, limits());
        let l = wait(&client);
        assert_eq!(l.kind, Kind::Document);
        assert!(matches!(l.content, Content::Rich(Rich::Office(_))));
        assert!(l.facts.iter().any(|(k, v)| k == "Author" && v == "Ada"));

        let stl = dir.path().join("m.stl");
        std::fs::write(&stl, "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid\n").unwrap();
        client.request(Source::File(stl), false, false, limits());
        let l = wait(&client);
        assert_eq!(l.kind, Kind::Model);

        let wav = dir.path().join("t.wav");
        std::fs::write(&wav, preview_audio::tests::wav(0.5, 8000, 0.5)).unwrap();
        client.request(Source::File(wav), false, false, limits());
        let l = wait(&client);
        match l.content {
            Content::Rich(Rich::Audio { info, .. }) => assert!(!info.peaks.is_empty()),
            _ => panic!("expected audio"),
        }

        // Toggle off: docx falls back to the archive listing.
        let mut off = limits();
        off.rich.office = false;
        let docx = dir.path().join("r2.docx");
        std::fs::write(&docx, fixtures::docx()).unwrap();
        client.request(Source::File(docx), false, false, off);
        assert_eq!(wait(&client).kind, Kind::Archive);

        // Video without ffmpeg: metadata plus a note.
        let mp4 = dir.path().join("v.mp4");
        std::fs::write(&mp4, b"\0\0\0\x18ftypisom\0\0\0\0isomiso2").unwrap();
        client.request(Source::File(mp4), false, false, limits());
        match wait(&client).content {
            Content::Rich(Rich::Video { poster, note }) => {
                assert!(poster.is_none());
                assert!(note.unwrap().contains("ffmpeg"));
            }
            _ => panic!("expected video"),
        }
    }
}
