//! Synchronized browsing (F7): when one pane of a tab navigates, the other visible panes
//! follow the same *relative* move (into `src/ui`, up two levels, over to a sibling)
//! where the resulting folder exists. Pure path mapping; existence is probed off the UI
//! thread by the lister.

use std::path::{Component, Path, PathBuf};

/// What a following pane should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncMove {
    /// Go up (always exists); select the folder we came out of.
    Up { target: PathBuf, select: Option<String> },
    /// Go to `target` if it exists (probe first).
    Probe(PathBuf),
}

fn eq_component(a: &Component<'_>, b: &Component<'_>) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// How the leading pane moved: `up` levels, then down through `down`.
pub fn relative_move(old: &Path, new: &Path) -> Option<(usize, Vec<PathBuf>)> {
    let a: Vec<Component<'_>> = old.components().collect();
    let b: Vec<Component<'_>> = new.components().collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| eq_component(x, y)).count();
    // Different drives / roots: nothing relative to follow.
    if common == 0 {
        return None;
    }
    let down = b[common..].iter().map(|c| PathBuf::from(c.as_os_str())).collect();
    Some((a.len() - common, down))
}

/// Where a pane showing `other` should go when the leading pane moved `old` → `new`.
/// Without `relative`, only single steps are followed (into a child, up one level).
pub fn sync_target(old: &Path, new: &Path, other: &Path, relative: bool) -> Option<SyncMove> {
    let (up, down) = relative_move(old, new)?;
    if up == 0 && down.is_empty() {
        return None;
    }
    if !relative && !((up == 0 && down.len() == 1) || (up == 1 && down.is_empty())) {
        return None;
    }
    let mut target = other.to_path_buf();
    let mut left = None;
    for _ in 0..up {
        left = target.file_name().map(|n| n.to_string_lossy().into_owned());
        if !target.pop() || target.as_os_str().is_empty() {
            return None;
        }
    }
    if down.is_empty() {
        return Some(SyncMove::Up { target, select: left });
    }
    for c in down {
        target.push(c);
    }
    (target != other).then_some(SyncMove::Probe(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn follows_relative_moves() {
        let other = p("/b/proj");
        // Into a child.
        assert_eq!(
            sync_target(&p("/a/proj"), &p("/a/proj/src"), &other, true),
            Some(SyncMove::Probe(p("/b/proj/src")))
        );
        // Several levels down at once.
        assert_eq!(
            sync_target(&p("/a/proj"), &p("/a/proj/src/ui"), &other, true),
            Some(SyncMove::Probe(p("/b/proj/src/ui")))
        );
        // Up one level selects where we came from.
        assert_eq!(
            sync_target(&p("/a/proj"), &p("/a"), &other, true),
            Some(SyncMove::Up { target: p("/b"), select: Some("proj".into()) })
        );
        // Sibling: up one, down one.
        assert_eq!(
            sync_target(&p("/a/proj/src"), &p("/a/proj/docs"), &p("/b/proj/src"), true),
            Some(SyncMove::Probe(p("/b/proj/docs")))
        );
        // Strict mode only follows single steps.
        assert_eq!(sync_target(&p("/a/proj/src"), &p("/a/proj/docs"), &p("/b/proj/src"), false), None);
        assert!(sync_target(&p("/a/proj"), &p("/a/proj/src"), &other, false).is_some());
        // Can't go above the other pane's root.
        assert_eq!(sync_target(&p("/a/b/c"), &p("/a"), &p("/x"), true), None);
        // Same place: nothing.
        assert_eq!(sync_target(&p("/a"), &p("/a"), &other, true), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_case_insensitively() {
        assert_eq!(relative_move(Path::new(r"C:\Data\X"), Path::new(r"c:\data\x\Y")), Some((0, vec![p("Y")])));
        assert_eq!(
            sync_target(Path::new(r"C:\a"), Path::new(r"D:\b"), Path::new(r"E:\c"), true),
            None,
            "different drives"
        );
    }
}
