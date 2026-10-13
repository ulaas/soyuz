//! The disk usage treemap screen's view (Phase T7, agent C): the core's
//! `models::treemap::TreemapModel` in cells, drawn over the whole area like the compare
//! screen.
//!
//! - The title row (the folder, `×` closes), the toolbar (`↑`, the breadcrumb, the totals,
//!   Rescan / Stop, the map / list switch on narrow screens) and the scan line (progress,
//!   failure, "scanned in").
//! - The map: [`cell_tiles`] squarifies each level for cells about twice as high as wide
//!   ([`ASPECT`]) and snaps the edges to whole cells (neighbours share edges: no gaps, no
//!   overlaps; tiles under one cell are left out). A folder tile of at least
//!   [`NEST_MIN_W`] × [`NEST_MIN_H`] cells shows its contents in a frame, [`MAX_NEST`]
//!   levels deep. Leaves fill with their legend bucket's colour (truecolor and 256
//!   colours) or a fill character per bucket (16 colours, mono; ASCII ones whenever the
//!   theme is ASCII only), the name (and the size) centred on them. The selection is
//!   reversed, loading tiles are dimmed.
//! - The side list: Contents / Largest files, rows with size and share (a share bar on a
//!   second line from [`BAR_MIN_COLS`] columns).
//! - The legend, the trash confirm bar and the key hints at the bottom.
//!
//! Sizes: map and list side by side from [`BOTH_MIN_COLS`] × [`BOTH_MIN_ROWS`]; one of
//! them (M switches) from [`ONE_MIN_COLS`] × [`ONE_MIN_ROWS`]; the list alone below; the
//! title alone under 4 rows. The hints go first (under [`HINTS_MIN_ROWS`]), then the
//! legend (under [`LEGEND_MIN_ROWS`]).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Clear;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::treemap::layout::{Tile, squarify};
use soyuz_appcore::models::treemap::scan::Tree;
use soyuz_appcore::models::treemap::{Bucket, SideTab, Status, percent, share};
use soyuz_appcore::ops::fileops::names::show_path;
use soyuz_appcore::tr;
use soyuz_core::config::TuiBorders;
use soyuz_core::format_size;

use super::chrome::{self, put, put_hit};
use crate::input::screens::treemap::State;
use crate::layout::{HitTarget, Hits};
use crate::theme::{ColorMode, Role, TuiTheme};
use crate::view::text;

/// Cells are about this many times as high as wide: the layout squarifies on that scale.
pub const ASPECT: f64 = 2.0;
/// A folder tile shows its contents in a frame from this size (cells).
pub const NEST_MIN_W: u16 = 12;
pub const NEST_MIN_H: u16 = 4;
/// Frames nest this many levels below the current one.
pub const MAX_NEST: u8 = 2;
/// Tiles laid out per level at most (the largest; the rest are under a cell anyway).
pub const MAX_TILES: usize = 400;
/// Map and list side by side from this size.
pub const BOTH_MIN_COLS: u16 = 70;
pub const BOTH_MIN_ROWS: u16 = 14;
/// One view at a time (map or list) from this size; the list alone below.
pub const ONE_MIN_COLS: u16 = 40;
pub const ONE_MIN_ROWS: u16 = 8;
/// The hints row shows from this height, the legend from this one.
pub const HINTS_MIN_ROWS: u16 = 16;
pub const LEGEND_MIN_ROWS: u16 = 12;
/// Side-list rows get a share bar on a second line from this width.
pub const BAR_MIN_COLS: u16 = 40;

/// `HitTarget::ScreenButton` ids of the treemap.
pub mod button {
    pub const UP: u8 = 0;
    /// Rescan, or Stop while scanning.
    pub const RESCAN: u8 = 1;
    /// Narrow screens: show the map / the list.
    pub const VIEW: u8 = 2;
}

/// `HitTarget::ScreenArea` id of the side list (the wheel scrolls it).
pub const AREA_SIDE: u8 = 0;

/// What the body shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// The title row only.
    #[default]
    Title,
    /// Map and side list.
    Both,
    Map,
    List,
}

/// The screen's layout (one-row parts are empty when they do not fit).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    pub view: View,
    pub header: Rect,
    pub toolbar: Rect,
    pub status: Rect,
    pub map: Rect,
    /// The side list's tabs row and its rows.
    pub tabs: Rect,
    pub side: Rect,
    pub legend: Rect,
    pub confirm: Rect,
    pub hints: Rect,
}

