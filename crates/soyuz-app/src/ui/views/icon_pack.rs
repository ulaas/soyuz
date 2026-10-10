//! Icon packs: picture icons for file types, kinds and named folders,
//! `[appearance] icon_pack`.
//!
//! * `""`: the built-in Phosphor glyphs (default; nothing is loaded).
//! * `"flat"`: a small built-in pack generated in code (document / folder silhouettes
//!   tinted with the theme's `glyph.*` colors), so it needs no image assets.
//! * anything else: the folder `icons/<name>/` in the config dir with a `pack.toml`:
//!
//! ```toml
//! name = "My icons"            # shown in Settings (default: the folder name)
//! tint = false                 # true = pictures are white masks tinted by file class
//! [kinds]                      # file classes: folder, image, video, audio, archive, code,
//! folder = "folder.svg"        # pdf, doc, sheet, slides, text, exe, font, link, file
//! file = "file.png"            # (`file` = everything else)
//! [extensions]
//! rs = "rust.png"
//! [folders]                    # folder names, case-insensitive
//! ".git" = "git.svg"
//! ```
//!
//! Pictures are PNG / JPEG / WebP / BMP / ICO or SVG (rasterized). Loading runs on a
//! worker thread (never the UI thread); until it finishes, and for anything the pack
//! does not cover, the Phosphor glyphs show.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, ColorImage, Context, Painter, Pos2, Rect, TextureHandle, TextureOptions, pos2, vec2};
use parking_lot::{Mutex, RwLock};
use serde::Deserialize;
use soyuz_core::EntryInfo;

use super::icons::{self, FileClass};
use crate::theme::Palette;

/// Id of the generated built-in pack.
pub const FLAT: &str = "flat";
/// Edge (pixels) pictures are rasterized / scaled to.
const PX: u32 = 96;
/// Largest picture file read.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// At most this many pictures per pack.
const MAX_ICONS: usize = 512;

const CLASS_KEYS: [(&str, FileClass); 15] = [
    ("folder", FileClass::Folder),
    ("image", FileClass::Image),
    ("video", FileClass::Video),
    ("audio", FileClass::Audio),
    ("archive", FileClass::Archive),
    ("code", FileClass::Code),
    ("pdf", FileClass::Pdf),
    ("doc", FileClass::Doc),
    ("sheet", FileClass::Sheet),
    ("slides", FileClass::Slides),
    ("text", FileClass::Text),
    ("exe", FileClass::Exe),
    ("font", FileClass::Font),
    ("link", FileClass::Link),
    ("file", FileClass::Other),
];

/// A loaded pack.
#[derive(Default)]
pub struct Loaded {
    pub id: String,
    pub tint: bool,
    by_ext: HashMap<String, TextureHandle>,
    by_folder: HashMap<String, TextureHandle>,
    by_class: HashMap<FileClass, TextureHandle>,
}

impl Loaded {
    fn lookup(&self, e: &EntryInfo) -> Option<&TextureHandle> {
        let class = icons::classify(e);
        if class == FileClass::Folder {
            return self.by_folder.get(&e.name.to_lowercase()).or_else(|| self.by_class.get(&FileClass::Folder));
        }
        e.extension()
            .and_then(|x| self.by_ext.get(&x.to_ascii_lowercase()))
            .or_else(|| self.by_class.get(&class))
            .or_else(|| self.by_class.get(&FileClass::Other))
    }

    /// The picture for a file class (Settings preview).
    pub fn class_texture(&self, class: FileClass) -> Option<&TextureHandle> {
        self.by_class.get(&class).or_else(|| self.by_class.get(&FileClass::Other))
    }
}

