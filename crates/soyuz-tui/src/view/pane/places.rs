//! The Trash and Computer pages (Phase T7 "places"): what a pane shows at `trash:` and
//! `computer:` instead of a listing, like the GUI's `ui/views/trash.rs` and
//! `ui/views/computer.rs`.
//!
//! - **Trash**: a header line (title, the core's summary of count / size / selection,
//!   `[Restore]` `[Delete permanently]` with a selection, `[Empty trash]` with items,
//!   `[Refresh]`), the column labels and one row per trashed item (name, original
//!   location, date deleted, size). The items, the selection and the worker that lists
//!   the bins are the core's `models::trash::TrashState`, shared with the GUI.
//! - **Computer**: a header line (title, `[Refresh]`) and one row per drive (the
//!   sidebar's drives, `services::places::drive_volumes`): name, a usage bar (`#` / `.`
//!   with ASCII icons, `█` / `░` otherwise, warning / danger colours like the GUI's),
//!   "free of total" and the filesystem. The volumes come from the core's places thread.
//!
//! Narrow panes drop columns (the rightmost detail first) and header buttons (Refresh
//! first); one or two rows still show the list. The cursor and scroll offset of each pane
//! live here ([`Nav`], per pane, kept while the pane exists), since neither page has
//! entries in the pane itself; `crate::input::list::places` moves them.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::classify;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::trash::{self, TrashState};
use soyuz_appcore::services::places::{UsageLevel, drive_volumes, usage_level, used_share, volume_label};
use soyuz_appcore::state::pane::{Pane, PaneId};
use soyuz_appcore::tr;
use soyuz_core::config::Config;
use soyuz_core::{EntryInfo, Location, SizeStyle, format_size, format_time};
use soyuz_fs::{VolumeInfo, VolumeKind};

use super::{RowKind, fitted, put};
use crate::dialog::safe;
use crate::icons::{self, IconSet};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// Header button ids ([`HitTarget::PlacesButton`]).
pub mod button {
    /// Restore the selection (Trash).
    pub const RESTORE: u8 = 0;
    /// Delete the selection permanently, after the core's confirmation (Trash).
    pub const PURGE: u8 = 1;
    /// Empty the trash, after the core's confirmation.
    pub const EMPTY: u8 = 2;
    /// List the bins / volumes again.
    pub const REFRESH: u8 = 3;
}

/// Which page a pane shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Trash,
    Computer,
}

/// The page of `pane`, if it shows one.
pub fn page(pane: &Pane) -> Option<Page> {
    match pane.location {
        Location::Trash => Some(Page::Trash),
        Location::Computer => Some(Page::Computer),
        _ => None,
    }
}

/// The drives the Computer page lists (the sidebar's Drives).
pub fn drives(core: &AppCore) -> Cow<'_, [VolumeInfo]> {
    drive_volumes(&core.ui.volumes, &core.ui.cloud)
}

/// Rows of `page`.
pub fn len(core: &AppCore, page: Page) -> usize {
    match page {
        Page::Trash => core.fileops.trash.items.len(),
        Page::Computer => drives(core).len(),
    }
}

// ---------------------------------------------------------------------------------------
// Cursor and scroll per pane.

/// A pane's cursor row and scroll offset on its page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nav {
    /// The page these belong to (another page starts over).
    pub page: Page,
    pub cursor: usize,
    /// First row shown.
    pub top: usize,
    /// Rows the list showed last frame (page up / down).
    pub rows: usize,
    /// Bring the cursor into view at the next [`prepare`].
    pub follow: bool,
}

thread_local! {
    static NAV: RefCell<HashMap<PaneId, Nav>> = RefCell::new(HashMap::new());
}

/// Pane `pane`'s state on `page` (a fresh one when it showed another page).
pub fn nav(pane: PaneId, page: Page) -> Nav {
    NAV.with_borrow(|m| m.get(&pane).copied().filter(|n| n.page == page).unwrap_or(Nav { page, ..Nav::default() }))
}

