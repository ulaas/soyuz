//! Details-view columns: cell text formatting and horizontal geometry (pure).

use soyuz_core::config::Config;
use soyuz_core::{Attributes, Column, ColumnSpec, EntryInfo, format_size, format_time};

use crate::state::listing::kind_label;

pub const MIN_COL_W: f32 = 48.0;
pub const MIN_NAME_W: f32 = 160.0;

/// Horizontal placement of one visible column (content coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColGeom {
    pub column: Column,
    pub x: f32,
    pub w: f32,
}

/// Lay out columns left to right starting at `pad`. The name column absorbs the free
/// width so the table always fills `avail`; returns the geometry and content width.
pub fn geometry(cols: &[ColumnSpec], avail: f32, pad: f32) -> (Vec<ColGeom>, f32) {
    let others: f32 = cols.iter().filter(|c| c.column != Column::Name).map(|c| c.width.max(MIN_COL_W)).sum();
    let mut x = pad;
    let mut out = Vec::with_capacity(cols.len());
    for c in cols {
        let w = if c.column == Column::Name {
            c.width.max(MIN_NAME_W).max(avail - 2.0 * pad - others)
        } else {
            c.width.max(MIN_COL_W)
        };
        out.push(ColGeom { column: c.column, x, w });
        x += w;
    }
    (out, (x + pad).max(avail))
}

/// Columns drawn with the `[fonts.numbers]` font (figures that line up row to row).
pub fn numeric(col: Column) -> bool {
    matches!(
        col,
        Column::Size | Column::ItemCount | Column::Modified | Column::Created | Column::Accessed | Column::Duration
    )
}

pub fn right_aligned(col: Column) -> bool {
    matches!(col, Column::Size | Column::ItemCount)
}

/// Display text of `col` for `e`.
pub fn cell_text(col: Column, e: &EntryInfo, cfg: &Config) -> String {
    match col {
        Column::Name => super::display_name(e).into_owned(),
        Column::Size => {
            if e.is_dir_like() {
                // Filled in (and animated) by "Calculate folder sizes".
                crate::services::search::dir_size_text(&e.path, cfg.general.size_style)
            } else {
                format_size(e.size, cfg.general.size_style)
            }
        }
        Column::Kind => kind_label(e),
        Column::Modified => e.modified.map(|t| format_time(t, &cfg.general.date_format)).unwrap_or_default(),
        Column::Created => e.created.map(|t| format_time(t, &cfg.general.date_format)).unwrap_or_default(),
        Column::Accessed => e.accessed.map(|t| format_time(t, &cfg.general.date_format)).unwrap_or_default(),
        Column::Extension => e.extension().map(str::to_uppercase).unwrap_or_default(),
        Column::Attributes => attr_letters(e.attrs),
        Column::Mode => e.mode.map(mode_string).unwrap_or_default(),
        Column::Owner => e.owner.as_deref().unwrap_or("").to_string(),
        Column::Path => {
            e.path.parent().map(|p| soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()).unwrap_or_default()
        }
        Column::Tags | Column::GitStatus => crate::ui::tools::decor::cell_text(col, e).unwrap_or_default(),
        Column::Dimensions | Column::Duration | Column::LinkTarget | Column::Checksum | Column::ItemCount => {
            String::new()
        }
        Column::Script1 | Column::Script2 | Column::Script3 | Column::Script4 => {
            crate::automation::scripts::cell_text(col, e)
        }
    }
}

/// Header label: script columns are named by their provider (F6).
pub fn header_label(col: Column) -> String {
    crate::automation::scripts::column_label(col).unwrap_or_else(|| crate::i18n::labels::column(col).to_string())
}

/// `RHSA…` attribute letters.
pub fn attr_letters(a: Attributes) -> String {
    let mut s = String::new();
    for (flag, c) in [
        (Attributes::READONLY, 'R'),
        (Attributes::HIDDEN, 'H'),
        (Attributes::SYSTEM, 'S'),
        (Attributes::ARCHIVE, 'A'),
        (Attributes::COMPRESSED, 'C'),
        (Attributes::ENCRYPTED, 'E'),
        (Attributes::REPARSE, 'L'),
        (Attributes::OFFLINE, 'O'),
        (Attributes::CLOUD_PLACEHOLDER, 'P'),
    ] {
        if a.contains(flag) {
            s.push(c);
        }
    }
    s
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_absorbs_free_width() {
        let cols = vec![
            ColumnSpec { column: Column::Name, width: 200.0, visible: true },
            ColumnSpec { column: Column::Size, width: 90.0, visible: true },
            ColumnSpec { column: Column::Modified, width: 150.0, visible: true },
        ];
        let (g, w) = geometry(&cols, 1000.0, 6.0);
        assert_eq!(g[0].w, 1000.0 - 12.0 - 240.0);
        assert_eq!(g[1].x, 6.0 + g[0].w);
        assert_eq!(w, 1000.0);
        // Narrow window: name keeps its configured width and content scrolls.
        let (g, w) = geometry(&cols, 300.0, 6.0);
        assert_eq!(g[0].w, 200.0);
        assert_eq!(w, 6.0 + 200.0 + 240.0 + 6.0);
    }

    #[test]
    fn formatting() {
        assert_eq!(mode_string(0o755), "rwxr-xr-x");
        assert_eq!(attr_letters(Attributes::READONLY | Attributes::HIDDEN), "RH");
        let cfg = Config::default();
        let mut e = EntryInfo::minimal("/x/a.txt".into(), soyuz_core::EntryKind::File);
        e.size = 2048;
        assert_eq!(cell_text(Column::Extension, &e, &cfg), "TXT");
        assert!(!cell_text(Column::Size, &e, &cfg).is_empty());
        let d = EntryInfo::minimal("/x/d".into(), soyuz_core::EntryKind::Dir);
        assert_eq!(cell_text(Column::Size, &d, &cfg), "");
        assert_eq!(cell_text(Column::Kind, &d, &cfg), "Folder");
    }
}
