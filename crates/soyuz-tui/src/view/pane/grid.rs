//! Grid and gallery views (step T4.5): icon / thumbnail cells, like the GUI's
//! `views/flow.rs` grid and gallery styles.
//!
//! # Cell size
//! The GUI sizes its cells from the pane's icon size (`pane.view.zoom`, points: the
//! `[appearance] thumbnail_size` default, Ctrl+wheel / zoom actions, 192 in the gallery).
//! The terminal UI maps that size to cells with `[tui] cell_pixels` (one point = one
//! pixel): a 96-point icon on 10 × 20 pixel cells is a 10 × 5 cell image. A cell is the
//! image area plus one name line below it ([`GridSpec`]); images smaller than
//! [`COMPACT_BELOW`] rows give compact one-line cells (mark, icon glyph, name) of
//! [`COMPACT_CELL_W`] columns instead ("small icons"). Cells are stretched to fill the
//! pane's width evenly (at most 1.45 times their base width, as in the GUI) with one blank
//! column between them. The gallery is the same grid with the gallery's bigger icon size
//! (the core sets it when switching), as in the GUI.
//!
//! # Navigation and scrolling
//! The pane's `FlowLayout` is built with these cells (`cell = (w, h)` in cells, headers
//! one row high), so the core's keyboard navigation works on what is drawn: Left / Right
//! move by item, Up / Down by line of cells, PageUp / PageDown by the lines that fit.
//! `pane.scroll_y` is the first layout line shown (a line of cells or a group header); only
//! lines that fit completely are drawn (the first always is).
//!
//! # Thumbnails
//! [`request_thumbs`] asks the core's thumbnail service (whose workers upload through the
//! terminal UI's `Uploader`, see `crate::images`) for the cells in view only, like the GUI:
//! pictures, videos, PDFs and (with `[appearance] folder_thumbnails`) folder mosaics, when
//! `[appearance] show_thumbnails` is on and `[tui] images` is not `off`. [`draw_lines`]
//! draws them with `images::draw_in`; pending, failed, non-image items and compact cells
//! show the item's icon glyph (in the icon set of `[tui] icons`) instead.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::services::preview_gfx::Tex;
use soyuz_appcore::services::thumb_video;
use soyuz_appcore::services::thumbs::{self, ThumbKey};
use soyuz_appcore::state::pane::Pane;
use soyuz_appcore::state::view_memory::{ZOOM_MAX, ZOOM_MIN};
use soyuz_core::config::Config;
use soyuz_core::{EntryKind, ViewMode};

use super::{PaneGeom, PaneLine, PaneVm, Row, RowKind, draw_name, fitted, mark, put, row_style};
use crate::images::{self, Drawn, ImageState};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// Images less than this many rows high give compact one-line cells.
pub const COMPACT_BELOW: u16 = 3;
/// Width of a compact cell.
pub const COMPACT_CELL_W: u16 = 18;
/// Narrowest image cell (room for a short name).
pub const GRID_CELL_MIN_W: u16 = 12;
/// Largest image area (cells).
const IMG_MAX: (u16, u16) = (64, 32);

/// The cell shape of a grid or gallery pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSpec {
    /// Image area in cells; `(0, 0)`: compact one-line cells.
    pub img_w: u16,
    pub img_h: u16,
    /// `[tui] cell_pixels` the image size was computed with.
    pub cell_px: (u16, u16),
}

impl Default for GridSpec {
    fn default() -> Self {
        let px = soyuz_core::config::TuiSection::CELL_PIXELS_DEFAULT;
        GridSpec::new(96, (px[0], px[1]))
    }
}

impl GridSpec {
    /// Cells for an icon of `zoom` points on cells of `cell_px` pixels.
    pub fn new(zoom: u16, cell_px: (u16, u16)) -> GridSpec {
        let (pw, ph) = (cell_px.0.max(1), cell_px.1.max(1));
        let z = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let img_w = z.div_ceil(pw).clamp(1, IMG_MAX.0);
        let img_h = z.div_ceil(ph).clamp(1, IMG_MAX.1);
        if img_h < COMPACT_BELOW {
            GridSpec { img_w: 0, img_h: 0, cell_px: (pw, ph) }
        } else {
            GridSpec { img_w, img_h, cell_px: (pw, ph) }
        }
    }

    /// The spec of `pane` (`pane.view.zoom`) with `[tui] cell_pixels` = `cell_px`.
    pub fn of(pane: &Pane, cell_px: (u16, u16)) -> GridSpec {
        GridSpec::new(pane.view.zoom, cell_px)
    }

    /// One-line cells (no image area).
    pub fn compact(self) -> bool {
        self.img_h == 0
    }

