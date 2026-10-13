//! The duplicate finder screen's view (Phase T7, agent C): the core's
//! `models::dupes::DupesModel` drawn over the whole area like the compare screen.
//!
//! - The title row (the folders searched, `×` closes).
//! - The setup: the roots as chips (a click removes one), the add-folder field with its
//!   folder completions and "Add other pane", the minimum-size chips and Find / Stop.
//!   Under [`SETUP_MIN_ROWS`] rows the setup is one summary row until the keyboard goes
//!   into it (F2, Tab), then it opens over the top of the list.
//! - The scan line (progress, or the summary), "Select in every group" with the keep
//!   rules, then the list: a header per group (copies, size, reclaimable, the keep keys)
//!   and its files (mark, name, folder relative to its root, date, size; the date drops
//!   under [`DATE_MIN_COLS`] columns, the size under [`SIZE_MIN_COLS`]).
//! - The footer: what is marked and "Delete marked", or the trash confirm bar; the key
//!   hints under it from [`SETUP_MIN_ROWS`] rows. Under [`TINY_ROWS`] rows only the list
//!   and the footer show.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dupes::{Keep, SIZES, other_pane_dir, short_folder, size_label};
use soyuz_appcore::tr;
use soyuz_core::{SizeStyle, format_size, format_time};

use super::chrome::{self, put, put_hit};
use crate::input::screens::dupes::{Line, State, Zone, lines};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// The setup shows in full (and the hints row) from this height.
pub const SETUP_MIN_ROWS: u16 = 16;
/// Under this height only the list and the footer show.
pub const TINY_ROWS: u16 = 6;
/// "Select in every group" shows from this height.
pub const BULK_MIN_ROWS: u16 = 10;
/// File rows show the date from this width, the size from this one.
pub const DATE_MIN_COLS: u16 = 60;
pub const SIZE_MIN_COLS: u16 = 40;

/// `HitTarget::ScreenButton` ids of the duplicate finder.
pub mod button {
    /// Find, or Stop while searching.
    pub const FIND: u8 = 0;
    pub const ADD: u8 = 1;
    pub const OTHER_PANE: u8 = 2;
    /// "Select in every group": keep newest / oldest / shortest path.
    pub const KEEP: u8 = 3;
    pub const CLEAR: u8 = 6;
    /// Delete marked… (asks first).
    pub const TRASH: u8 = 7;
    /// The collapsed setup row: open the setup.
    pub const SETUP: u8 = 8;
}

/// `HitTarget::ScreenChip` ids: the minimum sizes, then the folder completions.
pub mod chip {
    pub const COMPLETION: u8 = 10;
}

/// `HitTarget::ScreenField` id of the add-folder field.
pub const FIELD_ADD: u8 = 0;
/// `HitTarget::ScreenArea` id of the list (the wheel scrolls it).
pub const AREA_LIST: u8 = 0;

/// The screen's layout (one-row parts are empty when they do not fit).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    pub header: Rect,
    pub roots: Rect,
    pub add: Rect,
    pub sizes: Rect,
    /// The collapsed setup (one row).
    pub summary: Rect,
    pub status: Rect,
    pub bulk: Rect,
    pub list: Rect,
    pub footer: Rect,
    pub hints: Rect,
}

/// The layout in `area`; `open`: the setup is open on a short screen; `groups`: there are
/// groups (the bulk row).
pub fn geometry(area: Rect, open: bool, groups: bool) -> Geometry {
    let mut g = Geometry::default();
    if area.is_empty() {
        return g;
    }
    let inner = chrome::inner(area);
    let h = area.height;
    let mut y = inner.y;
    let mut end = inner.bottom();
    if h < TINY_ROWS {
        g.footer = chrome::take_bottom(inner, y, &mut end);
        g.list = Rect::new(inner.x, y, inner.width, end - y);
        return g;
    }
    g.header = chrome::take(inner, &mut y, end);
    if h >= SETUP_MIN_ROWS {
        g.hints = chrome::take_bottom(inner, y, &mut end);
    }
    g.footer = chrome::take_bottom(inner, y, &mut end);
    if h >= SETUP_MIN_ROWS || open {
        g.roots = chrome::take(inner, &mut y, end);
        g.add = chrome::take(inner, &mut y, end);
        g.sizes = chrome::take(inner, &mut y, end);
    } else {
        g.summary = chrome::take(inner, &mut y, end);
    }
    g.status = chrome::take(inner, &mut y, end);
    if groups && h >= BULK_MIN_ROWS {
        g.bulk = chrome::take(inner, &mut y, end);
    }
    if end > y {
        g.list = Rect::new(inner.x, y, inner.width, end - y);
    }
    g
}

