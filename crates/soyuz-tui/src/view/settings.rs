//! The view of the settings editor and its key bindings page, a full-screen overlay (Phase
//! T6), drawn into `Regions::area` after the compare screen (`HitTarget::Overlay` first,
//! then `Clear`).
//!
//! ```text
//!  Settings · Terminal UI                                   Saving…  ×
//!  / search
//!  General            │ [tui]
//!  Appearance         │   mouse                    [x]
//!  …                  │ • borders                  plain
//!  Terminal UI        │   sidebar_width            22                 ↻
//!                     │
//!  The description of the key under the cursor (English, from the schema) and
//!  its default / range; the last error in its place.
//!  ↑↓ move · Enter edit · ←→ change · Del reset · / search · Ctrl+E edit file · Esc close
//! ```
//!
//! The section list hides below [`NAV_MIN_COLS`] columns (Ctrl+PgUp / PgDn still switch);
//! the description lines show from 24 rows (F1 / `?` toggles them). A key's row: `•` when
//! it differs from the default, the name, the value (a colour with a swatch), `↻` when it
//! needs a restart; an error marks the name. Numbers and text are edited in place, the
//! choice list and the section reset question are boxes over the editor
//! (`view::dialogs::draw_dialog`).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::settings::Section;
use soyuz_appcore::models::settings_draft::Draft;
use soyuz_appcore::models::settings_schema::{self as schema, FieldKind, FieldRow};
use soyuz_appcore::tr;

use super::dialogs::DialogVm;
use super::settings_keys::{self, ChipVm};
use super::text;
use crate::input::InputState;
use crate::input::settings::{Line, Part, Popup, State, edit_text, format_float};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::widgets::{ListRow, SelectList};

/// Width of the section list (separator included).
pub const NAV_WIDTH: u16 = 22;
/// Narrower than this, the section list hides.
pub const NAV_MIN_COLS: u16 = 60;
/// Rows of description shown (when on).
pub const HELP_ROWS: u16 = 2;
/// How long "Saved" shows after a write.
const SAVED_FOR: std::time::Duration = std::time::Duration::from_secs(3);

/// What the view keeps between frames (`State::view`): the visible part of the fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// The search field as shown and its cursor column.
    pub search: (String, u16),
    /// The in-place edit as shown and its cursor column.
    pub edit: Option<(String, u16)>,
}

/// What kind of line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineKind {
    #[default]
    Field,
    Header,
    Note,
    Toggle,
    Action,
}

/// One line of the key list as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineVm {
    /// Index in the whole list (click targets).
    pub index: usize,
    pub kind: LineKind,
    pub label: String,
    pub value: String,
    /// The value's role (muted for "not set", key hints, …).
    pub value_role: Option<Role>,
    /// Differs from the default (an action: has its own bindings).
    pub modified: bool,
    /// Takes effect after a restart.
    pub restart: bool,
    /// The last edit of this key failed.
    pub error: bool,
    pub cursor: bool,
    /// A colour value's swatch.
    pub swatch: Option<[u8; 3]>,
    /// Being edited: the cursor column in the value.
    pub edit_cursor: Option<u16>,
    /// An action's bindings.
    pub chips: Vec<ChipVm>,
}

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsVm {
    pub visible: bool,
    pub title: String,
    /// "Saving…" / "Saved" / "".
    pub status: String,
    /// The search field (scrolled) or its placeholder.
    pub search: String,
    pub placeholder: bool,
    /// The search field's cursor column while it has the keyboard.
    pub search_cursor: Option<u16>,
    /// Section titles.
    pub nav: Vec<String>,
    pub nav_cursor: usize,
    pub nav_top: usize,
    pub nav_focused: bool,
    pub rows_focused: bool,
    /// The key bindings page (its lines are `SettingsKey*` targets).
    pub keys_page: bool,
    /// The visible lines of the key list.
    pub lines: Vec<LineVm>,
    /// Rows under the list: description lines and the error.
    pub desc_rows: u16,
    pub description: Vec<String>,
    pub error: Option<String>,
    pub hints: String,
    /// The choice list / reset question.
    pub popup: Option<DialogVm>,
}

