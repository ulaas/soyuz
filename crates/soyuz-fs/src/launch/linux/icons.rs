//! Icon lookup (freedesktop "Icon Theme Specification" 0.13): the user's theme (GTK
//! `settings.ini` `gtk-icon-theme-name`, else KDE `kdeglobals` `[Icons] Theme`), its
//! `Inherits=` chain, then `hicolor`, then `/usr/share/pixmaps`. Returns a file path
//! (PNG, SVG or XPM); loading it is the caller's job.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirType {
    Fixed,
    Scalable,
    Threshold,
}

/// One icon directory of a theme (`[48x48/apps]` …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IconDir {
    pub path: String,
    pub size: u32,
    pub scale: u32,
    pub kind: DirType,
    pub min: u32,
    pub max: u32,
    pub threshold: u32,
}

impl IconDir {
    fn matches(&self, size: u32, scale: u32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            DirType::Fixed => self.size == size,
            DirType::Scalable => self.min <= size && size <= self.max,
            DirType::Threshold => {
                self.size.saturating_sub(self.threshold) <= size && size <= self.size + self.threshold
            }
        }
    }

    fn distance(&self, size: u32, scale: u32) -> u32 {
        let want = size * scale;
        let (lo, hi) = match self.kind {
            DirType::Fixed => (self.size * self.scale, self.size * self.scale),
            DirType::Scalable => (self.min * self.scale, self.max * self.scale),
            DirType::Threshold => {
                (self.size.saturating_sub(self.threshold) * self.scale, (self.size + self.threshold) * self.scale)
            }
        };
        lo.saturating_sub(want).max(want.saturating_sub(hi))
    }
}

/// `index.theme` contents we use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ThemeIndex {
    pub inherits: Vec<String>,
    pub dirs: Vec<IconDir>,
}

pub(crate) fn parse_index(text: &str) -> ThemeIndex {
    let mut groups: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut cur = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            cur = g.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            groups.entry(cur.clone()).or_default().insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let main = groups.get("Icon Theme").cloned().unwrap_or_default();
    let list = |k: &str| -> Vec<String> {
        main.get(k)
            .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
            .unwrap_or_default()
    };
    let mut names = list("Directories");
    names.extend(list("ScaledDirectories"));
    let mut dirs = Vec::new();
    for name in names {
        let Some(g) = groups.get(&name) else { continue };
        let num = |k: &str| g.get(k).and_then(|v| v.parse::<u32>().ok());
        let Some(size) = num("Size") else { continue };
        let kind = match g.get("Type").map(String::as_str) {
            Some("Fixed") => DirType::Fixed,
            Some("Scalable") => DirType::Scalable,
            _ => DirType::Threshold,
        };
        if dirs.iter().any(|d: &IconDir| d.path == name) {
            continue;
        }
        dirs.push(IconDir {
            path: name,
            size,
            scale: num("Scale").unwrap_or(1).max(1),
            kind,
            min: num("MinSize").unwrap_or(size),
            max: num("MaxSize").unwrap_or(size),
            threshold: num("Threshold").unwrap_or(2),
        });
    }
    ThemeIndex { inherits: list("Inherits"), dirs }
}

/// `gtk-icon-theme-name` of a GTK `settings.ini`, or `Theme` of KDE's `[Icons]`.
pub(crate) fn ini_value(text: &str, group: &str, key: &str) -> Option<String> {
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == format!("[{group}]");
            continue;
        }
        if in_group
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            let v = v.trim().trim_matches('"').to_string();
            return (!v.is_empty()).then_some(v);
        }
    }
    None
}

/// The icon theme the user selected (GTK 4, GTK 3, then KDE).
pub(crate) fn current_theme_name() -> Option<String> {
    let cfg = super::config_home();
    for f in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        if let Some(v) =
            std::fs::read_to_string(cfg.join(f)).ok().and_then(|t| ini_value(&t, "Settings", "gtk-icon-theme-name"))
        {
            return Some(v);
        }
    }
    std::fs::read_to_string(cfg.join("kdeglobals")).ok().and_then(|t| ini_value(&t, "Icons", "Theme"))
}

/// Icon base directories: `~/.icons`, `<data dirs>/icons`, then `/usr/share/pixmaps`.
fn base_dirs() -> Vec<PathBuf> {
    let mut v = vec![crate::path::home().join(".icons")];
    v.extend(super::data_dirs().into_iter().map(|d| d.join("icons")));
    v
}

struct Theme {
    /// Base directories that contain this theme.
    roots: Vec<PathBuf>,
    index: ThemeIndex,
}

/// A theme chain resolved against the base directories, with a cache of directory
/// listings (one `read_dir` per icon directory instead of a `stat` per candidate).
pub(crate) struct Lookup {
    themes: Vec<Theme>,
    pixmaps: Vec<PathBuf>,
    listings: parking_lot::Mutex<HashMap<PathBuf, Arc<Vec<String>>>>,
}

