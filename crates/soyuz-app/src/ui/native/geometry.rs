//! Pure placement math for native popups and dialogs. Every rect here is in one
//! coordinate space (the callers use physical screen pixels): an anchor, the popup size,
//! and the work area of the monitor the anchor is on (the screen minus the taskbar).

use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Where a popup opens relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Below the anchor rect, left edges aligned (dropdowns, menu buttons); flips above
    /// when there is not enough room below (and more room above).
    Below,
    /// Below the anchor rect, right edges aligned.
    BelowEnd,
    /// Right of the anchor rect, top edges aligned (submenus); flips to the left.
    Right,
    /// At a point (context menus): down-right of it; flips up and / or left.
    Point,
}

/// The size a popup may take on a work area (larger popups scroll).
pub fn fit_size(size: Vec2, work: Rect) -> Vec2 {
    vec2(size.x.min(work.width()).max(1.0), size.y.min(work.height()).max(1.0))
}

/// Shift `r` into `work` (top-left wins when it is larger than `work`).
pub fn clamp_into(r: Rect, work: Rect) -> Rect {
    let x = if r.width() >= work.width() { work.left() } else { r.left().clamp(work.left(), work.right() - r.width()) };
    let y =
        if r.height() >= work.height() { work.top() } else { r.top().clamp(work.top(), work.bottom() - r.height()) };
    Rect::from_min_size(pos2(x, y), r.size())
}

/// One axis of a flip placement: start at `primary` (popup at `[primary, primary + len]`);
/// when that overflows `[lo, hi]`, try `flipped` (popup at `[flipped, flipped + len]`)
/// and keep whichever side has more room, then clamp into `[lo, hi]`.
fn flip_axis(primary: f32, flipped: f32, len: f32, lo: f32, hi: f32) -> f32 {
    let v = if primary + len <= hi {
        primary
    } else if flipped >= lo {
        flipped
    } else {
        // Neither side fits: take the side with more room.
        let room_primary = hi - primary;
        let room_flipped = flipped + len - lo;
        if room_flipped > room_primary { flipped } else { primary }
    };
    if len >= hi - lo { lo } else { v.clamp(lo, hi - len) }
}

/// Place a popup of `size` next to `anchor` on `work` (`gap` between them).
pub fn place(anchor: Rect, size: Vec2, work: Rect, side: Side, gap: f32) -> Rect {
    let size = fit_size(size, work);
    let (w, h) = (size.x, size.y);
    let (x, y) = match side {
        Side::Below | Side::BelowEnd => {
            let y = flip_axis(anchor.bottom() + gap, anchor.top() - gap - h, h, work.top(), work.bottom());
            let x0 = if side == Side::Below { anchor.left() } else { anchor.right() - w };
            let x = if w >= work.width() { work.left() } else { x0.clamp(work.left(), work.right() - w) };
            (x, y)
        }
        Side::Right => {
            let x = flip_axis(anchor.right() + gap, anchor.left() - gap - w, w, work.left(), work.right());
            let y = if h >= work.height() { work.top() } else { anchor.top().clamp(work.top(), work.bottom() - h) };
            (x, y)
        }
        Side::Point => {
            let p = anchor.min;
            let x = flip_axis(p.x, p.x - w, w, work.left(), work.right());
            let y = flip_axis(p.y, p.y - h, h, work.top(), work.bottom());
            (x, y)
        }
    };
    Rect::from_min_size(pos2(x, y), size)
}

/// A dialog of `size` centered over its `owner` window, kept on `work`.
pub fn center_over(owner: Rect, size: Vec2, work: Rect) -> Rect {
    clamp_into(Rect::from_center_size(owner.center(), size), work)
}

/// A remembered dialog rect put back on screen: at least `grip` (the title bar height)
/// of it stays reachable on `work` vertically and `grip * 3` horizontally, so it can
/// always be dragged back.
pub fn keep_reachable(r: Rect, work: Rect, grip: f32) -> Rect {
    let min_visible_x = (grip * 3.0).min(r.width());
    let x = r.left().clamp(work.left() - r.width() + min_visible_x, work.right() - min_visible_x);
    let y = r.top().clamp(work.top(), (work.bottom() - grip).max(work.top()));
    Rect::from_min_size(pos2(x, y), r.size())
}