/// Where each part goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    pub title: Rect,
    pub close: Rect,
    pub search: Rect,
    pub nav: Rect,
    pub sep: Rect,
    pub list: Rect,
    pub desc: Rect,
    pub hints: Rect,
}

/// The parts of an editor over `area` with `desc_rows` rows under the list (dropped when
/// the list would get fewer than 2 rows).
pub fn geometry(area: Rect, desc_rows: u16) -> Geometry {
    let mut g = Geometry::default();
    if area.is_empty() {
        return g;
    }
    let row = |y: u16| Rect::new(area.x, y, area.width, 1);
    let mut top = area.y;
    let mut bottom = area.bottom();
    g.title = row(top);
    top += 1;
    if area.width >= 3 {
        g.close = Rect::new(area.right() - 3, area.y, 3, 1);
    }
    if bottom - top >= 3 {
        g.search = row(top);
        top += 1;
    }
    if bottom - top >= 3 {
        bottom -= 1;
        g.hints = row(bottom);
    }
    if desc_rows > 0 && bottom - top >= desc_rows + 2 {
        bottom -= desc_rows;
        g.desc = Rect::new(area.x, bottom, area.width, desc_rows);
    }
    let body = Rect::new(area.x, top, area.width, bottom - top);
    if body.width >= NAV_MIN_COLS {
        g.nav = Rect::new(body.x, body.y, NAV_WIDTH - 1, body.height);
        g.sep = Rect::new(body.x + NAV_WIDTH - 1, body.y, 1, body.height);
        g.list = Rect::new(body.x + NAV_WIDTH + 1, body.y, body.width - NAV_WIDTH - 1, body.height);
    } else {
        g.list = body;
    }
    g
}

/// Columns of a key line `width` cells wide: (name width, value start, value width).
pub fn columns(width: u16) -> (u16, u16, u16) {
    let name = (width * 2 / 5).clamp(6, 34).min(width.saturating_sub(4));
    let value_x = (2 + name + 1).min(width);
    let value_w = width.saturating_sub(value_x + 2);
    (name, value_x, value_w)
}

/// Rows under the list for `st` in an editor `height` rows high.
fn desc_rows(st: &State, height: u16) -> u16 {
    let help = if st.help_shown(height) { HELP_ROWS } else { 0 };
    help + u16::from(st.error.is_some())
}

/// It is drawn this frame.
pub fn shown(core: &AppCore) -> bool {
    core.ui.settings_open
}

/// Step 1 of a frame (`view::prepare`): the list's page size and scroll, the fields' views,
/// a draft that follows the live config.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let st = &mut input.settings;
    if !st.open {
        st.view = Kept::default();
        return;
    }
    if st.draft.as_ref().is_none_or(|d| d.config != core.config) {
        st.draft = Some(Draft::from_config(&core.config));
    }
    let g = geometry(regions.area, desc_rows(st, regions.area.height));
    st.page = usize::from(g.list.height).max(1);
    st.height = regions.area.height;
    st.list.clamp(st.lines.len(), st.page);
    st.list.show_cursor(st.page);
    st.nav.clamp(Section::ALL.len(), usize::from(g.nav.height).max(1));
    st.nav.show_cursor(usize::from(g.nav.height).max(1));
    let search_w = g.search.width.saturating_sub(2);
    st.view.search = st.search.view(&st.query, search_w);
    let (_, _, value_w) = columns(g.list.width);
    st.view.edit = st.edit.as_mut().map(|e| e.line.view(&e.text, value_w));
}

