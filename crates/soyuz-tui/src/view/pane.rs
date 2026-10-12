//! A pane's views (step T2.3): details (columns), list (row-major cells; grid and gallery
//! map to it until T4) and Miller (parent | current | child).
//!
//! Every row shows the selection mark (`* `, or `[x] ` / `[ ] ` in checkbox mode), the
//! file's icon (not in the ASCII icon set, where the kind markers say it: directories end
//! in `/`, symlinks to files in `@`), and the name fitted with
//! [`super::text::truncate_name`] (the extension stays), coloured by its file class (or
//! the folder's tag colour). The cursor row uses [`Role::Cursor`] in the focused pane and
//! [`Role::CursorInactive`] elsewhere.
//!
//! # Layout and scrolling
//!
//! [`geometry`] splits a pane rect for its [`PaneMode`]; [`prepare`] and [`draw_pane`]
//! both use it, so keyboard navigation (`actions::cursor` on the pane's [`FlowLayout`])
//! always works on what is drawn. The layout is in cell units, rebuilt only when its
//! [`LayoutSig`] changes (like the GUI's `views/flow.rs`):
//! - details: one item per row (`cell = (width, 1)`), below a header line with the
//!   column labels;
//! - list: cells of a fixed width (at least [`LIST_CELL_MIN`] columns, stretched to fill
//!   the pane) with one blank column between them, laid out row-major; Left / Right move
//!   between cells;
//! - Miller: the current folder as one column in the middle, the parent folder on the
//!   left and the folder under the cursor on the right (both from the core's aux
//!   listings, see [`miller`]); narrower than [`MILLER_MIN_W`] only the middle column.
//!
//! Group headers are one row high. Scroll state is the pane's own:
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
//!
//! Only the lines in view are turned into rows: building the model and drawing cost the
//! same for 10 or 100 000 entries.
//!
//! The pending key sequence (`g …`) and the type-ahead buffer show as a small overlay on
//! the focused pane's last line.

pub mod columns;
pub mod miller;

use std::time::Instant;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::{FileClass, classify};
use soyuz_appcore::geom::vec2;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::services::folder_colors::{self, FolderColor};
use soyuz_appcore::state::layout::{FlowLayout, LineKind};
use soyuz_appcore::state::pane::{LayoutSig, ListingStatus, Pane, PaneId};
use soyuz_appcore::tr;
use soyuz_core::config::Config;
use soyuz_core::{Column, EntryInfo, EntryKind, Location, SortDir, ViewMode};

use self::columns::ColumnVm;
use self::miller::MillerVm;
use super::text;
use crate::dialog::safe;
use crate::icons::{self, IconSet};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};

/// `LayoutSig::mode` of the details view (distinct from the GUI's view styles).
pub const LAYOUT_MODE: u8 = 0x80;
/// `LayoutSig::mode` of the list view.
pub const LAYOUT_LIST: u8 = 0x81;
/// `LayoutSig::mode` of Miller's middle column.
pub const LAYOUT_MILLER: u8 = 0x82;
/// Narrowest cell of the list view.
pub const LIST_CELL_MIN: u16 = 20;
/// Narrowest pane that shows Miller's parent and child columns.
pub const MILLER_MIN_W: u16 = 40;

/// How a pane is drawn (the core's [`ViewMode`] mapped to what the terminal has).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneMode {
    #[default]
    Details,
    /// List, and grid / gallery until T4.
    List,
    /// Miller columns (folders only; other locations use details).
    Miller,
}

impl PaneMode {
    pub fn of(pane: &Pane) -> PaneMode {
        match pane.view.mode {
            ViewMode::Details => PaneMode::Details,
            ViewMode::List | ViewMode::Grid | ViewMode::Gallery => PaneMode::List,
            ViewMode::Miller if matches!(pane.location, Location::Dir(_)) => PaneMode::Miller,
            ViewMode::Miller => PaneMode::Details,
        }
    }

    fn layout_mode(self) -> u8 {
        match self {
            PaneMode::Details => LAYOUT_MODE,
            PaneMode::List => LAYOUT_LIST,
            PaneMode::Miller => LAYOUT_MILLER,
        }
    }
}