    /// Height of a cell: the image area and the name line.
    pub fn cell_h(self) -> u16 {
        if self.compact() { 1 } else { self.img_h + 1 }
    }

    /// Width of a cell before stretching.
    pub fn base_w(self) -> u16 {
        if self.compact() { COMPACT_CELL_W } else { self.img_w.max(GRID_CELL_MIN_W) }
    }

    /// The thumbnail size bucket (longest edge in pixels) for the image area.
    pub fn px(self) -> u32 {
        let w = u32::from(self.img_w) * u32::from(self.cell_px.0);
        let h = u32::from(self.img_h) * u32::from(self.cell_px.1);
        thumbs::bucket(w.max(h) as f32, 1.0)
    }
}

/// `[tui] cell_pixels` of `config` (clamped).
pub fn cell_px(config: &Config) -> (u16, u16) {
    images::cell_pixels(&config.tui)
}

/// Whether `mode` is drawn by this module.
pub fn is_grid(mode: ViewMode) -> bool {
    matches!(mode, ViewMode::Grid | ViewMode::Gallery)
}

/// Split the pane rect `area` into cells of `spec` (see the module docs).
pub fn geometry(spec: GridSpec, area: Rect) -> PaneGeom {
    let none = Rect::new(area.x, area.y, 0, 0);
    let gap = 1u16;
    let w = area.width.max(1);
    let base = spec.base_w().min(w).max(1);
    let cols = (w.saturating_add(gap) / (base + gap)).max(1);
    let max_w = (u32::from(base) * 145 / 100).min(u32::from(w)) as u16;
    let cell_w = ((w.saturating_add(gap)) / cols).saturating_sub(gap).clamp(base, max_w.max(base)).max(1);
    PaneGeom { list: area, header: none, parent: none, child: none, cell_w, cell_h: spec.cell_h(), gap }
}

/// A thumbnail of a cell in view (compared by size only: the model's equality is for
/// tests).
#[derive(Clone, Debug)]
pub struct GridThumb(pub Tex);

impl PartialEq for GridThumb {
    fn eq(&self, other: &Self) -> bool {
        self.0.size == other.0.size
    }
}

impl Eq for GridThumb {}

/// What a grid or gallery pane draws besides its rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridVm {
    pub spec: GridSpec,
    /// The thumbnails ready for items in view: `(visible index, texture)`.
    pub thumbs: Vec<(usize, GridThumb)>,
    /// The video duration labels of items in view.
    pub labels: Vec<(usize, String)>,
}

impl GridVm {
    /// The thumbnail of item `index`.
    pub fn thumb(&self, index: usize) -> Option<&Tex> {
        self.thumbs.iter().find(|(i, _)| *i == index).map(|(_, t)| &t.0)
    }

    fn label(&self, index: usize) -> Option<&str> {
        self.labels.iter().find(|(i, _)| *i == index).map(|(_, l)| l.as_str())
    }
}

/// Ask the core's thumbnail service for the items in view of `vm` (built from `pane`) and
/// keep the ready ones (see the module docs).
pub fn request_thumbs(core: &AppCore, pane: &Pane, vm: &mut PaneVm) {
    let Some(grid) = vm.grid.as_mut() else { return };
    let spec = grid.spec;
    let appearance = &core.config.appearance;
    if spec.compact() || !appearance.show_thumbnails || images::active().is_none() {
        return;
    }
    let px = spec.px();
    let thumbs = &core.services.thumbs;
    for row in vm.lines.iter().filter_map(|l| match l {
        PaneLine::Item(r) => Some(r),
        PaneLine::Header { .. } => None,
    }) {
        let Some(e) = pane.entry_at(row.index) else { continue };
        let key = || ThumbKey { path: e.path.clone(), mtime: e.modified, size: e.size, px };
        let (tex, label) = if matches!(e.kind, EntryKind::Dir) {
            (if appearance.folder_thumbnails { thumbs.get_folder(&key()) } else { None }, None)
        } else if thumbs::is_thumbnailable(&e.path)
            || thumb_video::is_video(&e.path)
            || thumbs::is_pdf_thumbnailable(&e.path)
        {
            thumbs.get_labeled(&key())
        } else {
            (None, None)
        };
        if let Some(t) = tex {
            grid.thumbs.push((row.index, GridThumb(t)));
        }
        if let Some(l) = label {
            grid.labels.push((row.index, crate::dialog::safe(&l)));
        }
    }
}