impl SettingsVm {
    pub fn from_core(core: &AppCore, input: &InputState, regions: &Regions) -> SettingsVm {
        let st = &input.settings;
        if !st.open || !core.ui.settings_open {
            return SettingsVm::default();
        }
        let blank = Draft::from_config(&core.config);
        let draft = st.draft.as_ref().unwrap_or(&blank);
        let mut lines = Vec::new();
        for i in st.list.visible(st.lines.len(), st.page) {
            let cursor = i == st.list.cursor && st.part != Part::Search;
            lines.push(line_vm(st, draft, i, cursor));
        }
        let title = if st.searching() {
            tr!("tui-settings-results", query = crate::dialog::safe(&st.query))
        } else {
            format!("{} · {}", tr!("tui-settings-title"), st.section.title())
        };
        let status = if core.config_save.due.is_some() {
            tr!("tui-settings-saving").to_string()
        } else if core.config_save.flashed.is_some_and(|t| t.elapsed() < SAVED_FOR) {
            tr!("tui-settings-saved").to_string()
        } else {
            String::new()
        };
        let placeholder = st.query.is_empty() && st.part != Part::Search;
        let search = if placeholder {
            if st.keys_page() { tr!("tui-settings-filter-hint") } else { tr!("tui-settings-search-hint") }.to_string()
        } else {
            st.view.search.0.clone()
        };
        let help = st.help_shown(regions.area.height);
        SettingsVm {
            visible: true,
            title,
            status,
            search,
            placeholder,
            search_cursor: (st.part == Part::Search && st.popup.is_none()).then_some(st.view.search.1),
            nav: Section::ALL.iter().map(|s| s.title().to_string()).collect(),
            nav_cursor: st.nav.cursor,
            nav_top: st.nav.top,
            nav_focused: st.part == Part::Nav,
            rows_focused: st.part == Part::Rows,
            keys_page: st.keys_page(),
            lines,
            desc_rows: desc_rows(st, regions.area.height),
            description: if help { description(st, draft) } else { Vec::new() },
            error: st.error.as_ref().map(|(_, e)| e.clone()),
            hints: hints(st),
            popup: popup(st),
        }
    }
}

/// The model of line `i`.
fn line_vm(st: &State, draft: &Draft, i: usize, cursor: bool) -> LineVm {
    match &st.lines[i] {
        Line::Header(h) => LineVm { index: i, kind: LineKind::Header, label: h.clone(), ..LineVm::default() },
        Line::Note(n) => LineVm { index: i, kind: LineKind::Note, label: n.clone(), ..LineVm::default() },
        Line::Toggle(f) => settings_keys::toggle_line(i, &st.keys, *f, cursor),
        Line::Action(a) => match st.keys.rows.get(*a) {
            Some(row) => settings_keys::action_line(i, row, cursor.then_some(st.keys.chip)),
            None => LineVm { index: i, kind: LineKind::Header, ..LineVm::default() },
        },
        Line::Field(path) => {
            let Some(row) = schema::row(path) else {
                return LineVm { index: i, label: path.clone(), ..LineVm::default() };
            };
            let (value, value_role) = value_text(draft, row);
            let editing = st.edit.as_ref().filter(|e| e.path == *path);
            let swatch = (row.kind == FieldKind::Color)
                .then(|| draft.value(path).and_then(toml::Value::as_str).and_then(soyuz_appcore::color::parse_hex))
                .flatten()
                .map(|c| [c.r, c.g, c.b]);
            LineVm {
                index: i,
                kind: LineKind::Field,
                label: row.name.clone(),
                value: match (editing, &st.view.edit) {
                    (Some(_), Some((shown, _))) => shown.clone(),
                    _ => value,
                },
                value_role: if editing.is_some() { Some(Role::Input) } else { value_role },
                modified: !draft.is_default(path),
                restart: row.restart,
                error: st.error.as_ref().is_some_and(|(p, _)| p == path),
                cursor,
                swatch,
                edit_cursor: editing.and(st.view.edit.as_ref()).map(|(_, c)| *c),
                chips: Vec::new(),
            }
        }
    }
}

fn list_text(items: &[toml::Value]) -> String {
    items.iter().map(edit_text).collect::<Vec<_>>().join(", ")
}