static ACTIVE: RwLock<Option<Arc<Loaded>>> = RwLock::new(None);
/// The id last asked for (`apply`), so a slow load of an older choice is dropped.
static WANTED: Mutex<String> = Mutex::new(String::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The active pack, when one is loaded.
pub fn active() -> Option<Arc<Loaded>> {
    ACTIVE.read().clone()
}

/// The folder user packs live in (`<config>/icons`).
pub fn packs_dir() -> PathBuf {
    soyuz_core::config::config_dir().join("icons")
}

/// Switch packs (startup and every config change). Loads in the background; a no-op
/// when `id` is already active or being loaded.
pub fn apply(ctx: &Context, id: &str) {
    let id = id.trim().to_string();
    {
        // WANTED only changes here: equal = already active, loading, or failed for it.
        let mut w = WANTED.lock();
        if *w == id {
            return;
        }
        *w = id.clone();
    }
    if id.is_empty() {
        *ACTIVE.write() = None;
        ctx.request_repaint();
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("soyuz-icon-pack".into()).spawn(move || {
        let loaded = if id == FLAT { Ok(flat(&ctx)) } else { load_dir(&ctx, &packs_dir().join(&id), &id) };
        if GENERATION.load(Ordering::Relaxed) != generation || *WANTED.lock() != id {
            return;
        }
        match loaded {
            Ok(l) => *ACTIVE.write() = Some(Arc::new(l)),
            Err(e) => {
                log::warn!("icon pack “{id}”: {e}; using the built-in icons");
                *ACTIVE.write() = None;
            }
        }
        ctx.request_repaint();
    });
    if let Err(e) = spawned {
        log::warn!("cannot start the icon pack loader: {e}");
    }
}

/// Paint the pack's picture for `e` centered at `center`, `size` points square.
/// `false` = the pack has none (or no pack is active): paint the glyph instead.
pub fn paint(painter: &Painter, center: Pos2, size: f32, e: &EntryInfo, p: &Palette, alpha: f32) -> bool {
    let guard = ACTIVE.read();
    let Some(pack) = guard.as_ref() else { return false };
    let Some(tex) = pack.lookup(e) else { return false };
    let tint = if pack.tint { icons::color(e, p) } else { Color32::WHITE };
    let rect = Rect::from_center_size(center, vec2(size, size));
    painter.image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint.gamma_multiply(alpha));
    true
}

/// `(id, display name)` of the packs offered in Settings: the built-in ones, then the
/// user packs found by the last [`scan`] (refreshed in the background).
pub fn available() -> Vec<(String, String)> {
    let mut v = vec![(String::new(), tr!("settings-appearance-icon-pack-default").to_string())];
    v.push((FLAT.to_string(), tr!("settings-appearance-icon-pack-flat").to_string()));
    v.extend(SCANNED.lock().1.iter().cloned());
    v
}

/// `(id, display name)` of the user packs.
type PackList = Vec<(String, String)>;

/// `(last scan, packs)`.
static SCANNED: Mutex<(Option<std::time::Instant>, PackList)> = Mutex::new((None, Vec::new()));

/// Refresh the user pack list on a worker, at most every few seconds.
pub fn scan(ctx: &Context) {
    {
        let mut s = SCANNED.lock();
        if s.0.is_some_and(|t| t.elapsed().as_secs() < 3) {
            return;
        }
        s.0 = Some(std::time::Instant::now());
    }
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("soyuz-icon-pack-scan".into()).spawn(move || {
        let found = list_packs(&packs_dir());
        let changed = {
            let mut s = SCANNED.lock();
            let changed = s.1 != found;
            s.1 = found;
            changed
        };
        if changed {
            ctx.request_repaint();
        }
    });
}

/// User packs in `dir`: sub-folders with a `pack.toml` (name from the file, else the folder).
pub fn list_packs(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|d| {
            let id = d.file_name().to_string_lossy().into_owned();
            if id == FLAT || id.is_empty() {
                return None;
            }
            let text = std::fs::read_to_string(d.path().join("pack.toml")).ok()?;
            let name = toml::from_str::<PackToml>(&text).ok().and_then(|p| p.name).unwrap_or_else(|| id.clone());
            Some((id, name))
        })
        .collect();
    out.sort_by_key(|p| p.1.to_lowercase());
    out
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PackToml {
    name: Option<String>,
    tint: bool,
    kinds: HashMap<String, String>,
    extensions: HashMap<String, String>,
    folders: HashMap<String, String>,
}

/// Read and decode a user pack (worker thread).
pub fn load_dir(ctx: &Context, dir: &Path, id: &str) -> Result<Loaded, String> {
    let text = std::fs::read_to_string(dir.join("pack.toml")).map_err(|e| format!("{}: {e}", dir.display()))?;
    let spec: PackToml = toml::from_str(&text).map_err(|e| format!("pack.toml: {e}"))?;
    let mut cache: HashMap<String, Option<TextureHandle>> = HashMap::new();
    let mut picture = |file: &str| -> Option<TextureHandle> {
        if let Some(t) = cache.get(file) {
            return t.clone();
        }
        if cache.len() >= MAX_ICONS {
            return None;
        }
        let tex = resolve(dir, file).and_then(|p| match decode_file(&p) {
            Ok(img) => Some(ctx.load_texture(format!("icon-pack:{id}:{file}"), img, TextureOptions::LINEAR)),
            Err(e) => {
                log::warn!("icon pack “{id}”: {}: {e}", p.display());
                None
            }
        });
        cache.insert(file.to_string(), tex.clone());
        tex
    };
    let mut l = Loaded { id: id.to_string(), tint: spec.tint, ..Default::default() };
    for (k, f) in &spec.kinds {
        match CLASS_KEYS.iter().find(|(n, _)| n.eq_ignore_ascii_case(k.trim())) {
            Some((_, class)) => {
                if let Some(t) = picture(f) {
                    l.by_class.insert(*class, t);
                }
            }
            None => log::warn!("icon pack “{id}”: unknown kind “{k}”"),
        }
    }
    for (x, f) in &spec.extensions {
        if let Some(t) = picture(f) {
            l.by_ext.insert(x.trim().trim_start_matches('.').to_ascii_lowercase(), t);
        }
    }
    for (n, f) in &spec.folders {
        if let Some(t) = picture(f) {
            l.by_folder.insert(n.trim().to_lowercase(), t);
        }
    }
    Ok(l)
}

/// A pack-relative picture path; refuses anything that leaves the pack folder.
fn resolve(dir: &Path, file: &str) -> Option<PathBuf> {
    // Packs are shared between OSes: accept `\` separators and reject drive prefixes everywhere.
    let file = file.trim().replace('\\', "/");
    let rel = Path::new(&file);
    let ok = !rel.as_os_str().is_empty()
        && !file.contains(':')
        && rel.components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir));
    ok.then(|| dir.join(rel))
}

