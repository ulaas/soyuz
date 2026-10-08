//! Live highlights (roadmap A6): files the assistant is reading, planning to change or
//! changing get a subtle animated outline and a badge in the file views. The harness
//! marks paths through [`mark`]; views call [`paint`] per visible row (a read lock and an
//! empty check when the assistant is idle).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, CornerRadius, Painter, Rect, Stroke, StrokeKind, pos2};
use parking_lot::RwLock;

use crate::theme::Palette;
use crate::theme::icons::{self, ph};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchKind {
    /// A read tool looked at it.
    Reading,
    /// Part of a plan waiting for approval.
    Planned,
    /// A job is changing it.
    Changing,
    /// Changed by the assistant (fades out).
    Done,
}

impl TouchKind {
    fn ttl(self) -> Duration {
        match self {
            TouchKind::Reading => Duration::from_secs(5),
            TouchKind::Planned => Duration::from_secs(900),
            TouchKind::Changing => Duration::from_secs(900),
            TouchKind::Done => Duration::from_secs(5),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TouchKind::Reading => "The assistant looked at this",
            TouchKind::Planned => "In a plan waiting for your approval",
            TouchKind::Changing => "Being changed by an assistant job",
            TouchKind::Done => "Changed by the assistant",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AiTouch {
    pub kind: TouchKind,
    pub at: Instant,
}

static TOUCHES: RwLock<Option<HashMap<PathBuf, AiTouch>>> = RwLock::new(None);

fn key(p: &Path) -> PathBuf {
    let p = soyuz_fs::strip_verbatim(p);
    if cfg!(windows) { PathBuf::from(p.to_string_lossy().to_lowercase().replace('/', "\\")) } else { p }
}

/// Mark paths (a stronger kind is not downgraded to `Reading`).
pub fn mark<'a>(paths: impl IntoIterator<Item = &'a PathBuf>, kind: TouchKind) {
    let now = Instant::now();
    let mut g = TOUCHES.write();
    let map = g.get_or_insert_with(HashMap::new);
    for p in paths {
        let k = key(p);
        if kind == TouchKind::Reading
            && map.get(&k).is_some_and(|t| matches!(t.kind, TouchKind::Planned | TouchKind::Changing))
        {
            continue;
        }
        map.insert(k, AiTouch { kind, at: now });
        if map.len() > 20_000 {
            break;
        }
    }
}

/// Forget paths (a plan was rejected).
pub fn unmark<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) {
    let mut g = TOUCHES.write();
    if let Some(map) = g.as_mut() {
        for p in paths {
            map.remove(&key(p));
        }
    }
}

pub fn clear() {
    *TOUCHES.write() = None;
}

/// Drop expired marks; `true` while anything is shown (the caller keeps repainting).
pub fn prune() -> bool {
    let now = Instant::now();
    let mut g = TOUCHES.write();
    let Some(map) = g.as_mut() else { return false };
    map.retain(|_, t| now.duration_since(t.at) < t.kind.ttl());
    !map.is_empty()
}

pub fn get(path: &Path) -> Option<AiTouch> {
    let g = TOUCHES.read();
    let map = g.as_ref()?;
    if map.is_empty() {
        return None;
    }
    map.get(&key(path)).copied()
}

fn color(kind: TouchKind, p: &Palette) -> Color32 {
    match kind {
        TouchKind::Reading => p.info,
        TouchKind::Planned => p.warning,
        TouchKind::Changing => p.accent,
        TouchKind::Done => p.success,
    }
}

/// Outline + badge on a row / cell of `path` (no-op when untouched).
pub fn paint(painter: &Painter, p: &Palette, rect: Rect, path: &Path, reduced_motion: bool) {
    let Some(t) = get(path) else { return };
    let age = t.at.elapsed().as_secs_f32();
    let fade = match t.kind {
        TouchKind::Reading | TouchKind::Done => (1.0 - age / t.kind.ttl().as_secs_f32()).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let pulse = if reduced_motion { 0.8 } else { 0.55 + 0.45 * (age * 3.2).sin().abs() };
    let c = color(t.kind, p);
    let r = rect.shrink(1.0);
    painter.rect_filled(r, CornerRadius::same(6), c.gamma_multiply(0.07 * fade));
    painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.5, c.gamma_multiply(pulse * fade)), StrokeKind::Inside);
    // Badge: a small sparkle in a pill at the right edge.
    let center = pos2(r.right() - 11.0, r.top() + 10.0_f32.min(r.height() / 2.0));
    painter.circle_filled(center, 7.5, c.gamma_multiply(0.9 * fade));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        ph::SPARKLE,
        icons::font_fill(9.5),
        Color32::WHITE.gamma_multiply(fade),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_expiry() {
        let p = PathBuf::from(if cfg!(windows) { "C:\\T\\A.txt" } else { "/t/a.txt" });
        mark([&p], TouchKind::Planned);
        mark([&p], TouchKind::Reading);
        assert_eq!(get(&p).unwrap().kind, TouchKind::Planned, "reading does not downgrade a plan");
        if cfg!(windows) {
            assert!(get(Path::new("c:\\t\\a.TXT")).is_some(), "case-insensitive on Windows");
        }
        assert!(prune());
        unmark([&p]);
        assert!(get(&p).is_none());
    }
}
