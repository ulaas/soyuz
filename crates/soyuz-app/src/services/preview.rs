//! Off-thread preview loading for the preview pane and Quick Look (PLAN M9, §3).
//!
//! A small process-wide worker pool (two loader threads + one hex-page thread) serves
//! [`PreviewClient`]s. Each client owns a result channel and a "latest request" counter:
//! issuing a new request makes every older one stale, and workers drop stale requests
//! before (and during) heavy work, so moving the selection quickly never queues up
//! decoding. Results carry ready-to-draw data — textures are uploaded on the worker via
//! `ctx.load_texture`, text is highlighted into `LayoutJob`s — so the UI thread only
//! paints.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::text::LayoutJob;
use egui::{Color32, ColorImage, Context, FontId, TextureHandle, TextureOptions};
use soyuz_core::{EntryInfo, EntryKind, split_vpath};
use soyuz_fs::ListOptions;

use super::preview_media;
use super::preview_text::{self, Block};

/// Size of one hex page.
pub const HEX_PAGE: u64 = 64 * 1024;
/// Archive entries are read fully into memory up to this size for previews.
const ENTRY_CAP: u64 = 64 * 1024 * 1024;
/// Archive listings show at most this many entries.
const LISTING_CAP: usize = 2_000;

/// What to preview.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    File(PathBuf),
    /// An entry inside an archive (`inner` is `/`-separated).
    Entry {
        archive: PathBuf,
        inner: String,
    },
}

impl Source {
    /// A filesystem path or an archive virtual path (`a.zip!/x/y`).
    pub fn from_path(p: &Path) -> Source {
        match split_vpath(p) {
            Some((archive, inner)) if !inner.is_empty() => Source::Entry { archive, inner },
            Some((archive, _)) => Source::File(archive),
            None => Source::File(p.to_path_buf()),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Source::File(p) => soyuz_core::entry::display_name(p),
            Source::Entry { inner, .. } => inner.rsplit('/').next().unwrap_or(inner).to_string(),
        }
    }

    pub fn is_entry(&self) -> bool {
        matches!(self, Source::Entry { .. })
    }
}

/// Broad preview category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Image,
    Svg,
    Text,
    Markdown,
    Archive,
    Audio,
    Video,
    Pdf,
    Binary,
    // Rich previews (roadmap F3, `preview_rich`).
    Document,
    Spreadsheet,
    Presentation,
    Ebook,
    Font,
    Model,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Folder => tr!("services-kind-folder"),
            Kind::Image => tr!("services-kind-image"),
            Kind::Svg => tr!("services-kind-svg"),
            Kind::Text => tr!("services-kind-text"),
            Kind::Markdown => tr!("services-kind-markdown"),
            Kind::Archive => tr!("services-kind-archive"),
            Kind::Audio => tr!("services-kind-audio"),
            Kind::Video => tr!("services-kind-video"),
            Kind::Pdf => tr!("services-kind-pdf"),
            Kind::Binary => tr!("services-kind-binary"),
            Kind::Document => tr!("services-kind-document"),
            Kind::Spreadsheet => tr!("services-kind-spreadsheet"),
            Kind::Presentation => tr!("services-kind-presentation"),
            Kind::Ebook => tr!("services-kind-ebook"),
            Kind::Font => tr!("services-kind-font"),
            Kind::Model => tr!("services-kind-model"),
        }
    }
}

