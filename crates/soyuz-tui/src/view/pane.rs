//! A pane's list (step T2.3 brings the other view modes, columns and Miller).
//!
//! Plain look of Phase T1: the cursor row is reversed, selected rows carry a `*`,
//! directories a trailing `/` and symlinks to files a trailing `@`. Names are fitted
//! with [`super::text::truncate_name`] (the extension stays).
//!
//! # Layout and scrolling
//!
//! [`prepare`] gives every drawn pane a [`FlowLayout`] in cell units: one item per row
//! (`cell = (width, 1)`, no gaps, group headers one row high), rebuilt only when its
//! [`LayoutSig`] changes (like the GUI's `views/flow.rs`), so keyboard navigation
//! (`actions::cursor`) works on what is drawn. Scroll state is the pane's own:
//! - `pane.scroll_y` is the first layout line shown, **in rows** (the GUI keeps pixels;
//!   each frontend process owns its panes, and the TUI saves its own session file);
//! - `pane.restore_scroll` (history, session, workspaces) is applied once and clamped to
//!   the listing; while the listing still loads and cannot reach it yet it stays pending.
//!   A value from the GUI (a workspace saved there, in pixels) just clamps to the end;
//!   after a restore the cursor is brought into view when it is outside, so a stale or
//!   pixel offset never hides it;
//! - the view follows the cursor only when `pane.scroll_to_cursor` is set (then cleared),
//!   so wheel scrolling (T2.9) is not undone on the next frame;
//! - `pane.viewport_h` is the list height in rows and `pane.page_rows` the rows per page.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::{FileClass, classify};
use soyuz_appcore::geom::vec2;
use soyuz_appcore::state::layout::{FlowLayout, LineKind};
use soyuz_appcore::state::pane::{LayoutSig, ListingStatus, Pane, PaneId};
use soyuz_appcore::tr;
use soyuz_core::EntryKind;

use super::text;
use crate::dialog::safe;
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};

/// `LayoutSig::mode` of the terminal list (distinct from the GUI's view styles).
pub const LAYOUT_MODE: u8 = 0x80;

/// What kind of item a row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Dir,
    /// A symlink to a file (links to folders count as [`RowKind::Dir`]).
    Link,
    File,
}

/// One item row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Index in the pane's visible order.
    pub index: usize,
    /// Display name (extension hidden when `[general] show_extensions` is off).
    pub name: String,
    pub kind: RowKind,
    pub class: FileClass,
    /// Formatted size (empty for folders).
    pub size: String,
    pub cursor: bool,
    pub selected: bool,
}

impl Row {
    /// The name with its kind marker (`docs/`, `link@`).
    pub fn label(&self) -> String {
        format!("{}{}", self.name, self.suffix())
    }

    fn suffix(&self) -> &'static str {
        match self.kind {
            RowKind::Dir => "/",
            RowKind::Link => "@",
            RowKind::File => "",
        }
    }

    /// The label fitted into `w` cells.
    pub fn fitted_label(&self, w: usize) -> String {
        let suffix = self.suffix();
        let name = text::truncate_name(&self.name, w.saturating_sub(suffix.len()), self.kind != RowKind::Dir);
        format!("{name}{suffix}")
    }
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneLine {
    /// Group header `group` (grouped listings).
    Header {
        group: usize,
        label: String,
        count: usize,
    },
    Item(Row),
}

/// A drawn pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneVm {
    /// Index in `Tab::panes`.
    pub index: usize,
    pub id: PaneId,
    /// `Pane::vis_version` the rows were built from (click targets carry it).
    pub vis_version: u64,
    /// The tab's focused pane.
    pub focused: bool,
    /// The lines in view (from `top`).
    pub lines: Vec<PaneLine>,
    /// First layout line shown (`pane.scroll_y`).
    pub top: usize,
    /// Items in the listing (after filters).
    pub total: usize,
    /// Shown instead of rows: loading, empty, failed, not available.
    pub message: Option<String>,
}