/// A one-line text field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldVm {
    pub shown: String,
    pub placeholder: String,
    /// The cursor's column while the field has the keyboard.
    pub cursor: Option<u16>,
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineKind {
    Header {
        group: usize,
        collapsed: bool,
        title: String,
        reclaim: String,
    },
    File {
        marked: bool,
        name: String,
        folder: String,
        date: String,
        size: String,
        /// "Keep at least one copy" flashes on it.
        refused: bool,
    },
}

/// A line in view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineVm {
    /// Index in the list (the click target).
    pub index: usize,
    pub kind: LineKind,
    pub cursor: bool,
}

/// The scan line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum StatusVm {
    #[default]
    None,
    Progress(String, u64, u64),
    Summary(String),
}

/// Fixed labels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Labels {
    pub search_in: String,
    pub add: String,
    pub other_pane: String,
    pub min_size: String,
    pub find: String,
    pub stop: String,
    pub every_group: String,
    pub keep: [String; 3],
    pub clear: String,
    pub delete_marked: String,
    pub trash: String,
    pub cancel: String,
    pub keep_one: String,
    pub keep_keys: String,
}

/// What the duplicate finder screen draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DupesVm {
    pub title: String,
    pub sub: String,
    pub roots: Vec<String>,
    /// The root chip under the keyboard (the roots row is focused).
    pub root_cursor: Option<usize>,
    pub add: FieldVm,
    pub completions: Vec<String>,
    pub pick: usize,
    /// "Add other pane" is offered.
    pub other_pane: bool,
    pub sizes: Vec<String>,
    pub min_idx: usize,
    pub sizes_focused: bool,
    pub running: bool,
    pub can_find: bool,
    /// The collapsed setup row.
    pub summary: String,
    /// The setup is open on a short screen.
    pub open: bool,
    pub status: StatusVm,
    pub groups: bool,
    pub lines: Vec<LineVm>,
    pub list_focused: bool,
    pub empty: Option<(String, String)>,
    pub footer: String,
    pub marked: usize,
    pub confirm: Option<String>,
    pub labels: Labels,
    pub hints: String,
}

/// Step 1 of a frame (`view::screens::prepare`): pump the model
/// (`crate::input::screens::dupes::pump`) and build the view model into `st.vm`.
pub fn prepare(core: &mut AppCore, st: &mut State, area: Rect) {
    st.geo = geometry(area, st.zone != Zone::List, !st.model.groups.is_empty());
    crate::input::screens::dupes::pump(core, st);
    st.vm = build(core, st);
}

fn labels() -> Labels {
    Labels {
        search_in: tr!("tools-dupes-search-in").into(),
        add: tr!("tools-add").into(),
        other_pane: tr!("tools-dupes-add-other-pane").into(),
        min_size: tr!("tools-dupes-min-size").into(),
        find: tr!("tools-menu-find-duplicates").into(),
        stop: tr!("tools-stop").into(),
        every_group: tr!("tools-dupes-select-every-group").into(),
        keep: Keep::ALL.map(|k| k.short_label().to_string()),
        clear: tr!("tools-clear").into(),
        delete_marked: tr!("tools-dupes-delete-marked").into(),
        trash: tr!("tools-move-to-trash").into(),
        cancel: tr!("common-cancel").into(),
        keep_one: tr!("tools-dupes-keep-one").into(),
        keep_keys: tr!("tui-dupes-keep-keys").into(),
    }
}

