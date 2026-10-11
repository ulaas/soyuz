//! System fallback fonts for scripts the bundled Inter / JetBrains Mono lack (CJK,
//! Hangul, emoji, right-to-left scripts), so file names and terminal output never render as tofu boxes.
//!
//! Windows / macOS: the system font folders are checked directly for a short list of
//! well-known files per script; the first one present wins.
//!
//! Linux: fonts are picked by what they actually cover. Each script has a few probe
//! characters (Hebrew and Arabic for right-to-left, Devanagari … Sinhala for Indic, …);
//! for every probe not covered yet, the preferred families (Noto Sans CJK / Source Han,
//! Noto Sans Arabic, …) are tried from the installed-font catalog, then faces whose
//! name mentions the script, then (bounded) any face, checking the probe against each
//! face's `cmap`. Color-only fonts (COLR / CBDT / sbix, e.g. Noto Color Emoji) are
//! skipped: epaint draws outlines only, so their emoji would come out blank; a
//! monochrome emoji / symbol font (Noto Emoji, Symbola, DejaVu Sans) is used instead.
//!
//! Nothing is
//! read on the UI thread: a background thread loads the file and re-installs the fonts
//! (`Context::set_fonts` is thread-safe), and the loaded fonts are appended to every
//! family — every font role (ui, semibold, terminal, code, hex, numbers) and the icon
//! families — by [`append`], which `fonts::build` calls.
//!
//! `[appearance] fallback_fonts`: `auto` loads a script's font the first time a name
//! that needs it is seen ([`note`]), `startup` loads all of them in the background
//! right away, `off` never loads any.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::{Context, FontData, FontDefinitions};
use parking_lot::Mutex;
use soyuz_core::config::{FallbackFonts, Fonts};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    /// Han ideographs, kana, bopomofo, CJK punctuation and full-width forms.
    Cjk,
    Hangul,
    Emoji,
    /// Hebrew, Arabic (incl. presentation forms), Syriac, Thaana, NKo: one font covers
    /// them on every platform we look at (Segoe UI / Arial / Tahoma on Windows), and
    /// the shaper (harfrust, in epaint) does the Arabic joining from its GSUB tables.
    Rtl,
    /// Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu, Kannada, Malayalam,
    /// Sinhala.
    Indic,
    /// Thai, Lao, Myanmar, Khmer.
    SouthEastAsian,
}

impl Script {
    pub const ALL: [Script; 6] =
        [Script::Cjk, Script::Hangul, Script::Emoji, Script::Rtl, Script::Indic, Script::SouthEastAsian];