/// Change pane `pane`'s state on `page`.
pub fn update(pane: PaneId, page: Page, f: impl FnOnce(&mut Nav)) {
    let mut n = nav(pane, page);
    f(&mut n);
    NAV.with_borrow_mut(|m| m.insert(pane, n));
}

/// Clamp `n` to `len` rows with `rows` in view; the cursor comes into view when asked.
pub fn clamp(n: &mut Nav, len: usize, rows: usize) {
    n.rows = rows;
    n.cursor = n.cursor.min(len.saturating_sub(1));
    if std::mem::take(&mut n.follow) && rows > 0 {
        if n.cursor < n.top {
            n.top = n.cursor;
        } else if n.cursor >= n.top + rows {
            n.top = n.cursor + 1 - rows;
        }
    }
    n.top = n.top.min(len.saturating_sub(rows.max(1)));
}

/// Where a page's parts go in the pane rect `area`: the header line (two rows and up),
/// the column labels (four rows and up) and the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geom {
    pub header: Rect,
    pub labels: Rect,
    pub list: Rect,
}

pub fn geometry(area: Rect) -> Geom {
    let none = Rect::new(area.x, area.y, 0, 0);
    let header = if area.height >= 2 { Rect { height: 1, ..area } } else { none };
    let labels = if area.height >= 4 { Rect { y: area.y + 1, height: 1, ..area } } else { none };
    let used = header.height + labels.height;
    let list = Rect { y: area.y + used, height: area.height - used, ..area };
    Geom { header, labels, list }
}

/// Every frame before the model: clamp each drawn page's cursor and scroll to its rows,
/// drop the state of closed panes, and show the trash reader's notices (as the GUI's view
/// does).
pub fn prepare(core: &mut AppCore, regions: &Regions) {
    if let Some(t) = core.fileops.trash.take_notice() {
        core.ui.toasts.push(t);
    }
    let tab = core.ws.tab();
    for p in &regions.panes {
        let Some(pane) = tab.panes.get(p.index) else { continue };
        let Some(page) = page(pane) else { continue };
        let n = len(core, page);
        let rows = usize::from(geometry(p.rect).list.height);
        update(pane.id, page, |nav| clamp(nav, n, rows));
    }
    NAV.with_borrow_mut(|m| m.retain(|id, _| core.ws.find_pane(*id).is_some()));
}

// ---------------------------------------------------------------------------------------
// The model.

/// A header button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonVm {
    pub id: u8,
    pub label: String,
    /// Drawn in the danger colour (delete, empty).
    pub danger: bool,
}

/// A column of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColVm {
    pub label: String,
    /// Share of the row's width (percent; the name column takes what is left).
    pub share: u16,
    /// Right-aligned (sizes).
    pub right: bool,
    /// Left out when the pane is narrower than this.
    pub min_width: u16,
}

/// A row: a trashed item or a drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceRow {
    /// Index in the trash items / drives.
    pub index: usize,
    pub icon: IconId,
    pub name: String,
    pub kind: RowKind,
    /// The text of each of [`PlacesVm::columns`] after the name (empty for the bar).
    pub cells: Vec<String>,
    /// The drive's usage bar (per mille used, level), shown in the column whose cell is
    /// empty.
    pub usage: Option<(u16, UsageLevel)>,
    pub cursor: bool,
    pub selected: bool,
}

/// A page as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlacesVm {
    pub page: Page,
    pub title: String,
    pub summary: String,
    /// Left to right; when the line is short the leftmost ones are left out first.
    pub buttons: Vec<ButtonVm>,
    /// The columns after the name, and the name's label first.
    pub name_label: String,
    pub columns: Vec<ColVm>,
    /// The rows in view (from `top`).
    pub rows: Vec<PlaceRow>,
    pub top: usize,
    pub total: usize,
    /// Shown instead of rows: (title, body, a failure).
    pub empty: Option<(String, String, bool)>,
    /// The pane has the keyboard.
    pub focused: bool,
    pub icons: IconSet,
}