fn build(core: &AppCore, st: &mut State) -> DupesVm {
    let cfg = &core.config.general;
    let m = &st.model;
    let labels = labels();
    let other_pane = other_pane_dir(core).is_some_and(|d| !m.roots.contains(&d));
    let completions = st.names.clone();
    let fr = field_rect(st.geo.add, &labels, other_pane, !completions.is_empty());
    let (shown, col) = st.add_edit.view(&st.add_text, fr.width.max(1));
    let add = FieldVm {
        shown,
        placeholder: tr!("tools-dupes-add-path-hint").into(),
        cursor: (st.zone == Zone::Add).then_some(col),
    };
    let all = lines(m);
    let height = st.list_height();
    let vis = st.list.visible(all.len(), height);
    let rows = vis
        .map(|k| {
            let kind = match all[k] {
                Line::Header(g) => {
                    let grp = &m.groups[g];
                    LineKind::Header {
                        group: g,
                        collapsed: grp.collapsed,
                        title: tr!(
                            "tools-dupes-group-title",
                            count = grp.files.len(),
                            size = format_size(grp.size(), cfg.size_style)
                        ),
                        reclaim: tr!("tools-dupes-reclaimable", size = format_size(grp.wasted(), cfg.size_style)),
                    }
                }
                Line::File(g, i) => {
                    let e = &m.groups[g].files[i];
                    LineKind::File {
                        marked: m.groups[g].marked[i],
                        name: e.name.clone(),
                        folder: e.path.parent().map(|f| short_folder(&m.roots, f)).unwrap_or_default(),
                        date: e.modified.map(|t| format_time(t, &cfg.date_format)).unwrap_or_default(),
                        size: format_size(e.size, cfg.size_style),
                        refused: m.refused_now(g, i),
                    }
                }
            };
            LineVm { index: k, kind, cursor: k == st.list.cursor }
        })
        .collect();
    let (marked, bytes, _) = m.marked();
    let status = match (m.progress(), m.summary()) {
        (Some((done, total, text)), _) => StatusVm::Progress(text, done, total),
        (None, Some(s)) => StatusVm::Summary(s),
        (None, None) => StatusVm::None,
    };
    let folders = match m.roots.as_slice() {
        [] => tr!("tui-dupes-no-folders").to_string(),
        [one] => soyuz_core::entry::display_name(one),
        many => tr!("tools-dupes-folders", count = many.len()),
    };
    let del = crate::input::screens::delete_label(core);
    let hints = if m.confirm {
        tr!("tui-dupes-hints-confirm").to_string()
    } else {
        match st.zone {
            Zone::List => tr!("tui-dupes-hints-list", del = del),
            Zone::Add => tr!("tui-dupes-hints-add").to_string(),
            Zone::Roots | Zone::Size => tr!("tui-dupes-hints-setup").to_string(),
        }
    };
    DupesVm {
        title: tr!("tools-dupes-title").into(),
        sub: m.subtitle(),
        roots: m.roots.iter().map(|r| soyuz_core::entry::display_name(r)).collect(),
        root_cursor: (st.zone == Zone::Roots && !m.roots.is_empty()).then_some(st.root_cursor),
        add,
        completions,
        pick: st.pick,
        other_pane,
        sizes: (0..SIZES.len()).map(|i| size_label(i).to_string()).collect(),
        min_idx: m.min_idx,
        sizes_focused: st.zone == Zone::Size,
        running: m.is_running(),
        can_find: !m.roots.is_empty(),
        summary: tr!("tui-dupes-setup-line", folders = folders, min = size_label(m.min_idx)),
        open: st.zone != Zone::List,
        status,
        groups: !m.groups.is_empty(),
        lines: rows,
        list_focused: st.zone == Zone::List,
        empty: m.empty_state().map(|e| (e.title().to_string(), e.body().to_string())),
        footer: m.marked_text(),
        marked,
        confirm: m
            .confirm
            .then(|| tr!("tui-dupes-confirm", count = marked, size = format_size(bytes, SizeStyle::Binary))),
        labels,
        hints,
    }
}