const TEXT_EXT: &[&str] = &[
    "txt",
    "log",
    "rs",
    "toml",
    "json",
    "jsonc",
    "json5",
    "yaml",
    "yml",
    "xml",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "js",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "jsx",
    "py",
    "pyi",
    "rb",
    "go",
    "java",
    "kt",
    "kts",
    "c",
    "h",
    "cpp",
    "cc",
    "cxx",
    "hpp",
    "hh",
    "cs",
    "fs",
    "swift",
    "m",
    "mm",
    "php",
    "pl",
    "lua",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "psm1",
    "psd1",
    "bat",
    "cmd",
    "ini",
    "cfg",
    "conf",
    "env",
    "sql",
    "csv",
    "tsv",
    "gradle",
    "properties",
    "lock",
    "gitignore",
    "gitattributes",
    "editorconfig",
    "dockerfile",
    "makefile",
    "cmake",
    "vue",
    "svelte",
    "dart",
    "r",
    "scala",
    "clj",
    "ex",
    "exs",
    "erl",
    "hs",
    "ml",
    "nim",
    "zig",
    "v",
    "tex",
    "rst",
    "adoc",
    "org",
    "diff",
    "patch",
    "csproj",
    "sln",
    "props",
    "targets",
    "xaml",
    "resx",
    "plist",
    "srt",
    "vtt",
    "nfo",
    "reg",
    "inf",
    "manifest",
];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff"];
const AUDIO_EXT: &[&str] = &["mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "wma", "aiff", "aif"];
const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mpg", "mpeg", "3gp"];

/// Category by file name alone (`None` = decide from the content).
pub fn kind_by_name(name: &str) -> Option<Kind> {
    let lower = name.to_ascii_lowercase();
    if [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst"].iter().any(|s| lower.ends_with(s)) {
        return Some(Kind::Archive);
    }
    let ext = match preview_text::name_ext(&lower) {
        Some(e) => e.to_string(),
        None => {
            return match lower.as_str() {
                "makefile" | "dockerfile" | "license" | "readme" | "changelog" | "authors" | "copying" => {
                    Some(Kind::Text)
                }
                l if l.starts_with('.') => Some(Kind::Text),
                _ => None,
            };
        }
    };
    let e = ext.as_str();
    Some(match e {
        "md" | "markdown" | "mdown" | "mkd" => Kind::Markdown,
        "svg" => Kind::Svg,
        "pdf" => Kind::Pdf,
        "zip" | "jar" | "tar" | "tgz" | "tbz2" | "tbz" | "txz" | "tzst" | "7z" => Kind::Archive,
        _ if IMAGE_EXT.contains(&e) => Kind::Image,
        _ if AUDIO_EXT.contains(&e) => Kind::Audio,
        _ if VIDEO_EXT.contains(&e) => Kind::Video,
        _ if TEXT_EXT.contains(&e) => Kind::Text,
        _ => return None,
    })
}

/// Category from magic bytes.
pub fn sniff(head: &[u8]) -> Option<Kind> {
    let starts = |m: &[u8]| head.starts_with(m);
    if starts(b"\x89PNG") || starts(&[0xFF, 0xD8, 0xFF]) || starts(b"GIF8") || starts(b"BM") && head.len() > 26 {
        return Some(Kind::Image);
    }
    if starts(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Some(Kind::Image);
    }
    if starts(b"%PDF") {
        return Some(Kind::Pdf);
    }
    if starts(b"PK\x03\x04") || starts(b"7z\xBC\xAF\x27\x1C") {
        return Some(Kind::Archive);
    }
    if starts(b"ID3") || starts(b"fLaC") || starts(b"OggS") || starts(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        return Some(Kind::Audio);
    }
    if head.get(4..8) == Some(b"ftyp") || starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(Kind::Video);
    }
    None
}

/// Final category: `force_hex` wins, then the name, then magic bytes, then text sniffing.
pub fn detect(name: &str, is_dir: bool, head: &[u8], force_hex: bool) -> Kind {
    if is_dir {
        return Kind::Folder;
    }
    if force_hex {
        return Kind::Binary;
    }
    match kind_by_name(name) {
        // A ".txt" that is really binary still goes to hex.
        Some(Kind::Text) if !preview_text::looks_like_text(head) => Kind::Binary,
        Some(k) => k,
        None => sniff(head).unwrap_or(if preview_text::looks_like_text(head) { Kind::Text } else { Kind::Binary }),
    }
}

/// Per-request limits and styling inputs.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Longest image side after downscaling.
    pub max_image_px: u32,
    /// Images above this many megapixels are not decoded.
    pub max_image_mp: u32,
    pub max_text_bytes: usize,
    pub dark: bool,
    pub mono: FontId,
    pub fg: Color32,
    /// Rich previewer switches and caps (`preview_rich`).
    pub rich: super::preview_rich::RichLimits,
}