/// How the value of `row` reads (and its role when it is not the value itself).
pub fn value_text(draft: &Draft, row: &FieldRow) -> (String, Option<Role>) {
    let v = draft.value(&row.path);
    let muted = |s: &str| (s.to_string(), Some(Role::Muted));
    match (&row.kind, v) {
        (FieldKind::Table { .. }, _) => {
            (tr!("tui-settings-table-summary", count = draft.table_len(&row.path)), Some(Role::Accent))
        }
        (_, None) => muted(tr!("tui-settings-unset")),
        (FieldKind::Opaque, Some(v)) => {
            let n = match v {
                toml::Value::Array(a) => a.len(),
                toml::Value::Table(t) => t.len(),
                _ => 1,
            };
            muted(&tr!("tui-settings-opaque-summary", count = n))
        }
        (_, Some(toml::Value::Boolean(b))) => ((if *b { "[x]" } else { "[ ]" }).to_string(), None),
        (_, Some(toml::Value::String(s))) if s.is_empty() => muted(tr!("tui-settings-empty")),
        (_, Some(toml::Value::String(s))) => (crate::dialog::safe(s), None),
        (_, Some(toml::Value::Float(f))) => (format_float(*f), None),
        (_, Some(toml::Value::Array(a))) if a.is_empty() => muted(tr!("tui-settings-list-empty")),
        (_, Some(toml::Value::Array(a))) => (crate::dialog::safe(&list_text(a)), None),
        (_, Some(toml::Value::Table(t))) if t.is_empty() => muted(tr!("tui-settings-list-empty")),
        (_, Some(toml::Value::Table(t))) => {
            let s = t.iter().map(|(k, v)| format!("{k} = {}", edit_text(v))).collect::<Vec<_>>().join(", ");
            (crate::dialog::safe(&s), None)
        }
        (_, Some(other)) => (other.to_string(), None),
    }
}

/// The description lines of the cursor line.
fn description(st: &State, draft: &Draft) -> Vec<String> {
    match st.current() {
        Some(Line::Field(path)) => schema::row(path).map(|r| field_description(draft, r)).unwrap_or_default(),
        Some(Line::Action(i)) => st.keys.rows.get(*i).map(settings_keys::description).unwrap_or_default(),
        Some(Line::Toggle(_)) => vec![tr!("tui-settings-keys-filter-desc").to_string()],
        _ => Vec::new(),
    }
}

fn field_description(draft: &Draft, r: &FieldRow) -> Vec<String> {
    let mut out = Vec::new();
    if !r.doc.is_empty() {
        out.push(r.doc.clone());
    }
    let mut facts = vec![r.path.clone()];
    match &r.kind {
        FieldKind::Int { min: Some(lo), max: Some(hi) } => {
            facts.push(tr!("tui-settings-range", min = lo.to_string(), max = hi.to_string()))
        }
        FieldKind::Float { min: Some(lo), max: Some(hi) } => {
            facts.push(tr!("tui-settings-range", min = format_float(*lo), max = format_float(*hi)))
        }
        FieldKind::Enum { values } => {
            let cur = draft.value(&r.path).and_then(toml::Value::as_str).unwrap_or_default();
            if let Some((_, doc)) = values.iter().find(|(v, d)| v == cur && !d.is_empty()) {
                out.push(format!("{cur}: {doc}"));
            }
            let names: Vec<&str> = values.iter().map(|(v, _)| v.as_str()).collect();
            facts.push(tr!("tui-settings-values", values = names.join(", ")));
        }
        FieldKind::Table { .. } => facts.push(tr!("tui-settings-table-hint").to_string()),
        FieldKind::Opaque => facts.push(tr!("tui-settings-opaque-hint").to_string()),
        FieldKind::EnumList { .. } | FieldKind::TextList | FieldKind::Map => {
            facts.push(tr!("tui-settings-list-desc").to_string())
        }
        _ => {}
    }
    if let Some(d) = Draft::default_value(&r.path)
        && !matches!(d, toml::Value::Table(_) | toml::Value::Array(_))
    {
        facts.push(tr!("tui-settings-default", value = edit_text(&d)));
    }
    if r.restart {
        facts.push(tr!("tui-settings-restart-note").to_string());
    }
    out.push(facts.join(" · "));
    out
}

/// The key hints of the moment.
fn hints(st: &State) -> String {
    if st.edit.is_some() {
        return tr!("tui-settings-hint-edit").into();
    }
    match st.part {
        Part::Search => tr!("tui-settings-hint-search").into(),
        Part::Nav => tr!("tui-settings-hint-nav").into(),
        Part::Rows if st.keys_page() => tr!("tui-settings-hint-keys").into(),
        Part::Rows => tr!("tui-settings-hint-rows").into(),
    }
}