/// The work area containing `p`, else the nearest one (`None`: no areas).
pub fn pick_area(p: Pos2, areas: &[Rect]) -> Option<Rect> {
    areas
        .iter()
        .copied()
        .find(|a| a.contains(p))
        .or_else(|| areas.iter().copied().min_by(|a, b| a.distance_sq_to_pos(p).total_cmp(&b.distance_sq_to_pos(p))))
}

/// Whether `inner` lies inside `outer` (with half a pixel of slack for rounding).
pub fn fits(inner: Rect, outer: Rect) -> bool {
    outer.expand(0.5).contains_rect(inner)
}

/// Points → physical pixels (rounded) for a window at `ppp` pixels per point.
pub fn to_px(r: Rect, ppp: f32) -> [i32; 4] {
    let x = (r.left() * ppp).round() as i32;
    let y = (r.top() * ppp).round() as i32;
    [x, y, (r.width() * ppp).round().max(1.0) as i32, (r.height() * ppp).round().max(1.0) as i32]
}

/// `[left, top, right, bottom]` pixels → rect.
pub fn ltrb(r: [i32; 4]) -> Rect {
    Rect::from_min_max(pos2(r[0] as f32, r[1] as f32), pos2(r[2] as f32, r[3] as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    const WORK: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 1920.0, y: 1040.0 } };

    #[test]
    fn below_fits_as_is() {
        let p = place(r(100.0, 100.0, 80.0, 24.0), vec2(200.0, 300.0), WORK, Side::Below, 2.0);
        assert_eq!(p, r(100.0, 126.0, 200.0, 300.0));
    }

    #[test]
    fn below_flips_above_near_the_bottom() {
        let p = place(r(100.0, 900.0, 80.0, 24.0), vec2(200.0, 300.0), WORK, Side::Below, 2.0);
        assert_eq!(p.bottom(), 898.0);
        assert_eq!(p.left(), 100.0);
    }

    #[test]
    fn below_end_aligns_right_edges_and_shifts_left_at_the_screen_edge() {
        let p = place(r(1500.0, 100.0, 80.0, 24.0), vec2(200.0, 100.0), WORK, Side::BelowEnd, 0.0);
        assert_eq!(p.right(), 1580.0);
        let p = place(r(1850.0, 100.0, 60.0, 24.0), vec2(300.0, 100.0), WORK, Side::Below, 0.0);
        assert_eq!(p.right(), 1920.0);
    }

    #[test]
    fn below_takes_the_roomier_side_and_clamps_when_neither_fits() {
        // 700 tall popup, anchor in the middle: neither 516 below nor 500 above fit.
        let p = place(r(100.0, 500.0, 80.0, 24.0), vec2(200.0, 700.0), WORK, Side::Below, 0.0);
        assert!(fits(p, WORK));
        assert_eq!(p.height(), 700.0);
    }

    #[test]
    fn taller_than_the_work_area_is_capped() {
        let p = place(r(100.0, 500.0, 80.0, 24.0), vec2(200.0, 5000.0), WORK, Side::Point, 0.0);
        assert_eq!(p, r(100.0, 0.0, 200.0, 1040.0));
    }

    #[test]
    fn context_menu_flips_left_and_up_at_the_bottom_right_corner() {
        let p = place(Rect::from_pos(pos2(1900.0, 1000.0)), vec2(260.0, 400.0), WORK, Side::Point, 0.0);
        assert_eq!(p, r(1640.0, 600.0, 260.0, 400.0));
        let p = place(Rect::from_pos(pos2(50.0, 60.0)), vec2(260.0, 400.0), WORK, Side::Point, 0.0);
        assert_eq!(p, r(50.0, 60.0, 260.0, 400.0));
    }

    #[test]
    fn submenu_opens_right_then_flips_left() {
        let row = r(1000.0, 300.0, 260.0, 28.0);
        let p = place(row, vec2(220.0, 200.0), WORK, Side::Right, 2.0);
        assert_eq!(p, r(1262.0, 300.0, 220.0, 200.0));
        let row = r(1700.0, 300.0, 200.0, 28.0);
        let p = place(row, vec2(220.0, 200.0), WORK, Side::Right, 2.0);
        assert_eq!(p.right(), 1698.0);
        // Near the bottom it shifts up to stay on screen.
        let row = r(1000.0, 1000.0, 260.0, 28.0);
        let p = place(row, vec2(220.0, 200.0), WORK, Side::Right, 2.0);
        assert_eq!(p.bottom(), 1040.0);
    }

    #[test]
    fn placement_stays_on_the_anchor_monitor() {
        // Two monitors side by side; the second one is offset and has a taskbar.
        let left = r(0.0, 0.0, 1920.0, 1040.0);
        let right = r(1920.0, -200.0, 2560.0, 1380.0);
        let areas = [left, right];
        // A context menu at the right edge of the left monitor flips left rather than
        // spanning the monitor boundary.
        let at = pos2(1910.0, 500.0);
        let work = pick_area(at, &areas).unwrap();
        assert_eq!(work, left);
        let p = place(Rect::from_pos(at), vec2(260.0, 300.0), work, Side::Point, 0.0);
        assert!(fits(p, left));
        // On the second monitor above the first one's top edge.
        let at = pos2(2000.0, -150.0);
        let work = pick_area(at, &areas).unwrap();
        assert_eq!(work, right);
        let p = place(Rect::from_pos(at), vec2(260.0, 300.0), work, Side::Point, 0.0);
        assert_eq!(p.min, at);
        // A point between monitors picks the nearest.
        assert_eq!(pick_area(pos2(5000.0, 0.0), &areas), Some(right));
        assert_eq!(pick_area(pos2(0.0, 0.0), &[]), None);
    }

    #[test]
    fn dialogs_center_over_the_owner_and_stay_on_screen() {
        let owner = r(100.0, 100.0, 1000.0, 700.0);
        assert_eq!(center_over(owner, vec2(400.0, 300.0), WORK), r(400.0, 300.0, 400.0, 300.0));
        // Owner mostly off the left edge: the dialog is pulled back on screen.
        let owner = r(-900.0, 100.0, 1000.0, 700.0);
        assert_eq!(center_over(owner, vec2(400.0, 300.0), WORK).left(), 0.0);
    }

    #[test]
    fn remembered_rects_stay_reachable() {
        let grip = 36.0;
        // Far off to the right / bottom: pulled back so the title bar can be grabbed.
        let k = keep_reachable(r(5000.0, 3000.0, 400.0, 300.0), WORK, grip);
        assert_eq!(k.left(), 1920.0 - grip * 3.0);
        assert_eq!(k.top(), 1040.0 - grip);
        // Partly outside but reachable: unchanged.
        let partly = r(1700.0, 100.0, 400.0, 300.0);
        assert_eq!(keep_reachable(partly, WORK, grip), partly);
        // Above the top: title bar back on screen.
        assert_eq!(keep_reachable(r(100.0, -50.0, 400.0, 300.0), WORK, grip).top(), 0.0);
    }

    #[test]
    fn clamp_and_fit() {
        assert_eq!(clamp_into(r(1800.0, 1000.0, 200.0, 100.0), WORK), r(1720.0, 940.0, 200.0, 100.0));
        assert_eq!(clamp_into(r(-10.0, -10.0, 3000.0, 100.0), WORK), r(0.0, 0.0, 3000.0, 100.0));
        assert_eq!(fit_size(vec2(3000.0, 50.0), WORK), vec2(1920.0, 50.0));
        assert!(fits(r(0.0, 0.0, 1920.0, 1040.0), WORK));
        assert!(!fits(r(0.0, 0.0, 1921.0, 1040.0), WORK));
    }

    #[test]
    fn pixel_conversion() {
        assert_eq!(to_px(r(10.0, 20.0, 100.0, 50.0), 1.5), [15, 30, 150, 75]);
        assert_eq!(ltrb([1, 2, 11, 22]), r(1.0, 2.0, 10.0, 20.0));
    }
}