/// Where the parts of a pane go. Parts not shown have an empty rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneGeom {
    /// The item list (Miller: the middle column).
    pub list: Rect,
    /// The column header line (details).
    pub header: Rect,
    /// Miller's parent and child columns.
    pub parent: Rect,
    pub child: Rect,
    /// Width of one item cell.
    pub cell_w: u16,
    /// Blank columns between cells.
    pub gap: u16,
}

impl PaneGeom {
    /// Item cells per line.
    pub fn cols(&self) -> u16 {
        ((self.list.width + self.gap) / (self.cell_w + self.gap).max(1)).max(1)
    }
}

/// Split the pane rect `area` for `mode` (see the module docs).
pub fn geometry(mode: PaneMode, area: Rect) -> PaneGeom {
    let none = Rect::new(area.x, area.y, 0, 0);
    let mut g = PaneGeom { list: area, header: none, parent: none, child: none, cell_w: area.width.max(1), gap: 0 };
    match mode {
        PaneMode::Details => {
            if area.height >= 2 {
                g.header = Rect { height: 1, ..area };
                g.list = Rect { y: area.y + 1, height: area.height - 1, ..area };
            }
        }
        PaneMode::List => {
            let w = area.width.max(1);
            let cols = ((w + 1) / (LIST_CELL_MIN + 1)).max(1);
            g.cell_w = ((w + 1) / cols).saturating_sub(1).max(1);
            g.gap = 1;
        }
        PaneMode::Miller => {
            if area.width >= MILLER_MIN_W {
                let side = (area.width - 2) / 4;
                let mid = area.width - 2 - 2 * side;
                g.parent = Rect { width: side, ..area };
                g.list = Rect { x: area.x + side + 1, width: mid, ..area };
                g.child = Rect { x: area.x + side + 1 + mid + 1, width: side, ..area };
                g.cell_w = mid;
            }
        }
    }
    g
}

/// What kind of item a row shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowKind {
    Dir,
    /// A symlink to a file (links to folders count as [`RowKind::Dir`]).
    Link,
    #[default]
    File,
}

impl RowKind {
    fn of(e: &EntryInfo) -> RowKind {
        if e.is_dir_like() {
            RowKind::Dir
        } else if matches!(e.kind, EntryKind::Symlink { .. }) {
            RowKind::Link
        } else {
            RowKind::File
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            RowKind::Dir => "/",
            RowKind::Link => "@",
            RowKind::File => "",
        }
    }
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
    pub icon: IconId,
    /// Formatted size (empty for folders).
    pub size: String,
    /// Details view: the text of each of [`PaneVm::columns`] (empty for the name).
    pub cells: Vec<String>,
    /// A hidden file (drawn muted).
    pub hidden: bool,
    /// The folder's tag colour (`[tags]` index).
    pub tag_color: Option<u8>,
    pub cursor: bool,
    pub selected: bool,
}

impl Default for Row {
    fn default() -> Self {
        Row {
            index: 0,
            name: String::new(),
            kind: RowKind::File,
            class: FileClass::Other,
            icon: IconId::File,
            size: String::new(),
            cells: Vec::new(),
            hidden: false,
            tag_color: None,
            cursor: false,
            selected: false,
        }
    }
}

impl Row {
    /// The name with its kind marker (`docs/`, `link@`).
    pub fn label(&self) -> String {
        format!("{}{}", self.name, self.kind.suffix())
    }

    /// The label fitted into `w` cells.
    pub fn fitted_label(&self, w: usize) -> String {
        fitted(&self.name, self.kind, w)
    }

    /// The style of the name (and icon).
    fn name_style(&self, theme: &TuiTheme) -> Style {
        let role = match (self.tag_color, self.kind) {
            (Some(t), RowKind::Dir) => Role::Tag(t),
            _ => Role::of_class(self.class),
        };
        let s = theme.style(Role::Text).patch(theme.style(role));
        if self.hidden { s.patch(theme.style(Role::Muted)) } else { s }
    }
}