impl PaneVm {
    /// The item rows in view.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.lines.iter().filter_map(|l| match l {
            PaneLine::Item(r) => Some(r),
            PaneLine::Header { .. } => None,
        })
    }

    /// The model of tab pane `index` (after [`prepare`]).
    pub fn from_core(core: &AppCore, index: usize) -> Option<PaneVm> {
        let tab = core.ws.tab();
        let pane = tab.panes.get(index)?;
        let total = pane.visible.len();
        let cursor = pane.cursor_pos();
        let show_ext = core.config.general.show_extensions;
        let size_style = core.config.general.size_style;
        let top = pane.scroll_y.max(0.0) as usize;
        let rows = pane.viewport_h.max(0.0) as usize;
        let mut lines = Vec::new();
        if let Some(layout) = pane.layout.as_ref().filter(|l| l.n == total) {
            for line in layout.lines().iter().skip(top).take(rows) {
                match line.kind {
                    LineKind::Header(g) => {
                        let Some(span) = pane.groups.get(g) else { continue };
                        lines.push(PaneLine::Header { group: g, label: safe(&span.label), count: span.count });
                    }
                    LineKind::Items { start, len } => {
                        for i in start..start + len {
                            let (Some(e), Some(key)) = (pane.entry_at(i), pane.key_at(i)) else { continue };
                            let kind = if e.is_dir_like() {
                                RowKind::Dir
                            } else if matches!(e.kind, EntryKind::Symlink { .. }) {
                                RowKind::Link
                            } else {
                                RowKind::File
                            };
                            let name = if show_ext || kind == RowKind::Dir { e.name.as_str() } else { e.stem() };
                            lines.push(PaneLine::Item(Row {
                                index: i,
                                name: safe(name),
                                kind,
                                class: classify(e),
                                size: if kind == RowKind::Dir {
                                    String::new()
                                } else {
                                    soyuz_core::size::format_size(e.size, size_style)
                                },
                                cursor: cursor == Some(i),
                                selected: pane.selection.contains(key),
                            }));
                        }
                    }
                }
            }
        }
        let message = match &pane.status {
            ListingStatus::Loading { .. } if total == 0 => Some(tr!("tui-loading").to_string()),
            ListingStatus::Failed(e) => Some(tr!("tui-failed", error = safe(&e.to_string()))),
            ListingStatus::Virtual => Some(tr!("tui-virtual").to_string()),
            _ if total == 0 && pane.is_loading() => Some(tr!("tui-loading").to_string()),
            _ if total == 0 && pane.filter_active() => Some(tr!("tui-no-matches").to_string()),
            _ if total == 0 => Some(tr!("tui-empty").to_string()),
            _ => None,
        };
        Some(PaneVm {
            index,
            id: pane.id,
            vis_version: pane.vis_version,
            focused: index == tab.focused,
            lines,
            top,
            total,
            message,
        })
    }
}

/// Lay out and scroll every drawn pane (see the module docs).
pub fn prepare(core: &mut AppCore, regions: &Regions) {
    let tab = core.ws.tab_mut();
    for p in &regions.panes {
        if let Some(pane) = tab.panes.get_mut(p.index) {
            prepare_pane(pane, p.rect.width, p.rect.height);
        }
    }
    // T2.3: the Miller view's parent / preview columns (`services.aux_entries`) load here.
}

/// The list of `pane` is `width` × `rows` cells: layout, scroll, page size.
pub fn prepare_pane(pane: &mut Pane, width: u16, rows: u16) {
    let w = f32::from(width.max(1));
    let cell = vec2(w, 1.0);
    let sig = LayoutSig { vis_version: pane.vis_version, width: w, cell, mode: LAYOUT_MODE };
    if pane.layout_sig != Some(sig) || pane.layout.is_none() {
        pane.layout =
            Some(FlowLayout::new(pane.visible.len(), &pane.groups, w, cell, vec2(0.0, 0.0), vec2(0.0, 0.0), 1.0));
        pane.layout_sig = Some(sig);
    }
    let Some(layout) = pane.layout.as_ref() else { return };
    let rows = usize::from(rows);
    let max_top = layout.lines().len().saturating_sub(rows);
    let mut top = if pane.scroll_y.is_finite() { pane.scroll_y.max(0.0) as usize } else { 0 };
    let mut restored = false;
    if let Some(y) = pane.restore_scroll {
        let y = if y.is_finite() { y.max(0.0) as usize } else { 0 };
        // Wait for the listing when it cannot reach the offset yet.
        if !(pane.is_loading() && y > max_top) {
            pane.restore_scroll = None;
            top = y;
            restored = true;
        }
    }
    top = top.min(max_top);
    let follow = std::mem::take(&mut pane.scroll_to_cursor);
    if (follow || restored)
        && let Some(line) = pane.cursor_pos().and_then(|c| layout.item_rect(c)).map(|r| r.min.y.max(0.0) as usize)
    {
        if line < top {
            top = line;
        } else if line >= top + rows.max(1) {
            top = line + 1 - rows.max(1);
        }
    }
    pane.scroll_y = top as f32;
    pane.viewport_h = rows as f32;
    pane.page_rows = layout.rows_per_page(pane.viewport_h);
}

/// Width of the size column.
const SIZE_W: u16 = 11;