#[derive(Clone)]
pub struct ImageData {
    pub tex: TextureHandle,
    /// Original dimensions.
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct TextData {
    pub lines: Arc<Vec<LayoutJob>>,
    pub truncated: bool,
    pub language: Option<String>,
    pub encoding: &'static str,
    /// Characters per line of the longest line (for horizontal scrolling).
    pub widest: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ArchiveListing {
    pub format: String,
    /// `(path, is_dir, size)`, at most [`LISTING_CAP`].
    pub entries: Vec<(String, bool, u64)>,
    pub total_entries: usize,
    pub total_size: u64,
    pub compressed: u64,
    pub encrypted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FolderSummary {
    pub files: u64,
    pub dirs: u64,
    pub hidden: u64,
    pub links: u64,
    /// Bytes of the direct child files.
    pub direct_bytes: u64,
    /// Most common extensions `(ext, count)`, descending.
    pub top_ext: Vec<(String, u64)>,
    pub newest: Option<(String, SystemTime)>,
    pub error: Option<String>,
}

/// Initial state of a hex document.
#[derive(Clone)]
pub struct HexInit {
    pub doc: u64,
    pub len: u64,
    /// Pages are loaded from this file on demand (`None` = everything is in `pages`).
    pub path: Option<PathBuf>,
    pub pages: HashMap<u64, Arc<[u8]>>,
}

#[derive(Clone)]
pub enum Content {
    Image(ImageData),
    Svg {
        uri: String,
        bytes: Arc<[u8]>,
    },
    Text(TextData),
    Markdown {
        blocks: Arc<Vec<Block>>,
        truncated: bool,
    },
    Hex(HexInit),
    Archive(ArchiveListing),
    Folder(FolderSummary),
    Media {
        video: bool,
    },
    /// Office / EPUB / font / 3D / audio / video / PDF (`preview_rich`).
    Rich(super::preview_rich::Rich),
    /// Nothing to render but a message (e.g. too large, unsupported, error).
    Message {
        icon: &'static str,
        title: String,
        body: String,
    },
}

/// One finished load.
#[derive(Clone)]
pub struct Loaded {
    pub kind: Kind,
    pub content: Content,
    /// Extra info rows (dimensions, codec, duration, …).
    pub facts: Vec<(String, String)>,
    /// Fresh metadata (created / accessed / owner) when available.
    pub meta: Option<EntryInfo>,
}

pub enum Msg {
    Loaded { id: u64, loaded: Box<Loaded> },
    Page { doc: u64, page: u64, bytes: Option<Arc<[u8]>> },
}

struct LoadJob {
    id: u64,
    latest: Arc<AtomicU64>,
    source: Source,
    is_dir: bool,
    force_hex: bool,
    limits: Limits,
    tx: Sender<Msg>,
    ctx: Context,
}

struct PageJob {
    doc: u64,
    path: PathBuf,
    page: u64,
    tx: Sender<Msg>,
    ctx: Context,
}

struct Pool {
    loads: Sender<LoadJob>,
    pages: Sender<PageJob>,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let (ltx, lrx) = unbounded::<LoadJob>();
        let (ptx, prx) = unbounded::<PageJob>();
        for i in 0..2 {
            let rx = lrx.clone();
            let _ = std::thread::Builder::new().name(format!("soyuz-preview-{i}")).spawn(move || {
                for job in rx.iter() {
                    run_load(job);
                }
            });
        }
        let _ = std::thread::Builder::new().name("soyuz-hex".into()).spawn(move || {
            for job in prx.iter() {
                // A panic must not kill the only hex worker; answer with "no bytes".
                let bytes =
                    std::panic::catch_unwind(AssertUnwindSafe(|| read_page(&job.path, job.page).ok())).ok().flatten();
                let _ = job.tx.send(Msg::Page { doc: job.doc, page: job.page, bytes });
                job.ctx.request_repaint();
            }
        });
        Pool { loads: ltx, pages: ptx }
    })
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A preview consumer (the preview pane, Quick Look).
pub struct PreviewClient {
    latest: Arc<AtomicU64>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: Context,
}

impl PreviewClient {
    pub fn new(ctx: &Context) -> Self {
        let (tx, rx) = unbounded();
        PreviewClient { latest: Arc::new(AtomicU64::new(0)), tx, rx, ctx: ctx.clone() }
    }

    /// Request a preview; every older request of this client becomes stale.
    pub fn request(&self, source: Source, is_dir: bool, force_hex: bool, limits: Limits) -> u64 {
        let id = next_id();
        self.latest.store(id, Ordering::SeqCst);
        let job = LoadJob {
            id,
            latest: self.latest.clone(),
            source,
            is_dir,
            force_hex,
            limits,
            tx: self.tx.clone(),
            ctx: self.ctx.clone(),
        };
        let _ = pool().loads.send(job);
        id
    }

    /// Make every pending request stale.
    pub fn cancel(&self) {
        self.latest.store(next_id(), Ordering::SeqCst);
    }

    /// Ask for one hex page of `path`.
    pub fn page(&self, doc: u64, path: &Path, page: u64) {
        let job = PageJob { doc, path: path.to_path_buf(), page, tx: self.tx.clone(), ctx: self.ctx.clone() };
        let _ = pool().pages.send(job);
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    pub fn latest(&self) -> u64 {
        self.latest.load(Ordering::SeqCst)
    }
}

fn read_page(path: &Path, page: u64) -> std::io::Result<Arc<[u8]>> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(page * HEX_PAGE))?;
    let mut buf = Vec::with_capacity(HEX_PAGE as usize);
    f.take(HEX_PAGE).read_to_end(&mut buf)?;
    Ok(buf.into())
}

// ---- loading ------------------------------------------------------------------------

fn run_load(job: LoadJob) {
    let stale = || job.latest.load(Ordering::SeqCst) != job.id;
    if stale() {
        return;
    }
    // A parser bug (hostile file) must not kill the worker thread or leave the pane on
    // "Loading…" forever: turn a panic into an error preview.
    let loaded = match std::panic::catch_unwind(AssertUnwindSafe(|| load(&job, &stale))) {
        Ok(l) => l,
        Err(e) => {
            let why = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| tr!("services-preview-internal-error").into());
            message(
                Kind::Binary,
                crate::theme::icons::ph::WARNING_CIRCLE,
                tr!("services-preview-failed"),
                tr!("services-preview-crashed", reason = why),
            )
        }
    };
    if stale() {
        return;
    }
    let _ = job.tx.send(Msg::Loaded { id: job.id, loaded: Box::new(loaded) });
    job.ctx.request_repaint();
}

