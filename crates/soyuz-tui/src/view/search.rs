//! The view of the search bar and its status row (Phase T5): drawn into `Regions::search`,
//! under the location line (the GUI's bar under the toolbar, `soyuz-app/src/ui/search.rs`).
//!
//! Rows, top to bottom (`rows` wants them at full width; the layout adds one below 80
//! columns and shrinks the region on short screens):
//! - the field row: `? query▏ [in files] [Search] [Save] [x]` (the glob field only in a
//!   content search);
//! - the pills (one row; two below 80 columns): `[Contains] [Aa] [Contents] [Size] [Date]
//!   [Kind] [Hidden] [.gitignore] [in Docs]`; an active pill is marked (`*`, or `•` with
//!   Unicode icons) and drawn in the accent role;
//! - the status row of the search on screen (`models::search::status_line`):
//!   `+ "a" — 3 results  scanned 12 in 0.1 s   [Cancel] [Relevance] [Smart folder] [Refine]`;
//!   with the bar closed below 80 columns the detail gets a row of its own.
//!
//! When the region is short the field row comes first, then the status row, then the pills.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::search as model;
use soyuz_appcore::services::search::{self as svc, DateRange, KindFilter};
use soyuz_appcore::tr;

use super::text;
use crate::hits::SearchButton;
use crate::icons::{self, IconSet};
use crate::input::search::{Stop, pill, status_button};
use crate::input::{InputState, TuiFocus};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};

/// A text field as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldVm {
    /// The visible part of the text (scrolled to the cursor).
    pub shown: String,
    /// The cursor's column while the field has the keyboard.
    pub cursor: Option<u16>,
    /// Shown while the text is empty.
    pub placeholder: String,
}

/// An option pill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PillVm {
    /// [`pill`] id.
    pub id: u8,
    pub label: String,
    pub active: bool,
    pub focused: bool,
}

/// A button of the field row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonVm {
    pub button: SearchButton,
    pub label: String,
    pub focused: bool,
}

/// The bar (while open).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarVm {
    /// A content search (the field's icon, the glob field).
    pub content: bool,
    pub field: FieldVm,
    /// The "in files" glob field of a content search.
    pub glob: Option<FieldVm>,
    pub pills: Vec<PillVm>,
    pub buttons: Vec<ButtonVm>,
    /// Why the form could not run.
    pub error: Option<String>,
}

/// How far the search on screen is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Progress {
    #[default]
    Running,
    Done,
    Cancelled,
}

/// A button of the status row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusButtonVm {
    /// [`status_button`] id.
    pub id: u8,
    pub label: String,
    pub active: bool,
}

/// The status row of the search on screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusVm {
    pub progress: Progress,
    /// `"a" — 3 results`.
    pub head: String,
    pub detail: String,
    pub buttons: Vec<StatusButtonVm>,
}

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchVm {
    pub bar: Option<BarVm>,
    pub status: Option<StatusVm>,
}

/// Cells before the query field (icon and a space).
const ICON_COLS: u16 = 3;
/// The query field keeps at least this many cells before buttons are dropped.
const FIELD_MIN: u16 = 8;

/// Where the parts of the field row go, as offsets from its left edge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RowGeo {
    /// (x, width) of the query field.
    field: (u16, u16),
    /// (x, width) of the glob field.
    glob: Option<(u16, u16)>,
    /// (x, index into the buttons) of the buttons shown.
    buttons: Vec<(u16, usize)>,
}

fn button_text(label: &str) -> String {
    format!("[{label}]")
}