/// The layout of the screen in `area`. `list`: a narrow screen shows the list (M);
/// `confirm`: the trash confirm bar shows.
pub fn geometry(area: Rect, list: bool, confirm: bool) -> Geometry {
    let mut g = Geometry::default();
    if area.is_empty() {
        return g;
    }
    let inner = chrome::inner(area);
    let (w, h) = (area.width, area.height);
    let mut y = inner.y;
    let mut end = inner.bottom();
    g.header = chrome::take(inner, &mut y, end);
    if h < 4 {
        return g;
    }
    g.view = if w >= BOTH_MIN_COLS && h >= BOTH_MIN_ROWS {
        View::Both
    } else if w >= ONE_MIN_COLS && h >= ONE_MIN_ROWS && !list {
        View::Map
    } else {
        View::List
    };
    g.toolbar = chrome::take(inner, &mut y, end);
    if h >= 6 {
        g.status = chrome::take(inner, &mut y, end);
    }
    if h >= HINTS_MIN_ROWS {
        g.hints = chrome::take_bottom(inner, y, &mut end);
    }
    if h >= LEGEND_MIN_ROWS && g.view != View::List {
        g.legend = chrome::take_bottom(inner, y, &mut end);
    }
    if confirm {
        g.confirm = chrome::take_bottom(inner, y, &mut end);
    }
    let body = Rect::new(inner.x, y, inner.width, end.saturating_sub(y));
    let side = match g.view {
        View::Both => {
            let sw = (inner.width / 3).clamp(24, 40).min(inner.width.saturating_sub(10));
            g.map = Rect::new(body.x, body.y, body.width.saturating_sub(sw + 1), body.height);
            Rect::new(body.right().saturating_sub(sw), body.y, sw, body.height)
        }
        View::Map => {
            g.map = body;
            Rect::default()
        }
        View::List => body,
        View::Title => Rect::default(),
    };
    if !side.is_empty() {
        g.tabs = Rect::new(side.x, side.y, side.width, 1);
        g.side = Rect::new(side.x, side.y + 1, side.width, side.height - 1);
    }
    g
}

/// Rows per side-list item in a list `w` columns wide.
pub fn side_row_h(w: u16) -> u16 {
    if w >= BAR_MIN_COLS { 2 } else { 1 }
}

/// A tile in cells, relative to the map's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellTile {
    /// The tree node.
    pub id: usize,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    /// Levels below the current one (0: its children).
    pub depth: u8,
    /// A folder frame (its contents follow it).
    pub frame: bool,
}

impl CellTile {
    /// Doubled centre (integer) in cells.
    pub fn centre2(&self) -> (i32, i32) {
        (2 * i32::from(self.x) + i32::from(self.w), 2 * i32::from(self.y) + i32::from(self.h))
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Snap squarified `tiles` (laid out in `w` × `h × aspect`) to whole cells: each edge is
/// rounded, so neighbours share it. Tiles under one cell are `None`.
pub fn snap(tiles: &[Tile], aspect: f64) -> Vec<Option<(u16, u16, u16, u16)>> {
    let r = |v: f64| v.round().max(0.0) as u16;
    tiles
        .iter()
        .map(|t| {
            let (x0, x1) = (r(t.x), r(t.x + t.w));
            let (y0, y1) = (r(t.y / aspect), r((t.y + t.h) / aspect));
            (x1 > x0 && y1 > y0).then(|| (x0, y0, x1 - x0, y1 - y0))
        })
        .collect()
}

/// The tiles of the tree's level `current` in a `w` × `h` map: frames before their
/// contents (the deepest tile is last).
pub fn cell_tiles(tree: &Tree, current: usize, w: u16, h: u16, frames: bool) -> Vec<CellTile> {
    let mut out = Vec::new();
    level(tree, current, (0, 0, w, h), 0, frames, &mut out);
    out
}

fn level(tree: &Tree, node: usize, r: (u16, u16, u16, u16), depth: u8, frames: bool, out: &mut Vec<CellTile>) {
    let (rx, ry, rw, rh) = r;
    if rw == 0 || rh == 0 {
        return;
    }
    let kids: Vec<usize> =
        tree.nodes[node].children.iter().copied().filter(|&k| tree.nodes[k].bytes > 0).take(MAX_TILES).collect();
    if kids.is_empty() {
        return;
    }
    let weights: Vec<f64> = kids.iter().map(|&k| tree.nodes[k].bytes as f64).collect();
    let tiles = squarify(&weights, Tile::new(0.0, 0.0, f64::from(rw), f64::from(rh) * ASPECT));
    for (k, cell) in kids.iter().zip(snap(&tiles, ASPECT)) {
        let Some((x, y, w, h)) = cell else { continue };
        let n = &tree.nodes[*k];
        let nest =
            n.is_dir && !n.other && !n.children.is_empty() && depth < MAX_NEST && w >= NEST_MIN_W && h >= NEST_MIN_H;
        let t = CellTile { id: *k, x: rx + x, y: ry + y, w, h, depth, frame: nest };
        out.push(t);
        if nest {
            let inner = if frames { (t.x + 1, t.y + 1, w - 2, h - 2) } else { (t.x, t.y + 1, w, h - 1) };
            level(tree, *k, inner, depth + 1, frames, out);
        }
    }
}

/// One tile as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileVm {
    pub id: u32,
    /// Relative to the map.
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub frame: bool,
    pub bucket: Bucket,
    pub label: String,
    pub size: String,
    pub selected: bool,
    pub loading: bool,
}

/// One row of the side list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideRowVm {
    pub id: u32,
    /// Index in the list (the click target).
    pub index: usize,
    pub name: String,
    pub size: String,
    pub pct: String,
    /// The share of the level, in thousandths.
    pub permille: u16,
    pub bucket: Bucket,
    pub dir: bool,
    pub cursor: bool,
}

/// The scan line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StatusVm {
    #[default]
    None,
    /// A scan: its line and progress.
    Scanning(String, u64, u64),
    Failed(String),
    Done(String),
}