/// Draw pane `m` into `rect`.
pub fn draw_pane(f: &mut Frame<'_>, rect: Rect, m: &PaneVm, theme: &TuiTheme, hits: &mut Hits) {
    hits.push(rect, HitTarget::PaneBackground { pane: m.id });
    if let Some(msg) = &m.message
        && m.lines.is_empty()
    {
        let msg = text::truncate_end(msg, usize::from(rect.width));
        f.render_widget(Paragraph::new(msg).alignment(Alignment::Center).style(theme.style(Role::Muted)), rect);
    }
    for (i, line) in m.lines.iter().enumerate().take(usize::from(rect.height)) {
        let r = Rect { y: rect.y + i as u16, height: 1, ..rect };
        match line {
            PaneLine::Header { group, label, count } => {
                let t = text::truncate_end(&format!("{label} ({count})"), usize::from(r.width));
                f.render_widget(Paragraph::new(t).style(theme.style(Role::GroupHeader)), r);
                hits.push(r, HitTarget::PaneHeader { pane: m.id, vis_version: m.vis_version, group: *group });
            }
            PaneLine::Item(row) => {
                let size_w = SIZE_W.min(r.width / 3);
                let [name_r, size_r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(size_w)]).areas(r);
                let mark = if row.selected { "* " } else { "  " };
                let label = row.fitted_label(usize::from(name_r.width).saturating_sub(2));
                f.render_widget(Paragraph::new(format!("{mark}{label}")), name_r);
                let size = text::truncate_end(&format!("{} ", row.size), usize::from(size_r.width));
                f.render_widget(Paragraph::new(size).alignment(Alignment::Right), size_r);
                let mut style = theme.style(Role::Text);
                if row.selected {
                    style = style.patch(theme.style(Role::Selected));
                }
                if row.cursor {
                    style = style.patch(theme.style(if m.focused { Role::Cursor } else { Role::CursorInactive }));
                }
                f.buffer_mut().set_style(r, style);
                hits.push(r, HitTarget::PaneRow { pane: m.id, vis_version: m.vis_version, index: row.index });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::state::view_memory::ViewState;
    use soyuz_core::{EntryInfo, Location};

    use super::*;

    fn pane(n: usize) -> Pane {
        let mut p = Pane::new(Location::Dir("/data".into()), ViewState::default());
        let entries: Vec<EntryInfo> =
            (0..n).map(|i| EntryInfo::minimal(format!("/data/f{i:03}.txt").into(), EntryKind::File)).collect();
        p.set_entries_ordered(entries);
        p.status = ListingStatus::Ready;
        p.refresh_visible(true, true);
        p
    }

    #[test]
    fn layout_is_cached_until_the_listing_or_width_changes() {
        let mut p = pane(50);
        prepare_pane(&mut p, 40, 10);
        let sig = p.layout_sig;
        assert_eq!(p.layout.as_ref().map(|l| (l.n, l.cols)), Some((50, 1)));
        assert_eq!((p.viewport_h, p.page_rows), (10.0, 10));
        prepare_pane(&mut p, 40, 12);
        assert_eq!(p.layout_sig, sig, "a height change keeps the layout");
        prepare_pane(&mut p, 41, 12);
        assert_ne!(p.layout_sig, sig);
    }

    #[test]
    fn scrolling_follows_the_cursor_only_when_asked() {
        let mut p = pane(100);
        prepare_pane(&mut p, 40, 10);
        assert_eq!(p.scroll_y, 0.0);
        p.move_cursor(25, false, false);
        assert!(p.scroll_to_cursor);
        prepare_pane(&mut p, 40, 10);
        assert_eq!(p.scroll_y, 16.0, "cursor on the last row");
        assert!(!p.scroll_to_cursor);
        // A wheel scroll (T2.9) moves the view without the cursor and stays.
        p.scroll_y = 60.0;
        prepare_pane(&mut p, 40, 10);
        assert_eq!(p.scroll_y, 60.0);
        // Clamped to the end of the list.
        p.scroll_y = 500.0;
        prepare_pane(&mut p, 40, 10);
        assert_eq!(p.scroll_y, 90.0);
        p.move_cursor(3, false, false);
        prepare_pane(&mut p, 40, 10);
        assert_eq!(p.scroll_y, 3.0, "cursor on the first row");
    }

    #[test]
    fn restore_clamps_and_keeps_the_cursor_visible() {
        let mut p = pane(100);
        p.restore_scroll = Some(40.0);
        prepare_pane(&mut p, 40, 10);
        assert_eq!((p.scroll_y, p.restore_scroll), (40.0, None));
        // A GUI offset in pixels: clamped, then the cursor is brought into view.
        p.move_cursor(5, false, false);
        p.scroll_to_cursor = false;
        p.restore_scroll = Some(2400.0);
        prepare_pane(&mut p, 40, 10);
        assert_eq!(p.scroll_y, 5.0);
        // While loading, an offset the listing cannot reach yet stays pending.
        let mut p = pane(5);
        p.status = ListingStatus::Loading { since: std::time::Instant::now() };
        p.restore_scroll = Some(30.0);
        prepare_pane(&mut p, 40, 10);
        assert_eq!((p.scroll_y, p.restore_scroll), (0.0, Some(30.0)));
    }
}
