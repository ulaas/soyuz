//! Details-view columns: header labels and cell text (like the GUI's `views/columns.rs`),
//! and the fitting of the columns into the pane width.
//!
//! The GUI stores column widths in points; a terminal cell counts as [`PT_PER_CELL`]
//! points (Size 90 pt → 11 cells, Modified 150 pt → 19). The name column fills the free
//! width (its stored width is only a minimum in the GUI too) and keeps at least
//! [`NAME_MIN`] cells after the mark and icon; columns that do not fit then are left out
//! from the right (a terminal has no horizontal scrolling).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::automation::scripts;
use soyuz_appcore::i18n::labels;
use soyuz_appcore::services::{git, search, tags};
use soyuz_appcore::state::listing::kind_label;
use soyuz_appcore::state::pane::Pane;
use soyuz_appcore::tr;
use soyuz_core::config::Config;
use soyuz_core::{Attributes, Column, EntryInfo, SortDir, format_time};

use super::{PaneVm, Row, draw_name, put, sort_glyph, sorted_by};
use crate::dialog::safe;
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// Narrowest name (cells, without the mark and icon).
pub const NAME_MIN: u16 = 12;
/// Narrowest / widest other column.
pub const COL_MIN: u16 = 4;
pub const COL_MAX: u16 = 40;
/// GUI points per terminal cell.
pub const PT_PER_CELL: f32 = 8.0;

/// A details column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnVm {
    pub column: Column,
    /// Header label (script columns are named by their provider).
    pub label: String,
    /// Wanted width in cells (`0` for the name: it fills).
    pub width: u16,
    /// Right-aligned (sizes, counts).
    pub right: bool,
    /// The pane sorts by this column.
    pub sort: Option<SortDir>,
}

impl ColumnVm {
    /// The column `column` at its default width.
    pub fn new(column: Column) -> ColumnVm {
        ColumnVm {
            column,
            label: header_label(column),
            width: cells(column, column.default_width()),
            right: right_aligned(column),
            sort: None,
        }
    }
}

/// The visible columns of `pane`, in order.
pub fn columns(pane: &Pane) -> Vec<ColumnVm> {
    pane.view
        .visible_columns()
        .into_iter()
        .map(|c| ColumnVm {
            width: cells(c.column, c.width),
            sort: sorted_by(pane, c.column),
            ..ColumnVm::new(c.column)
        })
        .collect()
}

/// A width in GUI points as cells (`0` for the name).
pub fn cells(col: Column, pts: f32) -> u16 {
    if col == Column::Name {
        return 0;
    }
    let w = if pts.is_finite() { (pts / PT_PER_CELL).round() } else { 12.0 };
    w.clamp(f32::from(COL_MIN), f32::from(COL_MAX)) as u16
}

pub fn right_aligned(col: Column) -> bool {
    matches!(col, Column::Size | Column::ItemCount)
}

/// Header label of `col`.
pub fn header_label(col: Column) -> String {
    safe(&scripts::column_label(col).unwrap_or_else(|| labels::column(col).to_string()))
}

/// A column placed in the list: `index` into the columns (`None`: a name column the
/// model did not list), `x` from the list's left edge, `w` cells. The name's width
/// includes the mark and icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub index: Option<usize>,
    pub column: Column,
    pub x: u16,
    pub w: u16,
}

/// Fit `cols` into `width` cells, `prefix` cells of mark and icon in front of the name
/// (see the module docs). Columns are separated by one blank cell.
pub fn fit(cols: &[ColumnVm], width: u16, prefix: u16) -> Vec<Placed> {
    let name_min = prefix + NAME_MIN.min(width.saturating_sub(prefix));
    let mut keep: Vec<usize> = (0..cols.len()).collect();
    let others = |keep: &[usize]| -> u16 {
        keep.iter().filter(|&&i| cols[i].column != Column::Name).map(|&i| cols[i].width + 1).sum()
    };
    while others(&keep) + name_min > width {
        let Some(pos) = keep.iter().rposition(|&i| cols[i].column != Column::Name) else { break };
        keep.remove(pos);
    }
    let name_w = width.saturating_sub(others(&keep));
    let mut placed = Vec::with_capacity(keep.len() + 1);
    if !keep.iter().any(|&i| cols[i].column == Column::Name) {
        placed.push(Placed { index: None, column: Column::Name, x: 0, w: name_w });
    }
    let mut x = 0;
    for (n, i) in keep.into_iter().enumerate() {
        let c = &cols[i];
        let w = if c.column == Column::Name { name_w } else { c.width };
        if n > 0 {
            x += 1;
        }
        placed.push(Placed { index: Some(i), column: c.column, x, w });
        x += w;
    }
    // A missing name column comes first: shift the others behind it.
    if placed.first().is_some_and(|p| p.index.is_none()) {
        let mut x = name_w;
        for p in placed.iter_mut().skip(1) {
            x += 1;
            p.x = x;
            x += p.w;
        }
    }
    placed
}