/// The box over the editor, if one is open.
fn popup(st: &State) -> Option<DialogVm> {
    match st.popup.as_ref()? {
        Popup::Choose { path, values, list } => {
            let rows = values
                .iter()
                .map(|(v, doc)| ListRow { right: text::truncate_end(doc, 36), ..ListRow::new(v.clone()) })
                .collect();
            Some(DialogVm {
                title: schema::row(path).map_or_else(|| path.clone(), |r| r.name.clone()),
                list: rows,
                list_state: SelectList { cursor: list.cursor, top: list.top },
                hints: tr!("tui-settings-choose-hint").into(),
                width: 64,
                ..DialogVm::default()
            })
        }
        Popup::Confirm { section, form } => Some(DialogVm {
            body: vec![tr!("tui-settings-reset-body").to_string()],
            alert: true,
            width: 56,
            ..DialogVm::from_form(&tr!("tui-settings-reset-title", section = section.title()), form)
        }),
    }
}

fn put(buf: &mut Buffer, x: u16, y: u16, s: &str, w: u16, style: Style) {
    if w > 0 {
        buf.set_stringn(x, y, s, usize::from(w), style);
    }
}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_settings(f: &mut Frame<'_>, area: Rect, m: &SettingsVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    hits.push(area, HitTarget::Overlay);
    f.render_widget(Clear, area);
    f.buffer_mut().set_style(area, theme.base().patch(theme.style(Role::Text)));
    let mut g = geometry(area, m.desc_rows);
    if theme.screen_reader() && !g.nav.is_empty() {
        // Screen-reader mode (T8): the section list or the keys at full width, whichever
        // has the keyboard, never both on one row.
        let body = Rect { x: area.x, width: area.width, ..g.list };
        let none = Rect::new(body.x, body.y, 0, 0);
        (g.nav, g.list) = if m.nav_focused { (body, none) } else { (none, body) };
        g.sep = none;
    }
    let mut cursor: Option<(u16, u16)> = None;
    {
        let buf = f.buffer_mut();
        draw_title(buf, &g, m, theme, hits);
        if !g.search.is_empty() {
            let s = g.search;
            put(buf, s.x, s.y, "/", 1, theme.style(Role::Accent));
            let role = if m.placeholder { Role::Muted } else { Role::Input };
            put(buf, s.x + 2, s.y, &m.search, s.width.saturating_sub(2), theme.style(role));
            hits.push(s, HitTarget::SettingsSearch);
            if let Some(c) = m.search_cursor {
                cursor = Some(((s.x + 2 + c).min(s.right().saturating_sub(1)), s.y));
            }
        }
        draw_nav(buf, &g, m, theme, hits);
        if let Some(c) = draw_lines(buf, g.list, m, theme, hits) {
            cursor = Some(c);
        }
        draw_desc(buf, g.desc, m, theme);
        if !g.hints.is_empty() {
            let h = text::truncate_end(&m.hints, usize::from(g.hints.width));
            put(buf, g.hints.x, g.hints.y, &h, g.hints.width, theme.style(Role::KeyHint));
        }
    }
    if let Some(p) = &m.popup {
        super::dialogs::draw_dialog(f, area, p, theme, hits);
        return;
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
        hits.set_caret(Position { x, y });
    }
}

