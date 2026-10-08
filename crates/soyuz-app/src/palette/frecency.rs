//! Persistent palette memory (`palette.toml` in the config dir): folder frecency
//! (zoxide-style rank × recency weight), recent commands, recent shell commands and
//! ranger-style marks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Sum of ranks above which every rank is aged (×0.9) and tiny ones are dropped.
pub const MAX_TOTAL_RANK: f64 = 2000.0;
pub const MAX_RECENT_COMMANDS: usize = 12;
pub const MAX_RECENT_SHELL: usize = 20;
const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FolderStat {
    pub path: PathBuf,
    pub rank: f64,
    /// Unix seconds of the last visit.
    pub last: u64,
}

impl FolderStat {
    /// Frecency: rank weighted by how recently the folder was visited.
    pub fn score(&self, now: u64) -> f64 {
        let age = now.saturating_sub(self.last);
        let w = if age < HOUR {
            4.0
        } else if age < DAY {
            2.0
        } else if age < WEEK {
            0.5
        } else {
            0.25
        };
        self.rank * w
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub folders: Vec<FolderStat>,
    /// Action strings, most recent first.
    pub commands: Vec<String>,
    /// Shell commands run from the palette, most recent first.
    pub shell: Vec<String>,
    /// Mark key → folder.
    pub marks: BTreeMap<String, PathBuf>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    } else {
        a == b
    }
}

impl Memory {
    /// Record a visit to `path` at `now`.
    pub fn visit(&mut self, path: &Path, now: u64) {
        match self.folders.iter_mut().find(|f| same_path(&f.path, path)) {
            Some(f) => {
                f.rank += 1.0;
                f.last = now;
            }
            None => self.folders.push(FolderStat { path: path.to_path_buf(), rank: 1.0, last: now }),
        }
        let total: f64 = self.folders.iter().map(|f| f.rank).sum();
        if total > MAX_TOTAL_RANK {
            for f in &mut self.folders {
                f.rank *= 0.9;
            }
            self.folders.retain(|f| f.rank >= 1.0);
        }
    }

    /// Folders by descending frecency.
    pub fn top_folders(&self, now: u64) -> Vec<&FolderStat> {
        let mut v: Vec<&FolderStat> = self.folders.iter().collect();
        v.sort_by(|a, b| b.score(now).total_cmp(&a.score(now)).then_with(|| b.last.cmp(&a.last)));
        v
    }

    /// Forget a folder (e.g. it no longer exists).
    pub fn forget(&mut self, path: &Path) {
        self.folders.retain(|f| !same_path(&f.path, path));
    }

    /// Move `action` to the front of the recent commands.
    pub fn used_command(&mut self, action: &str) {
        push_front(&mut self.commands, action, MAX_RECENT_COMMANDS);
    }

    pub fn used_shell(&mut self, line: &str) {
        push_front(&mut self.shell, line, MAX_RECENT_SHELL);
    }

    pub fn load(path: &Path) -> Memory {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            let _ = soyuz_fs::create_dir_all(dir);
        }
        soyuz_fs::write_atomic(path, text.as_bytes()).map_err(|e| e.to_string())
    }
}

fn push_front(v: &mut Vec<String>, item: &str, max: usize) {
    let item = item.trim();
    if item.is_empty() {
        return;
    }
    v.retain(|x| x != item);
    v.insert(0, item.to_string());
    v.truncate(max);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequent_and_recent_rank_higher() {
        let mut m = Memory::default();
        let now = 1_000_000_000;
        for _ in 0..5 {
            m.visit(Path::new("/old/frequent"), now - 3 * DAY);
        }
        m.visit(Path::new("/new"), now - 60);
        m.visit(Path::new("/stale"), now - 30 * DAY);
        let top: Vec<_> = m.top_folders(now).iter().map(|f| f.path.clone()).collect();
        // 5 visits 3 days ago score 5 × 0.5 = 2.5; one visit a minute ago scores 1 × 4.
        assert_eq!(top[0], PathBuf::from("/new"));
        assert_eq!(top[1], PathBuf::from("/old/frequent"));
        assert_eq!(top[2], PathBuf::from("/stale"));
        for _ in 0..10 {
            m.visit(Path::new("/old/frequent"), now - 2 * HOUR);
        }
        assert_eq!(m.top_folders(now)[0].path, PathBuf::from("/old/frequent"));
    }

    #[test]
    fn aging_keeps_total_bounded() {
        let mut m = Memory::default();
        for i in 0..3000u64 {
            m.visit(Path::new(&format!("/d{}", i % 7)), i);
        }
        let total: f64 = m.folders.iter().map(|f| f.rank).sum();
        assert!(total <= MAX_TOTAL_RANK + 1.0, "{total}");
        assert_eq!(m.folders.len(), 7);
        m.forget(Path::new("/d3"));
        assert_eq!(m.folders.len(), 6);
    }

    #[test]
    fn recent_lists() {
        let mut m = Memory::default();
        for i in 0..20 {
            m.used_command(&format!("a{i}"));
        }
        m.used_command("a5");
        assert_eq!(m.commands.len(), MAX_RECENT_COMMANDS);
        assert_eq!(m.commands[0], "a5");
        assert_eq!(m.commands.iter().filter(|c| *c == "a5").count(), 1);
        m.used_shell("  ");
        assert!(m.shell.is_empty());
    }

    #[test]
    fn persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("palette.toml");
        let mut m = Memory::default();
        m.visit(Path::new("/x"), 5);
        m.used_command("copy");
        m.marks.insert("a".into(), PathBuf::from("/x"));
        m.save(&path).unwrap();
        assert_eq!(Memory::load(&path), m);
        assert_eq!(Memory::load(&dir.path().join("missing.toml")), Memory::default());
    }
}