    /// Characters a fallback font for this script must cover (Linux picks one font per
    /// probe that the fonts chosen so far miss).
    pub fn probes(self) -> &'static [char] {
        match self {
            Script::Cjk => &['中', 'あ', 'ア'],
            Script::Hangul => &['한'],
            Script::Emoji => &['😀', '☀', '✔'],
            Script::Rtl => &['א', 'ب'],
            Script::Indic => &['क', 'ক', 'ਕ', 'ક', 'କ', 'க', 'క', 'ಕ', 'ക', 'ක'],
            Script::SouthEastAsian => &['ก', 'ກ', 'က', 'ក'],
        }
    }

    /// Preferred families on Linux, best first (looked up in the installed-font catalog).
    pub fn linux_families(self) -> &'static [&'static str] {
        match self {
            Script::Cjk => &[
                "Noto Sans CJK SC",
                "Noto Sans CJK JP",
                "Source Han Sans SC",
                "Source Han Sans CN",
                "Source Han Sans",
                "Noto Sans SC",
                "WenQuanYi Zen Hei",
                "WenQuanYi Micro Hei",
                "Droid Sans Fallback",
            ],
            Script::Hangul => &[
                "Noto Sans CJK KR",
                "Source Han Sans KR",
                "Source Han Sans K",
                "Noto Sans KR",
                "NanumGothic",
                "UnDotum",
            ],
            Script::Emoji => &["Noto Emoji", "Symbola", "Noto Sans Symbols 2", "Noto Sans Symbols", "DejaVu Sans"],
            Script::Rtl => &[
                "Noto Sans Hebrew",
                "Noto Sans Arabic",
                "Noto Naskh Arabic",
                "DejaVu Sans",
                "Droid Sans Hebrew",
                "Droid Arabic Naskh",
                "FreeSans",
            ],
            Script::Indic => &[
                "Noto Sans Devanagari",
                "Noto Sans Bengali",
                "Noto Sans Gurmukhi",
                "Noto Sans Gujarati",
                "Noto Sans Oriya",
                "Noto Sans Tamil",
                "Noto Sans Telugu",
                "Noto Sans Kannada",
                "Noto Sans Malayalam",
                "Noto Sans Sinhala",
                "Lohit Devanagari",
                "Droid Sans Devanagari",
                "FreeSans",
            ],
            Script::SouthEastAsian => &[
                "Noto Sans Thai",
                "Noto Sans Lao",
                "Noto Sans Myanmar",
                "Noto Sans Khmer",
                "Droid Sans Thai",
                "Waree",
                "Garuda",
            ],
        }
    }

    /// Words in face names that suggest the script (second pass of the Linux search).
    fn name_hints(self) -> &'static [&'static str] {
        match self {
            Script::Cjk => &["cjk", "han", "hei", "gothic", "mincho", "song", "ming"],
            Script::Hangul => &["cjk", "kr", "nanum", "hangul", "gothic"],
            Script::Emoji => &["emoji", "symbol"],
            Script::Rtl => &["hebrew", "arabic", "naskh", "kufi"],
            Script::Indic => &[
                "devanagari",
                "bengali",
                "bangla",
                "gurmukhi",
                "gujarati",
                "oriya",
                "odia",
                "tamil",
                "telugu",
                "kannada",
                "malayalam",
                "sinhala",
                "lohit",
            ],
            Script::SouthEastAsian => &["thai", "lao", "myanmar", "khmer"],
        }
    }

    /// Font files to try, in order (file names inside the system font folders).
    pub fn candidates(self) -> &'static [&'static str] {
        match self {
            Script::Cjk => &[
                "msyh.ttc",
                "YuGothM.ttc",
                "msgothic.ttc",
                "simsun.ttc",
                "NotoSansCJK-Regular.ttc",
                "NotoSansCJKsc-Regular.otf",
                "PingFang.ttc",
                "Hiragino Sans GB.ttc",
            ],
            Script::Hangul => &["malgun.ttf", "gulim.ttc", "NotoSansCJK-Regular.ttc", "AppleSDGothicNeo.ttc"],
            Script::Emoji => &["seguiemj.ttf", "seguisym.ttf", "NotoEmoji-Regular.ttf"],
            Script::Rtl => &[
                "segoeui.ttf",
                "arial.ttf",
                "tahoma.ttf",
                "NotoSansArabic-Regular.ttf",
                "NotoSansHebrew-Regular.ttf",
                "DejaVuSans.ttf",
                "GeezaPro.ttc",
                "ArialHB.ttc",
            ],
            Script::Indic => &[
                "Nirmala.ttc",
                "Nirmala.ttf",
                "mangal.ttf",
                "NotoSansDevanagari-Regular.ttf",
                "Kohinoor.ttc",
                "DevanagariMT.ttc",
            ],
            Script::SouthEastAsian => {
                &["LeelawUI.ttf", "leelawad.ttf", "tahoma.ttf", "NotoSansThai-Regular.ttf", "Thonburi.ttc"]
            }
        }
    }

    fn key(self) -> &'static str {
        match self {
            Script::Cjk => "fallback-cjk",
            Script::Hangul => "fallback-hangul",
            Script::Emoji => "fallback-emoji",
            Script::Rtl => "fallback-rtl",
            Script::Indic => "fallback-indic",
            Script::SouthEastAsian => "fallback-sea",
        }
    }
}

