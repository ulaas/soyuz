//! Rich previews: the dispatch between the generic preview loader
//! ([`super::preview`]) and the format modules — Office / ODF documents, EPUB, fonts,
//! 3D models, audio (waveform + player), video metadata and poster frames (the FFmpeg
//! libraries, `services::media`, with the pure-Rust `preview_media` fallback) and PDF
//! (pdfium, with a pure-Rust text fallback).
//!
//! Everything here runs on the preview worker threads: inputs are size-capped by the
//! `[preview]` limits, long loops check the request's staleness, and the caller's
//! `catch_unwind` turns a parser panic into an error card. Results are ready to draw
//! (textures uploaded, models normalized).

use std::io::{Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use soyuz_core::config::{Media as MediaCfg, Preview as PreviewCfg};
use soyuz_media::{MediaInfo, Source as MediaSource, StreamKind};

use super::media;
use super::preview::{Content, Kind, Limits, Loaded, ReadSeek, Source};
use super::preview_audio::{self, AudioInfo, AudioSrc};
use super::preview_epub::{self, EpubModel};
use super::preview_font::{self, FontMeta};
pub use super::preview_gfx::Tex;
use super::preview_gfx::{Pixels, Uploader};
use super::preview_media::{self, format_duration};
use super::preview_model::{self, Model};
use super::preview_office::{self, Office, OfficeKind};
use super::preview_pdf::{self, PdfInfo, PdfSrc};
use super::preview_text;
use super::preview_video;
use crate::icon::IconId;

/// Per-request switches and caps (from `[preview]` and `[media]`).
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
    /// Video previews (`[preview] video`).
    pub video: bool,
    /// Longest edge of a poster frame (`[media] max_px`).
    pub media_max_px: u32,
    /// Where the poster frame is taken (`[media] poster_secs`; 0 = a tenth of the
    /// duration, at most 5 s).
    pub poster_secs: f32,
    pub max_document: u64,
    pub sheet_rows: usize,
    pub max_model: u64,
    pub max_audio: u64,
    pub turntable: bool,
    /// Quick Look (large renders) vs the preview pane.
    pub big: bool,
    /// The frontend draws only uploaded pixels and shows facts as text (the terminal UI):
    /// SVGs and 3D models arrive rasterized as `Content::Image` (`preview_raster`) and
    /// pictures carry their EXIF rows. The GUI leaves it off.
    pub terminal: bool,
}

impl RichLimits {
    pub fn from_config(c: &PreviewCfg, m: &MediaCfg, big: bool) -> Self {
        RichLimits {
            pdf: c.pdf,
            pdfium_dir: c.pdfium_dir.clone(),
            office: c.office,
            epub: c.epub,
            fonts: c.fonts,
            models: c.models,
            waveform: c.audio_waveform,
            playback: c.audio_playback,
            video: c.video,
            media_max_px: m.max_px.clamp(64, 8192),
            poster_secs: m.poster_secs,
            max_document: c.max_document_mib.max(1).saturating_mul(1 << 20),
            sheet_rows: c.sheet_rows.clamp(1, 100_000) as usize,
            max_model: c.max_model_mib.max(1).saturating_mul(1 << 20),
            max_audio: c.max_audio_mib.saturating_mul(1 << 20),
            turntable: c.model_turntable,
            big,
            terminal: false,
        }
    }

    /// Render width in points for page-like content.
    fn width_pt(&self) -> f32 {
        if self.big { 1000.0 } else { 340.0 }
    }
}

impl Default for RichLimits {
    fn default() -> Self {
        RichLimits::from_config(&PreviewCfg::default(), &MediaCfg::default(), false)
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
    /// The file is encrypted and no (right) password was typed: Enter PDF password…
    /// unlocks it ([`preview_pdf::set_password`]).
    pub locked: Option<PathBuf>,
}

#[derive(Clone)]
pub enum Rich {
    Office(Arc<Office>),
    Epub {
        model: Arc<EpubModel>,
        cover: Option<Tex>,
    },
    Font {
        meta: Arc<FontMeta>,
        strips: Arc<Vec<(String, Tex)>>,
    },
    Model {
        model: Arc<Model>,
        turntable: bool,
    },
    Audio {
        info: Arc<AudioInfo>,
        src: AudioSrc,
        ext: String,
        playback: bool,
        cover: Option<Tex>,
    },
    /// `info`: what the FFmpeg libraries read (`None` without them; the facts then come
    /// from `preview_media`). `note`: why there is no poster (no libraries, not on disk,
    /// skipped after a crash, decode error).
    Video {
        poster: Option<Tex>,
        note: Option<String>,
        info: Option<Arc<MediaInfo>>,
    },
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
        Kind::Video if lim.video => Some(RichKind::Video),
        Kind::Pdf if lim.pdf => Some(RichKind::Pdf),
        _ => None,
    }
}