fn message(kind: Kind, icon: &'static str, title: impl Into<String>, body: impl Into<String>) -> Loaded {
    Loaded {
        kind,
        content: Content::Message { icon, title: title.into(), body: body.into() },
        facts: Vec::new(),
        meta: None,
    }
}

/// A readable, seekable view of the source plus its length.
enum Opened {
    File(File, u64),
    Mem(Cursor<Vec<u8>>, u64, bool),
}

impl Opened {
    fn len(&self) -> u64 {
        match self {
            Opened::File(_, n) | Opened::Mem(_, n, _) => *n,
        }
    }
    fn reader(&mut self) -> &mut dyn ReadSeek {
        match self {
            Opened::File(f, _) => f,
            Opened::Mem(c, _, _) => c,
        }
    }
    fn head(&mut self, n: usize) -> Vec<u8> {
        let r = self.reader();
        let _ = r.seek(SeekFrom::Start(0));
        let mut buf = Vec::with_capacity(n);
        let _ = r.take(n as u64).read_to_end(&mut buf);
        let _ = r.seek(SeekFrom::Start(0));
        buf
    }
    fn read_up_to(&mut self, n: u64) -> Vec<u8> {
        let r = self.reader();
        let _ = r.seek(SeekFrom::Start(0));
        let mut buf = Vec::new();
        let _ = r.take(n).read_to_end(&mut buf);
        buf
    }
}

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

fn open_source(source: &Source) -> Result<Opened, String> {
    match source {
        Source::File(p) => {
            let f = File::open(p).map_err(|e| e.to_string())?;
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            Ok(Opened::File(f, len))
        }
        Source::Entry { archive, inner } => {
            let bytes = read_archive_entry(archive, inner, ENTRY_CAP)?;
            let (data, truncated) = bytes;
            let len = data.len() as u64;
            Ok(Opened::Mem(Cursor::new(data), len, truncated))
        }
    }
}

/// Read an archive entry (up to `cap` bytes); `(bytes, truncated)`.
pub fn read_archive_entry(archive: &Path, inner: &str, cap: u64) -> Result<(Vec<u8>, bool), String> {
    let password = super::archive::password_for(archive);
    let mut reader = soyuz_archive::open(archive, password.as_deref()).map_err(|e| e.to_string())?;
    let idx = soyuz_archive::find_entry(reader.as_mut(), inner)
        .ok_or_else(|| tr!("services-preview-entry-not-found").to_string())?;
    let mut r = reader.open_entry(idx).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.by_ref().take(cap).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let mut probe = [0u8; 1];
    let truncated = buf.len() as u64 == cap && r.read(&mut probe).map(|n| n > 0).unwrap_or(false);
    Ok((buf, truncated))
}