fn decode_file(path: &Path) -> Result<ColorImage, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_FILE {
        return Err("larger than 4 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let is_svg = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("svg"));
    decode(&bytes, is_svg)
}

/// Decode PNG / JPEG / ... or rasterize SVG, fitted into `PX`×`PX`.
pub fn decode(bytes: &[u8], svg: bool) -> Result<ColorImage, String> {
    if svg {
        let opts = resvg::usvg::Options::default();
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            egui_extras::image::load_svg_bytes_with_size(
                bytes,
                egui::SizeHint::Size { width: PX, height: PX, maintain_aspect_ratio: true },
                &opts,
            )
        }))
        .map_err(|_| "SVG renderer failed".to_string())?;
    }
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let t = img.thumbnail(PX, PX).to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([t.width() as usize, t.height() as usize], t.as_raw()))
}

// ---- the generated "flat" pack ----

/// White silhouettes (tinted at paint time): a folder and a document per class, with a
/// small class emblem cut into the page.
fn flat(ctx: &Context) -> Loaded {
    let mut l = Loaded { id: FLAT.to_string(), tint: true, ..Default::default() };
    for (key, class) in CLASS_KEYS {
        let img = if class == FileClass::Folder { folder_image() } else { document_image(class) };
        l.by_class.insert(class, ctx.load_texture(format!("icon-pack:flat:{key}"), img, TextureOptions::LINEAR));
    }
    l
}