/// The model of `pane`'s page (`None`: it shows a listing).
pub fn vm(core: &AppCore, pane: &Pane, focused: bool) -> Option<PlacesVm> {
    let page = page(pane)?;
    let icons = IconSet::from_config(&core.config.tui);
    let n = nav(pane.id, page);
    Some(match page {
        Page::Trash => trash_vm(&core.fileops.trash, &core.config, n, focused, icons),
        Page::Computer => computer_vm(&drives(core), n, focused, icons),
    })
}

/// The Trash page of `ts`.
pub fn trash_vm(ts: &TrashState, config: &Config, n: Nav, focused: bool, icons: IconSet) -> PlacesVm {
    let general = &config.general;
    let mut buttons = Vec::new();
    if !ts.loading {
        buttons.push(ButtonVm { id: button::REFRESH, label: tr!("views-trash-refresh").into(), danger: false });
    }
    if !ts.selected.is_empty() {
        buttons.push(ButtonVm { id: button::RESTORE, label: tr!("views-trash-restore").into(), danger: false });
        buttons.push(ButtonVm { id: button::PURGE, label: tr!("views-trash-delete-permanently").into(), danger: true });
    }
    if !ts.items.is_empty() {
        buttons.push(ButtonVm { id: button::EMPTY, label: tr!("views-trash-empty-trash").into(), danger: true });
    }
    let empty = if let Some(e) = &ts.error {
        Some((tr!("views-trash-unreadable").to_string(), safe(e), true))
    } else if ts.items.is_empty() && ts.loaded {
        Some((tr!("views-trash-empty-title").to_string(), tr!("views-trash-empty-subtitle").to_string(), false))
    } else if ts.items.is_empty() {
        Some((tr!("views-trash-reading").to_string(), String::new(), false))
    } else {
        None
    };
    let rows = if empty.is_some() { &[][..] } else { visible(&ts.items, n) };
    let rows = rows
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let index = n.top + i;
            let e = EntryInfo::minimal(trash::name_path(it), it.kind);
            let class = classify(&e);
            PlaceRow {
                index,
                icon: icons::class_icon(class),
                name: safe(&e.name),
                kind: RowKind::of(&e),
                cells: vec![
                    safe(&trash::origin(it)),
                    format_time(it.deleted_at, &general.date_format),
                    format_size(it.size, general.size_style),
                ],
                usage: None,
                cursor: index == n.cursor,
                selected: ts.is_selected(index),
            }
        })
        .collect();
    PlacesVm {
        page: Page::Trash,
        title: tr!("views-trash-title").into(),
        summary: ts.summary(general.size_style),
        buttons,
        name_label: soyuz_appcore::i18n::labels::column(soyuz_core::Column::Name).into(),
        columns: vec![
            ColVm { label: tr!("views-trash-col-original").into(), share: 38, right: false, min_width: 64 },
            ColVm { label: tr!("views-trash-col-deleted").into(), share: 18, right: false, min_width: 44 },
            ColVm {
                label: soyuz_appcore::i18n::labels::column(soyuz_core::Column::Size).into(),
                share: 14,
                right: true,
                min_width: 24,
            },
        ],
        rows,
        top: n.top,
        total: ts.items.len(),
        empty,
        focused,
        icons,
    }
}

