//! Per-entry decorations for the file views: git status (colored names, a badge on the
//! icon, the GitStatus column) and color tags (dots after the name, the Tags column).
//! Lookups are cheap thread-local hash lookups, safe to call for every visible row.

use egui::{Align2, Color32, FontId, Painter, Pos2, pos2};
use soyuz_core::{Column, EntryInfo};

use crate::services::git::{self, GitCode};
use crate::services::tags::{self, TagMask};
use crate::theme::Palette;
use crate::ui::widgets::trunc_galley;

/// Color of a git status.
pub fn git_color(code: GitCode, p: &Palette) -> Color32 {
    match code {
        GitCode::Modified => p.warning,
        GitCode::Added | GitCode::Untracked => p.success,
        GitCode::Deleted | GitCode::Conflicted => p.danger,
        GitCode::Renamed | GitCode::TypeChange => p.info,
        GitCode::Ignored => p.text_tertiary,
    }
}

/// Display name of a git status (GitStatus column, tooltips).
pub fn git_label(code: GitCode) -> &'static str {
    match code {
        GitCode::Ignored => tr!("tools-git-ignored"),
        GitCode::Untracked => tr!("tools-git-untracked"),
        GitCode::TypeChange => tr!("tools-git-type-changed"),
        GitCode::Renamed => tr!("tools-git-renamed"),
        GitCode::Added => tr!("tools-git-added"),
        GitCode::Deleted => tr!("tools-git-deleted"),
        GitCode::Modified => tr!("tools-git-modified"),
        GitCode::Conflicted => tr!("tools-git-conflict"),
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Decorations of one entry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Deco {
    pub git: Option<GitCode>,
    pub tags: TagMask,
}

const DOT_R: f32 = 4.0;
const DOT_STEP: f32 = 6.0;
const MAX_DOTS: usize = 4;

impl Deco {
    pub fn of(e: &EntryInfo) -> Deco {
        Deco { git: git::status_of(&e.path, e.is_dir_like()), tags: tags::tags_of(&e.path) }
    }

    pub fn is_empty(&self) -> bool {
        self.git.is_none() && self.tags == 0
    }

    /// Name color: tinted towards the git status color (ignored items are dimmed).
    pub fn name_color(&self, p: &Palette, base: Color32) -> Color32 {
        match self.git {
            Some(GitCode::Ignored) => p.text_tertiary,
            Some(code) => mix(base, git_color(code, p), 0.72),
            None => base,
        }
    }

    /// Width reserved after the name for the tag dots.
    pub fn trailing_width(&self) -> f32 {
        let n = (self.tags.count_ones() as usize).min(MAX_DOTS);
        if n == 0 { 0.0 } else { 8.0 + DOT_R * 2.0 + (n - 1) as f32 * DOT_STEP }
    }

    /// Tag dots starting at `x` (left edge), vertically centered on `cy`.
    pub fn paint_tags(&self, painter: &Painter, p: &Palette, x: f32, cy: f32, ring: Color32) {
        let mut cx = x + 8.0 + DOT_R;
        for t in tags::mask_tags(self.tags).take(MAX_DOTS) {
            painter.circle_filled(pos2(cx, cy), DOT_R + 1.2, ring);
            painter.circle_filled(pos2(cx, cy), DOT_R, p.tags[t as usize]);
            cx += DOT_STEP;
        }
    }

    /// Git badge: a small dot at the bottom-right of an icon centered at `icon`.
    pub fn paint_badge(&self, painter: &Painter, p: &Palette, icon: Pos2, icon_size: f32, ring: Color32) {
        let Some(code) = self.git else { return };
        if code == GitCode::Ignored {
            return;
        }
        let r = (icon_size * 0.17).clamp(2.6, 6.0);
        let c = icon + egui::vec2(icon_size * 0.42, icon_size * 0.36);
        painter.circle_filled(c, r + 1.5, ring);
        painter.circle_filled(c, r, git_color(code, p));
    }
}

/// Paint a Tags / GitStatus cell (`x0..x0+w`, centered on `cy`). Returns `false` for
/// other columns (the caller draws text).
#[allow(clippy::too_many_arguments)]
pub fn paint_cell(
    painter: &Painter,
    p: &Palette,
    col: Column,
    e: &EntryInfo,
    x0: f32,
    w: f32,
    cy: f32,
    alpha: f32,
) -> bool {
    match col {
        Column::GitStatus => {
            if let Some(code) = git::status_of(&e.path, e.is_dir_like()) {
                let color = git_color(code, p).gamma_multiply(alpha);
                let staged = (!e.is_dir_like())
                    .then(|| git::file_status(&e.path))
                    .flatten()
                    .is_some_and(|s| s.staged() && s.y == b'.');
                painter.text(
                    pos2(x0 + 14.0, cy),
                    Align2::CENTER_CENTER,
                    code.letter(),
                    crate::ui::views::flow::semibold(12.0),
                    color,
                );
                let label = if staged {
                    tr!("tools-git-staged", status = git_label(code))
                } else {
                    git_label(code).to_string()
                };
                let g = trunc_galley(painter, &label, FontId::proportional(12.0), color, w - 34.0);
                painter.galley(pos2(x0 + 26.0, cy - g.size().y / 2.0), g, color);
            }
            true
        }
        Column::Tags => {
            let mask = tags::tags_of(&e.path);
            if mask != 0 {
                let n = mask.count_ones();
                let mut x = x0 + 12.0;
                for t in tags::mask_tags(mask).take(MAX_DOTS) {
                    painter.circle_filled(pos2(x, cy), DOT_R, p.tags[t as usize].gamma_multiply(alpha));
                    x += DOT_STEP + 2.0;
                }
                let label = if n == 1 {
                    super::tags_ui::tag_name(tags::mask_tags(mask).next().unwrap_or(0)).to_string()
                } else {
                    tr!("tools-tags-count", count = n)
                };
                let color = p.text_secondary.gamma_multiply(alpha);
                let g = trunc_galley(painter, &label, FontId::proportional(12.0), color, x0 + w - x - 8.0);
                painter.galley(pos2(x + 2.0, cy - g.size().y / 2.0), g, color);
            }
            true
        }
        _ => false,
    }
}

/// Plain-text value of the Tags / GitStatus columns (sorting, copying, tests).
pub fn cell_text(col: Column, e: &EntryInfo) -> Option<String> {
    match col {
        Column::GitStatus => {
            Some(git::status_of(&e.path, e.is_dir_like()).map(|c| git_label(c).to_string()).unwrap_or_default())
        }
        Column::Tags => {
            Some(tags::mask_tags(tags::tags_of(&e.path)).map(super::tags_ui::tag_name).collect::<Vec<_>>().join(", "))
        }
        _ => None,
    }
}