/// `name` with the marker of `kind`, fitted into `w` cells.
fn fitted(name: &str, kind: RowKind, w: usize) -> String {
    let suffix = kind.suffix();
    let name = text::truncate_name(name, w.saturating_sub(suffix.len()), kind != RowKind::Dir);
    format!("{name}{suffix}")
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneLine {
    /// Group header `group` (grouped listings).
    Header { group: usize, label: String, count: usize },
    /// An item. In the list view consecutive items fill a line's cells left to right
    /// (a header or a full line starts the next line).
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
    pub mode: PaneMode,
    /// The lines in view (from `top`).
    pub lines: Vec<PaneLine>,
    /// First layout line shown (`pane.scroll_y`).
    pub top: usize,
    /// Items in the listing (after filters).
    pub total: usize,
    /// Shown instead of rows: loading, empty, failed, not available.
    pub message: Option<String>,
    /// Details view: the visible columns in order.
    pub columns: Vec<ColumnVm>,
    /// Miller's parent and child columns.
    pub miller: Option<MillerVm>,
    /// `ui.checkbox_mode`: rows carry `[x]` / `[ ]`.
    pub checkboxes: bool,
    /// The icon glyphs to draw.
    pub icons: IconSet,
    /// The pending key sequence or the type-ahead text (focused pane only).
    pub footer: Option<String>,
}

impl Default for PaneVm {
    fn default() -> Self {
        PaneVm {
            index: 0,
            id: PaneId(0),
            vis_version: 0,
            focused: false,
            mode: PaneMode::Details,
            lines: Vec::new(),
            top: 0,
            total: 0,
            message: None,
            columns: Vec::new(),
            miller: None,
            checkboxes: false,
            icons: IconSet::default(),
            footer: None,
        }
    }
}

/// What a pane's model is built with besides the pane (see [`PaneVm::build`]).
pub struct BuildCx<'a> {
    pub config: &'a Config,
    /// The tab's focused pane.
    pub focused: bool,
    /// `ui.checkbox_mode`.
    pub checkboxes: bool,
    pub icons: IconSet,
    pub footer: Option<String>,
}

