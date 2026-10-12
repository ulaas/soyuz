//! Sidebar state (frontend-agnostic): favorites (user-ordered), the standard places,
//! collapsed sections, the Portal pins' state and the unmounted-devices cache. The GUI
//! draws it in `ui::sidebar`; devices (Linux UDisks2, GTK bookmarks) are
//! [`crate::models::devices`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use soyuz_core::config::SidebarSection;

use crate::icon::IconId;
use crate::models::devices::DevicesCache;
use crate::models::portal::PortalState;
use crate::state::session::FavoriteEntry;

/// A standard user folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceRole {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
}

impl PlaceRole {
    /// The standard folders under home, in sidebar order (without Home).
    pub const STANDARD: [PlaceRole; 6] = [
        PlaceRole::Desktop,
        PlaceRole::Documents,
        PlaceRole::Downloads,
        PlaceRole::Pictures,
        PlaceRole::Music,
        PlaceRole::Videos,
    ];

    /// The English folder name (`~/<name>` is the fallback path).
    pub fn english(self) -> &'static str {
        match self {
            PlaceRole::Home => "Home",
            PlaceRole::Desktop => "Desktop",
            PlaceRole::Documents => "Documents",
            PlaceRole::Downloads => "Downloads",
            PlaceRole::Pictures => "Pictures",
            PlaceRole::Music => "Music",
            PlaceRole::Videos => "Videos",
        }
    }

    /// The label in the current UI language.
    pub fn label(self) -> String {
        match self {
            PlaceRole::Home => tr!("app-place-home"),
            PlaceRole::Desktop => tr!("app-place-desktop"),
            PlaceRole::Documents => tr!("app-place-documents"),
            PlaceRole::Downloads => tr!("app-place-downloads"),
            PlaceRole::Pictures => tr!("app-place-pictures"),
            PlaceRole::Music => tr!("app-place-music"),
            PlaceRole::Videos => tr!("app-place-videos"),
        }
        .to_string()
    }
}

/// A standard place under the home directory.
#[derive(Clone, Debug)]
pub struct Place {
    pub label: String,
    pub role: PlaceRole,
    pub path: PathBuf,
}

static PROBED: std::sync::OnceLock<Vec<Place>> = std::sync::OnceLock::new();
static PROBE_STARTED: std::sync::Once = std::sync::Once::new();

/// Home plus the standard folders: `~/<English name>`, or (Linux / BSD) the folder
/// `user-dirs.dirs` names for that role (`~/Belgeler` on a Turkish desktop). A role
/// set to the home folder itself (the XDG way of disabling it) is left out. Labels are
/// the English names until [`localized`].
fn candidate_places(xdg: Option<&soyuz_fs::xdg_dirs::UserDirs>) -> Vec<Place> {
    let home = soyuz_fs::home();
    let mut out = vec![Place { label: "Home".into(), role: PlaceRole::Home, path: home.clone() }];
    for role in PlaceRole::STANDARD {
        let path = match xdg.and_then(|d| xdg_role(d, role)) {
            Some(p) if p == home => continue,
            Some(p) => p,
            None => home.join(role.english()),
        };
        out.push(Place { label: role.english().into(), role, path });
    }
    out
}

/// The XDG user folder for a standard place.
fn xdg_role(d: &soyuz_fs::xdg_dirs::UserDirs, role: PlaceRole) -> Option<PathBuf> {
    match role {
        PlaceRole::Desktop => d.desktop.clone(),
        PlaceRole::Documents => d.documents.clone(),
        PlaceRole::Downloads => d.downloads.clone(),
        PlaceRole::Pictures => d.pictures.clone(),
        PlaceRole::Music => d.music.clone(),
        PlaceRole::Videos => d.videos.clone(),
        PlaceRole::Home => None,
    }
}

fn localized(mut places: Vec<Place>) -> Vec<Place> {
    for p in &mut places {
        p.label = p.role.label();
    }
    places
}

/// Home plus the standard user folders that exist. The existence checks run once on a
/// worker thread (never on the UI thread); until they finish every candidate is listed.
/// Labels are in the current UI language.
pub fn standard_places() -> Vec<Place> {
    if let Some(p) = PROBED.get() {
        return localized(p.clone());
    }
    PROBE_STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("soyuz-places".into()).spawn(|| {
            // `user-dirs.dirs` (Linux / BSD; empty elsewhere): read here, never on the UI thread.
            let xdg = soyuz_fs::xdg_dirs::user_dirs();
            let places: Vec<Place> = candidate_places(Some(&xdg))
                .into_iter()
                .enumerate()
                .filter(|(i, p)| *i == 0 || p.path.is_dir())
                .map(|(_, p)| p)
                .collect();
            let _ = PROBED.set(places);
        });
    });
    localized(candidate_places(None))
}

/// The standard places probe has finished.
pub fn places_probed() -> bool {
    PROBED.get().is_some()
}

/// The standard place `path` is, if any: home, an XDG folder with a localized name
/// (`~/Belgeler`), or `~/<English name>` (any case).
pub fn place_role(path: &Path) -> Option<PlaceRole> {
    let home = soyuz_fs::home();
    if path == home {
        return Some(PlaceRole::Home);
    }
    if let Some(p) = PROBED.get().and_then(|v| v.iter().find(|p| p.path == path)) {
        return Some(p.role);
    }
    if path.parent() == Some(home.as_path())
        && let Some(n) = path.file_name().and_then(|n| n.to_str())
    {
        return PlaceRole::STANDARD.into_iter().find(|r| r.english().eq_ignore_ascii_case(n));
    }
    None
}

