//! Saved workspaces: named window layouts — tabs, groups, panes, splits and each
//! pane's view — kept in `workspaces.toml` next to the config. A workspace can have an
//! optional hotkey (`open_workspace:<n>` for the first nine).

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::session::WindowSession;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SavedWorkspace {
    pub name: String,
    /// Key chord (`"Ctrl+Alt+1"`); empty = none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hotkey: String,
    /// Unix seconds of the last save.
    pub saved: u64,
    pub layout: WindowSession,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WorkspaceStore {
    #[serde(rename = "workspace")]
    pub workspaces: Vec<SavedWorkspace>,
}

impl WorkspaceStore {
    /// Load (a broken file is kept and never overwritten, see `store_file`).
    pub fn load(path: &Path) -> WorkspaceStore {
        crate::services::store_file::load(path, Self::from_toml).value()
    }

    pub fn from_toml(text: &str) -> Result<WorkspaceStore, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.workspaces.iter().position(|w| w.name.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, name: &str) -> Option<&SavedWorkspace> {
        self.position(name).map(|i| &self.workspaces[i])
    }

    /// Save `layout` under `name` (replacing a workspace with that name, keeping its
    /// place and hotkey). Returns its index.
    pub fn upsert(&mut self, name: &str, layout: WindowSession, now: u64) -> usize {
        let name = name.trim().to_string();
        match self.position(&name) {
            Some(i) => {
                let w = &mut self.workspaces[i];
                w.layout = layout;
                w.saved = now;
                w.name = name;
                i
            }
            None => {
                self.workspaces.push(SavedWorkspace { name, hotkey: String::new(), saved: now, layout });
                self.workspaces.len() - 1
            }
        }
    }

    pub fn rename(&mut self, index: usize, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.position(name).is_some_and(|i| i != index) {
            return false;
        }
        match self.workspaces.get_mut(index) {
            Some(w) => {
                w.name = name.to_string();
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<SavedWorkspace> {
        (index < self.workspaces.len()).then(|| self.workspaces.remove(index))
    }

    /// Move a workspace up / down (its `open_workspace:<n>` slot follows its position).
    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.workspaces.len() && b < self.workspaces.len() {
            self.workspaces.swap(a, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::session::{GroupSession, PaneSession, TabSession};
    use crate::state::view_memory::ViewState;

    fn layout(n: usize) -> WindowSession {
        WindowSession {
            active_tab: n.saturating_sub(1),
            tabs: (0..n)
                .map(|i| TabSession {
                    dual: i == 0,
                    quad: i == 1,
                    split: 0.3,
                    split2: 0.6,
                    group: (i == 0).then_some(0),
                    panes: vec![PaneSession {
                        location: format!("/p{i}"),
                        view: Some(ViewState { zoom: 128, ..ViewState::default() }),
                        ..Default::default()
                    }],
                    ..Default::default()
                })
                .collect(),
            groups: vec![GroupSession { name: "G".into(), color: "green".into(), collapsed: false }],
            ..Default::default()
        }
    }

    #[test]
    fn round_trip_and_edit() {
        let mut s = WorkspaceStore::default();
        assert_eq!(s.upsert("Photos", layout(2), 10), 0);
        assert_eq!(s.upsert("Code", layout(3), 11), 1);
        s.workspaces[0].hotkey = "Ctrl+Alt+1".into();
        // Same name (any case) updates in place and keeps the hotkey.
        assert_eq!(s.upsert("photos", layout(1), 12), 0);
        assert_eq!(s.workspaces[0].layout.tabs.len(), 1);
        assert_eq!(s.workspaces[0].hotkey, "Ctrl+Alt+1");
        assert_eq!(s.workspaces[0].saved, 12);
        let text = s.to_toml();
        assert!(text.contains("[[workspace]]"), "{text}");
        let back = WorkspaceStore::from_toml(&text).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get("CODE").unwrap().layout.tabs[0].panes[0].view.as_ref().unwrap().zoom, 128);
        assert!(!s.rename(1, "Photos"), "names stay unique");
        assert!(s.rename(1, "Rust"));
        s.swap(0, 1);
        assert_eq!(s.workspaces[0].name, "Rust");
        assert_eq!(s.remove(0).unwrap().name, "Rust");
        assert!(s.remove(5).is_none());
        assert_eq!(WorkspaceStore::from_toml("").unwrap(), WorkspaceStore::default());
    }

    #[test]
    fn load_file_and_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspaces.toml");
        assert_eq!(WorkspaceStore::load(&path), WorkspaceStore::default(), "missing file");
        let mut s = WorkspaceStore::default();
        s.upsert("A", layout(1), 1);
        std::fs::write(&path, s.to_toml()).unwrap();
        assert_eq!(WorkspaceStore::load(&path), s);
        std::fs::write(&path, "workspace = 5").unwrap();
        assert_eq!(WorkspaceStore::load(&path), WorkspaceStore::default());
    }
}