/// The icon set of the terminal UI (`[tui] icons`).
fn icon_set(core: &AppCore) -> IconSet {
    IconSet::from_config(&core.config.tui)
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
        let focused = index == tab.focused;
        let cx = BuildCx {
            config: &core.config,
            focused,
            checkboxes: core.ui.checkbox_mode,
            icons: icon_set(core),
            footer: if focused { footer(core, pane) } else { None },
        };
        Some(PaneVm::build(pane, index, cx))
    }

    /// The model of `pane` (tab pane `index`, after [`prepare_pane`]).
    pub fn build(pane: &Pane, index: usize, cx: BuildCx<'_>) -> PaneVm {
        let BuildCx { config, focused, checkboxes, icons, footer } = cx;
        let mode = PaneMode::of(pane);
        let total = pane.visible.len();
        let top = pane.scroll_y.max(0.0) as usize;
        let rows = pane.viewport_h.max(0.0) as usize;
        let columns = if mode == PaneMode::Details { columns::columns(pane) } else { Vec::new() };
        let mut lines = Vec::new();
        if let Some(layout) = pane.layout.as_ref().filter(|l| l.n == total) {
            let all = layout.lines();
            let shown = &all[top.min(all.len())..(top + rows).min(all.len())];
            let cx = RowCx { config, pane, cursor: pane.cursor_pos(), columns: &columns };
            for line in shown {
                match line.kind {
                    LineKind::Header(g) => {
                        let Some(span) = pane.groups.get(g) else { continue };
                        lines.push(PaneLine::Header { group: g, label: safe(&span.label), count: span.count });
                    }
                    LineKind::Items { start, len } => {
                        lines.extend((start..start + len).filter_map(|i| cx.row(i)).map(PaneLine::Item));
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
        let miller = (mode == PaneMode::Miller).then(|| miller::vm(pane, rows));
        PaneVm {
            index,
            id: pane.id,
            vis_version: pane.vis_version,
            focused,
            mode,
            lines,
            top,
            total,
            message,
            columns,
            miller,
            checkboxes,
            icons,
            footer,
        }
    }
}

/// The pending key sequence, else the live type-ahead text.
fn footer(core: &AppCore, pane: &Pane) -> Option<String> {
    if let Some(l) = core.keyseq.pending_label() {
        return Some(safe(&l));
    }
    pane.type_ahead.is_active(Instant::now()).then(|| format!("› {}", safe(&pane.type_ahead.buf)))
}

/// What building a row needs.
struct RowCx<'a> {
    config: &'a Config,
    pane: &'a Pane,
    cursor: Option<usize>,
    columns: &'a [ColumnVm],
}

impl RowCx<'_> {
    fn row(&self, i: usize) -> Option<Row> {
        let (e, key) = (self.pane.entry_at(i)?, self.pane.key_at(i)?);
        let general = &self.config.general;
        let kind = RowKind::of(e);
        let name = if general.show_extensions || kind == RowKind::Dir { e.name.as_str() } else { e.stem() };
        let class = classify(e);
        let tag_color = match kind {
            RowKind::Dir => match folder_colors::folder_color(&e.path) {
                Some(FolderColor::Tag(t)) => Some(t),
                // T2.2: fixed colours need the theme's colour depth; tag colours follow it.
                Some(FolderColor::Rgb(_)) | None => None,
            },
            _ => None,
        };
        Some(Row {
            index: i,
            name: safe(name),
            kind,
            class,
            icon: icons::class_icon(class),
            size: if kind == RowKind::Dir {
                String::new()
            } else {
                soyuz_core::size::format_size(e.size, general.size_style)
            },
            cells: self.columns.iter().map(|c| columns::cell_text(c.column, e, self.config)).collect(),
            hidden: e.is_hidden(),
            tag_color,
            cursor: self.cursor == Some(i),
            selected: self.pane.selection.contains(key),
        })
    }
}

/// Lay out and scroll every drawn pane; load Miller's side columns (see the module docs).
pub fn prepare(core: &mut AppCore, regions: &Regions) {
    let tab = core.ws.tab_mut();
    for p in &regions.panes {
        if let Some(pane) = tab.panes.get_mut(p.index) {
            prepare_pane(pane, p.rect.width, p.rect.height);
        }
    }
    miller::prepare(core, regions);
}

/// The pane is `width` × `rows` cells: layout, scroll, page size.
pub fn prepare_pane(pane: &mut Pane, width: u16, rows: u16) {
    let mode = PaneMode::of(pane);
    let g = geometry(mode, Rect::new(0, 0, width, rows));
    let w = f32::from(g.list.width.max(1));
    let cell = vec2(f32::from(g.cell_w), 1.0);
    let gap = vec2(f32::from(g.gap), 0.0);
    let sig = LayoutSig { vis_version: pane.vis_version, width: w, cell, mode: mode.layout_mode() };
    if pane.layout_sig != Some(sig) || pane.layout.is_none() {
        pane.layout = Some(FlowLayout::new(pane.visible.len(), &pane.groups, w, cell, gap, vec2(0.0, 0.0), 1.0));
        pane.layout_sig = Some(sig);
    }
    let Some(layout) = pane.layout.as_ref() else { return };
    let rows = usize::from(g.list.height);
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

/// Write `s` at `(x, y)`, at most `w` cells, in `style`.
pub(crate) fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, w: u16, style: Style) {
    if w > 0 {
        buf.set_stringn(x, y, s, usize::from(w), style);
    }
}

/// The selection mark of a row (`* `, or `[x] ` / `[ ] ` in checkbox mode).
fn mark(m: &PaneVm, selected: bool) -> &'static str {
    match (m.checkboxes, selected) {
        (true, true) => "[x] ",
        (true, false) => "[ ] ",
        (false, true) => "* ",
        (false, false) => "  ",
    }
}

/// Cells taken by the mark and the icon in front of a name.
fn prefix_w(m: &PaneVm) -> u16 {
    let mark = if m.checkboxes { 4 } else { 2 };
    mark + icon_w(m.icons)
}

/// Cells of an icon plus its space (none in the ASCII set: the kind markers say it).
fn icon_w(set: IconSet) -> u16 {
    if set == IconSet::Ascii { 0 } else { 3 }
}

/// Draw the mark, icon and name of `row` into `w` cells at `(x, y)`.
fn draw_name(buf: &mut Buffer, m: &PaneVm, row: &Row, x: u16, y: u16, w: u16, theme: &TuiTheme) {
    let mk = mark(m, row.selected);
    let mut x = x;
    let end = x + w;
    let mark_style = if row.selected { theme.style(Role::Selected) } else { theme.style(Role::Muted) };
    put(buf, x, y, mk, end - x, mark_style);
    x = (x + text::width(mk) as u16).min(end);
    let name_style = row.name_style(theme);
    if m.icons != IconSet::Ascii {
        let icon = if row.kind == RowKind::Link { IconId::Link } else { row.icon };
        put(buf, x, y, icons::glyph(icon, m.icons), end - x, name_style);
        x = (x + icon_w(m.icons)).min(end);
    }
    let label = row.fitted_label(usize::from(end - x));
    put(buf, x, y, &label, end - x, name_style);
}

