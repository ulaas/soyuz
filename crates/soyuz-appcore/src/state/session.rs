//! `session.toml`: tabs, panes, splits, scroll/selection plus small UI preferences
//! (favorites order, hidden-files toggle, theme, collapsed sidebar sections). Saved
//! periodically and on exit; restored at startup when `[behaviour] restore_session`.
//! Tab groups, quad layouts and the other in-process windows (with their positions) are
//! part of it too; a saved workspace (`workspaces.toml`) is one [`WindowSession`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::config::Orientation;

pub const SESSION_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSession {
    /// `Location::display()` string form.
    pub location: String,
    pub cursor: Option<String>,
    pub selection: Vec<String>,
    pub scroll: f32,
    /// The pane's view (saved workspaces; `None` = the folder's remembered view).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<crate::state::view_memory::ViewState>,
}

impl Default for PaneSession {
    fn default() -> Self {
        PaneSession { location: String::new(), cursor: None, selection: Vec::new(), scroll: 0.0, view: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabSession {
    pub pinned: bool,
    pub dual: bool,
    pub orientation: Orientation,
    pub split: f32,
    pub focused: usize,
    pub sync_nav: bool,
    /// 2×2 layout (needs four panes).
    pub quad: bool,
    /// Second split fraction (rows of the quad layout).
    pub split2: f32,
    /// Index into the window's `groups`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
    pub panes: Vec<PaneSession>,
}

impl Default for TabSession {
    fn default() -> Self {
        TabSession {
            pinned: false,
            dual: false,
            orientation: Orientation::Vertical,
            split: 0.5,
            focused: 0,
            sync_nav: false,
            quad: false,
            split2: 0.5,
            group: None,
            panes: Vec::new(),
        }
    }
}

/// A named, colored tab group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GroupSession {
    pub name: String,
    /// Tag color name (`blue`, `green`, …).
    pub color: String,
    pub collapsed: bool,
}

/// One window's tabs and groups (+ where the window was).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WindowSession {
    pub active_tab: usize,
    pub tabs: Vec<TabSession>,
    pub groups: Vec<GroupSession>,
    /// Outer position / inner size in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
    pub maximized: bool,
    /// Secondary windows: the preview pane and the bottom panel (`None` = defaults).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<PanelSession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<PanelSession>,
    /// Secondary windows: this window shows the editors (`models::owners`; `false` = the
    /// main window does).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub editors: bool,
}

/// A side / bottom panel of a secondary window: shown, and its width or height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PanelSession {
    pub open: bool,
    pub size: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FavoriteEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub version: u32,
    pub active_tab: usize,
    pub tabs: Vec<TabSession>,
    /// User favorites in sidebar order (`None` = not customized yet → standard places).
    pub favorites: Option<Vec<FavoriteEntry>>,
    pub show_hidden: Option<bool>,
    /// `"system" | "dark" | "light"`; `None` = use the config.
    pub theme: Option<String>,
    pub collapsed_sections: Vec<String>,
    pub sidebar_visible: Option<bool>,
    pub checkbox_mode: bool,
    /// The first-run welcome card was dismissed.
    pub welcome_seen: bool,
    /// Drop shelf contents (kept across restarts).
    pub shelf: Vec<PathBuf>,
    /// Expanded folders of the folder tree.
    pub tree_expanded: Vec<PathBuf>,
    /// Files open in the built-in editor.
    pub editors: Vec<PathBuf>,
    /// Tab groups of the main window (`TabSession::group` indexes them).
    pub groups: Vec<GroupSession>,
    /// The other in-process windows.
    pub windows: Vec<WindowSession>,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            version: SESSION_VERSION,
            active_tab: 0,
            tabs: Vec::new(),
            favorites: None,
            show_hidden: None,
            theme: None,
            collapsed_sections: Vec::new(),
            sidebar_visible: None,
            checkbox_mode: false,
            welcome_seen: false,
            shelf: Vec::new(),
            tree_expanded: Vec::new(),
            editors: Vec::new(),
            groups: Vec::new(),
            windows: Vec::new(),
        }
    }
}