/// The field row's layout for `width` cells, the buttons' labels and whether the glob
/// field is shown.
fn row_geo(width: u16, labels: &[String], glob: bool) -> RowGeo {
    let widths: Vec<u16> = labels.iter().map(|l| text::width(&button_text(l)) as u16 + 1).collect();
    let all: u16 = widths.iter().sum();
    // Drop buttons from the left (Run first, close last) until the field has room.
    let mut first = 0;
    let mut used = all;
    while first < widths.len() && width.saturating_sub(ICON_COLS + used) < FIELD_MIN {
        used -= widths[first];
        first += 1;
    }
    let rest = width.saturating_sub(ICON_COLS + used);
    let glob_w = if glob && rest >= 24 { (rest * 32 / 100).clamp(8, 24) } else { 0 };
    let field_w = rest.saturating_sub(if glob_w > 0 { glob_w + 1 } else { 0 });
    let mut x = ICON_COLS + rest;
    let mut buttons = Vec::new();
    for (i, w) in widths.iter().enumerate().skip(first) {
        x += 1;
        buttons.push((x, i));
        x += w - 1;
    }
    RowGeo {
        field: (ICON_COLS.min(width), field_w),
        glob: (glob_w > 0).then(|| (ICON_COLS + field_w + 1, glob_w)),
        buttons,
    }
}

/// The field row's button labels (Run / Stop, Save, close).
fn button_labels(core: &AppCore) -> Vec<(SearchButton, String)> {
    let running = model::shown_search(core).is_some_and(svc::is_running);
    let run = if running { tr!("views-search-stop") } else { tr!("views-search-run") };
    vec![
        (SearchButton::Run, run.to_string()),
        (SearchButton::Save, tr!("common-save").to_string()),
        (SearchButton::Close, "x".to_string()),
    ]
}

fn field_row_geo(core: &AppCore, width: u16) -> RowGeo {
    let labels: Vec<String> = button_labels(core).into_iter().map(|(_, l)| l).collect();
    row_geo(width, &labels, core.ui.search.form.content)
}

impl SearchVm {
    pub fn from_core(core: &AppCore, input: &InputState, regions: &Regions) -> SearchVm {
        let st = &input.search;
        let keyboard = crate::input::focus(core, input) == TuiFocus::Search;
        let bar = core.ui.search.open.then(|| {
            let form = &core.ui.search.form;
            let geo = field_row_geo(core, regions.search.width);
            // `prepare` scrolled the fields already: these views do not move them again.
            let (shown, col) = st.edit.clone().view(&crate::dialog::safe(&form.text), geo.field.1);
            let indexed = soyuz_appcore::services::index::uses_query_language(form)
                && (form.everywhere || soyuz_appcore::services::index::covers(&form.root));
            let placeholder = if form.content {
                tr!("views-search-hint-content")
            } else if indexed {
                tr!("views-search-hint-index")
            } else {
                tr!("views-search-hint-names")
            };
            let field = FieldVm {
                shown,
                cursor: (keyboard && st.stop == Stop::Field).then_some(col),
                placeholder: placeholder.to_string(),
            };
            let glob = geo.glob.map(|(_, w)| {
                let (shown, col) = st.glob.clone().view(&crate::dialog::safe(&form.name_glob), w);
                FieldVm {
                    shown,
                    cursor: (keyboard && st.stop == Stop::Glob).then_some(col),
                    placeholder: tr!("views-search-hint-glob").to_string(),
                }
            });
            let focused = |s: Stop| keyboard && st.stop == s;
            let pills = pills(core)
                .into_iter()
                .map(|(id, label, active)| PillVm { id, label, active, focused: focused(Stop::Pill(id)) })
                .collect();
            let buttons = button_labels(core)
                .into_iter()
                .map(|(button, label)| ButtonVm { button, label, focused: focused(Stop::Button(button)) })
                .collect();
            BarVm { content: form.content, field, glob, pills, buttons, error: core.ui.search.error.clone() }
        });
        let status = model::shown_search(core).and_then(|id| model::status_line(core, id)).map(|s| {
            let progress = match (s.done, s.cancelled) {
                (false, _) => Progress::Running,
                (true, false) => Progress::Done,
                (true, true) => Progress::Cancelled,
            };
            let mut buttons = Vec::new();
            let mut add = |id, label: &str, active| buttons.push(StatusButtonVm { id, label: label.into(), active });
            if s.can_cancel {
                add(status_button::CANCEL, tr!("common-cancel"), false);
            }
            if s.fuzzy {
                add(status_button::RELEVANCE, tr!("views-search-relevance"), s.ranked);
            }
            if s.smart.is_some() {
                add(status_button::EDIT_SMART, tr!("views-search-edit"), false);
            } else if s.can_make_smart {
                add(status_button::MAKE_SMART, tr!("views-search-smart-folder"), false);
            }
            if s.can_refine {
                add(status_button::REFINE, tr!("views-search-refine"), false);
            }
            StatusVm { progress, head: crate::dialog::safe(&s.head), detail: crate::dialog::safe(&s.detail), buttons }
        });
        SearchVm { bar, status }
    }
}

