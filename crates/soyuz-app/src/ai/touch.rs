//! Live highlights: files the assistant or an MCP client is reading,
//! planning to change or changing get a subtle animated outline and a badge in the file
//! views and the treemap. The harness marks paths through [`mark`], the MCP bridge
//! through [`mark_mcp`]; views call [`paint`] per visible row (a read lock and an empty
//! check when nothing is marked). MCP marks carry a plug badge instead of the sparkle.

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
            TouchKind::Reading => tr!("ai-touch-reading"),
            TouchKind::Planned => tr!("ai-touch-planned"),
            TouchKind::Changing => tr!("ai-touch-changing"),
            TouchKind::Done => tr!("ai-touch-done"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AiTouch {
    pub kind: TouchKind,
    pub at: Instant,
    /// Marked for an external MCP client (else the in-app assistant).
    pub mcp: bool,
}

static TOUCHES: RwLock<Option<HashMap<PathBuf, AiTouch>>> = RwLock::new(None);

fn key(p: &Path) -> PathBuf {
    let p = soyuz_fs::strip_verbatim(p);
    if cfg!(windows) { PathBuf::from(p.to_string_lossy().to_lowercase().replace('/', "\\")) } else { p }
}

/// Mark paths for the assistant (a stronger kind is not downgraded to `Reading`).
pub fn mark<'a>(paths: impl IntoIterator<Item = &'a PathBuf>, kind: TouchKind) {
    mark_as(paths, kind, false);
}

/// Mark paths touched by an external MCP client.
pub fn mark_mcp<'a>(paths: impl IntoIterator<Item = &'a PathBuf>, kind: TouchKind) {
    mark_as(paths, kind, true);
}

fn mark_as<'a>(paths: impl IntoIterator<Item = &'a PathBuf>, kind: TouchKind, mcp: bool) {
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
        map.insert(k, AiTouch { kind, at: now, mcp });
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

/// The strongest mark at or below `dir` (treemap folder tiles whose contents are not
/// drawn). Exact marks are cheap; the subtree scan is skipped for huge mark sets.
pub fn get_within(dir: &Path) -> Option<AiTouch> {
    let g = TOUCHES.read();
    let map = g.as_ref()?;
    if map.is_empty() {
        return None;
    }
    let k = key(dir);
    if let Some(t) = map.get(&k) {
        return Some(*t);
    }
    if map.len() > 4000 {
        return None;
    }
    let rank = |t: &AiTouch| match t.kind {
        TouchKind::Changing => 3,
        TouchKind::Planned => 2,
        TouchKind::Done => 1,
        TouchKind::Reading => 0,
    };
    map.iter().filter(|(p, _)| p.starts_with(&k)).map(|(_, t)| *t).max_by_key(rank)
}

/// Whether anything is marked (views skip per-item lookups otherwise).
pub fn any() -> bool {
    TOUCHES.read().as_ref().is_some_and(|m| !m.is_empty())
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
    paint_touch(painter, p, rect, t, reduced_motion);
}

/// Outline (+ badge when the rect is big enough) for a known mark.
pub fn paint_touch(painter: &Painter, p: &Palette, rect: Rect, t: AiTouch, reduced_motion: bool) {
    let age = t.at.elapsed().as_secs_f32();
    let fade = match t.kind {
        TouchKind::Reading | TouchKind::Done => (1.0 - age / t.kind.ttl().as_secs_f32()).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let pulse = if reduced_motion { 0.8 } else { 0.55 + 0.45 * (age * 3.2).sin().abs() };
    let c = color(t.kind, p);
    let r = rect.shrink(1.0);
    let small = r.width() < 26.0 || r.height() < 14.0;
    let round = CornerRadius::same(if small { 2 } else { 6 });
    painter.rect_filled(r, round, c.gamma_multiply(if small { 0.18 } else { 0.07 } * fade));
    painter.rect_stroke(r, round, Stroke::new(1.5, c.gamma_multiply(pulse * fade)), StrokeKind::Inside);
    if small {
        return;
    }
    // Badge at the right edge: a sparkle (assistant) or a plug (MCP client).
    let center = pos2(r.right() - 11.0, r.top() + 10.0_f32.min(r.height() / 2.0));
    painter.circle_filled(center, 7.5, c.gamma_multiply(0.9 * fade));
    let glyph = if t.mcp { ph::PLUGS_CONNECTED } else { ph::SPARKLE };
    let font = if t.mcp { icons::font(9.5) } else { icons::font_fill(9.5) };
    painter.text(center, Align2::CENTER_CENTER, glyph, font, Color32::WHITE.gamma_multiply(fade));
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
        let dir = p.parent().unwrap().to_path_buf();
        assert_eq!(get_within(&dir).unwrap().kind, TouchKind::Planned, "folders show what happens inside");
        unmark([&p]);
        assert!(get(&p).is_none());
        mark_mcp([&p], TouchKind::Changing);
        assert!(get(&p).unwrap().mcp);
        unmark([&p]);
    }
}