fn card(kind: Kind, icon: IconId, title: impl Into<String>, body: impl Into<String>) -> Loaded {
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

pub fn upload(up: &Uploader, name: String, rgba: &[u8], w: usize, h: usize, premultiplied: bool) -> Tex {
    Tex::upload(up, name, Pixels { size: [w, h], rgba, premultiplied })
}

/// Decode an embedded image (cover art, poster) downscaled to `max` px.
fn decode_image(up: &Uploader, name: String, bytes: &[u8], max: u32) -> Option<Tex> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(256 << 20);
    r.limits(lim);
    let mut img = r.decode().ok()?;
    if img.width() > max || img.height() > max {
        img = img.resize(max, max, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    Some(Tex::upload_rgba(up, name, &rgba))
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
    up: &Uploader,
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
    let ppp = limits.pixels_per_point.clamp(1.0, 4.0);
    match rk {
        RichKind::Office(k) => office(k, reader, len, lim),
        RichKind::Epub => epub(reader, len, lim, up, id, ppp),
        RichKind::Font => font(reader, len, lim, up, id, ppp),
        RichKind::Model => model(reader, len, &ext, path, lim, up, id, ppp),
        RichKind::Audio => audio(reader, len, &ext, path, lim, up, id, stale),
        RichKind::Video => video(reader, len, &ext, path, lim, up, id, ppp, stale),
        RichKind::Pdf => pdf(reader, len, path, lim, up, id, ppp, stale),
    }
}

fn too_large(kind: Kind, icon: IconId, len: u64, cap: u64, key: &str) -> Loaded {
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
        OfficeKind::Docx | OfficeKind::Odt => (Kind::Document, IconId::FileDoc),
        OfficeKind::Xlsx | OfficeKind::Ods => (Kind::Spreadsheet, IconId::FileXls),
        OfficeKind::Pptx | OfficeKind::Odp => (Kind::Presentation, IconId::FilePpt),
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
        Err(e) => card(kind, IconId::WarningCircle, tr!("services-preview-cant-read-document"), e),
    }
}

fn epub(reader: &mut dyn ReadSeek, len: u64, lim: &RichLimits, up: &Uploader, id: u64, ppp: f32) -> Loaded {
    if len > lim.max_document.saturating_mul(8) {
        return too_large(Kind::Ebook, IconId::BookOpen, len, lim.max_document * 8, "max_document_mib");
    }
    let _ = reader.seek(SeekFrom::Start(0));
    match preview_epub::parse(&mut *reader, lim.max_document) {
        Ok(mut m) => {
            let max = (if lim.big { 900.0 } else { 420.0 } * ppp) as u32;
            let cover = m.cover.take().and_then(|b| decode_image(up, format!("preview:epub-cover:{id}"), &b, max));
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
        Err(e) => card(Kind::Ebook, IconId::WarningCircle, tr!("services-preview-cant-read-ebook"), e),
    }
}

fn font(reader: &mut dyn ReadSeek, len: u64, lim: &RichLimits, up: &Uploader, id: u64, ppp: f32) -> Loaded {
    if len > lim.max_document {
        return too_large(Kind::Font, IconId::TextAa, len, lim.max_document, "max_document_mib");
    }
    let data = match read_all(reader, lim.max_document) {
        Ok(d) => d,
        Err(e) => return card(Kind::Font, IconId::WarningCircle, tr!("services-preview-cant-read-font"), e),
    };
    let data = match preview_font::format_of(&data) {
        Some("WOFF") => match preview_font::woff_to_sfnt(&data, lim.max_document as usize) {
            Ok(d) => d,
            Err(e) => return card(Kind::Font, IconId::WarningCircle, tr!("services-preview-cant-read-font"), e),
        },
        Some("WOFF 2") => match preview_font::woff2_to_sfnt(&data, lim.max_document as usize) {
            Ok(d) => d,
            Err(e) => return card(Kind::Font, IconId::WarningCircle, tr!("services-preview-cant-read-font"), e),
        },
        _ => data,
    };
    let meta = match preview_font::meta(&data) {
        Ok(m) => m,
        Err(e) => return card(Kind::Font, IconId::WarningCircle, tr!("services-preview-cant-read-font"), e),
    };
    let width = (lim.width_pt() * ppp) as usize;
    let strips = match preview_font::specimen(&data, &meta.family, width, ppp) {
        Ok(s) => s,
        Err(e) => return card(Kind::Font, IconId::WarningCircle, tr!("services-preview-cant-render-font"), e),
    };
    let strips: Vec<(String, Tex)> = strips
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let t = upload(up, format!("preview:font:{id}:{i}"), &s.rgba, s.width, s.height, true);
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

#[allow(clippy::too_many_arguments)]
fn model(
    reader: &mut dyn ReadSeek,
    len: u64,
    ext: &str,
    path: Option<&Path>,
    lim: &RichLimits,
    up: &Uploader,
    id: u64,
    ppp: f32,
) -> Loaded {
    if len > lim.max_model {
        return too_large(Kind::Model, IconId::Cube, len, lim.max_model, "max_model_mib");
    }
    let data = match read_all(reader, lim.max_model) {
        Ok(d) => d,
        Err(e) => return card(Kind::Model, IconId::WarningCircle, tr!("services-preview-cant-read-model"), e),
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
            if lim.terminal {
                // A still from the default view (the GUI draws a live mesh instead).
                let px = (if lim.big { 900.0 } else { 480.0 } * ppp).min(2048.0) as u32;
                let img = super::preview_raster::model_still(&m, px);
                let tex = Tex::upload_rgba(up, format!("preview:model:{id}"), &img);
                let (width, height) = img.dimensions();
                let content = Content::Image(super::preview::ImageData { tex, width, height });
                return Loaded { kind: Kind::Model, content, facts, meta: None };
            }
            loaded(Kind::Model, Rich::Model { model: Arc::new(m), turntable: lim.turntable }, facts)
        }
        Err(e) => card(Kind::Model, IconId::WarningCircle, tr!("services-preview-cant-read-model"), e),
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
    up: &Uploader,
    id: u64,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let _ = reader.seek(SeekFrom::Start(0));
    let mut facts = preview_media::probe(&mut *reader, len, ext);
    let src = match path {
        Some(p) => AudioSrc::File(p.to_path_buf()),
        None => match read_all(reader, len) {
            Ok(b) => AudioSrc::Mem(b.into()),
            Err(e) => return card(Kind::Audio, IconId::WarningCircle, tr!("services-preview-cant-read-file"), e),
        },
    };
    let waveform = lim.waveform && len <= lim.max_audio;
    let mut info = match preview_audio::analyze(&src, ext, waveform, stale) {
        Ok(i) => i,
        Err(e) if e == "stale" => return card(Kind::Audio, IconId::Waveform, "", ""),
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
    if info.codec.is_some() || !info.peaks.is_empty() {
        fact(&mut facts, "Decoder", info.decoder.label());
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
    let cover = info.cover.take().and_then(|b| decode_image(up, format!("preview:audio-cover:{id}"), &b, 512));
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
    up: &Uploader,
    id: u64,
    ppp: f32,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let _ = ppp;
    let mut facts = Vec::new();
    let mut info: Option<MediaInfo> = None;
    let mut poster = None;
    let mut note = None;
    match (media::runtime(), path) {
        (None, _) => {
            let dir = soyuz_fs::strip_verbatim(&soyuz_core::config::config_dir().join("ffmpeg"));
            note = Some(tr!(
                "services-preview-no-ffmpeg-libs",
                tag = media::runtime_release_tag(),
                dir = dir.display().to_string()
            ));
        }
        (Some(_), None) => note = Some(tr!("services-preview-poster-disk-only").to_string()),
        (Some(rt), Some(p)) => {
            // Quick Look is the user's explicit choice: a file skipped after a crash gets
            // another go there; the preview pane keeps skipping it.
            let sentinel = media::sentinel();
            if lim.big {
                sentinel.forgive(p);
            }
            match sentinel.begin(p) {
                None => note = Some(tr!("services-preview-crashed-last-time").to_string()),
                Some(_inflight) => {
                    match soyuz_media::probe(&rt, &MediaSource::File(p.to_path_buf())) {
                        Ok(i) => info = Some(i),
                        Err(e) => log::debug!("probe {}: {e}", p.display()),
                    }
                    if stale() {
                        return card(Kind::Video, IconId::FilmSlate, "", "");
                    }
                    let dur = info.as_ref().and_then(|i| i.duration);
                    let at = preview_video::poster_at(lim.poster_secs, dur);
                    match preview_video::poster(&rt, p, at, lim.media_max_px, stale) {
                        Ok(img) => poster = Some(Tex::upload_rgba(up, format!("preview:poster:{id}"), &img)),
                        Err(e) if e == "stale" => return card(Kind::Video, IconId::FilmSlate, "", ""),
                        Err(e) => note = Some(e),
                    }
                }
            }
        }
    }
    if let Some(i) = &info {
        media_facts(&mut facts, i);
    }
    // The pure-Rust parser: everything without the libraries, the gaps with them.
    let _ = reader.seek(SeekFrom::Start(0));
    for (k, v) in preview_media::probe(&mut *reader, len, ext) {
        fact(&mut facts, &k, v);
    }
    loaded(Kind::Video, Rich::Video { poster, note, info: info.map(Arc::new) }, facts)
}

/// `1920 × 1080`-style size of a stream.
fn dimensions(w: u32, h: u32) -> String {
    format!("{w} × {h}")
}

fn kbps(bits: u64) -> String {
    format!("{} kbps", (bits as f64 / 1000.0).round())
}

/// The facts rows of a video from what the FFmpeg libraries read (keys are the English
/// names `models::preview_info::info_label` translates).
pub fn media_facts(facts: &mut Vec<(String, String)>, i: &MediaInfo) {
    // Container-level tags first: the title headline, then the usual music-style tags.
    for (tag, key) in [
        ("title", "Title"),
        ("artist", "Artist"),
        ("album", "Album"),
        ("date", "Year"),
        ("genre", "Genre"),
        ("comment", "Comment"),
        ("encoder", "Encoder"),
    ] {
        if let Some(v) = i.tag(tag) {
            fact(facts, key, v.chars().take(200).collect::<String>());
        }
    }
    let container = if i.container_long.is_empty() { i.container.as_str() } else { i.container_long.as_str() };
    fact(facts, "Container", container);
    if let Some(d) = i.duration {
        fact(facts, "Duration", format_duration(d));
    }
    if let Some(v) = i.video() {
        let mut codec = if v.codec_long.is_empty() { v.codec.clone() } else { v.codec_long.clone() };
        if let Some(p) = &v.profile {
            codec = format!("{codec} ({p})");
        }
        fact(facts, "Video", codec);
        if v.width > 0 && v.height > 0 {
            // The display size: rotation swaps the sides.
            let (w, h) = if v.rotation == 90 || v.rotation == 270 { (v.height, v.width) } else { (v.width, v.height) };
            fact(facts, "Dimensions", dimensions(w, h));
        }
        if let Some(f) = v.fps.filter(|f| f.is_finite() && *f > 0.0) {
            let text = if (f - f.round()).abs() < 0.005 { format!("{f:.0} fps") } else { format!("{f:.2} fps") };
            fact(facts, "Frame rate", text);
        }
        if let Some(p) = &v.pix_fmt {
            fact(facts, "Pixel format", p.clone());
        }
        if let Some(h) = v.hdr {
            fact(
                facts,
                "HDR",
                match h {
                    soyuz_media::Hdr::Hdr10 => "HDR10",
                    soyuz_media::Hdr::Hdr10Plus => "HDR10+",
                    soyuz_media::Hdr::Hlg => "HLG",
                    soyuz_media::Hdr::DolbyVision => "Dolby Vision",
                },
            );
        }
        if v.rotation != 0 {
            fact(facts, "Rotation", format!("{}°", v.rotation));
        }
        if let Some(b) = v.bit_rate.filter(|b| *b > 0) {
            fact(facts, "Video bitrate", kbps(b));
        }
    }
    if let Some(a) = i.audio() {
        let codec = if a.codec_long.is_empty() { a.codec.clone() } else { a.codec_long.clone() };
        fact(facts, "Audio", codec);
        if !a.channel_layout.is_empty() {
            fact(facts, "Channels", a.channel_layout.clone());
        } else if a.channels > 0 {
            fact(facts, "Channels", a.channels.to_string());
        }
        if a.sample_rate > 0 {
            fact(facts, "Sample rate", format!("{} Hz", a.sample_rate));
        }
        if let Some(l) = &a.language {
            fact(facts, "Language", l.clone());
        }
        if let Some(b) = a.bit_rate.filter(|b| *b > 0) {
            fact(facts, "Audio bitrate", kbps(b));
        }
    }
    if let Some(b) = i.bit_rate.filter(|b| *b > 0) {
        fact(facts, "Bitrate", kbps(b));
    }
    let subs = i.streams.iter().filter(|s| s.kind == Some(StreamKind::Subtitle)).count();
    if subs > 0 {
        fact(facts, "Subtitles", subs.to_string());
    }
    if !i.chapters.is_empty() {
        fact(facts, "Chapters", i.chapters.len().to_string());
    }
}

#[allow(clippy::too_many_arguments)]
fn pdf(
    reader: &mut dyn ReadSeek,
    len: u64,
    path: Option<&Path>,
    lim: &RichLimits,
    up: &Uploader,
    id: u64,
    ppp: f32,
    stale: &dyn Fn() -> bool,
) -> Loaded {
    let src = match path {
        Some(p) => PdfSrc::File(p.to_path_buf()),
        None => match read_all(reader, len) {
            Ok(b) => PdfSrc::Mem(b.into()),
            Err(e) => return card(Kind::Pdf, IconId::WarningCircle, tr!("services-preview-cant-read-file"), e),
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
                                Some(upload(up, format!("preview:pdf:{id}:0"), &pg.rgba, pg.width, pg.height, false))
                        }
                        Err(e) if e == "stale" => return card(Kind::Pdf, IconId::FilePdf, "", ""),
                        Err(e) => no_render = Some(e),
                    }
                }
                info = Some(i);
            }
            Err(e) if e == "stale" => return card(Kind::Pdf, IconId::FilePdf, "", ""),
            Err(e) => no_render = Some(e),
        }
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (up, id, stale, width_px);
        no_render = Some(tr!("services-preview-no-pdf-renderer").into());
    }
    if info.is_none() {
        if len > lim.max_document {
            let mut l = too_large(Kind::Pdf, IconId::FilePdf, len, lim.max_document, "max_document_mib");
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
            // Locked: the view asks for the password (whatever pdfium said).
            Err(e) if preview_pdf::is_password_error(&e) => {
                info = Some(PdfInfo { encrypted: true, ..Default::default() });
                no_render = Some(e);
            }
            Err(e) => {
                let mut body = e;
                if let Some(why) = &no_render {
                    body.push_str("\n\n");
                    body.push_str(why);
                }
                return card(Kind::Pdf, IconId::FilePdf, tr!("services-preview-cant-read-pdf"), body);
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
    let locked = match &src {
        PdfSrc::File(p) => no_render.as_deref().is_some_and(preview_pdf::is_password_error).then(|| p.clone()),
        PdfSrc::Mem(_) => None,
    };
    let cap = lim.max_document.max(1 << 30);
    let view = PdfView { src, info: Arc::new(info), first, no_render, dirs, cap, width_px, locked };
    loaded(Kind::Pdf, Rich::Pdf(view), facts)
}

/// Render another PDF page off the UI thread; the texture arrives on the receiver, then
/// `wake` runs.
pub fn pdf_page_async(
    view: &PdfView,
    page: u32,
    wake: &crate::host::Wake,
    up: &Uploader,
) -> crossbeam_channel::Receiver<Result<(u32, Tex), String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    #[cfg(feature = "pdf")]
    {
        let (src, dirs, cap, w) = (view.src.clone(), view.dirs.clone(), view.cap, view.width_px);
        let (wake, up) = (wake.clone(), up.clone());
        let _ = std::thread::Builder::new().name("soyuz-pdf-page".into()).spawn(move || {
            let r = preview_pdf::pdfium::render(&src, &dirs, cap, page, w, w * 3, &|| false).map(|pg| {
                let t = upload(
                    &up,
                    format!("preview:pdfpage:{page}:{}", src_key(&src)),
                    &pg.rgba,
                    pg.width,
                    pg.height,
                    false,
                );
                (page, t)
            });
            let _ = tx.send(r);
            wake();
        });
    }
    #[cfg(not(feature = "pdf"))]
    {
        let _ = (view, page, wake, up);
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
            mono: crate::services::preview_gfx::TextFont::monospace(12.0),
            fg: crate::services::preview_gfx::WHITE,
            pixels_per_point: 1.0,
            rich: RichLimits::default(),
            folder_notes: Vec::new(),
        }
    }

    #[test]
    fn media_facts_rows() {
        use soyuz_media::{Chapter, Hdr, StreamInfo};
        let info = MediaInfo {
            container: "matroska,webm".into(),
            container_long: "Matroska / WebM".into(),
            duration: Some(125.0),
            bit_rate: Some(2_500_000),
            streams: vec![
                StreamInfo {
                    index: 0,
                    kind: Some(StreamKind::Video),
                    codec: "hevc".into(),
                    codec_long: "H.265 / HEVC".into(),
                    profile: Some("Main 10".into()),
                    width: 1920,
                    height: 1080,
                    rotation: 90,
                    fps: Some(29.97),
                    pix_fmt: Some("yuv420p10le".into()),
                    hdr: Some(Hdr::Hdr10),
                    bit_rate: Some(2_000_000),
                    ..Default::default()
                },
                StreamInfo {
                    index: 1,
                    kind: Some(StreamKind::Audio),
                    codec: "aac".into(),
                    channel_layout: "stereo".into(),
                    channels: 2,
                    sample_rate: 48_000,
                    language: Some("eng".into()),
                    ..Default::default()
                },
                StreamInfo { index: 2, kind: Some(StreamKind::Subtitle), ..Default::default() },
            ],
            chapters: vec![Chapter { start: 0.0, end: 1.0, title: None }],
            tags: vec![("TITLE".into(), "Clip".into()), ("encoder".into(), "Lavf".into())],
            ..Default::default()
        };
        let mut facts = Vec::new();
        media_facts(&mut facts, &info);
        // Fallback facts never overwrite the library's.
        fact(&mut facts, "Duration", "0:00");
        let get = |k: &str| facts.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Title"), Some("Clip"));
        assert_eq!(get("Container"), Some("Matroska / WebM"));
        assert_eq!(get("Duration"), Some("2:05"));
        assert_eq!(get("Video"), Some("H.265 / HEVC (Main 10)"));
        assert_eq!(get("Dimensions"), Some("1080 × 1920"), "rotation swaps the display size");
        assert_eq!(get("Frame rate"), Some("29.97 fps"));
        assert_eq!(get("Pixel format"), Some("yuv420p10le"));
        assert_eq!(get("HDR"), Some("HDR10"));
        assert_eq!(get("Rotation"), Some("90°"));
        assert_eq!(get("Video bitrate"), Some("2000 kbps"));
        assert_eq!(get("Audio"), Some("aac"));
        assert_eq!(get("Channels"), Some("stereo"));
        assert_eq!(get("Sample rate"), Some("48000 Hz"));
        assert_eq!(get("Language"), Some("eng"));
        assert_eq!(get("Bitrate"), Some("2500 kbps"));
        assert_eq!(get("Subtitles"), Some("1"));
        assert_eq!(get("Chapters"), Some("1"));
        assert_eq!(get("Encoder"), Some("Lavf"));
        for (k, _) in &facts {
            assert_ne!(crate::models::preview_info::info_label(k), "", "{k}");
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
        let client = PreviewClient::new(crate::host::no_wake(), crate::services::preview_gfx::no_upload());
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

        // Video: 24 header bytes decode to nothing, with or without the libraries. Without
        // them the note says where the runtime comes from (its release tag).
        let mp4 = dir.path().join("v.mp4");
        std::fs::write(&mp4, b"\0\0\0\x18ftypisom\0\0\0\0isomiso2").unwrap();
        client.request(Source::File(mp4), false, false, limits());
        match wait(&client).content {
            Content::Rich(Rich::Video { poster, note, info }) => {
                assert!(poster.is_none());
                let note = note.expect("a note says why there is no poster");
                if media::runtime().is_none() {
                    assert!(note.contains(&media::runtime_release_tag()), "{note}");
                    assert!(note.contains("ffmpeg"), "{note}");
                    assert!(info.is_none());
                }
            }
            _ => panic!("expected video"),
        }
    }
}