/// The pills: (id, label, active), in [`pill`] order.
fn pills(core: &AppCore) -> Vec<(u8, String, bool)> {
    let f = &core.ui.search.form;
    let mode = if f.content {
        (".*".to_string(), f.content_regex)
    } else {
        (model::mode_label(f.mode).to_string(), f.mode != svc::NameMode::Substring)
    };
    let scope = if f.everywhere {
        tr!("views-search-everywhere").to_string()
    } else {
        let root = soyuz_fs::strip_verbatim(&f.root);
        let name = if root.as_os_str().is_empty() { "…".to_string() } else { soyuz_core::entry::display_name(&root) };
        tr!("views-search-in", folder = text::truncate_end(&crate::dialog::safe(&name), 20))
    };
    let sized = !f.min_size.trim().is_empty() || !f.max_size.trim().is_empty();
    vec![
        (pill::MODE, mode.0, mode.1),
        (pill::CASE, "Aa".to_string(), f.case_sensitive),
        (pill::CONTENT, tr!("views-search-contents").to_string(), f.content),
        (pill::SIZE, crate::dialog::safe(&model::size_label(f)), sized),
        (
            pill::DATE,
            if f.date == DateRange::Any { tr!("views-search-date") } else { model::date_label(f.date) }.to_string(),
            f.date != DateRange::Any,
        ),
        (
            pill::KIND,
            if f.kind == KindFilter::Any { tr!("views-search-kind") } else { model::kind_label(f.kind) }.to_string(),
            f.kind != KindFilter::Any,
        ),
        (pill::HIDDEN, tr!("views-search-hidden").to_string(), f.include_hidden),
        (pill::GITIGNORE, ".gitignore".to_string(), f.respect_gitignore),
        (pill::EVERYWHERE, scope, f.everywhere),
    ]
}

/// Rows the bar and its status row want at full width (`Facts::search_rows`; the layout
/// adds one below 80 columns); 0: hidden. The open bar takes two (field, pills), the
/// search on screen one.
pub fn rows(core: &AppCore) -> u16 {
    let bar = if core.ui.search.open { 2 } else { 0 };
    bar + u16::from(model::shown_search(core).is_some())
}

/// Where a click at screen column `x` lands in the query field (`glob`: the glob field)
/// of the bar drawn in `regions.search`: its column in the text box and the box's width
/// (`None` left of the box), for `LineEdit::click`.
pub fn field_click(core: &AppCore, regions: &Regions, x: u16, glob: bool) -> Option<(u16, u16)> {
    let geo = field_row_geo(core, regions.search.width);
    let (fx, fw) = if glob { geo.glob? } else { geo.field };
    let start = regions.search.x + fx;
    (x >= start && fw > 0).then(|| ((x - start).min(fw - 1), fw))
}

/// Step 1 of a frame (`view::prepare`): scroll the fields to their cursors.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    if !core.ui.search.open || regions.search.is_empty() {
        return;
    }
    let geo = field_row_geo(core, regions.search.width);
    let form = &core.ui.search.form;
    input.search.edit.view(&crate::dialog::safe(&form.text), geo.field.1);
    if let Some((_, w)) = geo.glob {
        input.search.glob.view(&crate::dialog::safe(&form.name_glob), w);
    }
}

/// Which line of the region shows what.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Field,
    Pills,
    Status,
    Detail,
}

/// The lines for a region `height` rows high, top to bottom (the field row first, then
/// the status row, then the pills or the detail get what is left).
fn lines(height: u16, bar: bool, status: bool) -> Vec<Line> {
    let mut left = height;
    let mut take = |want: bool| {
        let ok = want && left > 0;
        if ok {
            left -= 1;
        }
        ok
    };
    let field = take(bar);
    let status_row = take(status);
    let extra = if bar { left.min(2) } else { left.min(u16::from(status)) };
    let mut v = Vec::new();
    if field {
        v.push(Line::Field);
    }
    if bar {
        v.extend(std::iter::repeat_n(Line::Pills, usize::from(extra)));
    }
    if status_row {
        v.push(Line::Status);
    }
    if !bar && extra > 0 {
        v.push(Line::Detail);
    }
    v
}

