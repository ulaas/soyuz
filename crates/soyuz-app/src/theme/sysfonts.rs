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
//! Linux: the catalog comes from the fontconfig folder scan in `soyuz_fs::os_fonts`
//! (every face of every file, `.ttc` collections with their face index, matched by full
//! name or by family for the regular face). A cold scan takes about a second, so it
//! never runs on the UI thread: [`locate`] answers from a small on-disk cache of earlier
//! lookups (`font-locate.cache` in the config folder) while the catalog is being built
//! on a background thread, and `fonts::install` re-installs once it is ready
//! ([`when_catalog_ready`]).
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
    /// Build from the OS list: per-face names where the OS reports them (Linux: full name
    /// of every face, plus the family name for its regular face), else the registry
    /// value name (Windows).
    pub fn from_installed(fonts: &[soyuz_fs::os_fonts::InstalledFont]) -> Self {
        let mut faces = Vec::new();
        let mut plain = Vec::new();
        for f in fonts {
            if f.faces.is_empty() {
                plain.push((f.name.clone(), f.file.clone()));
                continue;
            }
            for x in &f.faces {
                faces.push(Face { name: x.full_name.clone(), file: f.file.clone(), index: x.index });
                let regular = matches!(norm(&x.style).as_str(), "regular" | "book" | "normal" | "roman" | "");
                if regular && norm(&x.family) != norm(&x.full_name) {
                    faces.push(Face { name: x.family.clone(), file: f.file.clone(), index: x.index });
                }
            }
        }
        let mut c = Catalog::from_entries(plain);
        c.faces.extend(faces);
        c
    }

    /// Every face (fallback font search).
    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

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
        let c = Catalog::from_installed(&soyuz_fs::os_fonts::installed());
        log::debug!("font catalog: {} faces in {:.1} ms", c.len(), t.elapsed().as_secs_f64() * 1000.0);
        c
    })
}

/// The catalog is slow to build here (a font folder scan) and must not be built on the
/// UI thread by [`locate`].
const DEFERRED_CATALOG: bool = cfg!(target_os = "linux");