/// Draw the lines of grid pane `m` (geometry `g`).
pub(super) fn draw_lines(buf: &mut Buffer, g: &PaneGeom, m: &PaneVm, gv: &GridVm, theme: &TuiTheme, hits: &mut Hits) {
    let cols = g.cols();
    let bottom = g.list.bottom();
    let (mut y, mut col) = (g.list.y, 0u16);
    for l in &m.lines {
        match l {
            PaneLine::Header { group, label, count } => {
                if col > 0 {
                    y = y.saturating_add(g.cell_h);
                    col = 0;
                }
                if y >= bottom {
                    break;
                }
                let r = Rect { y, height: 1, ..g.list };
                let t = text::truncate_end(&format!("{label} ({count})"), usize::from(r.width));
                put(buf, r.x, r.y, &t, r.width, theme.style(Role::GroupHeader));
                hits.push(r, HitTarget::PaneHeader { pane: m.id, vis_version: m.vis_version, group: *group });
                y += 1;
            }
            PaneLine::Item(row) => {
                if y >= bottom {
                    break;
                }
                let x = g.list.x.saturating_add(col.saturating_mul(g.cell_w + g.gap));
                let r = Rect::new(x, y, g.cell_w, g.cell_h).intersection(g.list);
                if !r.is_empty() {
                    draw_cell(buf, m, gv, row, r, theme);
                    hits.push(r, HitTarget::PaneRow { pane: m.id, vis_version: m.vis_version, index: row.index });
                    if m.focused && row.cursor {
                        // The name line (the only line in screen-reader mode).
                        let y = if gv.spec.compact() || theme.screen_reader() { r.y } else { r.bottom() - 1 };
                        hits.set_caret(Position { x: r.x, y });
                    }
                    if let Some(e) = &row.edit {
                        let (name_r, prefix) = if gv.spec.compact() {
                            (r, super::prefix_w(m))
                        } else {
                            (Rect { y: r.bottom() - 1, height: 1, ..r }, 0)
                        };
                        crate::view::rename::draw_edit(buf, name_r, prefix, e, theme, hits);
                    }
                }
                col += 1;
                if col >= cols {
                    col = 0;
                    y = y.saturating_add(g.cell_h);
                }
            }
        }
    }
}