/// What the treemap screen draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreemapVm {
    pub title: String,
    /// The root folder.
    pub sub: String,
    /// The breadcrumb: node and name.
    pub crumbs: Vec<(u32, String)>,
    pub can_up: bool,
    pub totals: String,
    pub scanning: bool,
    pub status: StatusVm,
    /// Narrow screens show the list (M).
    pub list: bool,
    pub tiles: Vec<TileVm>,
    /// The map's note while the level has no tiles.
    pub empty: Option<String>,
    pub tab: SideTab,
    pub tab_labels: [String; 2],
    pub rows: Vec<SideRowVm>,
    pub side_empty: Option<String>,
    /// The side list has the keyboard.
    pub list_focused: bool,
    pub legend: Vec<(Bucket, String)>,
    /// The trash question.
    pub confirm: Option<String>,
    pub labels: Labels,
    pub hints: String,
}

/// Button labels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Labels {
    pub rescan: String,
    pub stop: String,
    pub map: String,
    pub list: String,
    pub trash: String,
    pub cancel: String,
}

/// A bucket's colour role (the legend; the GUI's `class_color` groups).
pub fn bucket_role(b: Bucket) -> Role {
    match b {
        Bucket::Folder => Role::Folder,
        Bucket::Image => Role::Image,
        Bucket::Video => Role::Video,
        Bucket::Audio => Role::Audio,
        Bucket::Archive => Role::Archive,
        Bucket::Code => Role::Code,
        Bucket::Docs => Role::Doc,
        Bucket::Exe => Role::Exe,
        Bucket::Other => Role::File,
    }
}

/// A bucket's fill character in the character fills (`ascii`: ASCII only).
pub fn fill_char(b: Bucket, ascii: bool) -> &'static str {
    const UNICODE: [&str; 9] = ["█", "▓", "▒", "░", "▚", "▞", "▤", "▥", "▦"];
    const ASCII: [&str; 9] = ["#", "%", "@", "*", "+", "=", ":", "-", "."];
    let i = Bucket::LEGEND.iter().position(|x| *x == b).unwrap_or(8);
    if ascii { ASCII[i] } else { UNICODE[i] }
}

/// Tiles fill with colours (truecolor, 256 colours) or with characters (16 colours, mono).
pub fn colour_fill(theme: &TuiTheme) -> bool {
    matches!(theme.mode(), ColorMode::Truecolor | ColorMode::Ansi256)
}

/// The style and character a leaf of bucket `b` fills with.
fn fill(theme: &TuiTheme, b: Bucket) -> (Style, &'static str) {
    let role = theme.style(bucket_role(b));
    if colour_fill(theme) {
        let bg = role.fg.unwrap_or(Color::Reset);
        let fg = theme.base().bg.unwrap_or(Color::Reset);
        (Style::new().bg(bg).fg(fg), " ")
    } else {
        (role, fill_char(b, theme.ascii_only()))
    }
}

/// Step 1 of a frame (`view::screens::prepare`): take the scan's results, lay out the map
/// for `area` (cached) and build the view model into `st.vm`.
pub fn prepare(core: &AppCore, st: &mut State, area: Rect) {
    st.model.poll();
    st.geo = geometry(area, st.list, st.model.confirm_delete.is_some());
    if st.level != st.model.current {
        st.level = st.model.current;
        st.side = crate::widgets::SelectList::default();
    }
    let items = st.model.side_items();
    let height = st.side_height();
    st.side.clamp(items.len(), height);
    let frames = core.config.tui.borders != TuiBorders::None;
    let m = st.geo.map;
    let key = (st.model.current, st.model.tree.generation, m.width, m.height, frames);
    if st.cache != Some(key) {
        st.tiles = cell_tiles(&st.model.tree, st.model.current, m.width, m.height, frames);
        st.cache = Some(key);
    }
    st.vm = build(core, st, &items);
}