/// Draw the screen over `area`.
pub fn draw(f: &mut Frame<'_>, area: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    hits.push(area, HitTarget::Overlay);
    f.render_widget(Clear, area);
    f.buffer_mut().set_style(area, theme.base().patch(theme.style(Role::Text)));
    let g = geometry(area, m.open, m.groups);
    let mut cursor = None;
    {
        let buf = f.buffer_mut();
        chrome::header(buf, g.header, &m.title, &m.sub, theme, hits);
        roots(buf, g.roots, m, theme, hits);
        cursor = add_row(buf, g.add, m, theme, hits).or(cursor);
        sizes(buf, g.sizes, m, theme, hits);
        summary(buf, g.summary, m, theme, hits);
        match &m.status {
            StatusVm::None => {}
            StatusVm::Progress(t, done, total) => chrome::progress(buf, g.status, *done, *total, t, theme),
            StatusVm::Summary(t) => {
                put(buf, g.status.x, g.status.y, g.status.right(), t, theme.style(Role::Text));
            }
        }
        bulk(buf, g.bulk, m, theme, hits);
        list(buf, g.list, m, theme, hits);
        footer(buf, g.footer, m, theme, hits);
        if !g.hints.is_empty() {
            put(buf, g.hints.x, g.hints.y, g.hints.right(), &m.hints, theme.style(Role::KeyHint));
        }
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

fn label(buf: &mut Buffer, r: Rect, s: &str, focused: bool, theme: &TuiTheme) -> u16 {
    let x = put(buf, r.x, r.y, r.right(), s, theme.style(if focused { Role::Accent } else { Role::Muted }));
    x.saturating_add(1).min(r.right())
}

fn roots(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let mut x = label(buf, r, &m.labels.search_in, m.root_cursor.is_some(), theme);
    for (i, root) in m.roots.iter().enumerate() {
        let on = m.root_cursor == Some(i);
        let style = theme.style(if on { Role::PaletteSelected } else { Role::Button });
        x = put_hit(buf, x, r.y, r.right(), &format!(" {root} × "), style, hits, HitTarget::DupesRoot(i));
        x = x.saturating_add(1).min(r.right());
    }
}

/// The buttons after the add field.
fn add_buttons(labels: &Labels) -> (String, String) {
    (format!("Enter {}", labels.add), format!("Alt+O {}", labels.other_pane))
}

/// Where the add field is in the add row `r` (its label first; the completions, or the
/// buttons, after it). The field keeps at least 12 columns; buttons that no longer fit
/// are left out.
pub fn field_rect(r: Rect, labels: &Labels, other_pane: bool, completions: bool) -> Rect {
    if r.is_empty() {
        return Rect::default();
    }
    let x = (r.x + text::width(&labels.add) as u16 + 1).min(r.right());
    let room = r.right() - x;
    let w = if completions {
        (room / 2).min(32)
    } else {
        let (add, other) = add_buttons(labels);
        let bw = chrome::button_w(&add) + if other_pane { chrome::button_w(&other) } else { 0 };
        room.saturating_sub(bw).max(room.min(12))
    };
    Rect::new(x, r.y, w, 1)
}

fn add_row(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) -> Option<(u16, u16)> {
    if r.is_empty() {
        return None;
    }
    let focused = m.add.cursor.is_some();
    label(buf, r, &m.labels.add, focused, theme);
    let fr = field_rect(r, &m.labels, m.other_pane, !m.completions.is_empty());
    let cursor = field(buf, fr, &m.add, theme, hits);
    if !m.completions.is_empty() {
        // The completions replace the buttons while a name is typed.
        let mut cx = put(buf, (fr.right() + 1).min(r.right()), r.y, r.right(), "Tab ›", theme.style(Role::KeyHint));
        for (i, n) in m.completions.iter().enumerate() {
            cx = cx.saturating_add(1).min(r.right());
            let role = if i == m.pick { Role::PaletteSelected } else { Role::Folder };
            let t = HitTarget::ScreenChip(chip::COMPLETION.saturating_add(i as u8));
            cx = put_hit(buf, cx, r.y, r.right(), n, theme.style(role), hits, t);
        }
        return cursor;
    }
    let (add, other) = add_buttons(&m.labels);
    let mut bx = (fr.right() + 1).min(r.right());
    bx = chrome::button(buf, bx, r.y, r.right(), &add, Role::Button, theme, hits, HitTarget::ScreenButton(button::ADD));
    if m.other_pane {
        chrome::button(
            buf,
            bx,
            r.y,
            r.right(),
            &other,
            Role::Button,
            theme,
            hits,
            HitTarget::ScreenButton(button::OTHER_PANE),
        );
    }
    cursor
}

fn field(buf: &mut Buffer, r: Rect, f: &FieldVm, theme: &TuiTheme, hits: &mut Hits) -> Option<(u16, u16)> {
    if r.is_empty() {
        return None;
    }
    hits.push(r, HitTarget::ScreenField(FIELD_ADD));
    buf.set_style(r, theme.style(Role::Input));
    if f.shown.is_empty() && f.cursor.is_none() {
        put(buf, r.x, r.y, r.right(), &f.placeholder, theme.style(Role::Input).patch(theme.style(Role::Faint)));
    } else {
        put(buf, r.x, r.y, r.right(), &f.shown, theme.style(Role::Input));
    }
    f.cursor.map(|c| (r.x + c.min(r.width - 1), r.y))
}

fn sizes(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let find = if m.running { format!("Esc {}", m.labels.stop) } else { format!("F5 {}", m.labels.find) };
    let fw = chrome::button_w(&find);
    let end = r.right().saturating_sub(fw);
    let mut x = label(buf, Rect::new(r.x, r.y, end.saturating_sub(r.x), 1), &m.labels.min_size, m.sizes_focused, theme);
    for (i, s) in m.sizes.iter().enumerate() {
        if x >= end {
            break;
        }
        let on = m.min_idx == i;
        let item = format!("({}) {}", if on { "•" } else { " " }, chrome::ascii_text(theme, s));
        let style = match (on, m.sizes_focused) {
            (true, true) => theme.style(Role::PaletteSelected),
            (true, false) => theme.style(Role::Text).add_modifier(Modifier::BOLD),
            _ => theme.style(Role::Muted),
        };
        x = put_hit(buf, x, r.y, end, &item, style, hits, HitTarget::ScreenChip(i as u8)).saturating_add(2).min(end);
    }
    let role = match (m.running, m.can_find) {
        (true, _) => Role::Warning,
        (false, true) => Role::ButtonFocused,
        (false, false) => Role::Disabled,
    };
    chrome::button(buf, end, r.y, r.right(), &find, role, theme, hits, HitTarget::ScreenButton(button::FIND));
}

fn summary(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    hits.push(r, HitTarget::ScreenButton(button::SETUP));
    let find = if m.running { format!("Esc {}", m.labels.stop) } else { format!("F5 {}", m.labels.find) };
    let fw = chrome::button_w(&find);
    let end = r.right().saturating_sub(fw);
    put(buf, r.x, r.y, end, &chrome::ascii_text(theme, &m.summary), theme.style(Role::Muted));
    chrome::button(buf, end, r.y, r.right(), &find, Role::Button, theme, hits, HitTarget::ScreenButton(button::FIND));
}

fn bulk(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let mut x = label(buf, r, &m.labels.every_group, false, theme);
    for (i, (k, l)) in ["N", "O", "S"].iter().zip(&m.labels.keep).enumerate() {
        let t = HitTarget::ScreenButton(button::KEEP + i as u8);
        x = chrome::button(buf, x, r.y, r.right(), &format!("{k} {l}"), Role::Button, theme, hits, t);
    }
    let clear = format!("C {}", m.labels.clear);
    chrome::button(buf, x, r.y, r.right(), &clear, Role::Button, theme, hits, HitTarget::ScreenButton(button::CLEAR));
}

fn list(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    hits.push(r, HitTarget::ScreenArea(AREA_LIST));
    if let Some((t, b)) = &m.empty {
        let role = if m.running { Role::Accent } else { Role::Title };
        chrome::note(buf, r, t, b, role, theme);
        return;
    }
    for (line, y) in m.lines.iter().zip(r.y..r.bottom()) {
        let lr = Rect::new(r.x, y, r.width, 1);
        let base = if line.cursor {
            theme.style(if m.list_focused { Role::Cursor } else { Role::CursorInactive })
        } else {
            Style::default()
        };
        buf.set_style(lr, base);
        let st = |role: Role| theme.style(role).patch(base);
        match &line.kind {
            LineKind::Header { group, collapsed, title, reclaim } => {
                hits.push(lr, HitTarget::DupesHeader(*group));
                let caret = if *collapsed { "▸ " } else { "▾ " };
                let keys = &m.labels.keep_keys;
                let kw = text::width(keys) as u16;
                let end = if lr.width > kw + 20 { lr.right() - kw - 1 } else { lr.right() };
                let mut x = put(buf, lr.x, y, end, caret, st(Role::Faint));
                x = put(buf, x, y, end, title, st(Role::GroupHeader));
                put(buf, x.saturating_add(2).min(end), y, end, reclaim, st(Role::Success));
                if end < lr.right() {
                    put(buf, end + 1, y, lr.right(), keys, st(Role::Faint));
                }
            }
            LineKind::File { marked, name, folder, date, size, refused } => {
                hits.push(lr, HitTarget::DupesRow(line.index));
                let mut end = lr.right();
                if lr.width >= SIZE_MIN_COLS && !size.is_empty() {
                    let w = text::width(size) as u16;
                    put(buf, end - w, y, end, size, st(Role::Muted));
                    end = end.saturating_sub(w + 2);
                }
                if lr.width >= DATE_MIN_COLS && !date.is_empty() {
                    let w = text::width(date) as u16;
                    put(buf, end.saturating_sub(w), y, end, date, st(Role::Faint));
                    end = end.saturating_sub(w + 2);
                }
                let mark = if *marked { "  [x] " } else { "  [ ] " };
                let mut x = put(buf, lr.x, y, end, mark, st(if *marked { Role::Danger } else { Role::Faint }));
                let mut ns = st(if *marked { Role::Muted } else { Role::Text });
                if *marked {
                    ns = ns.add_modifier(Modifier::CROSSED_OUT);
                }
                x = put(buf, x, y, end, name, ns);
                x = x.saturating_add(2).min(end);
                if *refused {
                    put(buf, x, y, end, &m.labels.keep_one, st(Role::Warning));
                } else {
                    put(buf, x, y, end, folder, st(Role::Muted));
                }
            }
        }
    }
}

fn footer(buf: &mut Buffer, r: Rect, m: &DupesVm, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    if let Some(q) = &m.confirm {
        chrome::confirm_bar(buf, r, q, &m.labels.trash, &m.labels.cancel, theme, hits);
        return;
    }
    let del = format!("Del {}", m.labels.delete_marked);
    let bw = chrome::button_w(&del);
    let end = if r.width >= bw + 10 { r.right() - bw } else { r.right() };
    let role = if m.marked > 0 { Role::Text } else { Role::Faint };
    put(buf, r.x, r.y, end.saturating_sub(1).max(r.x), &m.footer, theme.style(role));
    if end < r.right() {
        let role = if m.marked > 0 { Role::Warning } else { Role::Disabled };
        chrome::button(buf, end, r.y, r.right(), &del, role, theme, hits, HitTarget::ScreenButton(button::TRASH));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::icons::IconSet;
    use crate::theme::ColorMode;
    use soyuz_core::config::TuiBorders;

    pub(crate) fn vm() -> DupesVm {
        let mut lines = Vec::new();
        for g in 0..6 {
            lines.push(LineVm {
                index: lines.len(),
                kind: LineKind::Header {
                    group: g,
                    collapsed: g == 5,
                    title: "3 copies · 120 MiB each".into(),
                    reclaim: "240 MiB reclaimable".into(),
                },
                cursor: false,
            });
            for i in 0..3 {
                lines.push(LineVm {
                    index: lines.len(),
                    kind: LineKind::File {
                        marked: i > 0,
                        name: format!("IMG_{g}{i}.jpg"),
                        folder: "Pictures/2019".into(),
                        date: "12 Oct 2019".into(),
                        size: "120 MiB".into(),
                        refused: g == 0 && i == 0,
                    },
                    cursor: lines.len() == 2,
                });
            }
        }
        DupesVm {
            title: "Duplicate finder".into(),
            sub: "~/Pictures".into(),
            roots: vec!["Pictures".into(), "日本語".into()],
            root_cursor: Some(1),
            add: FieldVm { shown: "/tmp/x".into(), placeholder: "Add a folder path…".into(), cursor: Some(6) },
            other_pane: true,
            sizes: vec!["Any".into(), "≥ 100 KB".into(), "≥ 1 MB".into()],
            min_idx: 1,
            running: true,
            can_find: true,
            summary: "2 folders · ≥ 100 KB".into(),
            status: StatusVm::Progress("Hashing 3 of 9".into(), 3, 9),
            groups: true,
            lines,
            list_focused: true,
            footer: "5 marked · 600 MiB".into(),
            marked: 5,
            labels: Labels {
                search_in: "Search in".into(),
                add: "Add".into(),
                other_pane: "Add other pane".into(),
                min_size: "Minimum size".into(),
                find: "Find".into(),
                stop: "Stop".into(),
                every_group: "Every group:".into(),
                keep: ["Newest".into(), "Oldest".into(), "Shortest".into()],
                clear: "Clear".into(),
                delete_marked: "Delete marked…".into(),
                trash: "Move to trash".into(),
                cancel: "Cancel".into(),
                keep_one: "Keep at least one copy".into(),
                keep_keys: "keep: n o s".into(),
            },
            hints: "hints".into(),
            ..DupesVm::default()
        }
    }

    fn draw_with(m: &DupesVm, w: u16, h: u16, theme: &TuiTheme) -> (Vec<String>, Hits) {
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

    fn themes() -> [TuiTheme; 3] {
        [
            TuiTheme::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_DARK, ColorMode::Truecolor, IconSet::Unicode),
            TuiTheme::plain(),
            TuiTheme::plain().with_icons(IconSet::Ascii).with_borders(TuiBorders::Ascii),
        ]
    }

    #[test]
    fn rows_and_targets() {
        let (lines, hits) = draw_with(&vm(), 100, 30, &themes()[0]);
        let g = geometry(Rect::new(0, 0, 100, 30), false, true);
        assert!(lines[0].contains("Duplicate finder"), "{lines:?}");
        assert_eq!(hits.at(98, 0), Some(HitTarget::ScreenClose));
        assert!(lines[usize::from(g.roots.y)].contains("Pictures"));
        assert!(matches!(hits.at(g.roots.x + 12, g.roots.y), Some(HitTarget::DupesRoot(0))));
        assert_eq!(hits.at(g.add.x + 6, g.add.y), Some(HitTarget::ScreenField(FIELD_ADD)));
        assert!(lines[usize::from(g.sizes.y)].contains("100 KB"));
        assert_eq!(hits.at(g.list.x + 3, g.list.y), Some(HitTarget::DupesHeader(0)));
        assert_eq!(hits.at(g.list.x + 3, g.list.y + 1), Some(HitTarget::DupesRow(1)));
        let file = &lines[usize::from(g.list.y + 2)];
        assert!(file.contains("[x]") && file.contains("12 Oct 2019") && file.contains("120 MiB"), "{file}");
        assert!(lines[usize::from(g.list.y + 1)].contains("Keep at least one copy"), "the refused flash");
        assert!(lines[usize::from(g.footer.y)].contains("5 marked"));
        assert_eq!(hits.at(g.footer.right() - 2, g.footer.y), Some(HitTarget::ScreenButton(button::TRASH)));
        assert_eq!(hits.at(50, 29), Some(HitTarget::Overlay));
    }

    #[test]
    fn columns_drop_on_narrow_screens() {
        let (lines, _) = draw_with(&vm(), 59, 30, &themes()[1]);
        let g = geometry(Rect::new(0, 0, 59, 30), false, true);
        let file = &lines[usize::from(g.list.y + 2)];
        assert!(!file.contains("2019") || !file.contains("12 Oct"), "no date: {file}");
        assert!(file.contains("120 MiB"));
        let (lines, _) = draw_with(&vm(), 39, 30, &themes()[1]);
        let g = geometry(Rect::new(0, 0, 39, 30), false, true);
        assert!(!lines[usize::from(g.list.y + 2)].contains("MiB"), "no size");
    }

    #[test]
    fn short_screens_collapse_the_setup() {
        let g = geometry(Rect::new(0, 0, 80, 12), false, true);
        assert!(g.roots.is_empty() && !g.summary.is_empty() && g.hints.is_empty());
        let g = geometry(Rect::new(0, 0, 80, 12), true, true);
        assert!(!g.roots.is_empty() && !g.sizes.is_empty() && g.summary.is_empty(), "F2 opens it");
        let g = geometry(Rect::new(0, 0, 80, 5), false, true);
        assert!(g.header.is_empty() && !g.footer.is_empty() && !g.list.is_empty());
        for h in 0..40 {
            for (open, groups) in [(false, false), (true, true), (false, true)] {
                let g = geometry(Rect::new(0, 0, 70, h), open, groups);
                let parts = [g.header, g.roots, g.add, g.sizes, g.summary, g.status, g.bulk, g.list, g.footer, g.hints];
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
    fn every_size_ascii_and_mono() {
        let mut confirm = vm();
        confirm.confirm = Some("Move 5 files (600 MiB) to the trash?".into());
        confirm.completions = vec!["sub".into(), "other".into()];
        confirm.open = true;
        let mut empty = vm();
        empty.lines.clear();
        empty.groups = false;
        empty.empty = Some(("Find identical files".into(), "Choose folders and a minimum size.".into()));
        empty.status = StatusVm::Summary("No duplicates".into());
        for m in [vm(), confirm, empty] {
            for theme in &themes() {
                for w in 0..=100 {
                    for h in 0..=30 {
                        draw_with(&m, w, h, theme);
                    }
                }
            }
        }
        let (lines, _) = draw_with(&vm(), 100, 30, &themes()[2]);
        for l in &lines {
            assert!(l.chars().all(|c| c.is_ascii() || "日本語".contains(c)), "{l}");
        }
    }
}