fn draw_title(buf: &mut Buffer, g: &Geometry, m: &SettingsVm, theme: &TuiTheme, hits: &mut Hits) {
    let t = g.title;
    if t.is_empty() {
        return;
    }
    let close_w = g.close.width;
    let status_w = text::width(&m.status) as u16;
    let status_shown = status_w > 0 && status_w + 12 + close_w <= t.width;
    let title_w = t.width.saturating_sub(close_w + if status_shown { status_w + 2 } else { 0 });
    let title = text::truncate_end(&format!(" {}", m.title), usize::from(title_w));
    put(buf, t.x, t.y, &title, title_w, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    if status_shown {
        let x = t.right() - close_w - status_w - 1;
        put(buf, x, t.y, &m.status, status_w, theme.style(Role::Muted));
    }
    if !g.close.is_empty() {
        put(buf, g.close.x + 1, t.y, "×", 1, theme.style(Role::Muted));
        hits.push(g.close, HitTarget::SettingsClose);
    }
}

fn draw_nav(buf: &mut Buffer, g: &Geometry, m: &SettingsVm, theme: &TuiTheme, hits: &mut Hits) {
    if g.nav.is_empty() {
        return;
    }
    let n = g.nav;
    let list = SelectList { cursor: m.nav_cursor, top: m.nav_top };
    for (y, i) in (n.y..n.bottom()).zip(list.visible(m.nav.len(), usize::from(n.height))) {
        let mut style = theme.style(Role::Text);
        if i == m.nav_cursor {
            style = style.patch(theme.style(if m.nav_focused { Role::Cursor } else { Role::Selected }));
        }
        let rect = Rect::new(n.x, y, n.width, 1);
        buf.set_style(rect, style);
        if i == m.nav_cursor && m.nav_focused {
            hits.set_caret(Position { x: rect.x, y });
        }
        put(
            buf,
            n.x + 1,
            y,
            &text::truncate_end(&m.nav[i], usize::from(n.width.saturating_sub(2))),
            n.width - 1,
            style,
        );
        hits.push(rect, HitTarget::SettingsNav(i));
    }
    let lines = crate::borders::lines(theme.borders());
    for y in g.sep.y..g.sep.bottom() {
        put(buf, g.sep.x, y, lines.vertical, 1, theme.style(Role::Border));
    }
}

/// Draw the visible lines; returns the edit cursor's position.
fn draw_lines(buf: &mut Buffer, area: Rect, m: &SettingsVm, theme: &TuiTheme, hits: &mut Hits) -> Option<(u16, u16)> {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return None;
    }
    let (name_w, value_x, value_w) = columns(area.width);
    let mut cursor = None;
    for (y, l) in (area.y..area.bottom()).zip(&m.lines) {
        let rect = Rect::new(area.x, y, area.width, 1);
        let target = if m.keys_page { HitTarget::SettingsKeyRow(l.index) } else { HitTarget::SettingsRow(l.index) };
        hits.push(rect, target);
        let mut base = theme.style(Role::Text);
        if l.cursor {
            base = base.patch(theme.style(if m.rows_focused { Role::Cursor } else { Role::CursorInactive }));
            buf.set_style(rect, base);
            if m.rows_focused {
                hits.set_caret(Position { x: rect.x, y });
            }
        }
        let w = area.width;
        match l.kind {
            LineKind::Header => {
                let s = text::truncate_end(&l.label, usize::from(w));
                put(buf, area.x, y, &s, w, theme.style(Role::GroupHeader));
            }
            LineKind::Note => {
                let s = text::truncate_end(&l.label, usize::from(w));
                put(buf, area.x, y, &s, w, theme.style(Role::Warning));
            }
            LineKind::Toggle => {
                let s = text::truncate_end(&l.label, usize::from(w.saturating_sub(2)));
                put(buf, area.x + 2.min(w), y, &s, w.saturating_sub(2), base);
                let vw = text::width(&l.value) as u16;
                if vw + text::width(&s) as u16 + 4 <= w {
                    put(buf, area.right() - vw - 1, y, &l.value, vw, base.patch(theme.style(Role::KeyHint)));
                }
            }
            LineKind::Action => {
                if l.modified && w > 0 {
                    put(buf, area.x, y, "•", 1, base.patch(theme.style(Role::Accent)));
                }
                let label_w = (w * 2 / 5).clamp(8, 36).min(w.saturating_sub(2));
                let s = text::truncate_end(&l.label, usize::from(label_w));
                put(buf, area.x + 2.min(w), y, &s, label_w, base);
                let chips_x = area.x + (2 + label_w + 1).min(w);
                let chips = Rect::new(chips_x, y, area.right().saturating_sub(chips_x), 1);
                settings_keys::draw_chips(buf, chips, l, theme, hits);
            }
            LineKind::Field => {
                if l.modified && w > 0 {
                    put(buf, area.x, y, "•", 1, base.patch(theme.style(Role::Accent)));
                }
                let name_style = if l.error { base.patch(theme.style(Role::Danger)) } else { base };
                let name = text::truncate_end(&l.label, usize::from(name_w));
                put(buf, area.x + 2.min(w), y, &name, name_w, name_style);
                let mut vx = area.x + value_x;
                let mut vw = value_w;
                if let Some([r, g, b]) = l.swatch
                    && vw > 4
                    && l.edit_cursor.is_none()
                {
                    let c = crate::theme::to_color(soyuz_appcore::color::Rgba8 { r, g, b, a: 255 }, theme.mode());
                    put(buf, vx, y, "■", 1, Style::new().fg(c));
                    vx += 2;
                    vw -= 2;
                }
                let mut vstyle = base;
                if let Some(r) = l.value_role {
                    vstyle = vstyle.patch(theme.style(r));
                }
                if l.edit_cursor.is_some() {
                    buf.set_style(Rect::new(vx, y, vw, 1), vstyle);
                }
                let v = if l.edit_cursor.is_some() {
                    l.value.clone()
                } else {
                    text::truncate_end(&l.value, usize::from(vw))
                };
                put(buf, vx, y, &v, vw, vstyle);
                if let Some(c) = l.edit_cursor {
                    cursor = Some(((vx + c).min(area.right().saturating_sub(1)), y));
                }
                if l.restart && w >= value_x + 2 {
                    let glyph = if theme.ascii_only() { "R" } else { "↻" };
                    put(buf, area.right() - 1, y, glyph, 1, base.patch(theme.style(Role::Warning)));
                }
            }
        }
    }
    cursor
}

