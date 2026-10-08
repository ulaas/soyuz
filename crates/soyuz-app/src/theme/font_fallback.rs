//! System fallback fonts for scripts the bundled Inter / JetBrains Mono lack (CJK,
//! Hangul, emoji, right-to-left scripts), so file names and terminal output never render as tofu boxes.
//!
//! Windows: `C:\Windows\Fonts` (and the per-user font folder) are scanned directly for a
//! short list of well-known files per script; the first one present wins. Nothing is
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
}

impl Script {
    pub const ALL: [Script; 4] = [Script::Cjk, Script::Hangul, Script::Emoji, Script::Rtl];

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
        }
    }

    fn key(self) -> &'static str {
        match self {
            Script::Cjk => "fallback-cjk",
            Script::Hangul => "fallback-hangul",
            Script::Emoji => "fallback-emoji",
            Script::Rtl => "fallback-rtl",
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
            let Some(path) = locate(script, &dirs) else {
                log::info!("no fallback font found for {script:?}");
                continue;
            };
            match load(&path) {
                Some(data) => {
                    log::info!("fallback font for {script:?}: {}", path.display());
                    with(|s| s.loaded.push(Loaded { script, data }));
                    any = true;
                }
                None => log::warn!("cannot read fallback font {}", path.display()),
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

fn load(path: &Path) -> Option<Arc<FontData>> {
    let bytes = std::fs::read(path).ok()?;
    // Collections (.ttc): face 0 is the regular face of every candidate above.
    let mut data = FontData::from_owned(bytes);
    data.index = 0;
    Some(Arc::new(data))
}

/// Append the loaded fallback fonts to every family (after the bundled ones).
pub fn append(fonts: &mut FontDefinitions) {
    if MODE.load(Ordering::Relaxed) == 2 {
        return;
    }
    let loaded: Vec<(Script, Arc<FontData>)> = with(|s| s.loaded.iter().map(|l| (l.script, l.data.clone())).collect());
    append_with(fonts, loaded);
}

fn append_with(fonts: &mut FontDefinitions, loaded: Vec<(Script, Arc<FontData>)>) {
    for (script, data) in loaded {
        let key = script.key().to_string();
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
        append_with(&mut defs, vec![(Script::Hangul, Arc::new(FontData::from_static(b"")))]);
        let mono = &defs.families[&egui::FontFamily::Monospace];
        assert_eq!(&mono[..before.len()], &before[..]);
        assert!(mono.contains(&"fallback-hangul".to_string()));
        assert!(defs.families[&egui::FontFamily::Proportional].contains(&"fallback-hangul".to_string()));
        append_with(&mut defs, vec![(Script::Hangul, Arc::new(FontData::from_static(b"")))]);
        let n = defs.families[&egui::FontFamily::Monospace].iter().filter(|f| *f == "fallback-hangul").count();
        assert_eq!(n, 1, "appending twice does not duplicate");
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_a_cjk_font() {
        // Every supported Windows edition ships at least one of the CJK candidates.
        assert!(locate(Script::Cjk, &font_dirs()).is_some());
    }
}