fn build(core: &AppCore, st: &State, items: &[usize]) -> TreemapVm {
    let model = &st.model;
    let tree = &model.tree;
    let style = core.config.general.size_style;
    let status = match model.status(style) {
        Status::Idle => StatusVm::None,
        Status::Scanning { done, total, text } => StatusVm::Scanning(text, done.min(total) as u64, total as u64),
        Status::Failed(t) => StatusVm::Failed(t),
        Status::Done(t) => StatusVm::Done(t),
    };
    let tiles = st
        .tiles
        .iter()
        .map(|t| {
            let n = &tree.nodes[t.id];
            TileVm {
                id: t.id as u32,
                x: t.x,
                y: t.y,
                w: t.w,
                h: t.h,
                frame: t.frame,
                bucket: model.bucket(t.id),
                label: n.label().to_string(),
                size: format_size(n.bytes, style),
                selected: model.selected == Some(t.id),
                loading: n.loading,
            }
        })
        .collect();
    let level = tree.nodes[model.current].bytes;
    let height = st.side_height();
    let rows = st
        .side
        .visible(items.len(), height)
        .map(|i| {
            let id = items[i];
            let n = &tree.nodes[id];
            let pct = share(n.bytes, level);
            SideRowVm {
                id: id as u32,
                index: i,
                name: n.label().to_string(),
                size: format_size(n.bytes, style),
                pct: percent(pct),
                permille: (pct * 10.0).round().clamp(0.0, 1000.0) as u16,
                bucket: model.bucket(id),
                dir: n.is_dir && !n.other,
                cursor: i == st.side.cursor,
            }
        })
        .collect();
    let list_focused = st.list_active();
    let confirm = model.confirm_text(style);
    let mut hints = if confirm.is_some() {
        tr!("tui-treemap-hints-confirm").to_string()
    } else if list_focused {
        tr!("tui-treemap-hints-list", del = crate::input::screens::delete_label(core))
    } else {
        tr!("tui-treemap-hints-map", del = crate::input::screens::delete_label(core))
    };
    if confirm.is_none() && st.geo.view != View::Both {
        hints = format!("{} · {hints}", tr!("tui-treemap-hint-switch"));
    }
    TreemapVm {
        title: tr!("tools-treemap-title").into(),
        sub: show_path(model.root_path()),
        crumbs: model.crumbs().into_iter().map(|id| (id as u32, tree.nodes[id].label().to_string())).collect(),
        can_up: tree.nodes[model.current].parent.is_some(),
        totals: model.totals(style),
        scanning: model.is_scanning(),
        status,
        list: st.list,
        tiles,
        empty: model.map_empty().map(str::to_string),
        tab: model.side,
        tab_labels: [tr!("tools-treemap-tab-contents").into(), tr!("tools-treemap-tab-largest").into()],
        rows,
        side_empty: items.is_empty().then(|| model.side_empty().to_string()),
        list_focused,
        legend: Bucket::LEGEND.iter().map(|b| (*b, b.label().to_string())).collect(),
        confirm,
        labels: Labels {
            rescan: tr!("tools-rescan").into(),
            stop: tr!("tools-stop").into(),
            map: tr!("tui-treemap-map").into(),
            list: tr!("tui-treemap-list").into(),
            trash: tr!("tools-move-to-trash").into(),
            cancel: tr!("common-cancel").into(),
        },
        hints,
    }
}

/// Draw the screen over `area`.
pub fn draw(f: &mut Frame<'_>, area: Rect, m: &TreemapVm, theme: &TuiTheme, hits: &mut Hits) {
    hits.push(area, HitTarget::Overlay);
    f.render_widget(Clear, area);
    f.buffer_mut().set_style(area, theme.base().patch(theme.style(Role::Text)));
    let g = geometry(area, m.list, m.confirm.is_some());
    let buf = f.buffer_mut();
    chrome::header(buf, g.header, &m.title, &m.sub, theme, hits);
    toolbar(buf, g.toolbar, &g, m, theme, hits);
    status(buf, g.status, m, theme);
    map(buf, g.map, m, theme, hits);
    side(buf, &g, m, theme, hits);
    legend(buf, g.legend, m, theme);
    if let Some(q) = &m.confirm {
        chrome::confirm_bar(buf, g.confirm, q, &m.labels.trash, &m.labels.cancel, theme, hits);
    }
    if !g.hints.is_empty() {
        put(buf, g.hints.x, g.hints.y, g.hints.right(), &m.hints, theme.style(Role::KeyHint));
    }
}

fn toolbar(buf: &mut Buffer, r: Rect, g: &Geometry, m: &TreemapVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    // Right: the totals and the buttons, when they fit.
    let action = if m.scanning { format!("Esc {}", m.labels.stop) } else { format!("F5 {}", m.labels.rescan) };
    let switch = match g.view {
        View::Map => Some(format!("M {}", m.labels.list)),
        View::List if r.width >= ONE_MIN_COLS => Some(format!("M {}", m.labels.map)),
        _ => None,
    };
    let mut right_w = chrome::button_w(&action) + switch.as_deref().map_or(0, chrome::button_w);
    let totals_w = text::width(&m.totals) as u16 + 1;
    if r.width < right_w + 12 {
        right_w = 0;
    }
    let show_totals = right_w > 0 && r.width >= right_w + totals_w + 20;
    let crumbs_end = r.right().saturating_sub(right_w + if show_totals { totals_w } else { 0 });
    // Left: ↑ and the breadcrumb (its start cut with "…" when long).
    let up_role = if m.can_up { Role::Button } else { Role::Disabled };
    let mut x =
        chrome::button(buf, r.x, r.y, crumbs_end, "↑", up_role, theme, hits, HitTarget::ScreenButton(button::UP));
    let sep = " › ";
    let total: usize = m.crumbs.iter().map(|(_, n)| text::width(n)).sum::<usize>() + 3 * m.crumbs.len();
    let room = usize::from(crumbs_end.saturating_sub(x));
    let skip = if total > room {
        // Keep the last crumbs that fit, behind "… ›".
        let mut w = 4;
        let mut keep = 0;
        for (_, n) in m.crumbs.iter().rev() {
            if w + text::width(n) + 3 > room && keep > 0 {
                break;
            }
            w += text::width(n) + 3;
            keep += 1;
        }
        m.crumbs.len() - keep
    } else {
        0
    };
    if skip > 0 {
        x = put(buf, x, r.y, crumbs_end, "…", theme.style(Role::Faint));
    }
    for (i, (id, name)) in m.crumbs.iter().enumerate().skip(skip) {
        if i > 0 {
            x = put(buf, x, r.y, crumbs_end, sep, theme.style(Role::Faint));
        }
        let last = i + 1 == m.crumbs.len();
        let style = if last { theme.style(Role::Title).add_modifier(Modifier::BOLD) } else { theme.style(Role::Muted) };
        x = put_hit(buf, x, r.y, crumbs_end, name, style, hits, HitTarget::TreemapCrumb(*id));
    }
    if right_w == 0 {
        return;
    }
    let mut bx = r.right() - right_w;
    if show_totals {
        put(buf, bx - totals_w, r.y, bx, &m.totals, theme.style(Role::Muted));
    }
    let role = if m.scanning { Role::Warning } else { Role::Button };
    bx = chrome::button(buf, bx, r.y, r.right(), &action, role, theme, hits, HitTarget::ScreenButton(button::RESCAN));
    if let Some(s) = switch {
        chrome::button(buf, bx, r.y, r.right(), &s, Role::Button, theme, hits, HitTarget::ScreenButton(button::VIEW));
    }
}

