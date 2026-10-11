//! Internal file clipboard (copy / cut). Copy / cut also put the files on the OS
//! clipboard (`services::os_clipboard`: CF_HDROP, `x-special/gnome-copied-files`, …) and
//! paste prefers files found there, see `services::os_clipboard::with_files`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipMode {
    Copy,
    Cut,
}

#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    mode: Option<ClipMode>,
    paths: Vec<PathBuf>,
    cut_set: HashSet<PathBuf>,
}

impl Clipboard {
    pub fn set(&mut self, mode: ClipMode, paths: Vec<PathBuf>) {
        let mut seen = HashSet::new();
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| seen.insert(p.clone())).collect();
        self.cut_set = if mode == ClipMode::Cut { paths.iter().cloned().collect() } else { HashSet::new() };
        self.mode = (!paths.is_empty()).then_some(mode);
        self.paths = paths;
    }

    pub fn clear(&mut self) {
        *self = Clipboard::default();
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn mode(&self) -> Option<ClipMode> {
        self.mode
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Is `path` waiting to be moved (drawn dimmed / dashed)?
    pub fn is_cut(&self, path: &Path) -> bool {
        !self.cut_set.is_empty() && self.cut_set.contains(path)
    }

    /// Take the contents for a paste: a cut clipboard is consumed by the paste (the
    /// caller clears it once the move succeeded), a copy clipboard stays.
    pub fn for_paste(&self) -> Option<(ClipMode, Vec<PathBuf>)> {
        self.mode.map(|m| (m, self.paths.clone()))
    }

    /// Forget paths that were moved / deleted elsewhere.
    pub fn forget(&mut self, gone: &[PathBuf]) {
        if gone.is_empty() || self.paths.is_empty() {
            return;
        }
        self.paths.retain(|p| !gone.contains(p));
        self.cut_set.retain(|p| !gone.contains(p));
        if self.paths.is_empty() {
            self.mode = None;
        }
    }

    /// Text placed on the OS clipboard: one path per line.
    pub fn os_text(paths: &[PathBuf]) -> String {
        paths.iter().map(|p| soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn copy_and_cut_state() {
        let mut c = Clipboard::default();
        assert!(c.is_empty());
        assert!(c.for_paste().is_none());
        c.set(ClipMode::Cut, vec![p("/a/x"), p("/a/y"), p("/a/x")]);
        assert_eq!(c.paths().len(), 2, "duplicates dropped");
        assert!(c.is_cut(&p("/a/x")));
        assert!(!c.is_cut(&p("/a/z")));
        assert_eq!(c.for_paste().unwrap().0, ClipMode::Cut);
        c.set(ClipMode::Copy, vec![p("/b")]);
        assert!(!c.is_cut(&p("/a/x")), "a copy replaces the cut");
        assert!(!c.is_cut(&p("/b")), "copied items are not dimmed");
        assert_eq!(c.mode(), Some(ClipMode::Copy));
        c.set(ClipMode::Copy, vec![]);
        assert!(c.mode().is_none());
    }

    #[test]
    fn forget_and_clear() {
        let mut c = Clipboard::default();
        c.set(ClipMode::Cut, vec![p("/a"), p("/b")]);
        c.forget(&[p("/a")]);
        assert!(!c.is_cut(&p("/a")));
        assert!(c.is_cut(&p("/b")));
        c.forget(&[p("/b")]);
        assert!(c.is_empty());
        assert!(c.mode().is_none());
        c.set(ClipMode::Copy, vec![p("/c")]);
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn os_text_lists_paths() {
        assert_eq!(Clipboard::os_text(&[p("/a/b"), p("/c")]), "/a/b\n/c");
    }
}
