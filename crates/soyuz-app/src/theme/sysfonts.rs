//! Installed system fonts: family name → font file (+ face index in a collection).
//!
//! Lookup order for a family name ([`locate`]):
//! 1. an absolute path to a `.ttf` / `.otf` / `.ttc` file;
//! 2. a short table of well-known Windows files (`Segoe UI` → `segoeui.ttf`,
//!    `Consolas` → `consola.ttf`, …) checked with one `stat` each, so the default fonts
//!    resolve without touching the registry on the startup path;
//! 3. the registry font list (`soyuz_fs::os_fonts`, machine + per-user), parsed once into
//!    a [`Catalog`] — also the source of the font-name autocomplete in Settings;
//! 4. a scan of the font folders by file name (`Fira Code` ↔ `FiraCode-Regular.ttf`) for
//!    fonts dropped into a folder without being registered (and on other platforms).
//!
//! Microsoft's fonts are never copied or bundled: they are read from the system font
//! folder of the machine Soyuz runs on.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One installed face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    /// Family name as listed (`Segoe UI Semibold`, `Cascadia Code`).
    pub name: String,
    pub file: PathBuf,
    /// Face index inside a `.ttc` collection.
    pub index: u32,
}

/// The installed fonts, from the registry entries.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    faces: Vec<Face>,
}

/// Normalized name for matching: lowercase alphanumerics only.
pub fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether two names denote the same face (`Cascadia Code Regular` = `Cascadia Code`).
fn same_name(listed: &str, want: &str) -> bool {
    let (a, b) = (norm(listed), norm(want));
    !b.is_empty() && (a == b || a == format!("{b}regular") || b == format!("{a}regular"))
}

/// Face names in a registry value name: `Cambria & Cambria Math (TrueType)` →
/// `["Cambria", "Cambria Math"]` (the index is the face index in the collection).
pub fn parse_value_name(raw: &str) -> Vec<String> {
    let mut s = raw.trim();
    if s.ends_with(')')
        && let Some(open) = s.rfind(" (")
    {
        s = &s[..open];
    }
    s.split(" & ").map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()
}

impl Catalog {
    /// Build from registry `(value name, file)` pairs.
    pub fn from_entries(entries: impl IntoIterator<Item = (String, PathBuf)>) -> Self {
        let mut faces = Vec::new();
        for (raw, file) in entries {
            let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if !matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                continue; // bitmap .fon / .fnt fonts cannot be drawn by egui
            }
            for (i, name) in parse_value_name(&raw).into_iter().enumerate() {
                let index = if matches!(ext.as_str(), "ttc" | "otc") { i as u32 } else { 0 };
                faces.push(Face { name, file: file.clone(), index });
            }
        }
        Catalog { faces }
    }

    /// The face registered as `family`.
    pub fn find(&self, family: &str) -> Option<&Face> {
        self.faces.iter().find(|f| same_name(&f.name, family))
    }

    /// Sorted, deduplicated face names (font picker suggestions).
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.faces.iter().map(|f| f.name.clone()).collect();
        v.sort_by_key(|n| n.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        v
    }

    pub fn len(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// The registry catalog, built on first use (a few ms; only needed for fonts outside the
/// well-known table and for Settings).
pub fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| {
        let t = std::time::Instant::now();
        let c = Catalog::from_entries(soyuz_fs::os_fonts::installed().into_iter().map(|f| (f.name, f.file)));
        log::debug!("font catalog: {} faces in {:.1} ms", c.len(), t.elapsed().as_secs_f64() * 1000.0);
        c
    })
}

/// Build the catalog on a background thread (Settings → Fonts opens).
pub fn warm_catalog() {
    if CATALOG.get().is_some() {
        return;
    }
    let _ = std::thread::Builder::new().name("soyuz-font-catalog".into()).spawn(|| {
        let _ = catalog();
    });
}

/// The catalog when it is already built (never blocks).
pub fn catalog_ready() -> Option<&'static Catalog> {
    CATALOG.get()
}

/// Well-known Windows font files (family → file in `%WINDIR%\Fonts`).
pub const WELL_KNOWN: &[(&str, &str)] = &[
    ("Segoe UI", "segoeui.ttf"),
    ("Segoe UI Semibold", "seguisb.ttf"),
    ("Segoe UI Bold", "segoeuib.ttf"),
    ("Segoe UI Light", "segoeuil.ttf"),
    ("Segoe UI Semilight", "segoeuisl.ttf"),
    ("Segoe UI Variable", "SegUIVar.ttf"),
    ("Consolas", "consola.ttf"),
    ("Consolas Bold", "consolab.ttf"),
    ("Cascadia Mono", "CascadiaMono.ttf"),
    ("Cascadia Code", "CascadiaCode.ttf"),
];

/// The well-known file name for `family`.
pub fn well_known(family: &str) -> Option<&'static str> {
    WELL_KNOWN.iter().find(|(n, _)| same_name(n, family)).map(|(_, f)| *f)
}