/// Icon of a standard place.
pub fn place_icon(role: PlaceRole) -> IconId {
    match role {
        PlaceRole::Home => IconId::House,
        PlaceRole::Desktop => IconId::Desktop,
        PlaceRole::Documents => IconId::FileText,
        PlaceRole::Downloads => IconId::DownloadSimple,
        PlaceRole::Pictures => IconId::Image,
        PlaceRole::Music => IconId::MusicNotes,
        PlaceRole::Videos => IconId::FilmStrip,
    }
}

/// Icon for a favorite path (standard places get their own icons).
pub fn favorite_icon(path: &Path) -> IconId {
    place_role(path).map_or(IconId::FolderSimple, place_icon)
}

pub struct SidebarState {
    pub favorites: Vec<FavoriteEntry>,
    /// Favorites were edited by the user (persist them in the session).
    pub favorites_customized: bool,
    /// Collapsed section names.
    pub collapsed: BTreeSet<String>,
    /// Favorite being dragged (index).
    pub drag: Option<usize>,
    /// Default favorites were refreshed from the background places probe.
    places_applied: bool,
    /// `i18n::generation()` the default favorites were labelled in.
    lang_gen: u64,
    /// The Portal section (pinned folders).
    pub portal: PortalState,
    /// Unmounted devices of the Devices section (Linux).
    pub devices: DevicesCache,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self::new()
    }
}

impl SidebarState {
    pub fn new() -> Self {
        SidebarState {
            favorites: crate::core::default_favorites(),
            favorites_customized: false,
            collapsed: BTreeSet::new(),
            drag: None,
            places_applied: places_probed(),
            lang_gen: crate::i18n::generation(),
            portal: Default::default(),
            devices: Default::default(),
        }
    }

    /// Per frame while the sidebar shows: default favorites follow the places probe and
    /// the UI language (customized ones are kept). Returns `true` while the probe is
    /// still running (ask for another frame soon).
    pub fn refresh_favorites(&mut self) -> bool {
        let mut waiting = false;
        if !self.places_applied {
            if places_probed() {
                self.places_applied = true;
                if !self.favorites_customized {
                    self.favorites = crate::core::default_favorites();
                }
            } else {
                waiting = true;
            }
        }
        // Language switched: relabel the default favorites (customized ones keep their names).
        let gen_now = crate::i18n::generation();
        if self.lang_gen != gen_now {
            self.lang_gen = gen_now;
            if !self.favorites_customized {
                self.favorites = crate::core::default_favorites();
            }
        }
        waiting
    }

    /// Whether section `sec` is expanded.
    pub fn is_open(&self, sec: SidebarSection) -> bool {
        !self.collapsed.contains(section_key(sec))
    }

    /// Expand or collapse section `sec`.
    pub fn set_open(&mut self, sec: SidebarSection, open: bool) {
        let key = section_key(sec);
        if open {
            self.collapsed.remove(key);
        } else {
            self.collapsed.insert(key.to_string());
        }
    }
}

/// Section heading shown to the user.
pub fn section_name(s: SidebarSection) -> &'static str {
    match s {
        SidebarSection::Favorites => tr!("app-sidebar-favorites"),
        SidebarSection::Devices => tr!("app-sidebar-devices"),
        SidebarSection::Portal => tr!("app-sidebar-portal"),
        SidebarSection::Cloud => tr!("app-sidebar-cloud"),
        SidebarSection::Tags => tr!("app-sidebar-tags"),
        SidebarSection::Network => tr!("app-sidebar-network"),
        SidebarSection::Trash => tr!("app-sidebar-other"),
        SidebarSection::Tree => tr!("app-sidebar-folders"),
        SidebarSection::Smart => tr!("app-sidebar-smart"),
    }
}

/// Stable section key (stored in the session's collapsed set; never translated).
pub fn section_key(s: SidebarSection) -> &'static str {
    match s {
        SidebarSection::Favorites => "Favorites",
        SidebarSection::Devices => "Devices",
        SidebarSection::Portal => "Portal",
        SidebarSection::Cloud => "Cloud",
        SidebarSection::Tags => "Tags",
        SidebarSection::Network => "Network",
        SidebarSection::Trash => "Other",
        SidebarSection::Tree => "Folders",
        SidebarSection::Smart => "Smart folders",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_folders_replace_the_english_defaults() {
        let home = soyuz_fs::home();
        let xdg = soyuz_fs::xdg_dirs::UserDirs {
            documents: Some(home.join("Belgeler")),
            desktop: Some(home.clone()), // disabled the XDG way
            ..Default::default()
        };
        let places = candidate_places(Some(&xdg));
        assert_eq!(places[0].path, home);
        let docs = places.iter().find(|p| p.label == "Documents").unwrap();
        assert_eq!(docs.path, home.join("Belgeler"));
        assert_eq!(docs.role, PlaceRole::Documents);
        assert!(places.iter().all(|p| p.label != "Desktop"), "a role pointing at home is left out");
        assert_eq!(places.iter().find(|p| p.label == "Music").unwrap().path, home.join("Music"), "fallback");
        assert_eq!(candidate_places(None).len(), 1 + PlaceRole::STANDARD.len());
    }

    #[test]
    fn standard_folders_are_recognized() {
        let home = soyuz_fs::home();
        assert_eq!(place_role(&home), Some(PlaceRole::Home));
        assert_eq!(place_role(&home.join("downloads")), Some(PlaceRole::Downloads));
        assert_eq!(place_role(&home.join("Projects")), None);
    }
}