fn draw_desc(buf: &mut Buffer, area: Rect, m: &SettingsVm, theme: &TuiTheme) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    let w = usize::from(area.width.saturating_sub(1)).max(1);
    let mut y = area.y;
    if let Some(e) = &m.error {
        put(buf, area.x + 1, y, &text::truncate_end(e, w), area.width.saturating_sub(1), theme.style(Role::Danger));
        y += 1;
    }
    for l in m.description.iter().flat_map(|d| text::wrap(d, w)) {
        if y >= area.bottom() {
            break;
        }
        put(buf, area.x + 1, y, &l, area.width.saturating_sub(1), theme.style(Role::Muted));
        y += 1;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn field(index: usize, label: &str, value: &str) -> LineVm {
        LineVm { index, label: label.into(), value: value.into(), ..LineVm::default() }
    }

    pub(crate) fn vm() -> SettingsVm {
        SettingsVm {
            visible: true,
            title: "Settings · Terminal UI".into(),
            status: "Saved".into(),
            search: "Search all settings".into(),
            placeholder: true,
            nav: Section::ALL.iter().map(|s| s.english_title().to_string()).collect(),
            nav_cursor: 7,
            rows_focused: true,
            lines: vec![
                LineVm { index: 0, kind: LineKind::Header, label: "[tui]".into(), ..LineVm::default() },
                field(1, "mouse", "[x]"),
                LineVm { modified: true, cursor: true, ..field(2, "borders", "plain") },
                LineVm { restart: true, ..field(3, "sidebar_width", "22") },
                LineVm { error: true, ..field(4, "jobs_rows", "8") },
                LineVm { swatch: Some([255, 0, 0]), ..field(5, "accent", "#ff0000") },
            ],
            desc_rows: 3,
            description: vec!["Box lines of the terminal UI.".into(), "tui.borders · Default: rounded".into()],
            error: Some("Invalid value: jobs_rows: not a number".into()),
            hints: "↑↓ move · Enter edit · Esc close".into(),
            ..SettingsVm::default()
        }
    }

    fn screen(m: &SettingsVm, w: u16, h: u16) -> (Vec<String>, Hits, Buffer) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_settings(f, f.area(), m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, hits, buf)
    }

    #[test]
    fn draws_at_80_by_24() {
        let (lines, hits, _) = screen(&vm(), 80, 24);
        assert!(lines[0].starts_with(" Settings · Terminal UI"), "{lines:?}");
        assert!(lines[0].contains("Saved") && lines[0].ends_with('×'), "{:?}", lines[0]);
        assert!(lines[1].starts_with("/ Search all settings"), "{:?}", lines[1]);
        // The section list beside the keys.
        assert!(lines[2].starts_with(" General"), "{:?}", lines[2]);
        assert!(lines[2].contains("│ [tui]"), "{:?}", lines[2]);
        let borders = lines.iter().find(|l| l.contains("borders")).unwrap();
        assert!(borders.contains("• borders") && borders.ends_with("plain"), "{borders:?}");
        assert!(lines.iter().any(|l| l.contains("sidebar_width") && l.ends_with('↻')));
        assert!(lines.iter().any(|l| l.contains("■ #ff0000")));
        // Error, description, hints at the bottom.
        assert!(lines[20].contains("not a number"), "{lines:?}");
        assert!(lines[21].contains("Box lines"), "{lines:?}");
        assert_eq!(lines[23], "↑↓ move · Enter edit · Esc close");
        let targets: Vec<HitTarget> = hits.iter().map(|(_, t)| *t).collect();
        for t in [
            HitTarget::Overlay,
            HitTarget::SettingsClose,
            HitTarget::SettingsSearch,
            HitTarget::SettingsNav(7),
            HitTarget::SettingsRow(2),
        ] {
            assert!(targets.contains(&t), "{t:?}");
        }
    }

    #[test]
    fn narrow_screens_hide_the_sections() {
        let mut m = vm();
        m.desc_rows = 0;
        m.description.clear();
        m.error = None;
        let (lines, hits, _) = screen(&m, 40, 12);
        assert!(lines[2].starts_with("[tui]"), "{lines:?}");
        assert!(!hits.iter().any(|(_, t)| matches!(t, HitTarget::SettingsNav(_))));
        assert_eq!(geometry(Rect::new(0, 0, 40, 12), 2).desc.height, 2);
        assert_eq!(geometry(Rect::new(0, 0, 40, 5), 2).desc.height, 0, "the list keeps two rows");
    }

    #[test]
    fn the_edit_cursor_is_in_the_value() {
        let mut m = vm();
        m.lines[2].edit_cursor = Some(3);
        m.lines[2].value = "pla".into();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| draw_settings(f, f.area(), &m, &TuiTheme::plain(), &mut hits)).unwrap();
        let (_, _, value_w) = columns(80 - NAV_WIDTH - 1);
        assert!(value_w > 10);
        let pos = t.get_cursor_position().unwrap();
        assert_eq!(pos.y, 4);
        let (_, value_x, _) = columns(80 - NAV_WIDTH - 1);
        assert_eq!(pos.x, NAV_WIDTH + 1 + value_x + 3);
    }

    #[test]
    fn a_popup_draws_on_top() {
        let mut m = vm();
        m.popup = Some(DialogVm {
            title: "borders".into(),
            list: vec![ListRow::new("rounded"), ListRow::new("plain")],
            ..DialogVm::default()
        });
        let (lines, hits, _) = screen(&m, 80, 24);
        assert!(lines.iter().any(|l| l.contains("rounded")), "{lines:?}");
        assert!(hits.iter().any(|(_, t)| *t == HitTarget::DialogRow(1)));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let mut keys = vm();
        keys.keys_page = true;
        keys.lines.push(LineVm {
            index: 6,
            kind: LineKind::Action,
            label: "Refresh 日本語".into(),
            chips: vec![ChipVm {
                text: "F5".into(),
                kind: crate::input::settings_keys::ChipKind::Conflict,
                chosen: true,
            }],
            cursor: true,
            ..LineVm::default()
        });
        keys.lines.push(LineVm {
            index: 7,
            kind: LineKind::Toggle,
            label: "[ ] Conflicts".into(),
            value: "Alt+C".into(),
            ..LineVm::default()
        });
        keys.lines.push(LineVm { index: 8, kind: LineKind::Note, label: "2 problems".into(), ..LineVm::default() });
        let mut edit = vm();
        edit.lines[2].edit_cursor = Some(40);
        let mut popup = vm();
        popup.popup = Some(DialogVm { title: "t".into(), list: vec![ListRow::new("x"); 20], ..DialogVm::default() });
        for m in [vm(), keys, edit, popup] {
            for w in 0..=24 {
                for h in 0..=8 {
                    let _ = screen(&m, w, h);
                }
            }
            let _ = screen(&m, 59, 30);
            let _ = screen(&m, 60, 30);
        }
    }
}