fn status(buf: &mut Buffer, r: Rect, m: &TreemapVm, theme: &TuiTheme) {
    match &m.status {
        StatusVm::None => {}
        StatusVm::Scanning(t, done, total) => chrome::progress(buf, r, *done, *total, t, theme),
        StatusVm::Failed(t) => {
            put(buf, r.x, r.y, r.right(), t, theme.style(Role::Danger));
        }
        StatusVm::Done(t) => {
            put(buf, r.x, r.y, r.right(), t, theme.style(Role::Faint));
        }
    }
}

fn map(buf: &mut Buffer, r: Rect, m: &TreemapVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    if let Some(e) = &m.empty {
        chrome::note(buf, r, e, "", Role::Muted, theme);
        return;
    }
    let lines = crate::borders::border_set(theme.borders());
    for t in &m.tiles {
        // Clip to the map (a stale layout never draws outside it).
        let tr = Rect::new(r.x.saturating_add(t.x), r.y.saturating_add(t.y), t.w, t.h).intersection(r);
        if tr.is_empty() {
            continue;
        }
        hits.push(tr, HitTarget::TreemapTile(t.id));
        if t.frame {
            frame(buf, tr, t, lines.as_ref(), theme);
        } else {
            leaf(buf, tr, t, theme);
        }
    }
}

/// A folder frame: a box (or just its header row without lines), the name and size on
/// the top edge.
fn frame(buf: &mut Buffer, r: Rect, t: &TileVm, lines: Option<&ratatui::symbols::border::Set<'_>>, theme: &TuiTheme) {
    let role = if t.selected { Role::BorderFocused } else { Role::Border };
    let mut style = theme.style(role);
    if t.selected {
        style = style.add_modifier(Modifier::BOLD);
    }
    if let Some(set) = lines
        && r.width >= 2
        && r.height >= 2
    {
        let (x1, y1) = (r.right() - 1, r.bottom() - 1);
        for x in r.x + 1..x1 {
            buf[(x, r.y)].set_symbol(set.horizontal_top).set_style(style);
            buf[(x, y1)].set_symbol(set.horizontal_bottom).set_style(style);
        }
        for y in r.y + 1..y1 {
            buf[(r.x, y)].set_symbol(set.vertical_left).set_style(style);
            buf[(x1, y)].set_symbol(set.vertical_right).set_style(style);
        }
        buf[(r.x, r.y)].set_symbol(set.top_left).set_style(style);
        buf[(x1, r.y)].set_symbol(set.top_right).set_style(style);
        buf[(r.x, y1)].set_symbol(set.bottom_left).set_style(style);
        buf[(x1, y1)].set_symbol(set.bottom_right).set_style(style);
    }
    let mut head = theme.style(Role::Folder).add_modifier(Modifier::BOLD);
    if t.selected {
        head = head.add_modifier(Modifier::REVERSED);
    }
    if t.loading {
        head = head.add_modifier(Modifier::DIM);
    }
    let label = format!(" {} {} ", t.label, t.size);
    let x0 = if lines.is_some() { r.x + 1 } else { r.x };
    put(buf, x0, r.y, r.right().saturating_sub(u16::from(lines.is_some())), &label, head);
}

/// A leaf: filled, its name (and size) centred.
fn leaf(buf: &mut Buffer, r: Rect, t: &TileVm, theme: &TuiTheme) {
    let (mut style, ch) = fill(theme, t.bucket);
    if t.loading {
        style = style.add_modifier(Modifier::DIM);
    }
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            buf[(x, y)].set_symbol(ch).set_style(style);
        }
    }
    // The label: on the fill colour, or plain text over the fill characters.
    let label_style = if colour_fill(theme) { style.add_modifier(Modifier::BOLD) } else { theme.style(Role::Text) };
    if r.width >= 3 {
        let mut rows: Vec<&str> = vec![&t.label];
        if r.height >= 3 && !t.size.is_empty() {
            rows.push(&t.size);
        }
        let top = r.y + r.height.saturating_sub(rows.len() as u16) / 2;
        for (s, y) in rows.iter().zip(top..r.bottom()) {
            let s = text::truncate_end(s, usize::from(r.width));
            let w = text::width(&s) as u16;
            put(buf, r.x + (r.width - w) / 2, y, r.right(), &s, label_style);
        }
    }
    if t.selected {
        buf.set_style(r, Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));
    }
}