/// Supersampled coverage of `inside(x, y)` (coordinates in 0..1) as a white mask image.
fn raster(shade: impl Fn(f32, f32) -> Option<u8>) -> ColorImage {
    const S: u32 = 4;
    let n = PX as usize;
    let mut px = vec![Color32::TRANSPARENT; n * n];
    for y in 0..n {
        for x in 0..n {
            let (mut a, mut l) = (0u32, 0u32);
            for sy in 0..S {
                for sx in 0..S {
                    let fx = (x as f32 + (sx as f32 + 0.5) / S as f32) / n as f32;
                    let fy = (y as f32 + (sy as f32 + 0.5) / S as f32) / n as f32;
                    if let Some(v) = shade(fx, fy) {
                        a += 1;
                        l += u32::from(v);
                    }
                }
            }
            if let Some(lum) = l.checked_div(a) {
                let lum = lum as u8;
                let alpha = (a * 255 / (S * S)) as u8;
                px[y * n + x] = Color32::from_rgba_unmultiplied(lum, lum, lum, alpha);
            }
        }
    }
    ColorImage { size: [n, n], source_size: vec2(PX as f32, PX as f32), pixels: px }
}

fn rounded(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn folder_image() -> ColorImage {
    raster(|x, y| {
        let tab = rounded(x, y, 0.08, 0.16, 0.46, 0.36, 0.05);
        let body = rounded(x, y, 0.08, 0.26, 0.92, 0.84, 0.07);
        let front = rounded(x, y, 0.08, 0.36, 0.92, 0.84, 0.07);
        if front {
            Some(255)
        } else if body || tab {
            Some(190)
        } else {
            None
        }
    })
}

fn document_image(class: FileClass) -> ColorImage {
    let (x0, y0, x1, y1, fold) = (0.18, 0.06, 0.82, 0.94, 0.22);
    raster(move |x, y| {
        if !rounded(x, y, x0, y0, x1, y1, 0.06) {
            return None;
        }
        // Folded corner (top right).
        let (fx, fy) = (x - (x1 - fold), y - y0);
        if fx > 0.0 && fy < fold {
            return if fx > fy { None } else { Some(170) };
        }
        let (u, v) = ((x - 0.5) / 0.32, (y - 0.6) / 0.32);
        if emblem(class, u, v) { Some(150) } else { Some(255) }
    })
}

/// A simple emblem per class in the page's lower half (`u`, `v` ≈ -1..1).
fn emblem(class: FileClass, u: f32, v: f32) -> bool {
    let bar = |y: f32, w: f32| (v - y).abs() < 0.07 && u.abs() < w;
    match class {
        FileClass::Image => (v <= 0.5 && v >= -0.3 + u.abs() * 1.33) || (u - 0.4).powi(2) + (v + 0.45).powi(2) < 0.03,
        FileClass::Video => u > -0.35 && u < 0.45 && v.abs() < 0.5 - (u + 0.35) * 0.55,
        FileClass::Audio => {
            (u + 0.15).powi(2) + (v - 0.35).powi(2) < 0.05 || ((u - 0.05).abs() < 0.06 && v > -0.4 && v < 0.35)
        }
        FileClass::Archive => u.abs() < 0.08 && v > -0.8 && v < 0.6 && ((v * 6.0).floor() as i32) % 2 == 0,
        FileClass::Code => v.abs() < 0.4 && ((u - (-0.55 + v.abs())).abs() < 0.1 || (u - (0.55 - v.abs())).abs() < 0.1),
        FileClass::Pdf => u.powi(2) + v.powi(2) < 0.3 && u.powi(2) + v.powi(2) > 0.12,
        FileClass::Sheet => {
            (u.abs() < 0.6 && v.abs() < 0.5) && ((u * 3.0).fract().abs() < 0.12 || (v * 3.0).fract().abs() < 0.12)
        }
        FileClass::Slides => u.abs() < 0.6 && v > -0.4 && v < 0.3 && !(u.abs() < 0.48 && v > -0.28 && v < 0.18),
        FileClass::Exe => u.abs() < 0.6 && v.abs() < 0.45 && !(u.abs() < 0.48 && v > -0.2 && v < 0.33),
        FileClass::Font => ((u.abs() - (v + 0.5) * 0.5).abs() < 0.09 && v.abs() < 0.5) || bar(0.15, 0.25),
        FileClass::Link => {
            let r = (u.powi(2) + v.powi(2)).sqrt();
            (r - 0.4).abs() < 0.08 && !(u > 0.0 && v < 0.0)
        }
        FileClass::Doc | FileClass::Text => bar(-0.45, 0.6) || bar(-0.05, 0.6) || bar(0.35, 0.4),
        FileClass::Folder | FileClass::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::EntryKind;

    #[test]
    fn user_pack_loads_and_maps() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("mine");
        std::fs::create_dir_all(&pack).unwrap();
        image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255])).save(pack.join("a.png")).unwrap();
        std::fs::write(
            pack.join("b.svg"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="red"/></svg>"#,
        )
        .unwrap();
        std::fs::write(
            pack.join("pack.toml"),
            "name = \"Mine\"\n[kinds]\nfolder = \"a.png\"\nfile = \"b.svg\"\nbogus = \"a.png\"\n[extensions]\n\".RS\" = \"b.svg\"\npdf = \"../escape.png\"\n[folders]\n\".Git\" = \"a.png\"\n",
        )
        .unwrap();
        let ctx = Context::default();
        let l = load_dir(&ctx, &pack, "mine").unwrap();
        let f = |n: &str| EntryInfo::minimal(PathBuf::from(n), EntryKind::File);
        let d = |n: &str| EntryInfo::minimal(PathBuf::from(n), EntryKind::Dir);
        assert_eq!(l.by_ext.len(), 1, "escaping path refused");
        assert!(l.lookup(&f("main.rs")).is_some());
        assert!(l.lookup(&f("x.pdf")).is_some(), "falls back to `file`");
        assert!(l.lookup(&d(".git")).is_some());
        assert_eq!(l.lookup(&d("src")).map(|t| t.id()), l.by_class.get(&FileClass::Folder).map(|t| t.id()));
        assert_eq!(l.by_ext["rs"].size(), [PX as usize, PX as usize], "svg rasterized at the pack size");
        assert_eq!(list_packs(dir.path()), vec![("mine".to_string(), "Mine".to_string())]);
        assert!(load_dir(&ctx, &dir.path().join("missing"), "missing").is_err());
    }

    #[test]
    fn flat_pack_covers_every_class() {
        let ctx = Context::default();
        let l = flat(&ctx);
        assert!(l.tint);
        for (_, c) in CLASS_KEYS {
            assert!(l.class_texture(c).is_some());
        }
        let img = document_image(FileClass::Pdf);
        assert!(img.pixels.iter().any(|p| p.a() == 0) && img.pixels.iter().any(|p| p.a() == 255));
    }

    #[test]
    fn resolve_stays_inside() {
        let d = Path::new("C:/packs/x");
        assert!(resolve(d, "a.png").is_some());
        assert!(resolve(d, "sub/a.png").is_some());
        assert!(resolve(d, "../a.png").is_none());
        assert!(resolve(d, "C:/a.png").is_none());
        assert!(resolve(d, "/a.png").is_none());
        assert!(resolve(d, r"..\a.png").is_none());
        assert_eq!(resolve(d, r"sub\a.png"), Some(d.join("sub").join("a.png")));
        assert!(resolve(d, "").is_none());
    }
}
