//! In-process windows, pure parts: window identities and where a dragged tab lands
//! when it is released — reordered in its own strip, moved into another window's strip
//! (or anywhere over another window), or torn off into a new window.

use crate::geom::{Point as Pos2, Rect, vec2};

/// A Soyuz window of this process. `WindowId::MAIN` is the root viewport.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct WindowId(pub u64);

impl WindowId {
    pub const MAIN: WindowId = WindowId(0);

    pub fn next() -> WindowId {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        WindowId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn is_main(self) -> bool {
        self == WindowId::MAIN
    }
}

/// Where a window and its tab strip were last frame, in global (screen) points.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowGeom {
    pub id: WindowId,
    pub inner: Rect,
    pub strip: Rect,
    /// Tab rects in strip order (collapsed group members excluded).
    pub tabs: Vec<(usize, Rect)>,
}

/// What releasing a dragged tab does.
#[derive(Clone, Debug, PartialEq)]
pub enum DropTarget {
    /// Stay in the source strip (normal reorder).
    Reorder,
    /// Move into another window at tab index `index`.
    Window { id: WindowId, index: usize },
    /// Open a new window whose top-left corner is near `pos`.
    TearOff { pos: Pos2 },
    /// Released somewhere that doesn't take tabs: nothing happens.
    Nothing,
}

/// Index a tab dropped at `x` is inserted at among `tabs` (by their centers).
pub fn insert_index(tabs: &[(usize, Rect)], x: f32) -> usize {
    match tabs.iter().find(|(_, r)| x < r.center().x) {
        Some((i, _)) => *i,
        None => tabs.last().map_or(0, |(i, _)| i + 1),
    }
}

/// Resolve a tab drop at global `pos` from window `src`.
/// `tear_off`: tearing off is enabled and the source can give the tab away (it has more
/// than one tab). `distance`: how far from the source strip counts as "out".
pub fn resolve_drop(src: WindowId, pos: Pos2, wins: &[WindowGeom], distance: f32, tear_off: bool) -> DropTarget {
    // Another window's strip (a little forgiving vertically).
    for w in wins.iter().filter(|w| w.id != src) {
        if w.strip.expand2(vec2(0.0, 10.0)).contains(pos) {
            return DropTarget::Window { id: w.id, index: insert_index(&w.tabs, pos.x) };
        }
    }
    let own = wins.iter().find(|w| w.id == src);
    if let Some(w) = own {
        let near = w.strip.expand2(vec2(0.0, distance.max(0.0)));
        if near.contains(pos) || (w.inner.contains(pos) && pos.y <= w.strip.bottom() + distance) {
            return DropTarget::Reorder;
        }
    }
    // Anywhere over another window appends there.
    for w in wins.iter().filter(|w| w.id != src) {
        if w.inner.contains(pos) {
            return DropTarget::Window { id: w.id, index: w.tabs.last().map_or(0, |(i, _)| i + 1) };
        }
    }
    if tear_off {
        DropTarget::TearOff { pos }
    } else if own.is_some() {
        DropTarget::Nothing
    } else {
        DropTarget::Reorder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::pt as pos2;

    fn geom(id: u64, x: f32) -> WindowGeom {
        let inner = Rect::from_min_size(pos2(x, 0.0), vec2(800.0, 600.0));
        let strip = Rect::from_min_size(pos2(x + 200.0, 6.0), vec2(500.0, 32.0));
        let tabs = (0..3)
            .map(|i| (i, Rect::from_min_size(pos2(x + 200.0 + i as f32 * 110.0, 6.0), vec2(108.0, 30.0))))
            .collect();
        WindowGeom { id: WindowId(id), inner, strip, tabs }
    }

    #[test]
    fn drop_targets() {
        let wins = [geom(0, 0.0), geom(1, 1000.0)];
        let main = WindowId::MAIN;
        // Inside the own strip, or just below it: reorder.
        assert_eq!(resolve_drop(main, pos2(300.0, 20.0), &wins, 48.0, true), DropTarget::Reorder);
        assert_eq!(resolve_drop(main, pos2(300.0, 70.0), &wins, 48.0, true), DropTarget::Reorder);
        // Far into the own window's content: tear off.
        assert_eq!(
            resolve_drop(main, pos2(300.0, 300.0), &wins, 48.0, true),
            DropTarget::TearOff { pos: pos2(300.0, 300.0) }
        );
        // Outside every window: tear off, or nothing when tearing off is not possible.
        assert!(matches!(resolve_drop(main, pos2(900.0, 700.0), &wins, 48.0, true), DropTarget::TearOff { .. }));
        assert_eq!(resolve_drop(main, pos2(900.0, 700.0), &wins, 48.0, false), DropTarget::Nothing);
        // Over the other window's strip: between its tabs.
        assert_eq!(
            resolve_drop(main, pos2(1000.0 + 200.0 + 120.0, 20.0), &wins, 48.0, true),
            DropTarget::Window { id: WindowId(1), index: 1 }
        );
        assert_eq!(
            resolve_drop(main, pos2(1000.0 + 690.0, 20.0), &wins, 48.0, true),
            DropTarget::Window { id: WindowId(1), index: 3 }
        );
        // Over the other window's content: appended there (even with tear-off off).
        assert_eq!(
            resolve_drop(main, pos2(1400.0, 400.0), &wins, 48.0, false),
            DropTarget::Window { id: WindowId(1), index: 3 }
        );
        // From the secondary window back onto the main strip.
        assert_eq!(
            resolve_drop(WindowId(1), pos2(205.0, 20.0), &wins, 48.0, true),
            DropTarget::Window { id: main, index: 0 }
        );
    }

    #[test]
    fn insert_index_by_centers() {
        let g = geom(0, 0.0);
        assert_eq!(insert_index(&g.tabs, 0.0), 0);
        assert_eq!(insert_index(&g.tabs, 260.0), 1);
        assert_eq!(insert_index(&g.tabs, 10_000.0), 3);
        assert_eq!(insert_index(&[], 5.0), 0);
        assert!(WindowId::MAIN.is_main() && !WindowId::next().is_main());
    }
}