/// The script whose fallback font draws `c` (`None`: the bundled fonts cover it).
pub fn script_of(c: char) -> Option<Script> {
    let u = c as u32;
    match u {
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF => {
            Some(Script::Rtl)
        }
        0x0900..=0x0DFF => Some(Script::Indic),
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF | 0x19E0..=0x19FF => Some(Script::SouthEastAsian),
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF => Some(Script::Hangul),
        0x2E80..=0x2FDF
        | 0x3000..=0x312F
        | 0x31A0..=0x31FF
        | 0x3200..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x323AF => Some(Script::Cjk),
        0x2600..=0x27BF | 0x1F000..=0x1FAFF => Some(Script::Emoji),
        _ => None,
    }
}

/// Scripts needed by `text` (deduplicated, in first-seen order).
pub fn scripts_in(text: &str) -> Vec<Script> {
    let mut out = Vec::new();
    for c in text.chars() {
        if (c as u32) < 0x0590 {
            continue;
        }
        if let Some(s) = script_of(c)
            && !out.contains(&s)
        {
            out.push(s);
        }
    }
    out
}

/// First candidate of `script` for which `exists(file name)` holds.
pub fn pick(script: Script, exists: impl Fn(&str) -> bool) -> Option<&'static str> {
    script.candidates().iter().copied().find(|n| exists(n))
}

/// Folders scanned for fallback fonts.
fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        v.push(windir.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        v.extend(["/System/Library/Fonts", "/Library/Fonts", "/System/Library/Fonts/Supplemental"].map(PathBuf::from));
    } else {
        for base in ["/usr/share/fonts", "/usr/local/share/fonts"] {
            v.push(PathBuf::from(base));
            if let Ok(rd) = std::fs::read_dir(base) {
                v.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
            }
        }
    }
    v
}

fn locate(script: Script, dirs: &[PathBuf]) -> Option<PathBuf> {
    let found = |name: &str| dirs.iter().map(|d| d.join(name)).find(|p| p.is_file());
    pick(script, |n| found(n).is_some()).and_then(found)
}

struct Loaded {
    script: Script,
    /// `font_data` key (one script can load several fonts on Linux).
    key: String,
    data: Arc<FontData>,
}

#[derive(Default)]
struct State {
    ctx: Option<Context>,
    requested: HashSet<Script>,
    loaded: Vec<Loaded>,
    /// `[fonts]` of the last install (re-install after a load).
    fonts: Fonts,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
/// 0 = auto, 1 = startup, 2 = off (fast path for [`note`]).
static MODE: AtomicU8 = AtomicU8::new(2);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock();
    f(g.get_or_insert_with(State::default))
}

/// Remember the context, the configured fonts and the mode; `startup` starts loading
/// every script now. Called by `theme::install` (startup and every config change).
pub fn configure(ctx: &Context, mode: FallbackFonts, fonts: &Fonts) {
    MODE.store(
        match mode {
            FallbackFonts::Auto => 0,
            FallbackFonts::Startup => 1,
            FallbackFonts::Off => 2,
        },
        Ordering::SeqCst,
    );
    with(|s| {
        s.ctx = Some(ctx.clone());
        s.fonts = fonts.clone();
    });
    if mode == FallbackFonts::Startup {
        request(&Script::ALL);
    }
}

/// A file name (or any text) is about to be shown: load the fonts it needs (`auto`).
pub fn note(text: &str) {
    if MODE.load(Ordering::Relaxed) != 0 || text.is_ascii() {
        return;
    }
    let need = scripts_in(text);
    if !need.is_empty() {
        request(&need);
    }
}