fn side(buf: &mut Buffer, g: &Geometry, m: &TreemapVm, theme: &TuiTheme, hits: &mut Hits) {
    let r = g.tabs;
    if !r.is_empty() {
        let mut x = r.x;
        for (i, label) in m.tab_labels.iter().enumerate() {
            let on = (i == 0) == (m.tab == SideTab::Contents);
            let key = if i == 0 { "C" } else { "L" };
            let style = if on {
                theme.style(if m.list_focused { Role::TabActive } else { Role::Title })
            } else {
                theme.style(Role::Muted)
            };
            x = put_hit(
                buf,
                x,
                r.y,
                r.right(),
                &format!(" {key} {label} "),
                style,
                hits,
                HitTarget::ScreenTab(i as u8),
            );
            x = x.saturating_add(1).min(r.right());
        }
    }
    let r = g.side;
    if r.is_empty() {
        return;
    }
    hits.push(r, HitTarget::ScreenArea(AREA_SIDE));
    if let Some(e) = &m.side_empty {
        chrome::note(buf, r, e, "", Role::Muted, theme);
        return;
    }
    let rh = side_row_h(r.width);
    let mut y = r.y;
    for row in &m.rows {
        if y + rh > r.bottom() {
            break;
        }
        let line = Rect::new(r.x, y, r.width, rh);
        hits.push(line, HitTarget::TreemapSide(row.index));
        side_row(buf, line, row, m.list_focused, theme);
        y += rh;
    }
}

fn side_row(buf: &mut Buffer, r: Rect, row: &SideRowVm, focused: bool, theme: &TuiTheme) {
    let base = if row.cursor {
        theme.style(if focused { Role::Cursor } else { Role::CursorInactive })
    } else {
        Style::default()
    };
    buf.set_style(r, base);
    let st = |role: Role| theme.style(role).patch(base);
    let right = if r.height >= 2 { row.size.clone() } else { format!("{} {}", row.size, row.pct) };
    let rw = (text::width(&right) as u16).min(r.width / 2);
    let name_end = r.right().saturating_sub(rw + 1);
    let name = if row.dir { format!("{}/", row.name) } else { row.name.clone() };
    put(buf, r.x, r.y, name_end, &name, st(if row.dir { Role::Folder } else { Role::Text }));
    put(buf, r.right() - rw, r.y, r.right(), &right, st(Role::Muted));
    if r.height >= 2 {
        // The share bar and the percentage.
        let pw = text::width(&row.pct) as u16 + 1;
        let bw = r.width.saturating_sub(pw + 2);
        let fill = ((u32::from(bw) * u32::from(row.permille)).div_ceil(1000) as u16).min(bw);
        let (full, empty) = if theme.ascii_only() { ("#", ".") } else { ("▆", "─") };
        let y = r.y + 1;
        put(buf, r.x + 2, y, r.x + 2 + fill, &full.repeat(usize::from(fill)), st(bucket_role(row.bucket)));
        put(buf, r.x + 2 + fill, y, r.x + 2 + bw, &empty.repeat(usize::from(bw - fill)), st(Role::Faint));
        put(buf, r.right().saturating_sub(pw - 1), y, r.right(), &row.pct, st(Role::Muted));
    }
}

