//! Installed fonts for fonts a PDF does not embed (`[pdf] system_fonts`).
//!
//! The engine asks for a substitute when a font program is missing and no standard-14
//! font fits, typically a CJK CID font. [`find`] looks for a system face: the PDF's own
//! family name first, then the usual families of the character collection on this OS.
//! Lookups and file reads happen on engine workers only (never the UI thread); the
//! installed-font list and every result are cached for the process.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use soyuz_fs::os_fonts::{self, InstalledFont};

/// A CJK character collection (CID `Registry-Ordering`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cjk {
    /// Adobe-Japan1.
    Japanese,
    /// Adobe-GB1.
    SimplifiedChinese,
    /// Adobe-CNS1.
    TraditionalChinese,
    /// Adobe-Korea1.
    Korean,
}

/// What the PDF says about a missing font.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontWant {
    /// Family / font name from the descriptor or `BaseFont` (subset prefix removed).
    pub names: Vec<String>,
    pub cjk: Option<Cjk>,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
}

/// Font bytes shared by every document that uses them.
pub type FontBytes = Arc<Vec<u8>>;

/// Families tried for a collection on this OS, serif or sans first.
pub fn candidates(cjk: Cjk, serif: bool) -> Vec<&'static str> {
    let (sans, ser): (&[&str], &[&str]) = if cfg!(windows) {
        match cjk {
            Cjk::Japanese => {
                (&["Yu Gothic", "Meiryo", "MS Gothic", "MS PGothic"], &["Yu Mincho", "MS Mincho", "MS PMincho"])
            }
            Cjk::SimplifiedChinese => (&["Microsoft YaHei", "DengXian", "SimHei"], &["SimSun", "NSimSun", "FangSong"]),
            Cjk::TraditionalChinese => (&["Microsoft JhengHei", "MingLiU", "PMingLiU"], &["PMingLiU", "MingLiU"]),
            Cjk::Korean => (&["Malgun Gothic", "Gulim", "Dotum"], &["Batang", "BatangChe"]),
        }
    } else if cfg!(target_os = "macos") {
        match cjk {
            Cjk::Japanese => (&["Hiragino Sans", "Hiragino Kaku Gothic ProN", "Osaka"], &["Hiragino Mincho ProN"]),
            Cjk::SimplifiedChinese => (&["PingFang SC", "Heiti SC", "STHeiti"], &["Songti SC", "STSong"]),
            Cjk::TraditionalChinese => (&["PingFang TC", "Heiti TC"], &["Songti TC", "LiSong Pro"]),
            Cjk::Korean => (&["Apple SD Gothic Neo", "AppleGothic"], &["AppleMyungjo"]),
        }
    } else {
        match cjk {
            Cjk::Japanese => (
                &["Noto Sans CJK JP", "Source Han Sans JP", "IPAexGothic", "IPAGothic", "VL Gothic", "TakaoGothic"],
                &["Noto Serif CJK JP", "Source Han Serif JP", "IPAexMincho", "IPAMincho"],
            ),
            Cjk::SimplifiedChinese => (
                &[
                    "Noto Sans CJK SC",
                    "Source Han Sans SC",
                    "WenQuanYi Micro Hei",
                    "WenQuanYi Zen Hei",
                    "Droid Sans Fallback",
                ],
                &["Noto Serif CJK SC", "Source Han Serif SC", "AR PL UMing CN", "AR PL SungtiL GB"],
            ),
            Cjk::TraditionalChinese => (
                &["Noto Sans CJK TC", "Source Han Sans TC", "WenQuanYi Zen Hei", "Droid Sans Fallback"],
                &["Noto Serif CJK TC", "Source Han Serif TC", "AR PL UMing TW", "AR PL New Sung"],
            ),
            Cjk::Korean => (
                &["Noto Sans CJK KR", "Source Han Sans KR", "NanumGothic", "UnDotum", "Baekmuk Gulim"],
                &["Noto Serif CJK KR", "Source Han Serif KR", "NanumMyeongjo", "UnBatang", "Baekmuk Batang"],
            ),
        }
    };
    let (first, second) = if serif { (ser, sans) } else { (sans, ser) };
    first.iter().chain(second.iter()).copied().collect()
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Face `family` in `fonts`: the face list where the OS gives one
/// ([`os_fonts::find_in`]), else (Windows) the registry display name
/// `Name A & Name B (TrueType)`, whose parts are taken as the collection's faces in order.
pub fn lookup(fonts: &[InstalledFont], family: &str) -> Option<(PathBuf, u32)> {
    if let Some(hit) = os_fonts::find_in(fonts, family) {
        return Some(hit);
    }
    let want = norm(family);
    if want.is_empty() {
        return None;
    }
    fonts.iter().filter(|f| f.faces.is_empty() && usable(&f.file)).find_map(|f| {
        let name = f.name.rsplit_once(" (").map_or(f.name.as_str(), |(n, _)| n);
        name.split(" & ").position(|n| norm(n) == want).map(|i| (f.file.clone(), i as u32))
    })
}

