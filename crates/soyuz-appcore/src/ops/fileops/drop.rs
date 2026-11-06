//! Drag and drop, the frontend-free part: what a drag carries, which effect a drop has
//! (modifiers, `[behaviour] drag_default`, same volume or not), which targets are valid,
//! and where an external drop lands. The GUI (hover, highlight, badge, spring-loading,
//! the "ask" menu) is `crate::ui::dnd`.
//!
//! Modifier semantics (Explorer-like): Ctrl = copy, Shift = move, Alt (or Ctrl+Shift) =
//! link; otherwise `[behaviour] drag_default` (auto = move on the same volume, copy
//! across volumes; `ask` shows a small menu at the drop point).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use soyuz_core::config::DragDefault;

use crate::geom::{Point, Rect};
use crate::state::pane::PaneId;
use crate::state::workspace_windows::WindowId;

/// What an internal drag carries.
#[derive(Clone, Debug)]
pub struct DragPayload {
    pub paths: Vec<PathBuf>,
    pub source_pane: Option<PaneId>,
    pub default: DragDefault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropEffect {
    Copy,
    Move,
    Link,
}

/// Modifier state relevant to a drop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropMods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Effect of a drop; `None` = ask the user (`drag_default = "ask"` without modifiers).
pub fn resolve_effect(m: DropMods, same_volume: bool, default: DragDefault) -> Option<DropEffect> {
    if m.alt || (m.ctrl && m.shift) {
        return Some(DropEffect::Link);
    }
    if m.ctrl {
        return Some(DropEffect::Copy);
    }
    if m.shift {
        return Some(DropEffect::Move);
    }
    match default {
        DragDefault::Auto => Some(if same_volume { DropEffect::Move } else { DropEffect::Copy }),
        DragDefault::Copy => Some(DropEffect::Copy),
        DragDefault::Move => Some(DropEffect::Move),
        DragDefault::Ask => None,
    }
}

/// Effect for `mods`, dropping `payload` into `dest`.
pub fn effect_for(mods: DropMods, payload: &DragPayload, dest: &Path) -> Option<DropEffect> {
    let same = payload.paths.first().is_none_or(|p| same_volume(p, dest));
    resolve_effect(mods, same, payload.default)
}

/// Can `paths` be dropped into `dest`? Not into one of the dragged items or below it.
pub fn valid_target(paths: &[PathBuf], dest: &Path) -> bool {
    !paths.is_empty() && !paths.iter().any(|p| dest == p.as_path() || dest.starts_with(p))
}

/// A move into the folder the items already live in does nothing.
pub fn is_noop(paths: &[PathBuf], dest: &Path, effect: Option<DropEffect>) -> bool {
    effect == Some(DropEffect::Move) && paths.iter().all(|p| p.parent() == Some(dest))
}

/// Volume identity derived from the path alone (drive letter / UNC share) on Windows.
pub fn volume_key(p: &Path) -> Option<String> {
    let stripped = soyuz_fs::strip_verbatim(p);
    match stripped.components().next()? {
        Component::Prefix(pre) => {
            use std::path::Prefix as P;
            Some(match pre.kind() {
                P::Disk(d) | P::VerbatimDisk(d) => (d as char).to_ascii_uppercase().to_string(),
                P::UNC(s, h) | P::VerbatimUNC(s, h) => {
                    format!("\\\\{}\\{}", s.to_string_lossy().to_lowercase(), h.to_string_lossy().to_lowercase())
                }
                _ => return None,
            })
        }
        _ => None,
    }
}

thread_local! {
    static DEVICES: RefCell<HashMap<PathBuf, Option<u64>>> = RefCell::new(HashMap::new());
}

/// Forget the cached devices of [`same_volume`] (a new drag starts).
pub fn reset_volume_cache() {
    DEVICES.with(|c| c.borrow_mut().clear());
}

/// Whether two paths live on the same volume (decides move vs copy for plain drags).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    if let (Some(x), Some(y)) = (volume_key(a), volume_key(b)) {
        return x == y;
    }
    let dev = |p: &Path| {
        DEVICES.with(|c| {
            *c.borrow_mut().entry(p.to_path_buf()).or_insert_with(|| soyuz_fs::device_of(p).ok().map(|d| d.0))
        })
    };
    match (dev(a), dev(b)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

/// Where the pointer was over a pane last frame (external drops have no pointer
/// position on some platforms, so the last known pane wins, else the focused one).
#[derive(Clone, Debug, Default)]
pub struct DndFrame {
    pub panes: Vec<(PaneId, Rect, Option<PathBuf>)>,
    /// A folder item claimed the hover this frame (the pane background yields).
    pub item_claimed: bool,
    pub ask: Option<AskMenu>,
    /// A drag released over another window, waiting for that window's frame.
    pub own_drop: Option<OwnDrop>,
}

impl DndFrame {
    /// End of frame: pane rects stay until the next draw replaces them (external drops
    /// read them at the start of a frame); an open "ask" menu and a waiting drop stay.
    pub fn next_frame(&mut self) {
        self.item_claimed = false;
    }

    /// Where pane `id` was drawn. Every pane has one entry, whichever window it is in.
    pub fn record_pane(&mut self, id: PaneId, rect: Rect, dir: Option<PathBuf>) {
        self.panes.retain(|(p, _, _)| *p != id);
        self.panes.push((id, rect, dir));
    }

    /// The rects of `shown` only: rects are per window, and other windows' panes (or this
    /// window's other tabs) can sit at the same coordinates.
    pub fn only_panes(&self, shown: &[PaneId]) -> DndFrame {
        let panes = self.panes.iter().filter(|(id, _, _)| shown.contains(id)).cloned().collect();
        DndFrame { panes, ..Default::default() }
    }
}

/// A drag released over another Soyuz window (`ops::fileops::dragout`).
#[derive(Clone, Debug)]
pub struct OwnDrop {
    /// The window under the pointer.
    pub window: WindowId,
    /// The pointer in that window, points.
    pub pos: Point,
    pub payload: DragPayload,
    pub mods: DropMods,
    /// Where the "ask" menu opens (the focused window draws it).
    pub ask_pos: Point,
}

/// The folder of the pane under `pos`, if it has one.
pub fn pane_under(frame: &DndFrame, pos: Point) -> Option<PathBuf> {
    frame.panes.iter().find(|(_, r, _)| r.contains(pos)).and_then(|(_, _, dir)| dir.clone())
}

/// "Copy here / Move here / Link here" menu for `drag_default = "ask"`.
#[derive(Clone, Debug)]
pub struct AskMenu {
    pub paths: Vec<PathBuf>,
    pub dest: PathBuf,
    pub pos: Point,
}

/// External drop target: the pane under the pointer, else the focused pane.
pub fn external_target(frame: &DndFrame, pointer: Option<Point>, focused: PaneId) -> Option<PathBuf> {
    if let Some(pos) = pointer
        && let Some((_, _, dir)) = frame.panes.iter().find(|(_, r, _)| r.contains(pos))
    {
        return dir.clone();
    }
    frame.panes.iter().find(|(id, _, _)| *id == focused).and_then(|(_, _, d)| d.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{pt, vec2};

    const NONE: DropMods = DropMods { ctrl: false, shift: false, alt: false };

    #[test]
    fn modifier_semantics() {
        let ctrl = DropMods { ctrl: true, ..NONE };
        let shift = DropMods { shift: true, ..NONE };
        let alt = DropMods { alt: true, ..NONE };
        let ctrl_shift = DropMods { ctrl: true, shift: true, alt: false };
        for same in [true, false] {
            assert_eq!(resolve_effect(ctrl, same, DragDefault::Auto), Some(DropEffect::Copy));
            assert_eq!(resolve_effect(shift, same, DragDefault::Auto), Some(DropEffect::Move));
            assert_eq!(resolve_effect(alt, same, DragDefault::Auto), Some(DropEffect::Link));
            assert_eq!(resolve_effect(ctrl_shift, same, DragDefault::Copy), Some(DropEffect::Link));
        }
        assert_eq!(resolve_effect(NONE, true, DragDefault::Auto), Some(DropEffect::Move), "same device moves");
        assert_eq!(resolve_effect(NONE, false, DragDefault::Auto), Some(DropEffect::Copy), "cross device copies");
        assert_eq!(resolve_effect(NONE, true, DragDefault::Copy), Some(DropEffect::Copy));
        assert_eq!(resolve_effect(NONE, false, DragDefault::Move), Some(DropEffect::Move));
        assert_eq!(resolve_effect(NONE, true, DragDefault::Ask), None);
        assert_eq!(resolve_effect(ctrl, true, DragDefault::Ask), Some(DropEffect::Copy), "modifiers win over ask");
    }

    #[test]
    fn targets() {
        let paths = vec![PathBuf::from("/a/x"), PathBuf::from("/a/y")];
        assert!(valid_target(&paths, Path::new("/b")));
        assert!(!valid_target(&paths, Path::new("/a/x")), "not into itself");
        assert!(!valid_target(&paths, Path::new("/a/x/sub")), "not below itself");
        assert!(valid_target(&paths, Path::new("/a")));
        assert!(!valid_target(&[], Path::new("/a")));
        assert!(is_noop(&paths, Path::new("/a"), Some(DropEffect::Move)));
        assert!(!is_noop(&paths, Path::new("/a"), Some(DropEffect::Copy)), "copy into the same folder duplicates");
        assert!(!is_noop(&paths, Path::new("/b"), Some(DropEffect::Move)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_volume_keys() {
        assert_eq!(volume_key(Path::new(r"c:\Users\x")), Some("C".into()));
        assert_eq!(volume_key(Path::new(r"\\?\C:\Users\x")), Some("C".into()));
        assert_eq!(volume_key(Path::new(r"\\Server\Share\dir")), Some(r"\\server\share".into()));
        assert!(same_volume(Path::new(r"C:\a"), Path::new(r"c:\b\c")));
        assert!(!same_volume(Path::new(r"C:\a"), Path::new(r"D:\a")));
        assert!(!same_volume(Path::new(r"\\s\one\a"), Path::new(r"\\s\two\a")));
    }

    #[test]
    fn external_target_prefers_pane_under_pointer() {
        let a = PaneId(1);
        let b = PaneId(2);
        let f = DndFrame {
            panes: vec![
                (a, Rect::from_min_size(pt(0.0, 0.0), vec2(100.0, 100.0)), Some("/a".into())),
                (b, Rect::from_min_size(pt(100.0, 0.0), vec2(100.0, 100.0)), Some("/b".into())),
            ],
            ..Default::default()
        };
        assert_eq!(external_target(&f, Some(pt(150.0, 50.0)), a), Some("/b".into()));
        assert_eq!(external_target(&f, None, a), Some("/a".into()));
        assert_eq!(external_target(&f, Some(pt(500.0, 50.0)), b), Some("/b".into()));
    }

    #[test]
    fn next_frame_keeps_the_ask_menu() {
        let mut f = DndFrame {
            panes: vec![(PaneId(1), Rect::from_min_size(pt(0.0, 0.0), vec2(1.0, 1.0)), None)],
            item_claimed: true,
            ask: Some(AskMenu { paths: vec![], dest: "/d".into(), pos: pt(1.0, 2.0) }),
            own_drop: Some(OwnDrop {
                window: WindowId(3),
                pos: pt(0.0, 0.0),
                payload: DragPayload { paths: vec![], source_pane: None, default: DragDefault::Auto },
                mods: NONE,
                ask_pos: pt(0.0, 0.0),
            }),
        };
        f.next_frame();
        assert!(!f.item_claimed);
        assert_eq!(f.panes.len(), 1, "rects stay until the next draw records new ones");
        assert!(f.own_drop.is_some(), "a drop waits for its window's frame");
        assert_eq!(f.ask.map(|a| a.pos), Some(pt(1.0, 2.0)));
    }

    #[test]
    fn a_pane_has_one_rect_the_newest() {
        let mut f = DndFrame::default();
        let a = PaneId(1);
        for x in 0..1000 {
            f.record_pane(a, Rect::from_min_size(pt(x as f32, 0.0), vec2(10.0, 10.0)), Some("/a".into()));
        }
        assert_eq!(f.panes.len(), 1, "does not grow per frame");
        assert_eq!(pane_under(&f, pt(5.0, 5.0)), None, "an old rect is forgotten");
        assert_eq!(pane_under(&f, pt(1002.0, 5.0)), Some("/a".into()));
    }

    #[test]
    fn panes_of_other_windows_are_not_targets() {
        let mut f = DndFrame::default();
        let here = PaneId(1);
        let elsewhere = PaneId(2);
        let r = Rect::from_min_size(pt(0.0, 0.0), vec2(100.0, 100.0));
        // The other window recorded its pane first, at the same window coordinates.
        f.record_pane(elsewhere, r, Some("/elsewhere".into()));
        f.record_pane(here, r, Some("/here".into()));
        assert_eq!(pane_under(&f.only_panes(&[here]), pt(5.0, 5.0)), Some("/here".into()));
        assert_eq!(external_target(&f.only_panes(&[here]), None, here), Some("/here".into()));
        assert_eq!(pane_under(&f.only_panes(&[]), pt(5.0, 5.0)), None);
    }
}