/// The Computer page of `drives`.
pub fn computer_vm(drives: &[VolumeInfo], n: Nav, focused: bool, icons: IconSet) -> PlacesVm {
    let rows = visible(drives, n)
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let index = n.top + i;
            let free = if v.total > 0 {
                tr!(
                    "views-computer-free-of",
                    free = format_size(v.free, SizeStyle::Decimal),
                    total = format_size(v.total, SizeStyle::Decimal)
                )
            } else {
                String::new()
            };
            let fs = if v.fs.is_empty() { tr!("views-computer-drive").to_string() } else { safe(&v.fs) };
            PlaceRow {
                index,
                icon: volume_icon(v),
                name: safe(&volume_label(v)),
                kind: RowKind::Dir,
                cells: vec![String::new(), free, fs],
                usage: used_share(v).map(|u| ((u.clamp(0.0, 1.0) * 1000.0).round() as u16, usage_level(u))),
                cursor: index == n.cursor,
                selected: false,
            }
        })
        .collect();
    PlacesVm {
        page: Page::Computer,
        title: tr!("views-computer-title").into(),
        summary: String::new(),
        buttons: vec![ButtonVm { id: button::REFRESH, label: tr!("views-trash-refresh").into(), danger: false }],
        name_label: soyuz_appcore::i18n::labels::column(soyuz_core::Column::Name).into(),
        columns: vec![
            ColVm { label: tr!("tui-places-usage").into(), share: 22, right: false, min_width: 48 },
            ColVm { label: tr!("tui-places-free").into(), share: 28, right: false, min_width: 30 },
            ColVm { label: tr!("tui-places-filesystem").into(), share: 12, right: false, min_width: 64 },
        ],
        rows,
        top: n.top,
        total: drives.len(),
        empty: drives.is_empty().then(|| (tr!("views-computer-looking").to_string(), String::new(), false)),
        focused,
        icons,
    }
}

/// The icon of a drive (the GUI's glyphs, as far as the terminal sets have them).
fn volume_icon(v: &VolumeInfo) -> IconId {
    match v.kind {
        VolumeKind::Network => IconId::Globe,
        _ => IconId::HardDrive,
    }
}

/// The items of `all` in view.
fn visible<T>(all: &[T], n: Nav) -> &[T] {
    let start = n.top.min(all.len());
    let end = if n.rows == 0 { all.len() } else { (start + n.rows).min(all.len()) };
    &all[start..end]
}

// ---------------------------------------------------------------------------------------
// Drawing.

/// Draw page `m` of pane `pane` into `rect`.
pub fn draw(buf: &mut Buffer, rect: Rect, pane: PaneId, m: &PlacesVm, theme: &TuiTheme, hits: &mut Hits) {
    // Screen-reader mode (T8): no icons.
    let plain;
    let m = if theme.screen_reader() && m.icons != IconSet::Ascii {
        plain = PlacesVm { icons: IconSet::Ascii, ..m.clone() };
        &plain
    } else {
        m
    };
    hits.push(rect, HitTarget::PlacesArea { pane });
    if m.focused {
        hits.set_caret(Position { x: rect.x, y: rect.y });
    }
    buf.set_style(rect, theme.style(Role::Text));
    let g = geometry(rect);
    draw_header(buf, g.header, pane, m, theme, hits);
    if let Some((title, body, failed)) = &m.empty {
        let area = Rect { y: g.labels.y, height: g.labels.height + g.list.height, ..g.list };
        draw_empty(buf, area, title, body, *failed, theme);
        return;
    }
    let cols = place_columns(&m.columns, rect.width, prefix_w(m.icons));
    if !g.labels.is_empty() {
        let style = theme.style(Role::Muted);
        put(buf, g.labels.x + prefix_w(m.icons), g.labels.y, &m.name_label, cols.name_w, style);
        for (c, (x, w)) in m.columns.iter().zip(&cols.cells) {
            if *w > 0 {
                cell(buf, *x, g.labels.y, *w, &c.label, c.right, style);
            }
        }
    }
    let ascii = m.icons == IconSet::Ascii || theme.ascii_only();
    for (line, row) in m.rows.iter().enumerate() {
        let Ok(dy) = u16::try_from(line) else { break };
        if dy >= g.list.height {
            break;
        }
        let r = Rect { y: g.list.y + dy, height: 1, ..g.list };
        let mark = if row.selected { "* " } else { "  " };
        let mark_style = if row.selected { theme.style(Role::Selected) } else { theme.style(Role::Muted) };
        put(buf, r.x, r.y, mark, r.width, mark_style);
        let mut x = r.x + 2.min(r.width);
        let end = r.x + (prefix_w(m.icons) + cols.name_w).min(r.width);
        let name_style = theme.style(Role::Text).patch(theme.style(if row.kind == RowKind::Dir {
            Role::Folder
        } else {
            Role::File
        }));
        if m.icons != IconSet::Ascii {
            put(buf, x, r.y, icons::glyph(row.icon, m.icons), end.saturating_sub(x), name_style);
            x = (x + 3).min(end);
        }
        let room = usize::from(end.saturating_sub(x));
        let label = match m.page {
            Page::Trash => fitted(&row.name, row.kind, room),
            Page::Computer => text::truncate_end(&row.name, room),
        };
        put(buf, x, r.y, &label, end.saturating_sub(x), name_style);
        for (i, (cx, w)) in cols.cells.iter().enumerate() {
            if *w == 0 {
                continue;
            }
            let (Some(c), Some(t)) = (m.columns.get(i), row.cells.get(i)) else { continue };
            match row.usage.filter(|_| t.is_empty()) {
                Some((used, level)) => usage_bar(buf, *cx, r.y, *w, used, level, ascii, theme),
                None => cell(buf, *cx, r.y, *w, t, c.right, theme.style(Role::Muted)),
            }
        }
        let mut style = Style::new();
        if row.selected {
            style = style.patch(theme.style(Role::Selected));
        }
        if row.cursor {
            style = style.patch(theme.style(if m.focused { Role::Cursor } else { Role::CursorInactive }));
        }
        buf.set_style(r, style);
        hits.push(r, HitTarget::PlacesRow { pane, index: row.index });
        if row.cursor && m.focused {
            hits.set_caret(Position { x: r.x, y: r.y });
        }
    }
}