fn legend(buf: &mut Buffer, r: Rect, m: &TreemapVm, theme: &TuiTheme) {
    if r.is_empty() {
        return;
    }
    let mut x = r.x;
    for (b, label) in &m.legend {
        let (style, ch) = fill(theme, *b);
        let sw = text::width(label) as u16 + 3;
        if x + sw > r.right() {
            break;
        }
        x = put(buf, x, r.y, r.right(), &ch.repeat(2), style);
        x = put(buf, x + 1, r.y, r.right(), label, theme.style(Role::Muted));
        x = x.saturating_add(2).min(r.right());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_appcore::models::treemap::scan::SubTree;

    use super::*;
    use crate::icons::IconSet;
    use soyuz_core::config::TuiBorders;

    pub(crate) fn vm() -> TreemapVm {
        let tiles = vec![
            TileVm {
                id: 1,
                x: 0,
                y: 0,
                w: 30,
                h: 10,
                frame: true,
                bucket: Bucket::Folder,
                label: "deps".into(),
                size: "5 GiB".into(),
                selected: false,
                loading: false,
            },
            TileVm {
                id: 2,
                x: 1,
                y: 1,
                w: 28,
                h: 8,
                frame: false,
                bucket: Bucket::Code,
                label: "lib.rs".into(),
                size: "4 GiB".into(),
                selected: true,
                loading: false,
            },
            TileVm {
                id: 3,
                x: 30,
                y: 0,
                w: 10,
                h: 10,
                frame: false,
                bucket: Bucket::Image,
                label: "日本語.png".into(),
                size: "1 GiB".into(),
                selected: false,
                loading: true,
            },
        ];
        let rows = (0..20)
            .map(|i| SideRowVm {
                id: i as u32 + 1,
                index: i,
                name: format!("item{i}"),
                size: "1 GiB".into(),
                pct: "12.5%".into(),
                permille: 125,
                bucket: Bucket::LEGEND[i % 9],
                dir: i % 2 == 0,
                cursor: i == 1,
            })
            .collect();
        TreemapVm {
            title: "Disk usage".into(),
            sub: "/home/me".into(),
            crumbs: vec![(0, "me".into()), (1, "Projects".into()), (2, "soyuz".into())],
            can_up: true,
            totals: "12 GiB · 48 files".into(),
            scanning: true,
            status: StatusVm::Scanning("Scanning 3 of 9".into(), 3, 9),
            tiles,
            tab_labels: ["Contents".into(), "Largest".into()],
            rows,
            legend: Bucket::LEGEND.iter().map(|b| (*b, format!("{b:?}"))).collect(),
            confirm: Some("Move “a” (1 GiB) to the trash?".into()),
            labels: Labels {
                rescan: "Rescan".into(),
                stop: "Stop".into(),
                map: "Map".into(),
                list: "List".into(),
                trash: "Move to trash".into(),
                cancel: "Cancel".into(),
            },
            hints: "hints".into(),
            ..TreemapVm::default()
        }
    }

    fn draw_with(m: &TreemapVm, w: u16, h: u16, theme: &TuiTheme) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| {
            super::draw(f, f.area(), m, theme, &mut hits);
            if theme.ascii_only() {
                crate::borders::asciify(f.buffer_mut());
            }
        })
        .unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        (lines, hits)
    }

    fn colour() -> TuiTheme {
        TuiTheme::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_DARK, ColorMode::Truecolor, IconSet::Unicode)
    }

    fn ascii() -> TuiTheme {
        TuiTheme::plain().with_icons(IconSet::Ascii).with_borders(TuiBorders::Ascii)
    }

    #[test]
    fn layout_rows_targets_and_tiles() {
        let m = vm();
        let (lines, hits) = draw_with(&m, 120, 30, &colour());
        let g = geometry(Rect::new(0, 0, 120, 30), false, true);
        assert_eq!(g.view, View::Both);
        assert!(lines[0].contains("Disk usage") && lines[0].contains("/home/me"), "{lines:?}");
        assert_eq!(hits.at(118, 0), Some(HitTarget::ScreenClose));
        assert!(lines[usize::from(g.toolbar.y)].contains("Projects"));
        // The deepest tile wins: the leaf inside the frame, the frame on its edge.
        assert_eq!(hits.at(g.map.x + 5, g.map.y + 4), Some(HitTarget::TreemapTile(2)));
        assert_eq!(hits.at(g.map.x, g.map.y), Some(HitTarget::TreemapTile(1)));
        assert_eq!(hits.at(g.side.x + 1, g.side.y), Some(HitTarget::TreemapSide(0)));
        assert_eq!(hits.at(g.tabs.x + 1, g.tabs.y), Some(HitTarget::ScreenTab(0)));
        assert!(lines[usize::from(g.confirm.y)].contains("to the trash?"));
        assert_eq!(hits.at(g.confirm.right() - 2, g.confirm.y), Some(HitTarget::ConfirmBar(chrome::confirm::CANCEL)));
        assert!(lines[usize::from(g.map.y)].contains("deps"), "frame header");
        assert!(lines[usize::from(g.legend.y)].contains("Folder"));
        // Clicks never fall through.
        assert_eq!(hits.at(60, 29), Some(HitTarget::Overlay));
    }

    #[test]
    fn breakpoints() {
        let r = |w, h| Rect::new(0, 0, w, h);
        assert_eq!(geometry(r(80, 3), false, false).view, View::Title);
        assert_eq!(geometry(r(39, 20), false, false).view, View::List);
        assert_eq!(geometry(r(60, 20), false, false).view, View::Map);
        assert_eq!(geometry(r(60, 20), true, false).view, View::List);
        assert_eq!(geometry(r(80, 13), false, false).view, View::Map);
        assert_eq!(geometry(r(80, 14), false, false).view, View::Both);
        let g = geometry(r(80, 15), false, false);
        assert!(g.hints.is_empty() && !g.legend.is_empty(), "the hints go first");
        let g = geometry(r(80, 11), false, false);
        assert!(g.legend.is_empty());
        for h in 0..40 {
            for (list, confirm) in [(false, false), (true, true)] {
                let g = geometry(r(90, h), list, confirm);
                let parts = [g.header, g.toolbar, g.status, g.map, g.tabs, g.side, g.legend, g.confirm, g.hints];
                for (i, a) in parts.iter().enumerate() {
                    assert!(a.is_empty() || a.bottom() <= h, "part {i} out of the area at {h}");
                    for b in &parts[i + 1..] {
                        assert!(a.is_empty() || b.is_empty() || !a.intersects(*b), "overlap at height {h}");
                    }
                }
            }
        }
    }

    #[test]
    fn fills_follow_the_colour_mode_and_ascii() {
        let mut m = vm();
        m.confirm = None;
        // Colours: the leaf is a background colour behind spaces.
        let (lines, _) = draw_with(&m, 120, 30, &colour());
        let g = geometry(Rect::new(0, 0, 120, 30), false, false);
        let row = &lines[usize::from(g.map.y + 2)];
        assert!(!row.contains('▓') && !row.contains('▥'), "{row}");
        // Mono: fill characters per bucket; the legend shows them too.
        let (lines, _) = draw_with(&m, 120, 30, &TuiTheme::plain());
        assert!(lines[usize::from(g.map.y + 1)].contains(fill_char(Bucket::Image, false)), "{lines:?}");
        assert!(lines[usize::from(g.legend.y)].contains(fill_char(Bucket::Code, false)));
        // ASCII only: every cell is ASCII (box lines, fills, punctuation).
        let (lines, _) = draw_with(&m, 120, 30, &ascii());
        for l in &lines {
            assert!(l.chars().all(|c| c.is_ascii() || "日本語".contains(c)), "{l}");
        }
        assert!(lines.iter().any(|l| l.contains(fill_char(Bucket::Image, true))));
        let all: Vec<&str> = Bucket::LEGEND.iter().map(|b| fill_char(*b, true)).collect();
        assert!(all.iter().all(|c| c.is_ascii()));
        let mut uniq = all.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 9, "one character per bucket");
    }

    #[test]
    fn every_size_ascii_and_mono() {
        let mut busy = vm();
        busy.status = StatusVm::Failed("Can't scan".into());
        let mut list = vm();
        list.list = true;
        list.side_empty = Some("Measuring…".into());
        let mut empty = vm();
        empty.tiles.clear();
        empty.empty = Some("This folder is empty".into());
        empty.confirm = None;
        let themes = [colour(), TuiTheme::plain(), ascii()];
        for m in [vm(), busy, list, empty] {
            for theme in &themes {
                for w in 0..=100 {
                    for h in 0..=30 {
                        draw_with(&m, w, h, theme);
                    }
                }
            }
        }
    }

    fn tree(weights: &[u64]) -> Tree {
        let mut t = Tree::new(std::path::Path::new("/r"));
        for (i, w) in weights.iter().enumerate() {
            t.add_child(0, SubTree::file(format!("/r/f{i}.bin").into(), *w));
        }
        t
    }

    #[test]
    fn cell_tiles_cover_without_overlap() {
        // A small deterministic generator: many uneven weights over many map sizes.
        let mut seed = 0x2545_f491_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let n = (next() % 40 + 1) as usize;
            let weights: Vec<u64> = (0..n).map(|_| next() % 10_000 + 1).collect();
            let (w, h) = ((next() % 120 + 1) as u16, (next() % 40 + 1) as u16);
            let t = tree(&weights);
            let tiles = cell_tiles(&t, 0, w, h, true);
            for (i, a) in tiles.iter().enumerate() {
                assert!(a.w > 0 && a.h > 0);
                assert!(a.x + a.w <= w && a.y + a.h <= h, "inside: {a:?} in {w}x{h}");
                for b in &tiles[i + 1..] {
                    let ra = Rect::new(a.x, a.y, a.w, a.h);
                    assert!(!ra.intersects(Rect::new(b.x, b.y, b.w, b.h)), "{a:?} overlaps {b:?}");
                }
            }
            // Snapped edges are shared: the cells covered add up to the map when no tile
            // was dropped.
            if tiles.len() == n {
                let cells: u32 = tiles.iter().map(|t| u32::from(t.w) * u32::from(t.h)).sum();
                assert_eq!(cells, u32::from(w) * u32::from(h), "no gaps");
            }
        }
    }

    #[test]
    fn aspect_makes_tiles_wider_than_high() {
        // One square-ish half: 40 x 20 cells holds two equal tiles side by side (each
        // 20 x 20 cells, i.e. 20 x 40 "pixels"), not stacked.
        let t = tree(&[50, 50]);
        let tiles = cell_tiles(&t, 0, 40, 10, true);
        assert_eq!(tiles.len(), 2);
        assert!(tiles.iter().all(|t| t.w >= 2 * t.h), "{tiles:?}");
    }

    #[test]
    fn folders_nest_from_the_threshold() {
        let mut t = Tree::new(std::path::Path::new("/r"));
        let dir = SubTree {
            name: "d".into(),
            path: "/r/d".into(),
            bytes: 300,
            files: 2,
            is_dir: true,
            other: false,
            complete: true,
            children: vec![SubTree::file("/r/d/x.png".into(), 200), SubTree::file("/r/d/y.rs".into(), 100)],
        };
        t.add_child(0, dir);
        let big = cell_tiles(&t, 0, NEST_MIN_W, NEST_MIN_H, true);
        assert!(big[0].frame && big.len() == 3, "{big:?}");
        assert!(big[1..].iter().all(|c| c.depth == 1 && c.x >= 1 && c.y >= 1));
        let small = cell_tiles(&t, 0, NEST_MIN_W - 1, NEST_MIN_H, true);
        assert!(!small[0].frame && small.len() == 1);
        // Without box lines the contents start under the header row.
        let flat = cell_tiles(&t, 0, NEST_MIN_W, NEST_MIN_H, false);
        assert!(flat[1..].iter().all(|c| c.y >= 1 && c.x + c.w <= NEST_MIN_W));
    }
}