impl Lookup {
    /// Resolve `theme` (+ its parents, then `hicolor`) against `bases`.
    pub fn new(theme: Option<&str>, bases: &[PathBuf], pixmaps: Vec<PathBuf>) -> Lookup {
        let mut order: Vec<String> = Vec::new();
        let mut themes = Vec::new();
        let mut queue: Vec<String> = theme.map(|t| vec![t.to_string()]).unwrap_or_default();
        queue.push("hicolor".into());
        while let Some(name) = (!queue.is_empty()).then(|| queue.remove(0)) {
            if order.contains(&name) {
                continue;
            }
            order.push(name.clone());
            let roots: Vec<PathBuf> = bases.iter().map(|b| b.join(&name)).filter(|p| p.is_dir()).collect();
            let Some(index) = roots.iter().find_map(|r| std::fs::read_to_string(r.join("index.theme")).ok()) else {
                continue;
            };
            let index = parse_index(&index);
            // Parents come before hicolor (which is always last).
            // A separate cursor: skipped entries must not advance the insertion point.
            let mut at = queue.iter().position(|q| q == "hicolor").unwrap_or(queue.len());
            for p in &index.inherits {
                if p != "hicolor" && !order.contains(p) {
                    queue.insert(at, p.clone());
                    at += 1;
                }
            }
            themes.push(Theme { roots, index });
        }
        Lookup { themes, pixmaps, listings: parking_lot::Mutex::new(HashMap::new()) }
    }

    fn listing(&self, dir: &Path) -> Arc<Vec<String>> {
        if let Some(l) = self.listings.lock().get(dir) {
            return l.clone();
        }
        let names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let l = Arc::new(names);
        self.listings.lock().insert(dir.to_path_buf(), l.clone());
        l
    }

    fn find_in(&self, dir: &Path, name: &str) -> Option<PathBuf> {
        let l = self.listing(dir);
        for ext in ["png", "svg", "xpm"] {
            let f = format!("{name}.{ext}");
            if l.contains(&f) {
                return Some(dir.join(f));
            }
        }
        None
    }

    /// The best file for `name` at `size` px (logical) × `scale`.
    pub fn find(&self, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        if name.starts_with('/') {
            return Path::new(name).is_file().then(|| PathBuf::from(name));
        }
        if name.is_empty() || name.contains('/') {
            return None;
        }
        for t in &self.themes {
            // Exact matches first, then the closest directory.
            for d in t.index.dirs.iter().filter(|d| d.matches(size, scale)) {
                for r in &t.roots {
                    if let Some(p) = self.find_in(&r.join(&d.path), name) {
                        return Some(p);
                    }
                }
            }
            let mut best: Option<(u32, PathBuf)> = None;
            for d in &t.index.dirs {
                let dist = d.distance(size, scale);
                if best.as_ref().is_some_and(|b| b.0 <= dist) {
                    continue;
                }
                for r in &t.roots {
                    if let Some(p) = self.find_in(&r.join(&d.path), name) {
                        best = Some((dist, p));
                        break;
                    }
                }
            }
            if let Some((_, p)) = best {
                return Some(p);
            }
        }
        self.pixmaps.iter().find_map(|d| self.find_in(d, name))
    }
}

struct Cached {
    theme: Option<String>,
    built: Instant,
    lookup: Arc<Lookup>,
}

static CACHE: parking_lot::Mutex<Option<Cached>> = parking_lot::Mutex::new(None);

/// Theme and directory listings are rebuilt after this long (icons installed later
/// show up without a restart).
const TTL: Duration = Duration::from_secs(60);

fn lookup() -> Arc<Lookup> {
    let mut g = CACHE.lock();
    if let Some(c) = g.as_ref()
        && c.built.elapsed() < TTL
    {
        return c.lookup.clone();
    }
    let theme = current_theme_name();
    if let Some(c) = g.as_mut()
        && c.theme == theme
        && c.built.elapsed() < TTL * 10
    {
        // Same theme: keep the parsed indexes, drop the listings.
        c.lookup.listings.lock().clear();
        c.built = Instant::now();
        return c.lookup.clone();
    }
    let mut pixmaps: Vec<PathBuf> = super::data_dirs().into_iter().map(|d| d.join("pixmaps")).collect();
    pixmaps.dedup();
    let l = Arc::new(Lookup::new(theme.as_deref(), &base_dirs(), pixmaps));
    *g = Some(Cached { theme, built: Instant::now(), lookup: l.clone() });
    l
}