/// Cells taken by the mark and the icon in front of a name.
fn prefix_w(icons: IconSet) -> u16 {
    if icons == IconSet::Ascii { 2 } else { 5 }
}

/// Where the columns after the name go in a row `width` wide.
#[derive(Debug, PartialEq, Eq)]
struct Placed {
    name_w: u16,
    /// (x offset from the row's left, width) per column; width 0: left out.
    cells: Vec<(u16, u16)>,
}

fn place_columns(columns: &[ColVm], width: u16, prefix: u16) -> Placed {
    let widths: Vec<u16> = columns
        .iter()
        .map(|c| if width >= c.min_width { (u32::from(width) * u32::from(c.share) / 100) as u16 } else { 0 })
        .collect();
    let used: u16 = widths.iter().map(|w| if *w > 0 { w + 1 } else { 0 }).sum();
    let name_w = width.saturating_sub(prefix + used);
    let mut x = prefix + name_w;
    let mut cells = Vec::new();
    for w in widths {
        if w == 0 {
            cells.push((0, 0));
        } else {
            cells.push((x + 1, w));
            x += w + 1;
        }
    }
    Placed { name_w, cells }
}

/// `s` in `w` cells at `(x, y)`, cut with "…", right-aligned when `right`.
fn cell(buf: &mut Buffer, x: u16, y: u16, w: u16, s: &str, right: bool, style: Style) {
    let t = text::truncate_end(s, usize::from(w));
    let tw = text::width(&t) as u16;
    let x = if right { x + w.saturating_sub(tw) } else { x };
    put(buf, x, y, &t, w, style);
}

/// A usage bar `w` cells wide (one blank after it), `used` per mille filled.
#[allow(clippy::too_many_arguments)]
fn usage_bar(buf: &mut Buffer, x: u16, y: u16, w: u16, used: u16, level: UsageLevel, ascii: bool, theme: &TuiTheme) {
    let w = w.saturating_sub(1);
    if w == 0 {
        return;
    }
    let fill = ((u32::from(w) * u32::from(used.min(1000)) + 500) / 1000) as u16;
    // Something used shows at least one cell (the GUI's minimum width).
    let fill = if used > 0 { fill.max(1) } else { fill };
    let (full, rest) = if ascii { ("#", ".") } else { ("█", "░") };
    let role = match level {
        UsageLevel::Normal => Role::Progress,
        UsageLevel::Warning => Role::Warning,
        UsageLevel::Danger => Role::Danger,
    };
    put(buf, x, y, &full.repeat(usize::from(fill)), fill, theme.style(role));
    put(buf, x + fill, y, &rest.repeat(usize::from(w - fill)), w - fill, theme.style(Role::ProgressTrack));
}