/// Load the fonts of `scripts` that were not requested yet, on a background thread
/// (unless `fallback_fonts = off`).
pub fn request(scripts: &[Script]) {
    if MODE.load(Ordering::Relaxed) == 2 {
        return;
    }
    let todo: Vec<Script> = with(|s| scripts.iter().copied().filter(|sc| s.requested.insert(*sc)).collect());
    if todo.is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new().name("soyuz-fonts".into()).spawn(move || {
        let dirs = font_dirs();
        let mut any = false;
        for script in todo {
            // Linux: by coverage from the font catalog (built on this thread if needed).
            let found = if cfg!(target_os = "linux") {
                by_coverage(script, super::sysfonts::catalog().faces())
            } else {
                Vec::new()
            };
            let found: Vec<(PathBuf, Arc<FontData>)> = if found.is_empty() {
                locate(script, &dirs).and_then(|p| Some((p.clone(), load(&p, 0)?))).into_iter().collect()
            } else {
                found
            };
            if found.is_empty() {
                log::info!("no fallback font found for {script:?}");
            }
            for (n, (path, data)) in found.into_iter().enumerate() {
                log::info!("fallback font for {script:?}: {} #{}", path.display(), data.index);
                let key = if n == 0 { script.key().to_string() } else { format!("{}-{n}", script.key()) };
                with(|s| s.loaded.push(Loaded { script, key, data }));
                any = true;
            }
        }
        if any {
            let (ctx, fonts) = with(|s| (s.ctx.clone(), s.fonts.clone()));
            if let Some(ctx) = ctx {
                let _ = super::fonts::install(&ctx, &fonts);
                ctx.request_repaint();
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("cannot spawn the font loader: {e}");
    }
}

fn load(path: &Path, index: u32) -> Option<Arc<FontData>> {
    let bytes = std::fs::read(path).ok()?;
    // Collections (.ttc): face 0 is the regular face of every Windows / macOS candidate.
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    Some(Arc::new(data))
}

/// Largest file the last-resort pass reads just to look at its `cmap`.
const SCAN_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Most files the Linux search reads per script.
const SCAN_MAX_FILES: usize = 300;

/// Which of `chars` the face covers with drawable outlines (`None`: not a usable font:
/// unreadable, or color-only, which epaint cannot draw).
pub fn covered(bytes: &[u8], index: u32, chars: &[char]) -> Option<Vec<char>> {
    use skrifa::MetadataProvider as _;
    let font = skrifa::FontRef::from_index(bytes, index).ok()?;
    let has = |tag: &[u8; 4]| font.table_data(skrifa::Tag::new(tag)).is_some();
    if has(b"COLR") || has(b"CBDT") || has(b"sbix") || !(has(b"glyf") || has(b"CFF ") || has(b"CFF2")) {
        return None;
    }
    let cmap = font.charmap();
    Some(chars.iter().copied().filter(|c| cmap.map(*c).is_some()).collect())
}

/// Linux: fonts covering the probes of `script`, chosen by `cmap` (see the module docs).
fn by_coverage(script: Script, faces: &[super::sysfonts::Face]) -> Vec<(PathBuf, Arc<FontData>)> {
    use super::sysfonts::{Face, norm};
    let mut missing: Vec<char> = script.probes().to_vec();
    let mut out: Vec<(PathBuf, Arc<FontData>)> = Vec::new();
    let mut tried: HashSet<(PathBuf, u32)> = HashSet::new();
    let mut take = |f: &Face, missing: &mut Vec<char>, out: &mut Vec<(PathBuf, Arc<FontData>)>| {
        if tried.len() >= SCAN_MAX_FILES || !tried.insert((f.file.clone(), f.index)) {
            return;
        }
        let Ok(bytes) = std::fs::read(&f.file) else { return };
        let Some(cov) = covered(&bytes, f.index, missing) else { return };
        if cov.is_empty() {
            return;
        }
        missing.retain(|c| !cov.contains(c));
        let mut data = FontData::from_owned(bytes);
        data.index = f.index;
        out.push((f.file.clone(), Arc::new(data)));
    };
    let upright = |f: &&Face| {
        let n = f.name.to_lowercase();
        !["bold", "italic", "oblique", "thin", "black", "mono", "serif", "condensed"].iter().any(|w| n.contains(w))
    };
    // 1. Preferred families, in order.
    for fam in script.linux_families() {
        if missing.is_empty() {
            return out;
        }
        let want = norm(fam);
        let exact = faces.iter().find(|f| norm(&f.name) == want || norm(&f.name) == format!("{want}regular"));
        // Variable fonts may only list a named instance (`Noto Sans Hebrew Light`).
        let any_weight =
            || faces.iter().filter(|f| norm(&f.name).starts_with(&want) && upright(f)).min_by_key(|f| f.name.len());
        if let Some(f) = exact.or_else(any_weight) {
            take(f, &mut missing, &mut out);
        }
    }
    // 2. Upright faces named after the script; 3. any (small) upright face.
    let named = faces.iter().filter(upright).filter(|f| {
        let n = f.name.to_lowercase();
        script.name_hints().iter().any(|h| n.contains(h))
    });
    for f in named {
        if missing.is_empty() {
            return out;
        }
        take(f, &mut missing, &mut out);
    }
    for f in faces.iter().filter(upright) {
        if missing.is_empty() {
            break;
        }
        if std::fs::metadata(&f.file).is_ok_and(|m| m.len() <= SCAN_MAX_BYTES) {
            take(f, &mut missing, &mut out);
        }
    }
    out
}

/// Append the loaded fallback fonts to every family (after the bundled ones).
pub fn append(fonts: &mut FontDefinitions) {
    if MODE.load(Ordering::Relaxed) == 2 {
        return;
    }
    let loaded: Vec<(String, Arc<FontData>)> =
        with(|s| s.loaded.iter().map(|l| (l.key.clone(), l.data.clone())).collect());
    append_with(fonts, loaded);
}

fn append_with(fonts: &mut FontDefinitions, loaded: Vec<(String, Arc<FontData>)>) {
    for (key, data) in loaded {
        fonts.font_data.insert(key.clone(), data);
        for fam in fonts.families.values_mut() {
            if !fam.contains(&key) {
                fam.push(key.clone());
            }
        }
    }
}

/// Scripts whose fallback font is loaded (for diagnostics and tests).
pub fn loaded_scripts() -> Vec<Script> {
    with(|s| s.loaded.iter().map(|l| l.script).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_detected() {
        assert_eq!(scripts_in("report_final (2).pdf"), []);
        assert_eq!(scripts_in("Ünïcödé façade — ok"), [], "Latin-1 / punctuation stay on Inter");
        assert_eq!(scripts_in("報告書.docx"), [Script::Cjk]);
        assert_eq!(scripts_in("カタカナ"), [Script::Cjk]);
        assert_eq!(scripts_in("한국어 文書"), [Script::Hangul, Script::Cjk]);
        assert_eq!(scripts_in("party 🎉 ☀"), [Script::Emoji]);
        assert_eq!(script_of('Ａ'), Some(Script::Cjk), "full-width forms");
        assert_eq!(script_of('a'), None);
        assert_eq!(scripts_in("تقرير 2024.pdf"), [Script::Rtl]);
        assert_eq!(scripts_in("שלום.txt"), [Script::Rtl]);
        assert_eq!(scripts_in("Ωmega Ж"), [], "Greek / Cyrillic stay on Inter");
        assert_eq!(scripts_in("नमस्ते.txt"), [Script::Indic]);
        assert_eq!(scripts_in("தமிழ் ภาษาไทย"), [Script::Indic, Script::SouthEastAsian]);
        for s in Script::ALL {
            for c in s.probes() {
                assert_eq!(script_of(*c), Some(s), "probe {c} of {s:?}");
            }
        }
    }

    #[test]
    fn first_available_candidate_wins() {
        let have = ["YuGothM.ttc", "simsun.ttc", "malgun.ttf"];
        let exists = |n: &str| have.contains(&n);
        assert_eq!(pick(Script::Cjk, exists), Some("YuGothM.ttc"));
        assert_eq!(pick(Script::Hangul, exists), Some("malgun.ttf"));
        assert_eq!(pick(Script::Emoji, exists), None);
        let all = |_: &str| true;
        assert_eq!(pick(Script::Cjk, all), Some("msyh.ttc"));
        assert_eq!(pick(Script::Emoji, all), Some("seguiemj.ttf"));
    }

    #[test]
    fn locate_scans_the_given_folders() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("malgun.ttf"), b"not really a font").unwrap();
        let dirs = vec![d.path().join("missing"), d.path().to_path_buf()];
        assert_eq!(locate(Script::Hangul, &dirs), Some(d.path().join("malgun.ttf")));
        assert_eq!(locate(Script::Cjk, &dirs), None);
    }

    #[test]
    fn appended_fonts_go_last_in_every_family() {
        let mut defs = FontDefinitions::default();
        let before = defs.families[&egui::FontFamily::Monospace].clone();
        append_with(&mut defs, vec![(Script::Hangul.key().into(), Arc::new(FontData::from_static(b"")))]);
        let mono = &defs.families[&egui::FontFamily::Monospace];
        assert_eq!(&mono[..before.len()], &before[..]);
        assert!(mono.contains(&"fallback-hangul".to_string()));
        assert!(defs.families[&egui::FontFamily::Proportional].contains(&"fallback-hangul".to_string()));
        append_with(&mut defs, vec![(Script::Hangul.key().into(), Arc::new(FontData::from_static(b"")))]);
        let n = defs.families[&egui::FontFamily::Monospace].iter().filter(|f| *f == "fallback-hangul").count();
        assert_eq!(n, 1, "appending twice does not duplicate");
    }

    #[test]
    fn coverage_needs_drawable_outlines() {
        let inter = include_bytes!("../../assets/fonts/inter/Inter-Regular.ttf");
        assert_eq!(covered(inter, 0, &['a', '中', 'Ж']), Some(vec!['a', 'Ж']));
        assert_eq!(covered(b"not a font", 0, &['a']), None);
        assert_eq!(covered(inter, 3, &['a']), None, "no such face");
    }

    #[test]
    fn coverage_search_takes_only_covering_faces() {
        let d = tempfile::tempdir().unwrap();
        let inter = d.path().join("Inter.ttf");
        std::fs::write(&inter, include_bytes!("../../assets/fonts/inter/Inter-Regular.ttf")).unwrap();
        let junk = d.path().join("junk.ttf");
        std::fs::write(&junk, b"junk").unwrap();
        let face = |name: &str, file: &Path| super::super::sysfonts::Face {
            name: name.into(),
            file: file.to_path_buf(),
            index: 0,
        };
        let faces = vec![face("Noto Sans CJK SC", &junk), face("Inter", &inter)];
        // Neither the unreadable "preferred" face nor Inter covers CJK / Hebrew / Arabic.
        assert!(by_coverage(Script::Cjk, &faces).is_empty());
        assert!(by_coverage(Script::Rtl, &faces).is_empty());
    }

    /// The fonts of this machine: every font found by coverage is drawable (never a color
    /// emoji font) and covers a probe; CJK is found when Noto Sans CJK / Source Han is.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_fallbacks_by_coverage() {
        let cat = super::super::sysfonts::catalog();
        for s in Script::ALL {
            for (path, data) in by_coverage(s, cat.faces()) {
                eprintln!("{s:?}: {} #{}", path.display(), data.index);
                let bytes = std::fs::read(&path).unwrap();
                let cov = covered(&bytes, data.index, s.probes());
                assert!(cov.is_some_and(|c| !c.is_empty()), "{s:?}: {} is not usable", path.display());
            }
        }
        if cat.find("Noto Sans CJK SC").is_some() || cat.find("Source Han Sans SC").is_some() {
            assert!(!by_coverage(Script::Cjk, cat.faces()).is_empty());
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_a_cjk_font() {
        // Every supported Windows edition ships at least one of the CJK candidates.
        assert!(locate(Script::Cjk, &font_dirs()).is_some());
    }
}