/// Font folders: system, then per-user (Windows); the usual folders elsewhere.
pub fn font_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        v.push(soyuz_fs::os_fonts::system_font_dir());
        v.extend(soyuz_fs::os_fonts::user_font_dir());
    } else if cfg!(target_os = "macos") {
        v.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        v.push(soyuz_fs::home().join("Library/Fonts"));
    } else {
        v.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        v.push(soyuz_fs::home().join(".local/share/fonts"));
        v.push(soyuz_fs::home().join(".fonts"));
    }
    v
}

/// Resolve `family` (name or path) to a font file and face index.
pub fn locate(family: &str) -> Option<(PathBuf, u32)> {
    let name = family.trim();
    if name.is_empty() {
        return None;
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| (path.to_path_buf(), 0));
    }
    let dirs = font_dirs();
    if let Some(file) = well_known(name)
        && let Some(p) = dirs.iter().map(|d| d.join(file)).find(|p| p.is_file())
    {
        return Some((p, 0));
    }
    let cat = catalog();
    if let Some(f) = cat.find(name)
        && f.file.is_file()
    {
        return Some((f.file.clone(), f.index));
    }
    scan_dirs(&dirs, name).map(|p| (p, 0))
}

/// Whether a font file stem matches `want` (normalized): `Cascadia Mono` ↔
/// `CascadiaMono.ttf`, `Fira Code` ↔ `FiraCode-Regular.otf`.
fn stem_matches(stem: &str, want: &str) -> bool {
    let s = norm(stem);
    s == want || s == format!("{want}regular")
}

/// Search the font folders (4 levels deep) for a file named after `family`.
pub fn scan_dirs(dirs: &[PathBuf], family: &str) -> Option<PathBuf> {
    let want = norm(family);
    if want.is_empty() {
        return None;
    }
    let mut stack: Vec<(PathBuf, u8)> = dirs.iter().map(|d| (d.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 4 {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let ext = p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if matches!(ext.as_str(), "ttf" | "otf")
                && p.file_stem().is_some_and(|s| stem_matches(&s.to_string_lossy(), &want))
            {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> Catalog {
        Catalog::from_entries(
            [
                ("Segoe UI (TrueType)", r"C:\W\Fonts\segoeui.ttf"),
                ("Segoe UI Semibold (TrueType)", r"C:\W\Fonts\seguisb.ttf"),
                ("Cambria & Cambria Math (TrueType)", r"C:\W\Fonts\cambria.ttc"),
                ("Cascadia Code Regular (TrueType)", r"C:\W\Fonts\CascadiaCode.ttf"),
                ("Modern (All res)", r"C:\W\Fonts\modern.fon"),
                ("Fira Sans (OpenType)", r"C:\Users\u\AppData\Local\Microsoft\Windows\Fonts\FiraSans.otf"),
            ]
            .map(|(n, f)| (n.to_string(), PathBuf::from(f))),
        )
    }

    #[test]
    fn value_names_are_parsed() {
        assert_eq!(parse_value_name("Segoe UI Semibold (TrueType)"), ["Segoe UI Semibold"]);
        assert_eq!(parse_value_name("Cambria & Cambria Math (TrueType)"), ["Cambria", "Cambria Math"]);
        assert_eq!(parse_value_name("Plain"), ["Plain"]);
    }

    #[test]
    fn catalog_finds_faces() {
        let c = fake();
        assert_eq!(c.len(), 6, "bitmap fonts are skipped");
        assert_eq!(c.find("segoe ui").unwrap().file, PathBuf::from(r"C:\W\Fonts\segoeui.ttf"));
        assert_eq!(c.find("Segoe UI SemiBold").unwrap().file, PathBuf::from(r"C:\W\Fonts\seguisb.ttf"));
        let math = c.find("Cambria Math").unwrap();
        assert_eq!((math.index, c.find("Cambria").unwrap().index), (1, 0));
        assert!(c.find("Cascadia Code").is_some(), "`Regular` suffix ignored");
        assert!(c.find("Fira Sans").is_some());
        assert!(c.find("Modern").is_none());
        assert!(c.find("").is_none());
        let names = c.names();
        assert_eq!(names.first().map(String::as_str), Some("Cambria"));
        assert!(names.contains(&"Segoe UI".to_string()));
    }

    #[test]
    fn well_known_table_and_scan() {
        assert_eq!(well_known("segoe ui"), Some("segoeui.ttf"));
        assert_eq!(well_known("Consolas"), Some("consola.ttf"));
        assert_eq!(well_known("Comic Sans"), None);
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub").join("FiraCode-Regular.otf"), b"x").unwrap();
        std::fs::write(d.path().join("FiraCode-Bold.otf"), b"x").unwrap();
        let dirs = vec![d.path().to_path_buf()];
        assert_eq!(scan_dirs(&dirs, "Fira Code"), Some(d.path().join("sub").join("FiraCode-Regular.otf")));
        assert_eq!(scan_dirs(&dirs, "No Such Font 1234"), None);
        assert!(locate("No Such Font Family 1234").is_none());
        assert!(locate("").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_defaults_resolve() {
        let (p, i) = locate("Segoe UI").expect("Segoe UI");
        assert!(p.to_string_lossy().to_ascii_lowercase().ends_with("segoeui.ttf"));
        assert_eq!(i, 0);
        assert!(locate("Consolas").is_some());
        assert!(catalog().find("Segoe UI Semibold").is_some());
    }
}