/// TrueType / OpenType files (the engine reads no bitmap or Type 1 system fonts).
fn usable(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        let e = e.to_ascii_lowercase();
        matches!(e.as_str(), "ttf" | "otf" | "ttc" | "otc")
    })
}

/// The file and face index for `want` in `fonts`, without reading font files.
pub fn choose(fonts: &[InstalledFont], want: &FontWant) -> Option<(PathBuf, u32)> {
    let mut names: Vec<String> = Vec::new();
    for n in &want.names {
        if want.bold {
            names.push(format!("{n} Bold"));
        }
        names.push(n.clone());
    }
    if let Some(cjk) = want.cjk {
        for c in candidates(cjk, want.serif) {
            if want.bold {
                names.push(format!("{c} Bold"));
            }
            names.push(c.to_string());
        }
    }
    names.iter().find_map(|n| lookup(fonts, n).filter(|(p, _)| usable(p)))
}

/// The installed fonts, read once per process.
fn installed() -> &'static [InstalledFont] {
    static LIST: OnceLock<Vec<InstalledFont>> = OnceLock::new();
    LIST.get_or_init(os_fonts::installed)
}

/// Font bytes and face index for `want`, from the installed fonts. Cached per request and
/// per file. Blocking file I/O: engine workers only.
pub fn find(want: &FontWant) -> Option<(FontBytes, u32)> {
    type Memo = HashMap<FontWant, Option<(FontBytes, u32)>>;
    static MEMO: OnceLock<parking_lot::Mutex<Memo>> = OnceLock::new();
    static FILES: OnceLock<parking_lot::Mutex<HashMap<PathBuf, FontBytes>>> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    if let Some(hit) = memo.lock().get(want) {
        return hit.clone();
    }
    let found = choose(installed(), want).and_then(|(path, index)| {
        let files = FILES.get_or_init(Default::default);
        if let Some(b) = files.lock().get(&path) {
            return Some((b.clone(), index));
        }
        let bytes = Arc::new(std::fs::read(&path).ok()?);
        log::debug!("PDF font fallback: {} face {index}", path.display());
        files.lock().insert(path, bytes.clone());
        Some((bytes, index))
    });
    memo.lock().insert(want.clone(), found.clone());
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(name: &str, file: &str, faces: &[(&str, &str)]) -> InstalledFont {
        InstalledFont {
            name: name.into(),
            file: PathBuf::from(file),
            per_user: false,
            faces: faces
                .iter()
                .enumerate()
                .map(|(i, (fam, style))| os_fonts::FontFace {
                    index: i as u32,
                    family: fam.to_string(),
                    style: style.to_string(),
                    full_name: format!("{fam} {style}"),
                })
                .collect(),
        }
    }

    #[test]
    fn registry_names_and_face_lists() {
        let list = vec![
            font("MS Gothic & MS UI Gothic & MS PGothic (TrueType)", "C:/Windows/Fonts/msgothic.ttc", &[]),
            font("Old Bitmap (All res)", "C:/Windows/Fonts/old.fon", &[]),
            font(
                "x",
                "/usr/share/fonts/NotoSansCJK-Regular.ttc",
                &[("Noto Sans CJK JP", "Regular"), ("Noto Sans CJK KR", "Regular")],
            ),
        ];
        assert_eq!(lookup(&list, "MS PGothic"), Some((PathBuf::from("C:/Windows/Fonts/msgothic.ttc"), 2)));
        assert_eq!(lookup(&list, "Old Bitmap"), None, "not a usable format");
        assert_eq!(
            lookup(&list, "noto sans cjk kr"),
            Some((PathBuf::from("/usr/share/fonts/NotoSansCJK-Regular.ttc"), 1))
        );
        let want = FontWant { cjk: Some(Cjk::Korean), ..Default::default() };
        let all_candidates: Vec<_> = [Cjk::Japanese, Cjk::Korean].iter().flat_map(|c| candidates(*c, false)).collect();
        if all_candidates.contains(&"Noto Sans CJK KR") {
            assert_eq!(choose(&list, &want).map(|h| h.1), Some(1));
        }
        let named = FontWant { names: vec!["MS Gothic".into()], ..Default::default() };
        assert_eq!(choose(&list, &named).map(|h| h.1), Some(0));
        assert_eq!(choose(&list, &FontWant::default()), None);
        assert!(candidates(Cjk::Japanese, true).len() > 2);
    }

    /// A real installed CJK face resolves (skipped where none is installed).
    #[test]
    fn cjk_fallback_uses_an_installed_face() {
        let want = FontWant { cjk: Some(Cjk::Japanese), ..Default::default() };
        if choose(installed(), &want).is_none() {
            eprintln!("no Japanese system font installed; skipping");
            return;
        }
        let (bytes, index) = find(&want).expect("a face");
        assert!(bytes.len() > 1000);
        assert!(skrifa::FontRef::from_index(&bytes, index).is_ok(), "a parsable face");
        let again = find(&want).unwrap();
        assert!(Arc::ptr_eq(&again.0, &bytes), "cached");
    }
}