fn load(job: &LoadJob, stale: &dyn Fn() -> bool) -> Loaded {
    use crate::theme::icons::ph;
    let name = job.source.name();
    if job.is_dir {
        return folder(&job.source);
    }
    // Script previewers (F6) take precedence for their file types.
    if let Source::File(p) = &job.source
        && let Some(r) = crate::automation::scripts::preview(p)
    {
        return script_preview(r);
    }
    let meta = match &job.source {
        Source::File(p) => soyuz_fs::stat(p, true).ok(),
        Source::Entry { .. } => None,
    };
    let mut opened = match open_source(&job.source) {
        Ok(o) => o,
        Err(e) => return message(Kind::Binary, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-file"), e),
    };
    let head = opened.head(8192);
    let kind = detect(&name, false, &head, job.force_hex);
    if !job.force_hex
        && let Some(rk) = super::preview_rich::route(&name, &head, kind, &job.limits.rich)
    {
        let len = opened.len();
        let mut out = super::preview_rich::load(
            rk,
            &job.source,
            opened.reader(),
            len,
            &name,
            &job.ctx,
            job.id,
            &job.limits,
            stale,
        );
        if let Opened::Mem(_, _, true) = opened {
            out.facts.push(("Note".into(), tr!("services-preview-limited").into()));
        }
        out.meta = out.meta.or(meta);
        return out;
    }
    let mut out = match kind {
        Kind::Image => image(&mut opened, &name, job, stale),
        Kind::Svg => {
            let bytes = opened.read_up_to(16 * 1024 * 1024);
            Loaded {
                kind,
                content: Content::Svg { uri: format!("bytes://preview/{}/{name}", job.id), bytes: bytes.into() },
                facts: Vec::new(),
                meta: None,
            }
        }
        Kind::Text | Kind::Markdown => text(&mut opened, &name, kind, job, stale),
        Kind::Archive => archive(&job.source, &mut opened),
        Kind::Audio | Kind::Video => {
            let ext = preview_text::name_ext(&name).map(str::to_ascii_lowercase).unwrap_or_default();
            let len = opened.len();
            let facts = preview_media::probe(opened.reader(), len, &ext);
            Loaded { kind, content: Content::Media { video: kind == Kind::Video }, facts, meta: None }
        }
        Kind::Pdf => {
            let mut l = message(kind, ph::FILE_PDF, tr!("services-kind-pdf"), tr!("services-preview-pdf-no-feature"));
            let head = String::from_utf8_lossy(&head[..head.len().min(16)]).into_owned();
            if let Some(v) = head.strip_prefix("%PDF-") {
                l.facts.push(("PDF version".into(), v.chars().take(3).collect()));
            }
            l
        }
        Kind::Folder
        | Kind::Binary
        | Kind::Document
        | Kind::Spreadsheet
        | Kind::Presentation
        | Kind::Ebook
        | Kind::Font
        | Kind::Model => hex(&job.source, &mut opened),
    };
    if let Opened::Mem(_, _, true) = opened {
        out.facts.push(("Note".into(), tr!("services-preview-limited").into()));
    }
    if out.meta.is_none() {
        out.meta = meta;
    }
    out
}

fn hex(source: &Source, opened: &mut Opened) -> Loaded {
    let doc = next_id();
    let len = opened.len();
    let (path, pages) = match (source, opened) {
        (Source::File(p), o) => {
            let first = o.head(HEX_PAGE as usize);
            (Some(p.clone()), HashMap::from([(0u64, Arc::<[u8]>::from(first))]))
        }
        (Source::Entry { .. }, Opened::Mem(c, _, _)) => {
            let data = c.get_ref();
            let pages = data
                .chunks(HEX_PAGE as usize)
                .enumerate()
                .map(|(i, ch)| (i as u64, Arc::<[u8]>::from(ch)))
                .collect::<HashMap<_, _>>();
            (None, pages)
        }
        (Source::Entry { .. }, o) => {
            let first = o.head(HEX_PAGE as usize);
            (None, HashMap::from([(0u64, Arc::<[u8]>::from(first))]))
        }
    };
    Loaded {
        kind: Kind::Binary,
        content: Content::Hex(HexInit { doc, len, path, pages }),
        facts: Vec::new(),
        meta: None,
    }
}

fn image(opened: &mut Opened, name: &str, job: &LoadJob, stale: &dyn Fn() -> bool) -> Loaded {
    use crate::theme::icons::ph;
    let limits = &job.limits;
    let reader: Box<dyn ReadSeek> = match opened {
        Opened::File(f, _) => match f.try_clone() {
            Ok(mut c) => {
                let _ = c.seek(SeekFrom::Start(0));
                Box::new(std::io::BufReader::new(c))
            }
            Err(e) => {
                return message(
                    Kind::Image,
                    ph::WARNING_CIRCLE,
                    tr!("services-preview-cant-read-image"),
                    e.to_string(),
                );
            }
        },
        Opened::Mem(c, _, _) => Box::new(Cursor::new(c.get_ref().clone())),
    };
    let reader = match image::ImageReader::new(std::io::BufReader::new(reader)).with_guessed_format() {
        Ok(r) => r,
        Err(e) => {
            return message(Kind::Image, ph::WARNING_CIRCLE, tr!("services-preview-cant-read-image"), e.to_string());
        }
    };
    let format = reader.format().map(|f| format!("{f:?}").to_uppercase());
    let mut reader = reader;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some((limits.max_image_mp.max(1) as u64) * 1_000_000 * 4 + 64 * 1024 * 1024);
    reader.limits(lim);
    let mut decoder = match reader.into_decoder() {
        Ok(d) => d,
        Err(e) => {
            return message(Kind::Image, ph::WARNING_CIRCLE, tr!("services-preview-unsupported-image"), e.to_string());
        }
    };
    let (w, h) = image::ImageDecoder::dimensions(&decoder);
    let mut facts = vec![("Dimensions".to_string(), format!("{w} × {h} px"))];
    if let Some(f) = format {
        facts.push(("Format".into(), f));
    }
    let color = format!("{:?}", image::ImageDecoder::color_type(&decoder));
    facts.push(("Color".into(), color));
    let mp = w as u64 * h as u64;
    if mp > limits.max_image_mp as u64 * 1_000_000 {
        let mut l = message(
            Kind::Image,
            ph::IMAGE,
            tr!("services-preview-image-too-large"),
            tr!(
                "services-preview-image-too-large-body",
                mp = format!("{:.0}", mp as f64 / 1e6),
                limit = limits.max_image_mp.to_string()
            ),
        );
        l.facts = facts;
        return l;
    }
    if stale() {
        return message(Kind::Image, ph::IMAGE, "", "");
    }
    let orientation = image::ImageDecoder::orientation(&mut decoder).ok();
    let mut img = match image::DynamicImage::from_decoder(decoder) {
        Ok(i) => i,
        Err(e) => {
            return message(Kind::Image, ph::WARNING_CIRCLE, tr!("services-preview-cant-decode-image"), e.to_string());
        }
    };
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    if stale() {
        return message(Kind::Image, ph::IMAGE, "", "");
    }
    let max = limits.max_image_px.max(64);
    if img.width() > max || img.height() > max {
        img = img.resize(max, max, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let tex = job.ctx.load_texture(
        format!("preview:{name}:{}", job.id),
        ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        TextureOptions::LINEAR,
    );
    Loaded { kind: Kind::Image, content: Content::Image(ImageData { tex, width: w, height: h }), facts, meta: None }
}

/// A script previewer's result as preview content.
fn script_preview(r: Result<soyuz_script::Preview, soyuz_script::ScriptError>) -> Loaded {
    use crate::theme::icons::ph;
    let facts = vec![("Previewer".to_string(), tr!("services-preview-script").to_string())];
    match r {
        Ok(soyuz_script::Preview::Markdown(md)) => Loaded {
            kind: Kind::Markdown,
            content: Content::Markdown { blocks: Arc::new(preview_text::parse_markdown(&md)), truncated: false },
            facts,
            meta: None,
        },
        Ok(soyuz_script::Preview::Text(t)) => {
            let fence = if t.contains("```") { "~~~~" } else { "```" };
            let md = format!(
                "{fence}
{t}
{fence}"
            );
            Loaded {
                kind: Kind::Text,
                content: Content::Markdown { blocks: Arc::new(preview_text::parse_markdown(&md)), truncated: false },
                facts,
                meta: None,
            }
        }
        Err(e) => {
            let mut l = message(Kind::Text, ph::WARNING_CIRCLE, tr!("services-preview-script-failed"), e.to_string());
            l.facts = facts;
            l
        }
    }
}

fn text(opened: &mut Opened, name: &str, kind: Kind, job: &LoadJob, stale: &dyn Fn() -> bool) -> Loaded {
    let cap = job.limits.max_text_bytes.max(4096) as u64;
    let len = opened.len();
    let bytes = opened.read_up_to(cap);
    let truncated = len > bytes.len() as u64;
    let (text, encoding) = preview_text::decode_text(&bytes);
    let mut facts = vec![("Encoding".to_string(), encoding.to_string())];
    let line_count = text.lines().count();
    facts.push(("Lines".into(), if truncated { format!("{line_count}+") } else { line_count.to_string() }));
    if kind == Kind::Markdown {
        return Loaded {
            kind,
            content: Content::Markdown { blocks: Arc::new(preview_text::parse_markdown(&text)), truncated },
            facts,
            meta: None,
        };
    }
    let lines = preview_text::highlight(
        &text,
        name,
        job.limits.dark,
        job.limits.mono.clone(),
        job.limits.fg,
        Duration::from_millis(400),
        stale,
    );
    let widest = lines.iter().map(|j| j.text.chars().count()).max().unwrap_or(0);
    let language = preview_text::language_name(name);
    if let Some(l) = &language {
        facts.insert(0, ("Language".into(), l.clone()));
    }
    Loaded {
        kind,
        content: Content::Text(TextData { lines: Arc::new(lines), truncated, language, encoding, widest }),
        facts,
        meta: None,
    }
}

fn archive(source: &Source, opened: &mut Opened) -> Loaded {
    use crate::theme::icons::ph;
    let Source::File(path) = source else {
        let _ = opened;
        return message(
            Kind::Archive,
            ph::FILE_ZIP,
            tr!("services-preview-nested-archive"),
            tr!("services-preview-nested-archive-body"),
        );
    };
    let password = super::archive::password_for(path);
    let mut reader = match soyuz_archive::open(path, password.as_deref()) {
        Ok(r) => r,
        Err(soyuz_archive::ArchiveError::PasswordRequired) => {
            return message(
                Kind::Archive,
                ph::LOCK_SIMPLE,
                tr!("services-preview-password"),
                tr!("services-preview-password-body"),
            );
        }
        Err(e) => {
            return message(
                Kind::Archive,
                ph::WARNING_CIRCLE,
                tr!("services-preview-cant-read-archive"),
                e.to_string(),
            );
        }
    };
    let format = format!("{:?}", reader.format());
    let entries = match reader.entries() {
        Ok(e) => e,
        Err(soyuz_archive::ArchiveError::PasswordRequired) => {
            return message(
                Kind::Archive,
                ph::LOCK_SIMPLE,
                tr!("services-preview-password"),
                tr!("services-preview-password-body"),
            );
        }
        Err(e) => {
            return message(
                Kind::Archive,
                ph::WARNING_CIRCLE,
                tr!("services-preview-cant-read-archive"),
                e.to_string(),
            );
        }
    };
    let mut l = ArchiveListing { format: format.clone(), total_entries: entries.len(), ..Default::default() };
    for e in entries {
        l.total_size += e.size;
        l.compressed += e.compressed;
        l.encrypted |= e.encrypted;
        if l.entries.len() < LISTING_CAP {
            l.entries.push((e.path.clone(), e.is_dir, e.size));
        }
    }
    // Archives without explicit folder entries still show their folders.
    let mut seen: std::collections::HashSet<String> =
        l.entries.iter().filter(|e| e.1).map(|e| e.0.trim_end_matches('/').to_string()).collect();
    let mut implicit = Vec::new();
    for (p, _, _) in &l.entries {
        let mut acc = String::new();
        let parts: Vec<&str> = p.trim_end_matches('/').split('/').collect();
        for seg in &parts[..parts.len().saturating_sub(1)] {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            if seen.insert(acc.clone()) {
                implicit.push((format!("{acc}/"), true, 0));
            }
        }
    }
    l.entries.extend(implicit);
    l.entries.sort_by_key(|a| a.0.trim_end_matches('/').to_lowercase());
    let mut facts =
        vec![("Format".to_string(), format), ("Entries".into(), soyuz_core::locale::group_int(l.total_entries as u64))];
    if l.total_size > 0 && l.compressed > 0 {
        let ratio = 100.0 - (l.compressed as f64 / l.total_size as f64 * 100.0);
        facts.push(("Compression".into(), tr!("services-fact-compression-saved", ratio = format!("{ratio:.0}"))));
    }
    if l.encrypted {
        facts.push(("Encrypted".into(), tr!("common-yes").into()));
    }
    Loaded { kind: Kind::Archive, content: Content::Archive(l), facts, meta: None }
}

/// Summarize a folder (direct children only; the recursive size comes from
/// `soyuz_search::folder_sizes`, driven by the UI).
pub fn summarize(entries: &[EntryInfo]) -> FolderSummary {
    let mut s = FolderSummary::default();
    let mut ext: HashMap<String, u64> = HashMap::new();
    for e in entries {
        if e.is_hidden() {
            s.hidden += 1;
        }
        match e.kind {
            EntryKind::Symlink { .. } | EntryKind::Junction => s.links += 1,
            _ => {}
        }
        if e.is_dir_like() {
            s.dirs += 1;
        } else {
            s.files += 1;
            s.direct_bytes += e.size;
            if let Some(x) = e.extension() {
                *ext.entry(x.to_ascii_lowercase()).or_default() += 1;
            }
        }
        if let Some(m) = e.modified
            && s.newest.as_ref().is_none_or(|(_, t)| m > *t)
        {
            s.newest = Some((e.name.clone(), m));
        }
    }
    let mut top: Vec<(String, u64)> = ext.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(6);
    s.top_ext = top;
    s
}

fn folder(source: &Source) -> Loaded {
    let result: Result<Vec<EntryInfo>, String> = match source {
        Source::File(p) => {
            let opts = ListOptions { include_hidden: true, ..Default::default() };
            soyuz_fs::read_dir(p, &opts).map_err(|e| e.to_string())
        }
        Source::Entry { archive, inner } => {
            let password = super::archive::password_for(archive);
            soyuz_archive::open(archive, password.as_deref())
                .and_then(|mut r| soyuz_archive::list_dir(r.as_mut(), inner))
                .map_err(|e| e.to_string())
        }
    };
    let meta = match source {
        Source::File(p) => soyuz_fs::stat(p, true).ok(),
        Source::Entry { .. } => None,
    };
    let summary = match result {
        Ok(entries) => summarize(&entries),
        Err(e) => FolderSummary { error: Some(e), ..Default::default() },
    };
    Loaded { kind: Kind::Folder, content: Content::Folder(summary), facts: Vec::new(), meta }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn detection_by_name_and_magic() {
        assert_eq!(detect("a.PNG", false, b"", false), Kind::Image);
        assert_eq!(detect("a.svg", false, b"<svg", false), Kind::Svg);
        assert_eq!(detect("README.md", false, b"# x", false), Kind::Markdown);
        assert_eq!(detect("x.tar.gz", false, b"", false), Kind::Archive);
        assert_eq!(detect("main.rs", false, b"fn main(){}", false), Kind::Text);
        assert_eq!(detect("main.rs", false, b"fn main(){}", true), Kind::Binary, "force hex");
        assert_eq!(detect("dir", true, b"", false), Kind::Folder);
        assert_eq!(detect("noext", false, b"\x89PNG\r\n", false), Kind::Image);
        assert_eq!(detect("noext", false, b"plain words", false), Kind::Text);
        assert_eq!(detect("noext", false, &[0, 1, 2, 3, 0xFF], false), Kind::Binary);
        assert_eq!(detect("fake.txt", false, &[0, 1, 2, 0, 0], false), Kind::Binary);
        assert_eq!(detect("song.mp3", false, b"", false), Kind::Audio);
        assert_eq!(detect("clip.MKV", false, b"", false), Kind::Video);
        assert_eq!(detect("blob", false, b"%PDF-1.7", false), Kind::Pdf);
        assert_eq!(detect("Makefile", false, b"all:", false), Kind::Text);
        assert_eq!(detect(".gitignore", false, b"target", false), Kind::Text);
    }

    #[test]
    fn source_from_vpath() {
        let s = Source::from_path(Path::new("C:/x/a.zip!/dir/f.txt"));
        assert_eq!(s, Source::Entry { archive: PathBuf::from("C:/x/a.zip"), inner: "dir/f.txt".into() });
        assert_eq!(s.name(), "f.txt");
        assert_eq!(Source::from_path(Path::new("C:/x/a.txt")), Source::File(PathBuf::from("C:/x/a.txt")));
    }

    #[test]
    fn folder_summary_counts() {
        let mk = |n: &str, k: EntryKind, size: u64| {
            let mut e = EntryInfo::minimal(PathBuf::from("/d").join(n), k);
            e.size = size;
            e.modified = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(size));
            e
        };
        let s = summarize(&[
            mk("a.rs", EntryKind::File, 10),
            mk("b.rs", EntryKind::File, 20),
            mk("c.md", EntryKind::File, 5),
            mk(".hidden", EntryKind::File, 1),
            mk("sub", EntryKind::Dir, 0),
        ]);
        assert_eq!((s.files, s.dirs, s.hidden), (4, 1, 1));
        assert_eq!(s.direct_bytes, 36);
        assert_eq!(s.top_ext[0], ("rs".to_string(), 2));
        assert_eq!(s.newest.as_ref().unwrap().0, "b.rs");
    }

    fn limits() -> Limits {
        Limits {
            max_image_px: 256,
            max_image_mp: 40,
            max_text_bytes: 512 * 1024,
            dark: true,
            mono: FontId::monospace(12.0),
            fg: Color32::WHITE,
            rich: Default::default(),
        }
    }

    fn wait(client: &PreviewClient) -> Loaded {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if let Some(Msg::Loaded { loaded, .. }) = client.try_recv() {
                return *loaded;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("preview timed out");
    }

    #[test]
    fn loads_image_text_and_hex_off_thread() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::default();
        let client = PreviewClient::new(&ctx);

        let png = dir.path().join("big.png");
        image::RgbaImage::from_pixel(1200, 600, image::Rgba([1, 2, 3, 255])).save(&png).unwrap();
        client.request(Source::File(png), false, false, limits());
        let l = wait(&client);
        match l.content {
            Content::Image(img) => {
                assert_eq!((img.width, img.height), (1200, 600));
                assert_eq!(img.tex.size(), [256, 128], "downscaled on the worker");
            }
            _ => panic!("expected an image"),
        }

        let txt = dir.path().join("a.rs");
        std::fs::write(&txt, "fn main() {}\n// two\n").unwrap();
        client.request(Source::File(txt.clone()), false, false, limits());
        match wait(&client).content {
            Content::Text(t) => {
                assert_eq!(t.lines.len(), 2);
                assert_eq!(t.language.as_deref(), Some("Rust"));
            }
            _ => panic!("expected text"),
        }

        client.request(Source::File(txt), false, true, limits());
        match wait(&client).content {
            Content::Hex(h) => {
                assert_eq!(h.len, 20);
                assert_eq!(&h.pages[&0][..2], b"fn");
            }
            _ => panic!("expected hex"),
        }
    }

    #[test]
    fn stale_requests_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::default();
        let client = PreviewClient::new(&ctx);
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "aaa").unwrap();
        std::fs::write(&b, "bbb").unwrap();
        client.request(Source::File(a), false, false, limits());
        let second = client.request(Source::File(b), false, false, limits());
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < Duration::from_secs(10) {
            if let Some(Msg::Loaded { id, .. }) = client.try_recv() {
                seen.push(id);
                if id == second {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen.last(), Some(&second));
        assert!(seen.len() <= 2);
    }

    #[test]
    fn hex_pages_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("blob.bin");
        let data: Vec<u8> = (0..(HEX_PAGE as usize * 2 + 10)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let ctx = Context::default();
        let client = PreviewClient::new(&ctx);
        client.page(7, &p, 2);
        let start = Instant::now();
        loop {
            if let Some(Msg::Page { doc, page, bytes }) = client.try_recv() {
                assert_eq!((doc, page), (7, 2));
                let b = bytes.unwrap();
                assert_eq!(b.len(), 10);
                assert_eq!(b[0], data[HEX_PAGE as usize * 2]);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