/// Callbacks to run once the catalog is built (`fonts::install` re-installs).
type ReadyHook = Box<dyn FnOnce() + Send>;
static READY_HOOKS: parking_lot::Mutex<Vec<ReadyHook>> = parking_lot::Mutex::new(Vec::new());
/// A [`locate`] call missed because the catalog was still being built.
static MISSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a [`locate`] since the last call answered "not found" only because the catalog
/// was not built yet (clears the flag).
pub fn take_missed() -> bool {
    MISSED.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// Run `f` on the catalog thread once the catalog is built (right away if it is).
pub fn when_catalog_ready(f: impl FnOnce() + Send + 'static) {
    {
        let mut hooks = READY_HOOKS.lock();
        if CATALOG.get().is_none() {
            hooks.push(Box::new(f));
            drop(hooks);
            warm_catalog();
            return;
        }
    }
    f();
}

fn run_ready_hooks() {
    let hooks = std::mem::take(&mut *READY_HOOKS.lock());
    for h in hooks {
        h();
    }
}

/// Build the catalog on a background thread (Settings → Fonts opens).
pub fn warm_catalog() {
    if CATALOG.get().is_some() {
        return;
    }
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("soyuz-font-catalog".into()).spawn(|| {
        let _ = catalog();
        run_ready_hooks();
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
    let cat = if DEFERRED_CATALOG {
        match catalog_ready() {
            Some(c) => c,
            None => {
                if let Some(hit) = cache::get(name) {
                    return Some(hit);
                }
                MISSED.store(true, std::sync::atomic::Ordering::SeqCst);
                warm_catalog();
                return None;
            }
        }
    } else {
        catalog()
    };
    if let Some(f) = cat.find(name)
        && f.file.is_file()
    {
        if DEFERRED_CATALOG {
            cache::put(name, &f.file, f.index);
        }
        return Some((f.file.clone(), f.index));
    }
    // The fontconfig scan behind the catalog already covers every font folder here.
    if DEFERRED_CATALOG {
        return None;
    }
    scan_dirs(&dirs, name).map(|p| (p, 0))
}

/// Earlier [`locate`] results kept across runs (`font-locate.cache`, one
/// `name<TAB>index<TAB>path` per line), so a system font configured on Linux is used from
/// the first frame instead of after the background font scan. Entries whose file is gone
/// are ignored; the catalog result always wins once it is built.
mod cache {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use parking_lot::Mutex;

    static MAP: Mutex<Option<HashMap<String, (PathBuf, u32)>>> = Mutex::new(None);

    fn file() -> PathBuf {
        soyuz_core::config::config_dir().join("font-locate.cache")
    }

    pub fn parse(text: &str) -> HashMap<String, (PathBuf, u32)> {
        text.lines()
            .filter_map(|l| {
                let mut it = l.splitn(3, '\t');
                let (name, index, path) = (it.next()?, it.next()?, it.next()?);
                Some((name.to_string(), (PathBuf::from(path), index.parse().ok()?)))
            })
            .collect()
    }

    pub fn render(map: &HashMap<String, (PathBuf, u32)>) -> String {
        let mut lines: Vec<String> =
            map.iter().map(|(n, (p, i))| format!("{n}\t{i}\t{}", p.to_string_lossy())).collect();
        lines.sort();
        lines.join("\n") + "\n"
    }

    fn with<R>(f: impl FnOnce(&mut HashMap<String, (PathBuf, u32)>) -> R) -> R {
        let mut g = MAP.lock();
        // A few hundred bytes, read once.
        let map = g.get_or_insert_with(|| std::fs::read_to_string(file()).map(|t| parse(&t)).unwrap_or_default());
        f(map)
    }

    pub fn get(name: &str) -> Option<(PathBuf, u32)> {
        let hit = with(|m| m.get(&super::norm(name)).cloned())?;
        hit.0.is_file().then_some(hit)
    }

    pub fn put(name: &str, path: &Path, index: u32) {
        let key = super::norm(name);
        let text = with(|m| {
            if m.get(&key).is_some_and(|(p, i)| p == path && *i == index) {
                return None;
            }
            m.insert(key, (path.to_path_buf(), index));
            Some(render(m))
        });
        if let Some(text) = text {
            crate::services::store_file::save_async(file(), text.into_bytes());
        }
    }
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

    #[test]
    fn catalog_from_face_details() {
        use soyuz_fs::os_fonts::{FontFace, InstalledFont};
        let face = |index, family: &str, style: &str, full: &str| FontFace {
            index,
            family: family.into(),
            style: style.into(),
            full_name: full.into(),
        };
        let fonts = vec![
            InstalledFont {
                name: "Noto Sans CJK JP & Noto Sans CJK KR (TrueType)".into(),
                file: PathBuf::from("/f/NotoSansCJK.ttc"),
                per_user: false,
                faces: vec![
                    face(0, "Noto Sans CJK JP", "Regular", "Noto Sans CJK JP"),
                    face(1, "Noto Sans CJK KR", "Regular", "Noto Sans CJK KR"),
                ],
            },
            InstalledFont {
                name: "DejaVu Sans Bold (TrueType)".into(),
                file: PathBuf::from("/f/DejaVuSans-Bold.ttf"),
                per_user: false,
                faces: vec![face(0, "DejaVu Sans", "Bold", "DejaVu Sans Bold")],
            },
            InstalledFont {
                name: "DejaVu Sans (TrueType)".into(),
                file: PathBuf::from("/f/DejaVuSans.ttf"),
                per_user: false,
                faces: vec![face(0, "DejaVu Sans", "Book", "DejaVu Sans Book")],
            },
            InstalledFont {
                name: "Segoe UI (TrueType)".into(),
                file: PathBuf::from("/w/segoeui.ttf"),
                per_user: false,
                faces: vec![],
            },
        ];
        let c = Catalog::from_installed(&fonts);
        let kr = c.find("Noto Sans CJK KR").unwrap();
        assert_eq!((kr.file.to_str(), kr.index), (Some("/f/NotoSansCJK.ttc"), 1));
        assert_eq!(c.find("DejaVu Sans").unwrap().file, PathBuf::from("/f/DejaVuSans.ttf"), "family = regular face");
        assert_eq!(c.find("DejaVu Sans Bold").unwrap().file, PathBuf::from("/f/DejaVuSans-Bold.ttf"));
        assert!(c.find("Segoe UI").is_some(), "entries without face details use the value name");
    }

    #[test]
    fn locate_cache_round_trip() {
        let mut m = std::collections::HashMap::new();
        m.insert("notosanscjksc".to_string(), (PathBuf::from("/f/a b.ttc"), 2));
        m.insert("dejavusans".to_string(), (PathBuf::from("/f/DejaVuSans.ttf"), 0));
        let text = cache::render(&m);
        assert_eq!(cache::parse(&text), m);
        assert!(cache::parse("garbage\nx\tnotanumber\t/p\n").is_empty());
    }

    /// Linux: every face the font scan reports can be found again by its full name, with
    /// its own file and collection index (`.ttc` included).
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_catalog_finds_collection_faces() {
        let fonts = soyuz_fs::os_fonts::installed();
        let c = Catalog::from_installed(&fonts);
        for f in fonts.iter().filter(|f| f.faces.len() > 1).take(5) {
            for x in &f.faces {
                let found = c.find(&x.full_name).unwrap();
                assert!(found.name.eq_ignore_ascii_case(&x.full_name) || norm(&found.name) == norm(&x.full_name));
            }
        }
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