impl Session {
    pub fn path() -> PathBuf {
        soyuz_core::config::config_dir().join("session.toml")
    }

    pub fn load(path: &Path) -> Option<Session> {
        let text = std::fs::read_to_string(path).ok()?;
        match toml::from_str::<Session>(&text) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("{}: {e}; ignoring the saved session", path.display());
                None
            }
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Write atomically (temp file + rename); creates the config dir if needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            soyuz_fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        soyuz_fs::write_atomic(path, self.to_toml().as_bytes()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            active_tab: 1,
            tabs: vec![
                TabSession {
                    pinned: true,
                    panes: vec![PaneSession { location: "/a".into(), ..Default::default() }],
                    ..Default::default()
                },
                TabSession {
                    pinned: false,
                    dual: true,
                    orientation: Orientation::Horizontal,
                    split: 0.3,
                    focused: 1,
                    sync_nav: true,
                    quad: false,
                    split2: 0.5,
                    group: Some(0),
                    panes: vec![
                        PaneSession {
                            location: "/b".into(),
                            cursor: Some("x.txt".into()),
                            selection: vec!["x.txt".into(), "y.txt".into()],
                            scroll: 240.0,
                            view: None,
                        },
                        PaneSession { location: "trash:".into(), ..Default::default() },
                    ],
                },
            ],
            favorites: Some(vec![FavoriteEntry { name: "Src".into(), path: "/src".into() }]),
            show_hidden: Some(true),
            theme: Some("light".into()),
            collapsed_sections: vec!["cloud".into()],
            sidebar_visible: Some(false),
            checkbox_mode: true,
            groups: vec![GroupSession { name: "Work".into(), color: "blue".into(), collapsed: true }],
            windows: vec![WindowSession {
                active_tab: 0,
                tabs: vec![TabSession {
                    quad: true,
                    split2: 0.4,
                    panes: (0..4).map(|i| PaneSession { location: format!("/q{i}"), ..Default::default() }).collect(),
                    ..Default::default()
                }],
                groups: Vec::new(),
                pos: Some([100.0, 80.0]),
                size: Some([900.0, 600.0]),
                maximized: false,
                preview: Some(PanelSession { open: true, size: 410.0 }),
                editors: true,
                bottom: Some(PanelSession { open: false, size: 300.0 }),
            }],
            ..Default::default()
        }
    }

    /// Sessions written before a field existed still load (`#[serde(default)]`).
    #[test]
    fn old_window_sessions_load() {
        let s: WindowSession = toml::from_str(
            "active_tab = 1
maximized = true
",
        )
        .unwrap();
        assert!(s.maximized && !s.editors && s.preview.is_none());
        let s = WindowSession { editors: true, ..Default::default() };
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("editors = true"));
        assert_eq!(toml::from_str::<WindowSession>(&text).unwrap(), s);
        assert!(!toml::to_string(&WindowSession::default()).unwrap().contains("editors"), "not written when false");
    }

    #[test]
    fn round_trip_text() {
        let s = sample();
        let back: Session = toml::from_str(&s.to_toml()).unwrap();
        assert_eq!(back, s);
        let empty: Session = toml::from_str("").unwrap();
        assert_eq!(empty, Session::default());
        // Sessions written before the per-window panels were saved still load.
        let old: Session = toml::from_str("[[windows]]\nactive_tab = 0\nmaximized = true\n").unwrap();
        assert_eq!((old.windows[0].preview, old.windows[0].bottom), (None, None));
    }

    #[test]
    fn save_and_load_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("session.toml");
        sample().save(&path).unwrap();
        assert_eq!(Session::load(&path), Some(sample()));
        std::fs::write(&path, "tabs = 5").unwrap();
        assert_eq!(Session::load(&path), None, "corrupt sessions are ignored");
    }
}