/// The style a row ends with: selection, then cursor.
fn row_style(m: &PaneVm, row: &Row, theme: &TuiTheme) -> Style {
    let mut style = Style::new();
    if row.selected {
        style = style.patch(theme.style(Role::Selected));
    }
    if row.cursor {
        style = style.patch(theme.style(if m.focused { Role::Cursor } else { Role::CursorInactive }));
    }
    style
}

/// Draw pane `m` into `rect`.
pub fn draw_pane(f: &mut Frame<'_>, rect: Rect, m: &PaneVm, theme: &TuiTheme, hits: &mut Hits) {
    hits.push(rect, HitTarget::PaneBackground { pane: m.id });
    let rect = rect.intersection(f.area());
    if rect.is_empty() {
        return;
    }
    let g = geometry(m.mode, rect);
    let buf = f.buffer_mut();
    buf.set_style(rect, theme.style(Role::Text));
    let fit = columns::fit(&m.columns, g.list.width, prefix_w(m));
    if m.mode == PaneMode::Details {
        columns::draw_header(buf, g.header, m, &fit, theme, hits);
    }
    if let Some(msg) = &m.message
        && m.lines.is_empty()
    {
        let msg = text::truncate_end(msg, usize::from(g.list.width));
        let x = g.list.x + (g.list.width - text::width(&msg) as u16) / 2;
        put(buf, x, g.list.y, &msg, g.list.width, theme.style(Role::Muted));
    }
    let cols = if m.mode == PaneMode::List { g.cols() } else { 1 };
    let (mut line, mut col) = (0u16, 0u16);
    for l in &m.lines {
        if line >= g.list.height {
            break;
        }
        let y = g.list.y + line;
        match l {
            PaneLine::Header { group, label, count } => {
                if col > 0 {
                    line += 1;
                    col = 0;
                    if line >= g.list.height {
                        break;
                    }
                }
                let r = Rect { y: g.list.y + line, height: 1, ..g.list };
                let t = text::truncate_end(&format!("{label} ({count})"), usize::from(r.width));
                put(buf, r.x, r.y, &t, r.width, theme.style(Role::GroupHeader));
                hits.push(r, HitTarget::PaneHeader { pane: m.id, vis_version: m.vis_version, group: *group });
                line += 1;
            }
            PaneLine::Item(row) => {
                let r = Rect::new(g.list.x + col * (g.cell_w + g.gap), y, g.cell_w, 1).intersection(g.list);
                if m.mode == PaneMode::Details {
                    columns::draw_row(buf, r, m, row, &fit, theme);
                } else {
                    draw_name(buf, m, row, r.x, r.y, r.width, theme);
                }
                buf.set_style(r, row_style(m, row, theme));
                hits.push(r, HitTarget::PaneRow { pane: m.id, vis_version: m.vis_version, index: row.index });
                col += 1;
                if col >= cols {
                    col = 0;
                    line += 1;
                }
            }
        }
    }
    if let Some(mv) = &m.miller {
        miller::draw(buf, &g, m, mv, theme, hits);
    }
    if let Some(footer) = &m.footer {
        draw_footer(buf, rect, footer, theme);
    }
}

/// The footer overlay: right-aligned on the pane's last line.
fn draw_footer(buf: &mut Buffer, rect: Rect, footer: &str, theme: &TuiTheme) {
    let t = text::truncate_end(&format!(" {footer} "), usize::from(rect.width));
    let w = text::width(&t) as u16;
    let r = Rect::new(rect.right() - w, rect.bottom() - 1, w, 1);
    buf.set_style(r, Style::reset());
    put(buf, r.x, r.y, &t, r.width, theme.style(Role::StatusBar));
}

/// The sort marker of a column header.
fn sort_glyph(dir: SortDir, set: IconSet) -> &'static str {
    icons::glyph(if dir == SortDir::Asc { IconId::CaretUp } else { IconId::CaretDown }, set)
}

/// `true` when `col` is the column the pane sorts by.
fn sorted_by(pane: &Pane, col: Column) -> Option<SortDir> {
    (pane.view.sort.column == col).then_some(pane.view.sort.dir)
}

#[cfg(test)]
mod tests;