/// The header line: labels, the sort column marked.
pub fn draw_header(buf: &mut Buffer, r: Rect, m: &PaneVm, fit: &[Placed], theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let prefix = super::prefix_w(m);
    for p in fit {
        let c = p.index.and_then(|i| m.columns.get(i));
        let label = c.map_or_else(|| header_label(Column::Name), |c| c.label.clone());
        let sort = c.and_then(|c| c.sort);
        let label = match sort {
            // Screen-reader mode (T8): the direction as a word, not `^` / `v`.
            Some(dir) if theme.screen_reader() => {
                let word = if dir == SortDir::Asc { tr!("tui-sr-ascending") } else { tr!("tui-sr-descending") };
                format!("{label} {word}")
            }
            Some(dir) => format!("{label} {}", sort_glyph(dir, m.icons)),
            None => label,
        };
        let (x, w) = if p.column == Column::Name {
            let pre = prefix.min(p.w);
            (r.x + p.x + pre, p.w - pre)
        } else {
            (r.x + p.x, p.w)
        };
        let t = text::truncate_end(&label, usize::from(w));
        let tw = text::width(&t) as u16;
        let x = if c.is_some_and(|c| c.right) { x + w - tw } else { x };
        let style = if sort.is_some() {
            theme.style(Role::Title).patch(theme.style(Role::Accent))
        } else {
            theme.style(Role::Muted)
        };
        put(buf, x, r.y, &t, w, style);
        hits.push(
            Rect::new(r.x + p.x, r.y, p.w, 1).intersection(r),
            HitTarget::PaneColumn { pane: m.id, column: p.column },
        );
    }
}

/// One item row of the details view.
pub fn draw_row(buf: &mut Buffer, r: Rect, m: &PaneVm, row: &Row, fit: &[Placed], theme: &TuiTheme) {
    for p in fit {
        let x = r.x + p.x;
        if x >= r.right() {
            break;
        }
        let w = p.w.min(r.right() - x);
        if p.column == Column::Name {
            draw_name(buf, m, row, x, r.y, w, theme);
            continue;
        }
        let Some(cell) = p.index.and_then(|i| row.cells.get(i)) else { continue };
        let t = text::truncate_end(cell, usize::from(w));
        let tw = text::width(&t) as u16;
        let right = p.index.and_then(|i| m.columns.get(i)).is_some_and(|c| c.right);
        let x = if right { x + w - tw } else { x };
        put(buf, x, r.y, &t, w, theme.style(Role::Muted));
    }
}

/// Display text of `col` for `e` (the name is drawn from the row itself).
pub fn cell_text(col: Column, e: &EntryInfo, cfg: &Config) -> String {
    let style = cfg.general.size_style;
    let time =
        |t: Option<std::time::SystemTime>| t.map(|t| format_time(t, &cfg.general.date_format)).unwrap_or_default();
    let s = match col {
        Column::Name => String::new(),
        Column::Size if e.is_dir_like() => search::dir_size_text(&e.path, style),
        Column::Size => soyuz_core::size::format_size(e.size, style),
        Column::Kind => kind_label(e),
        Column::Modified => time(e.modified),
        Column::Created => time(e.created),
        Column::Accessed => time(e.accessed),
        Column::Extension => e.extension().map(str::to_uppercase).unwrap_or_default(),
        Column::Attributes => attr_letters(e.attrs),
        Column::Mode => e.mode.map(mode_string).unwrap_or_default(),
        Column::Owner => e.owner.as_deref().unwrap_or_default().to_string(),
        Column::Path => {
            e.path.parent().map(|p| soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()).unwrap_or_default()
        }
        Column::Tags => tags::mask_tags(tags::tags_of(&e.path)).map(tags::name).collect::<Vec<_>>().join(", "),
        Column::GitStatus => {
            git::status_of(&e.path, e.is_dir_like()).map(|c| c.label().to_string()).unwrap_or_default()
        }
        Column::Dimensions | Column::Duration | Column::LinkTarget | Column::Checksum | Column::ItemCount => {
            String::new()
        }
        Column::Script1 | Column::Script2 | Column::Script3 | Column::Script4 => scripts::cell_text(col, e),
    };
    safe(&s)
}

/// `RHSA…` attribute letters.
pub fn attr_letters(a: Attributes) -> String {
    [
        (Attributes::READONLY, 'R'),
        (Attributes::HIDDEN, 'H'),
        (Attributes::SYSTEM, 'S'),
        (Attributes::ARCHIVE, 'A'),
        (Attributes::COMPRESSED, 'C'),
        (Attributes::ENCRYPTED, 'E'),
        (Attributes::REPARSE, 'L'),
        (Attributes::OFFLINE, 'O'),
        (Attributes::CLOUD_PLACEHOLDER, 'P'),
    ]
    .into_iter()
    .filter(|(f, _)| a.contains(*f))
    .map(|(_, c)| c)
    .collect()
}

/// `rwxr-xr-x` from a unix mode.
pub fn mode_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}