/// The icon file for `name` (a theme icon name or an absolute path) at `size` px.
pub(crate) fn icon_path(name: &str, size: u32, scale: u32) -> Option<PathBuf> {
    lookup().find(name, size.max(1), scale.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "[Icon Theme]\nName=T\nInherits=Parent,hicolor\nDirectories=16x16/apps,48x48/apps,scalable/apps\nScaledDirectories=16x16@2/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[16x16@2/apps]\nSize=16\nScale=2\nType=Fixed\n\n[48x48/apps]\nSize=48\n\n[scalable/apps]\nSize=64\nType=Scalable\nMinSize=8\nMaxSize=512\n";

    #[test]
    fn index_parsing() {
        let i = parse_index(INDEX);
        assert_eq!(i.inherits, ["Parent", "hicolor"]);
        assert_eq!(i.dirs.len(), 4);
        let t = i.dirs.iter().find(|d| d.path == "48x48/apps").unwrap();
        assert_eq!((t.kind, t.threshold), (DirType::Threshold, 2));
        assert!(t.matches(50, 1) && !t.matches(51, 1));
        assert!(i.dirs[3].matches(16, 2));
        assert_eq!(i.dirs[0].distance(24, 1), 8);
    }

    #[cfg(unix)]
    #[test]
    fn lookup_follows_sizes_inheritance_and_pixmaps() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().join("icons");
        let mk = |rel: &str| {
            let p = base.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
            p
        };
        std::fs::create_dir_all(base.join("T")).unwrap();
        std::fs::write(base.join("T/index.theme"), INDEX).unwrap();
        std::fs::create_dir_all(base.join("Parent")).unwrap();
        std::fs::write(
            base.join("Parent/index.theme"),
            "[Icon Theme]\nDirectories=32x32/apps\n[32x32/apps]\nSize=32\nType=Fixed\n",
        )
        .unwrap();
        std::fs::create_dir_all(base.join("hicolor")).unwrap();
        std::fs::write(
            base.join("hicolor/index.theme"),
            "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\nType=Fixed\n",
        )
        .unwrap();
        let small = mk("T/16x16/apps/app.png");
        let svg = mk("T/scalable/apps/app.svg");
        let hi = mk("T/16x16@2/apps/app.png");
        let parent_only = mk("Parent/32x32/apps/only.png");
        let hicolor = mk("hicolor/48x48/apps/fallback.png");
        let pix_dir = d.path().join("pixmaps");
        std::fs::create_dir_all(&pix_dir).unwrap();
        std::fs::write(pix_dir.join("legacy.xpm"), b"x").unwrap();
        let l = Lookup::new(Some("T"), std::slice::from_ref(&base), vec![pix_dir.clone()]);
        assert_eq!(l.find("app", 16, 1), Some(small));
        assert_eq!(l.find("app", 128, 1), Some(svg.clone()), "scalable covers 128");
        assert_eq!(l.find("app", 16, 2), Some(hi));
        assert_eq!(l.find("only", 16, 1), Some(parent_only), "inherited theme");
        assert_eq!(l.find("fallback", 16, 1), Some(hicolor), "hicolor last");
        assert_eq!(l.find("legacy", 16, 1), Some(pix_dir.join("legacy.xpm")));
        assert_eq!(l.find("missing", 16, 1), None);
        assert_eq!(l.find(&svg.to_string_lossy(), 16, 1), Some(svg), "absolute icon paths");
        assert_eq!(l.find("../escape", 16, 1), None);
    }

    #[test]
    fn inherits_with_skipped_entries_keeps_order_and_does_not_panic() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().join("icons");
        let theme = |name: &str, inherits: &str| {
            std::fs::create_dir_all(base.join(name)).unwrap();
            let text = format!("[Icon Theme]\nInherits={inherits}\nDirectories=\n");
            std::fs::write(base.join(name).join("index.theme"), text).unwrap();
        };
        theme("Mine", "hicolor,Mine,Extra,Other");
        theme("Extra", "");
        theme("Other", "");
        theme("hicolor", "");
        let l = Lookup::new(Some("Mine"), std::slice::from_ref(&base), Vec::new());
        let names: Vec<_> = l.themes.iter().map(|t| t.roots[0].file_name().unwrap().to_owned()).collect();
        assert_eq!(names, ["Mine", "Extra", "Other", "hicolor"]);
    }

    #[test]
    fn ini_values() {
        let gtk = "[Settings]\ngtk-icon-theme-name = Papirus-Dark\ngtk-application-prefer-dark-theme=1\n";
        assert_eq!(ini_value(gtk, "Settings", "gtk-icon-theme-name").as_deref(), Some("Papirus-Dark"));
        assert_eq!(ini_value("[Icons]\nTheme=breeze\n", "Icons", "Theme").as_deref(), Some("breeze"));
        assert_eq!(ini_value("[Other]\nTheme=x\n", "Icons", "Theme"), None);
    }
}
