//! Motion helpers: damped springs for live layout (tab reordering, zoom),
//! eased transitions and the spring-loaded-folder hover timer. Everything collapses to
//! the end state with `[appearance] reduced_motion` (see [`crate::theme::anim`]).

use egui::{Context, Id};

/// A 1-D damped spring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub pos: f32,
    pub vel: f32,
}

/// Stiffness / damping used for UI layout (slightly under-damped: a hint of overshoot).
pub const STIFFNESS: f32 = 420.0;
pub const DAMPING: f32 = 34.0;

impl Spring {
    pub fn at(pos: f32) -> Self {
        Spring { pos, vel: 0.0 }
    }

    /// Advance by `dt` seconds towards `target` (semi-implicit Euler, fixed sub-steps so
    /// a long frame never explodes).
    pub fn step(&mut self, target: f32, dt: f32, stiffness: f32, damping: f32) {
        let mut left = dt.clamp(0.0, 0.25);
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let accel = -stiffness * (self.pos - target) - damping * self.vel;
            self.vel += accel * h;
            self.pos += self.vel * h;
            left -= h;
        }
    }

    /// Close enough to rest at `target` to snap.
    pub fn settled(&self, target: f32) -> bool {
        (self.pos - target).abs() < 0.3 && self.vel.abs() < 2.0
    }
}

/// The animated value for `id` moving towards `target` (spring physics). Jumps when
/// reduced motion is on or the value is new.
pub fn spring_to(ctx: &Context, id: Id, target: f32) -> f32 {
    if crate::theme::reduced_motion(ctx) {
        ctx.data_mut(|d| d.remove::<Spring>(id));
        return target;
    }
    let dt = ctx.input(|i| i.stable_dt);
    let mut s = ctx.data(|d| d.get_temp::<Spring>(id)).unwrap_or(Spring::at(target));
    s.step(target, dt, STIFFNESS, DAMPING);
    if s.settled(target) {
        s = Spring::at(target);
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(id, s));
    s.pos
}

/// Place the spring for `id` at `pos` (e.g. where a dragged item was let go).
pub fn spring_set(ctx: &Context, id: Id, pos: f32) {
    ctx.data_mut(|d| d.insert_temp(id, Spring::at(pos)));
}

/// Ease-out cubic.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Hover timer for spring-loaded folders: fires once after the pointer has rested on
/// the same target for `delay` seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverTimer {
    pub target: Option<std::path::PathBuf>,
    pub since: f64,
    pub fired: bool,
}

impl HoverTimer {
    /// Update with the hovered target at time `now`; returns `true` when it fires.
    pub fn update(&mut self, target: &std::path::Path, now: f64, delay: f64) -> bool {
        if self.target.as_deref() != Some(target) {
            *self = HoverTimer { target: Some(target.to_path_buf()), since: now, fired: false };
            return false;
        }
        if !self.fired && now - self.since >= delay {
            self.fired = true;
            return true;
        }
        false
    }

    /// Progress 0..=1 towards firing (for a visual hint).
    pub fn progress(&self, now: f64, delay: f64) -> f32 {
        if self.fired || delay <= 0.0 { 1.0 } else { ((now - self.since) / delay).clamp(0.0, 1.0) as f32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn spring_converges_with_a_little_overshoot() {
        let mut s = Spring::at(0.0);
        let mut peak = 0.0f32;
        for _ in 0..120 {
            s.step(100.0, 1.0 / 60.0, STIFFNESS, DAMPING);
            peak = peak.max(s.pos);
        }
        assert!(s.settled(100.0), "{s:?}");
        assert!(peak > 100.0 && peak < 106.0, "slight overshoot only: {peak}");
    }

    #[test]
    fn spring_is_stable_on_long_frames() {
        let mut s = Spring::at(0.0);
        s.step(50.0, 5.0, STIFFNESS, DAMPING); // clamped to 0.25 s, sub-stepped
        assert!(s.pos.is_finite() && (s.pos - 50.0).abs() < 10.0, "{s:?}");
        let mut c = Spring::at(10.0);
        c.step(10.0, 1.0 / 60.0, STIFFNESS, DAMPING);
        assert_eq!(c, Spring::at(10.0), "at rest stays at rest");
    }

    #[test]
    fn easing() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(7.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
    }

    #[test]
    fn hover_timer_fires_once_per_target() {
        let mut t = HoverTimer::default();
        let a = Path::new("/a");
        let b = Path::new("/b");
        assert!(!t.update(a, 0.0, 0.7));
        assert!(!t.update(a, 0.5, 0.7));
        assert!((t.progress(0.35, 0.7) - 0.5).abs() < 1e-6);
        assert!(t.update(a, 0.71, 0.7));
        assert!(!t.update(a, 2.0, 0.7), "fires once");
        assert!(!t.update(b, 2.1, 0.7), "a new target restarts the timer");
        assert!(t.update(b, 2.9, 0.7));
    }
}