/// Draw one cell of `row` into `r`.
fn draw_cell(buf: &mut Buffer, m: &PaneVm, gv: &GridVm, row: &Row, r: Rect, theme: &TuiTheme) {
    let style = row_style(m, row, theme);
    // Screen-reader mode (T8): the name line only (no images, no big icons).
    if gv.spec.compact() || r.height < 2 || theme.screen_reader() {
        let line = Rect { height: 1, ..r };
        draw_name(buf, m, row, line.x, line.y, line.width, theme);
        buf.set_style(line, style);
        return;
    }
    let img = Rect { height: r.height - 1, ..r };
    let name = Rect { y: r.bottom() - 1, height: 1, ..r };
    let shown = gv.thumb(row.index).is_some_and(|tex| {
        let mut st = ImageState::default();
        matches!(images::draw_in(buf, img, tex, &mut st), Drawn::Image | Drawn::Pending | Drawn::Placeholder)
    });
    let name_style = row.name_style(theme);
    if !shown {
        let icon = if row.kind == RowKind::Link { IconId::Link } else { row.icon };
        let glyph = crate::icons::glyph(icon, m.icons);
        let gw = (text::width(glyph) as u16).min(img.width);
        put(buf, img.x + (img.width - gw) / 2, img.y + img.height / 2, glyph, img.width, name_style);
        if let Some(l) = gv.label(row.index) {
            let l = text::truncate_end(l, usize::from(img.width));
            let lw = text::width(&l) as u16;
            put(buf, img.right() - lw, img.bottom() - 1, &l, lw, theme.style(Role::Muted));
        }
        buf.set_style(img, style);
    }
    let mk = mark(m, row.selected);
    let mk = if row.selected || m.checkboxes { mk.trim_end() } else { "" };
    let mw = if mk.is_empty() { 0 } else { text::width(mk) as u16 + 1 };
    let dots = if name.width >= mw + row.dots_w() + 6 { row.dots_w() } else { 0 };
    let label = fitted(&row.name, row.kind, usize::from(name.width.saturating_sub(mw + dots)));
    let tw = (mw + text::width(&label) as u16 + dots).min(name.width);
    let mut x = name.x + (name.width - tw) / 2;
    if !mk.is_empty() {
        let ms = if row.selected { theme.style(Role::Selected) } else { theme.style(Role::Muted) };
        put(buf, x, name.y, mk, name.right() - x, ms);
        x = (x + mw).min(name.right());
    }
    put(buf, x, name.y, &label, name.right() - x, name_style);
    if dots > 0 {
        let after = (x + text::width(&label) as u16).min(name.right());
        row.draw_dots(buf, after, name.y, name.right() - after, m.icons, theme);
    }
    buf.set_style(name, style);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_appcore::state::pane::PaneId;

    use super::*;
    use crate::icons::IconSet;
    use crate::theme::TuiTheme;
    use crate::view::pane::{PaneMode, draw_pane};

    #[test]
    fn spec_follows_zoom_and_cell_pixels() {
        let s = GridSpec::new(96, (10, 20));
        assert_eq!((s.img_w, s.img_h, s.cell_h(), s.base_w()), (10, 5, 6, 12));
        assert!(!s.compact());
        assert_eq!(s.px(), 128);
        let g = GridSpec::new(192, (10, 20));
        assert_eq!((g.img_w, g.img_h, g.px()), (20, 10, 256));
        let small = GridSpec::new(32, (10, 20));
        assert!(small.compact());
        assert_eq!((small.cell_h(), small.base_w()), (1, COMPACT_CELL_W));
        // Square cells: taller images.
        assert_eq!(GridSpec::new(96, (12, 12)).img_h, 8);
        // Out-of-range values are clamped.
        let big = GridSpec::new(u16::MAX, (1, 1));
        assert_eq!((big.img_w, big.img_h), IMG_MAX);
        assert!(!GridSpec::new(0, (0, 0)).compact());
    }

    #[test]
    fn geometry_stretches_cells_to_fill() {
        let s = GridSpec::new(96, (10, 20));
        let g = geometry(s, Rect::new(0, 0, 80, 20));
        // 81 / 13 = 6 cells, stretched to 12 + 0 (81 / 6 - 1 = 12).
        assert_eq!((g.cols(), g.cell_w, g.cell_h, g.gap), (6, 12, 6, 1));
        let g = geometry(s, Rect::new(0, 0, 30, 20));
        assert_eq!((g.cols(), g.cell_w), (2, 14));
        // Narrower than a cell: one cell, the pane's width.
        let g = geometry(s, Rect::new(0, 0, 5, 20));
        assert_eq!((g.cols(), g.cell_w), (1, 5));
        for w in 0..=40u16 {
            let g = geometry(s, Rect::new(0, 0, w, 10));
            assert!(g.cols() * (g.cell_w + g.gap) <= w.max(1) + g.gap, "{w}: {g:?}");
        }
    }

    fn row(index: usize, name: &str, cursor: bool) -> PaneLine {
        PaneLine::Item(Row { index, name: name.into(), cursor, ..Row::default() })
    }

    fn vm(spec: GridSpec) -> PaneVm {
        PaneVm {
            id: PaneId(4),
            vis_version: 2,
            focused: true,
            mode: PaneMode::Grid,
            lines: vec![row(0, "a.png", true), row(1, "b.txt", false), row(2, "c.txt", false)],
            total: 3,
            icons: IconSet::Ascii,
            grid: Some(GridVm { spec, ..GridVm::default() }),
            ..PaneVm::default()
        }
    }

    #[test]
    fn draws_cells_with_names_and_targets() {
        let m = vm(GridSpec::new(96, (10, 20)));
        let mut t = Terminal::new(TestBackend::new(26, 12)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_pane(f, f.area(), &m, &TuiTheme::plain(), &mut hits)).unwrap();
        let rows: Vec<_> = hits
            .iter()
            .filter_map(|(r, t)| match t {
                HitTarget::PaneRow { index, .. } => Some((*index, *r)),
                _ => None,
            })
            .collect();
        // Two 12-wide cells per line (+ 1 gap), 6 rows high.
        assert_eq!(rows, [(0, Rect::new(0, 0, 12, 6)), (1, Rect::new(13, 0, 12, 6)), (2, Rect::new(0, 6, 12, 6))]);
        let buf = t.backend().buffer();
        let line = |y: u16| (0..26).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(5).contains("a.png") && line(5).contains("b.txt"), "{}", line(5));
        assert!(line(11).contains("c.txt"));
    }

    #[test]
    fn tiny_sizes_do_not_panic() {
        for spec in [GridSpec::new(96, (10, 20)), GridSpec::new(16, (10, 20)), GridSpec::new(256, (1, 1))] {
            let m = vm(spec);
            for w in 0..=20u16 {
                for h in 0..=6u16 {
                    let mut t = Terminal::new(TestBackend::new(w.max(1), h.max(1))).unwrap();
                    let mut hits = Hits::default();
                    t.draw(|f| draw_pane(f, Rect::new(0, 0, w, h), &m, &TuiTheme::plain(), &mut hits)).unwrap();
                }
            }
        }
    }
}