/// Write `s` at `(x, y)`, at most up to `end` (cut with "…"); returns the next column.
fn put(buf: &mut Buffer, x: u16, y: u16, end: u16, s: &str, style: Style) -> u16 {
    if x >= end || s.is_empty() {
        return x;
    }
    let max = usize::from(end - x);
    let s = text::truncate_end(s, max);
    let (nx, _) = buf.set_stringn(x, y, &s, max, style);
    nx.min(end)
}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_search(f: &mut Frame<'_>, area: Rect, m: &SearchVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || (m.bar.is_none() && m.status.is_none()) {
        return;
    }
    f.buffer_mut().set_style(area, theme.base().patch(theme.style(Role::Text)));
    let mut cursor = None;
    let plan = lines(area.height, m.bar.is_some(), m.status.is_some());
    let mut pill_rows = Vec::new();
    for (line, y) in plan.iter().zip(area.y..area.bottom()) {
        let row = Rect::new(area.x, y, area.width, 1);
        match (line, &m.bar, &m.status) {
            (Line::Field, Some(bar), _) => cursor = field_row(f.buffer_mut(), row, bar, theme, hits),
            (Line::Pills, Some(_), _) => pill_rows.push(row),
            (Line::Status, _, Some(s)) => {
                status_row(f.buffer_mut(), row, s, m.bar.is_none() && plan.contains(&Line::Detail), theme, hits)
            }
            (Line::Detail, _, Some(s)) => {
                put(f.buffer_mut(), row.x + 2, row.y, row.right(), &s.detail, theme.style(Role::Muted));
            }
            _ => {}
        }
    }
    if let Some(bar) = &m.bar {
        draw_pills(f.buffer_mut(), &pill_rows, &bar.pills, theme, hits);
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

/// The field row; returns the text cursor while a field has the keyboard.
fn field_row(buf: &mut Buffer, r: Rect, bar: &BarVm, theme: &TuiTheme, hits: &mut Hits) -> Option<(u16, u16)> {
    let labels: Vec<String> = bar.buttons.iter().map(|b| b.label.clone()).collect();
    let geo = row_geo(r.width, &labels, bar.glob.is_some());
    let icon = icons::glyph(if bar.content { IconId::FileText } else { IconId::MagnifyingGlass }, theme.icons());
    let focused = bar.field.cursor.is_some() || bar.glob.as_ref().is_some_and(|g| g.cursor.is_some());
    put(buf, r.x + 1, r.y, r.x + geo.field.0, icon, theme.style(if focused { Role::Accent } else { Role::Muted }));
    let mut cursor = None;
    let (fx, fw) = geo.field;
    if fw > 0 {
        let fr = Rect::new(r.x + fx, r.y, fw, 1);
        hits.push(Rect::new(r.x, r.y, fx + fw, 1), HitTarget::SearchField);
        cursor = field(buf, fr, &bar.field, theme);
        if let Some(err) = &bar.error
            && (bar.field.shown.is_empty() || bar.field.cursor.is_none())
        {
            let w = (text::width(err) as u16).min(fw / 2 + fw % 2);
            if w > 0 {
                let ex = fr.right() - w;
                put(buf, ex, r.y, fr.right(), err, theme.style(Role::Input).patch(theme.style(Role::Danger)));
            }
        }
    }
    if let (Some((gx, gw)), Some(g)) = (geo.glob, &bar.glob) {
        let gr = Rect::new(r.x + gx, r.y, gw, 1);
        hits.push(gr, HitTarget::SearchPill(pill::GLOB));
        cursor = field(buf, gr, g, theme).or(cursor);
    }
    for (x, i) in geo.buttons {
        let Some(b) = bar.buttons.get(i) else { continue };
        let role = if b.focused { Role::ButtonFocused } else { Role::Button };
        let x0 = r.x + x;
        let nx = put(buf, x0, r.y, r.right(), &button_text(&b.label), theme.style(role));
        if nx > x0 {
            hits.push(Rect::new(x0, r.y, nx - x0, 1), HitTarget::SearchButton(b.button));
        }
    }
    cursor
}

/// A text field in `r`; returns its cursor cell while it has the keyboard.
fn field(buf: &mut Buffer, r: Rect, t: &FieldVm, theme: &TuiTheme) -> Option<(u16, u16)> {
    buf.set_style(r, theme.style(Role::Input));
    if t.shown.is_empty() {
        let faint = theme.style(Role::Input).patch(theme.style(Role::Faint));
        put(buf, r.x, r.y, r.right(), &t.placeholder, faint);
    } else {
        put(buf, r.x, r.y, r.right(), &t.shown, theme.style(Role::Input));
    }
    t.cursor.map(|c| (r.x + c.min(r.width.saturating_sub(1)), r.y))
}

/// The pill's text as drawn.
fn pill_text(p: &PillVm, set: IconSet) -> String {
    let mark = match (p.active, set) {
        (false, _) => "",
        (true, IconSet::Ascii) => "*",
        (true, _) => "•",
    };
    format!("[{mark}{}]", p.label)
}

/// The pills, wrapped over `rows` (those that do not fit are left out).
fn draw_pills(buf: &mut Buffer, rows: &[Rect], pills: &[PillVm], theme: &TuiTheme, hits: &mut Hits) {
    let mut it = rows.iter();
    let Some(mut row) = it.next().copied() else { return };
    let mut x = row.x + 1;
    for p in pills {
        let s = pill_text(p, theme.icons());
        let w = text::width(&s) as u16;
        if x + w > row.right() && x > row.x + 1 {
            match it.next() {
                Some(r) => {
                    row = *r;
                    x = row.x + 1;
                }
                None => break,
            }
        }
        let role = if p.focused {
            Role::ButtonFocused
        } else if p.active {
            Role::Accent
        } else {
            Role::Muted
        };
        let mut style = theme.style(role);
        if p.active {
            style = style.add_modifier(Modifier::BOLD);
        }
        let nx = put(buf, x, row.y, row.right(), &s, style);
        if nx > x {
            hits.push(Rect::new(x, row.y, nx - x, 1), HitTarget::SearchPill(p.id));
        }
        x = nx.saturating_add(1);
    }
}

/// The status row (`own_detail`: the detail has a row of its own below).
fn status_row(buf: &mut Buffer, r: Rect, s: &StatusVm, own_detail: bool, theme: &TuiTheme, hits: &mut Hits) {
    let ascii = theme.icons() == IconSet::Ascii;
    let (mark, role) = match s.progress {
        Progress::Running => (icons::glyph(IconId::Hourglass, theme.icons()), Role::Accent),
        Progress::Done => (if ascii { "+" } else { "✓" }, Role::Success),
        Progress::Cancelled => (icons::glyph(IconId::StopCircle, theme.icons()), Role::Warning),
    };
    // Buttons from the right edge.
    let texts: Vec<String> = s.buttons.iter().map(|b| button_text(&b.label)).collect();
    let bw: u16 = texts.iter().map(|t| text::width(t) as u16 + 1).sum();
    let mut bx = r.right().saturating_sub(bw).max(r.x);
    let end = if bw + 12 <= r.width { bx } else { r.right() };
    let mut x = put(buf, r.x + 1, r.y, end, mark, theme.style(role));
    x = x.saturating_add(1).min(end);
    let head_max = (r.width * 45 / 100).max(8);
    let head_end = end.min(x.saturating_add(head_max));
    x = put(buf, x, r.y, head_end, &s.head, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    if !own_detail {
        put(buf, x.saturating_add(2).min(end), r.y, end.saturating_sub(1), &s.detail, theme.style(Role::Muted));
    }
    if end == r.right() {
        return; // too narrow for the buttons
    }
    for (b, t) in s.buttons.iter().zip(&texts) {
        let role = if b.active { Role::Accent } else { Role::Button };
        let nx = put(buf, bx, r.y, r.right(), t, theme.style(role));
        if nx > bx {
            hits.push(Rect::new(bx, r.y, nx - bx, 1), HitTarget::SearchStatusButton(b.id));
        }
        bx = nx.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn labels() -> Vec<String> {
        vec!["Search".into(), "Save".into(), "x".into()]
    }

    #[test]
    fn the_field_row_drops_buttons_before_the_field_gets_too_small() {
        let g = row_geo(80, &labels(), false);
        assert_eq!(g.buttons.len(), 3);
        assert_eq!(g.field.0, ICON_COLS);
        // icon + field + " [Search] [Save] [x]"
        assert_eq!(ICON_COLS + g.field.1 + 9 + 7 + 4, 80);
        let g = row_geo(80, &labels(), true);
        assert!(g.glob.is_some_and(|(x, w)| x == ICON_COLS + g.field.1 + 1 && w >= 8));
        let g = row_geo(15, &labels(), true);
        assert_eq!(g.buttons.iter().map(|b| b.1).collect::<Vec<_>>(), [2]);
        assert!(g.glob.is_none() && g.field.1 >= FIELD_MIN);
        for w in 0..12 {
            let g = row_geo(w, &labels(), true);
            assert!(g.field.0 + g.field.1 <= w.max(ICON_COLS), "{w}: {g:?}");
        }
    }

    #[test]
    fn short_regions_keep_the_field_then_the_status() {
        use Line::*;
        assert_eq!(lines(3, true, true), [Field, Pills, Status]);
        assert_eq!(lines(4, true, true), [Field, Pills, Pills, Status]);
        assert_eq!(lines(2, true, true), [Field, Status]);
        assert_eq!(lines(1, true, true), [Field]);
        assert_eq!(lines(1, false, true), [Status]);
        assert_eq!(lines(2, false, true), [Status, Detail]);
        assert_eq!(lines(2, true, false), [Field, Pills]);
        assert!(lines(0, true, true).is_empty());
    }

    fn sample() -> SearchVm {
        let pills = (0..pill::COUNT)
            .map(|id| PillVm { id, label: format!("pill{id}"), active: id == pill::CASE, focused: id == pill::KIND })
            .collect();
        let buttons = [SearchButton::Run, SearchButton::Save, SearchButton::Close]
            .into_iter()
            .zip(labels())
            .map(|(button, label)| ButtonVm { button, label, focused: false })
            .collect();
        SearchVm {
            bar: Some(BarVm {
                content: true,
                field: FieldVm { shown: "abc".into(), cursor: Some(3), placeholder: String::new() },
                glob: Some(FieldVm::default()),
                pills,
                buttons,
                error: Some("bad".into()),
            }),
            status: Some(StatusVm {
                progress: Progress::Running,
                head: "\"abc\" — 2 results".into(),
                detail: "scanned 5 in 0.1 s".into(),
                buttons: vec![StatusButtonVm { id: status_button::CANCEL, label: "Cancel".into(), active: false }],
            }),
        }
    }

    #[test]
    fn draws_every_target_and_survives_tiny_sizes() {
        let m = sample();
        let theme = TuiTheme::plain();
        for (w, h) in [(100, 3), (79, 4), (40, 4), (12, 2), (3, 1), (1, 1), (0, 0)] {
            let mut t = Terminal::new(TestBackend::new(w.max(1), h.max(1))).unwrap();
            let mut hits = Hits::default();
            t.draw(|f| draw_search(f, Rect::new(0, 0, w, h), &m, &theme, &mut hits)).unwrap();
            if (w, h) == (100, 3) {
                let has = |t: HitTarget| hits.iter().any(|(_, h)| *h == t);
                assert!(has(HitTarget::SearchField) && has(HitTarget::SearchPill(pill::GLOB)));
                assert!(
                    (0..pill::COUNT).all(|p| has(HitTarget::SearchPill(p))),
                    "{:?}",
                    hits.iter().collect::<Vec<_>>()
                );
                assert!(has(HitTarget::SearchButton(SearchButton::Close)));
                assert!(has(HitTarget::SearchStatusButton(status_button::CANCEL)));
                let row: String = (0..100).map(|x| t.backend().buffer()[(x, 1)].symbol().to_string()).collect();
                assert!(row.contains("[*pill1]"), "{row}");
            }
        }
    }
}