/// The header line: the bold title, the summary, the buttons at the right edge.
fn draw_header(buf: &mut Buffer, r: Rect, pane: PaneId, m: &PlacesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let title_w = text::width(&m.title) as u16;
    // Buttons from the right; the ones that would squeeze the title out are left out.
    let mut shown: Vec<(&ButtonVm, u16)> = Vec::new();
    let mut used = 0u16;
    for b in m.buttons.iter().rev() {
        let w = text::width(&b.label) as u16 + 3;
        if used + w + title_w.min(8) + 1 > r.width {
            break;
        }
        shown.push((b, w));
        used += w;
    }
    // The last button ends at the right edge (its trailing blank is cut).
    let used = used.saturating_sub(u16::from(!shown.is_empty())).min(r.width);
    let mut x = r.right() - used;
    for (b, w) in shown.iter().rev() {
        let style = if b.danger { theme.style(Role::Danger) } else { theme.style(Role::Button) };
        put(buf, x, r.y, &format!("[{}]", b.label), *w - 1, style);
        hits.push(Rect::new(x, r.y, *w - 1, 1), HitTarget::PlacesButton { pane, button: b.id });
        x += w;
    }
    let end = r.right() - used;
    let room = usize::from(end.saturating_sub(r.x + 1));
    let title = text::truncate_end(&m.title, room);
    put(buf, r.x, r.y, &title, end - r.x, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    let sx = r.x + text::width(&title) as u16 + 2;
    if sx < end && !m.summary.is_empty() {
        let s = text::truncate_end(&m.summary, usize::from(end - sx - 1));
        put(buf, sx, r.y, &s, end - sx, theme.style(Role::Muted));
    }
}

/// The empty state: the title in the middle of `r`, the body wrapped below it.
fn draw_empty(buf: &mut Buffer, r: Rect, title: &str, body: &str, failed: bool, theme: &TuiTheme) {
    if r.is_empty() {
        return;
    }
    let w = usize::from(r.width);
    let mut lines = vec![(text::truncate_end(title, w), if failed { Role::Danger } else { Role::Text })];
    if !body.is_empty() {
        lines.extend(text::wrap(body, w.saturating_sub(2).max(1)).into_iter().map(|l| (l, Role::Muted)));
    }
    lines.truncate(usize::from(r.height));
    let top = r.y + (r.height - lines.len() as u16) / 3;
    for (i, (l, role)) in lines.iter().enumerate() {
        let lw = text::width(l).min(w) as u16;
        put(buf, r.x + (r.width - lw) / 2, top + i as u16, l, r.width, theme.style(*role));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::SystemTime;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_core::EntryKind;
    use soyuz_fs::TrashItem;

    use super::*;
    use crate::theme::ColorMode;

    fn item(name: &str, size: u64) -> TrashItem {
        TrashItem {
            id: format!("id-{name}"),
            original: PathBuf::from(format!("/home/me/{name}")),
            deleted_at: SystemTime::UNIX_EPOCH,
            size,
            kind: if name.ends_with('d') { EntryKind::Dir } else { EntryKind::File },
            trash_root: "/bin".into(),
        }
    }

    fn trash(n: usize) -> TrashState {
        let mut ts = TrashState::new(soyuz_appcore::host::no_wake());
        ts.items = (0..n).map(|i| item(&format!("f{i:02}.txt"), 1000 * i as u64)).collect();
        ts.loaded = true;
        ts
    }

    fn vol(mount: &str, free: u64, total: u64) -> VolumeInfo {
        VolumeInfo {
            mount: mount.into(),
            label: format!("Disk {mount}"),
            fs: "ext4".into(),
            kind: VolumeKind::Fixed,
            total,
            free,
            read_only: false,
            device: soyuz_core::entry::DeviceId(0),
            supports: Default::default(),
        }
    }

    fn nav_rows(rows: usize) -> Nav {
        Nav { rows, ..Nav::default() }
    }

    fn draw_vm(m: &PlacesVm, w: u16, h: u16, theme: &TuiTheme) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| {
            let area = f.area();
            draw(f.buffer_mut(), area, PaneId(1), m, theme, &mut hits);
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, hits)
    }

    #[test]
    fn trash_rows_columns_and_buttons() {
        let mut ts = trash(3);
        ts.click(1, false, false);
        let m = trash_vm(&ts, &Config::default(), Nav { cursor: 1, rows: 10, ..Nav::default() }, true, IconSet::Ascii);
        assert_eq!(m.rows.len(), 3);
        assert!(m.rows[1].cursor && m.rows[1].selected && !m.rows[0].selected);
        let ids: Vec<u8> = m.buttons.iter().map(|b| b.id).collect();
        assert_eq!(ids, vec![button::REFRESH, button::RESTORE, button::PURGE, button::EMPTY]);
        let (lines, hits) = draw_vm(&m, 100, 8, &TuiTheme::plain());
        assert!(
            lines[0].starts_with("Trash") && lines[0].ends_with("[Restore] [Delete permanently] [Empty trash]"),
            "{lines:?}"
        );
        assert!(lines[0].contains("[Refresh]") && lines[0].contains("1 selected"), "{lines:?}");
        assert!(lines[1].contains("Original location") && lines[1].contains("Date deleted"), "{lines:?}");
        assert!(lines[3].starts_with("* f01.txt") && lines[3].contains("/home/me"), "{lines:?}");
        assert!(lines[3].ends_with(" B") && lines[3].contains("1970"), "{lines:?}");
        assert_eq!(hits.at(5, 3), Some(HitTarget::PlacesRow { pane: PaneId(1), index: 1 }));
        assert_eq!(hits.at(5, 7), Some(HitTarget::PlacesArea { pane: PaneId(1) }));
        assert_eq!(hits.at(99, 0), Some(HitTarget::PlacesButton { pane: PaneId(1), button: button::EMPTY }));
        assert_eq!(hits.at(54, 0), Some(HitTarget::PlacesButton { pane: PaneId(1), button: button::REFRESH }));
    }

    #[test]
    fn narrow_panes_drop_columns_and_buttons() {
        let ts = trash(2);
        let m = trash_vm(&ts, &Config::default(), nav_rows(5), true, IconSet::Ascii);
        let (lines, _) = draw_vm(&m, 30, 6, &TuiTheme::plain());
        assert!(!lines[1].contains("Original"), "{lines:?}");
        assert!(lines[2].contains("f00.txt"), "{lines:?}");
        assert_eq!(lines[0], "Trash  [Refresh] [Empty trash]", "{lines:?}");
        let (lines, _) = draw_vm(&m, 24, 6, &TuiTheme::plain());
        let l = &lines[0];
        assert!(l.starts_with("Trash") && l.ends_with(" [Empty trash]") && !l.contains("Refresh"), "{lines:?}");
    }

    #[test]
    fn empty_states() {
        let mut ts = trash(0);
        let m = trash_vm(&ts, &Config::default(), nav_rows(5), true, IconSet::Ascii);
        let (lines, _) = draw_vm(&m, 60, 8, &TuiTheme::plain());
        assert!(lines.iter().any(|l| l.contains("Trash is empty")), "{lines:?}");
        ts.loaded = false;
        ts.loading = true;
        let m = trash_vm(&ts, &Config::default(), nav_rows(5), true, IconSet::Ascii);
        assert_eq!(m.summary, tr!("views-trash-reading"));
        assert!(m.buttons.is_empty(), "no refresh while reading");
        ts.error = Some("denied\u{7}".into());
        let m = trash_vm(&ts, &Config::default(), nav_rows(5), true, IconSet::Ascii);
        assert!(m.empty.as_ref().is_some_and(|(_, b, failed)| *failed && !b.contains('\u{7}')));
        let m = computer_vm(&[], nav_rows(5), true, IconSet::Ascii);
        let (lines, _) = draw_vm(&m, 60, 6, &TuiTheme::plain());
        assert!(lines.iter().any(|l| l.contains("Looking for drives")), "{lines:?}");
    }

    #[test]
    fn computer_rows_have_usage_bars() {
        let vols = [vol("/", 500, 1000), vol("/mnt/full", 10, 1000), vol("/mnt/unknown", 0, 0)];
        let m = computer_vm(&vols, Nav { cursor: 2, rows: 10, ..Nav::default() }, true, IconSet::Ascii);
        assert_eq!(m.rows[0].usage, Some((500, UsageLevel::Normal)));
        assert_eq!(m.rows[1].usage, Some((990, UsageLevel::Danger)));
        assert_eq!(m.rows[2].usage, None);
        assert!(m.rows[2].cursor);
        let (lines, _) = draw_vm(&m, 100, 6, &TuiTheme::plain());
        assert!(lines[2].contains("Disk /") && lines[2].contains("#####") && lines[2].contains("....."), "{lines:?}");
        assert!(lines[2].contains("free of") && lines[2].contains("ext4"), "{lines:?}");
        assert!(lines.iter().all(|l| l.is_ascii()), "{lines:?}");
        assert!(lines[2].starts_with("  Disk /  "), "no folder marker: {lines:?}");
        let m = computer_vm(&vols, nav_rows(10), true, IconSet::Unicode);
        let (lines, _) = draw_vm(&m, 100, 6, &TuiTheme::plain().with_icons(IconSet::Unicode));
        assert!(lines[2].contains('█') && lines[2].contains('░'), "{lines:?}");
    }

    #[test]
    fn cursor_follows_and_clamps() {
        let mut n = Nav { cursor: 30, follow: true, ..Nav::default() };
        clamp(&mut n, 50, 10);
        assert_eq!((n.cursor, n.top, n.follow), (30, 21, false));
        n.cursor = 2;
        n.follow = true;
        clamp(&mut n, 50, 10);
        assert_eq!(n.top, 2);
        clamp(&mut n, 5, 10);
        assert_eq!((n.cursor, n.top), (2, 0));
        clamp(&mut n, 0, 0);
        assert_eq!((n.cursor, n.top), (0, 0));
        update(PaneId(99), Page::Trash, |n| n.cursor = 4);
        assert_eq!(nav(PaneId(99), Page::Trash).cursor, 4);
        assert_eq!(nav(PaneId(99), Page::Computer).cursor, 0, "another page starts over");
    }

    /// Every size from nothing up, ASCII and mono, both pages, with and without rows.
    #[test]
    fn tiny_sizes_ascii_and_mono() {
        let mut ts = trash(4);
        ts.click(0, false, false);
        let vols = [vol("/", 500, 1000)];
        let themes = [
            TuiTheme::plain(),
            TuiTheme::plain().with_icons(IconSet::Nerd),
            TuiTheme::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_DARK, ColorMode::Truecolor, IconSet::Unicode),
        ];
        for theme in &themes {
            let icons = theme.icons();
            let vms = [
                trash_vm(&ts, &Config::default(), nav_rows(3), true, icons),
                trash_vm(&trash(0), &Config::default(), nav_rows(3), false, icons),
                computer_vm(&vols, nav_rows(3), true, icons),
                computer_vm(&[], nav_rows(3), true, icons),
            ];
            for m in &vms {
                for w in 0..=60u16 {
                    for h in 0..=8u16 {
                        let (lines, _) = draw_vm(m, w, h, theme);
                        if icons == IconSet::Ascii {
                            assert!(lines.iter().all(|l| l.is_ascii() || l.contains('…')), "{w}x{h}: {lines:?}");
                        }
                    }
                }
            }
        }
    }
}
