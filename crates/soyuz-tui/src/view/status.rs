//! The status line, like the GUI's status bar (`soyuz-app/src/ui/statusbar.rs`), for the
//! focused pane: the filter field (while it is open or a filter is set), item counts
//! (`3 of 10 items` while filtered), hidden items, the selection with its total size on
//! the left; on a smart folder its name and the index status (`models::smart::status_line`,
//! the GUI's index chip), the volume's free space, the view mode, the sort and a key hint
//! on the right (dropped from the end when the line is too narrow).
//!
//! The line follows `ui.status_visible`. When it is hidden, an open filter field still
//! takes the bottom row (over the list), so typing is never invisible.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::i18n::labels;
use soyuz_appcore::tr;
use soyuz_core::{Location, SizeStyle, SortDir, ViewMode, format_size};
use unicode_segmentation::UnicodeSegmentation;

use super::text;
use crate::dialog::safe;
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};

/// The filter field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterVm {
    /// "Filter".
    pub label: String,
    /// The filter text; while editing, the part that fits the field
    /// (`crate::input::filter::decorate`).
    pub text: String,
    /// The cursor's column in `text` while the field is open.
    pub cursor: Option<u16>,
}

/// The status line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusVm {
    /// The line is shown (`ui.status_visible`).
    pub visible: bool,
    /// "12 items", or "3 of 12 items" while filtered.
    pub counts: String,
    /// "· 2 hidden" (entries the hidden-files setting leaves out), or empty.
    pub hidden: String,
    /// "2 selected  (1.5 KB)", or empty.
    pub selected: String,
    /// On a smart folder: its name and the index status ("Photos · Index ready …"), or
    /// empty.
    pub smart: String,
    /// "12 GB free" on the volume of the pane's folder, or empty.
    pub free: String,
    /// The view mode ("Details").
    pub mode: String,
    /// The sort ("Name ↑").
    pub sort: String,
    /// Key hint on the right.
    pub hint: String,
    /// The filter field, while it is open or a filter is set.
    pub filter: Option<FilterVm>,
}

/// The label of a view mode (the GUI's `context_menu::mode_label`).
fn mode_label(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Details => tr!("app-view-details"),
        ViewMode::List => tr!("app-view-list"),
        ViewMode::Grid => tr!("app-view-icons"),
        ViewMode::Gallery => tr!("app-view-gallery"),
        ViewMode::Miller => tr!("app-view-columns"),
    }
}

/// Free and total bytes of the volume holding the focused pane's folder.
fn free_space(core: &AppCore) -> Option<(u64, u64)> {
    let Location::Dir(path) = &core.ws.pane().location else { return None };
    core.ui
        .volumes
        .iter()
        .filter(|v| path.starts_with(&v.mount))
        .max_by_key(|v| v.mount.as_os_str().len())
        .map(|v| (v.free, v.total))
}

impl StatusVm {
    pub fn from_core(core: &AppCore) -> StatusVm {
        let pane = core.ws.pane();
        let filtered = pane.filter_active();
        let counts = if filtered {
            tr!("app-status-items-filtered", visible = pane.visible.len(), count = pane.entries.len())
        } else {
            tr!("app-status-items", count = pane.visible.len())
        };
        let hidden = pane.entries.len().saturating_sub(pane.visible.len());
        let hidden = if filtered || !pane.collapsed_groups.is_empty() || hidden == 0 {
            String::new()
        } else {
            tr!("app-status-hidden", count = hidden)
        };
        let (n, bytes) = pane.selection_stats();
        let selected = match (n, bytes) {
            (0, _) => String::new(),
            (n, 0) => tr!("app-status-selected", count = n),
            (n, b) => {
                tr!("app-status-selected-size", count = n, size = format_size(b, core.config.general.size_style))
            }
        };
        let free = match free_space(core) {
            Some((free, total)) if total > 0 => tr!("app-status-free", free = format_size(free, SizeStyle::Decimal)),
            _ => String::new(),
        };
        let sort = &pane.view.sort;
        let arrow = if sort.dir == SortDir::Asc { "↑" } else { "↓" };
        let filter = filtered.then(|| FilterVm {
            label: tr!("app-toolbar-filter").to_string(),
            text: safe(&pane.filter),
            cursor: None,
        });
        StatusVm {
            visible: core.ui.status_visible,
            counts,
            hidden,
            selected,
            smart: smart_segment(&pane.location),
            free,
            mode: mode_label(pane.view.mode).to_string(),
            sort: format!("{} {arrow}", labels::column(sort.column)),
            hint: tr!("tui-hint").to_string(),
            filter,
        }
    }
}

/// While the model is built (`ViewModel::from_core`): the search bar's key hint while it
/// has the keyboard (in a field, or on a pill or a button).
pub fn decorate_search(core: &AppCore, input: &crate::input::InputState, status: &mut StatusVm) {
    use crate::input::search::Stop;
    if crate::input::focus(core, input) != crate::input::TuiFocus::Search {
        return;
    }
    status.hint = match input.search.stop {
        Stop::Field | Stop::Glob => tr!("tui-search-hint-field"),
        Stop::Pill(_) | Stop::Button(_) => tr!("tui-search-hint-controls"),
    }
    .to_string();
}

/// The smart folder segment for a pane at `loc`: the folder's name, then the index status
/// while the index runs.
fn smart_segment(loc: &Location) -> String {
    use soyuz_appcore::models::smart;
    let Some(name) = smart::shown_name(loc) else { return String::new() };
    match soyuz_appcore::services::index::status() {
        Some(st) => format!("{} · {}", safe(&name), smart::status_line(&st)),
        None => safe(&name),
    }
}

/// Whether the status line takes a row (`layout::Facts::status`): `ui.status_visible`.
pub fn shown(core: &AppCore) -> bool {
    core.ui.status_visible
}

/// Cells of the filter field's text box in a line `width` cells wide (the input side
/// scrolls the text to fit it).
pub fn field_width(width: u16) -> u16 {
    (width / 3).clamp(4, 32).min(width)
}

/// The filter field's spans (the label, then the text; while the field is open, in a box
/// of [`field_width`] cells with the cursor cell reversed) and the cursor's column from the
/// start of the line.
fn filter_spans(fv: &FilterVm, width: u16, theme: &TuiTheme) -> (Vec<Span<'static>>, Option<u16>) {
    let label = format!(" {} ", fv.label);
    let fw = usize::from(field_width(width));
    let shown = text::truncate_end(&fv.text, fw);
    // Over the reset style: the field must not inherit the bar's look (reversed in plain).
    let input = Style::reset().patch(theme.style(Role::Input));
    let mut spans = vec![Span::styled(label.clone(), theme.style(Role::StatusBar))];
    let cursor = fv.cursor.map(|c| (text::width(&label) + usize::from(c).min(fw.saturating_sub(1))) as u16);
    match fv.cursor {
        // The cursor cell is reversed, so it shows without the terminal's own cursor.
        Some(c) => {
            let c = usize::from(c);
            let (mut before, mut at, mut after) = (String::new(), String::new(), String::new());
            let mut x = 0;
            for g in shown.graphemes(true) {
                if x < c {
                    before.push_str(g);
                } else if at.is_empty() {
                    at.push_str(g);
                } else {
                    after.push_str(g);
                }
                x += text::width(g);
            }
            if at.is_empty() {
                at.push(' ');
            }
            let pad = fw.saturating_sub(text::width(&before) + text::width(&at) + text::width(&after));
            spans.push(Span::styled(before, input));
            spans.push(Span::styled(at, Style::reset().patch(theme.style(Role::Cursor))));
            spans.push(Span::styled(after, input));
            spans.push(Span::styled(" ".repeat(pad), input));
        }
        // Closed: only the text, no box.
        None => spans.push(Span::styled(shown, input)),
    }
    spans.push(Span::styled(" ", theme.style(Role::StatusBar)));
    (spans, cursor)
}

/// Draw the filter field alone on the row `rect` (the status line is hidden).
fn draw_filter_line(f: &mut Frame<'_>, rect: Rect, fv: &FilterVm, theme: &TuiTheme, hits: &mut Hits) {
    let (spans, cursor) = filter_spans(fv, rect.width, theme);
    f.render_widget(Paragraph::new(Line::from(spans)).style(theme.style(Role::StatusBar)), rect);
    if let Some(c) = cursor.filter(|&c| c < rect.width) {
        f.set_cursor_position(Position { x: rect.x + c, y: rect.y });
        hits.set_caret(Position { x: rect.x + c, y: rect.y });
    }
    hits.push(rect, HitTarget::Filter);
}

/// Draw the status line into `rect` (an empty rect when it is hidden: then only an open
/// filter field is drawn, on the bottom row of the screen).
///
/// Screen-reader mode (T8): only an open filter field is drawn here; otherwise the row is
/// `crate::a11y`'s, which writes the status sentence (or the newest toast) into it.
pub fn draw_status(f: &mut Frame<'_>, rect: Rect, m: &StatusVm, theme: &TuiTheme, hits: &mut Hits) {
    if theme.screen_reader() && !rect.is_empty() {
        hits.push(rect, HitTarget::Status);
        if let Some(fv) = m.filter.as_ref().filter(|fv| fv.cursor.is_some()) {
            draw_filter_line(f, rect, fv, theme, hits);
        }
        return;
    }
    if rect.is_empty() {
        let area = f.area();
        if let Some(fv) = m.filter.as_ref().filter(|fv| fv.cursor.is_some())
            && !area.is_empty()
        {
            let row = Rect { y: area.bottom() - 1, height: 1, ..area };
            draw_filter_line(f, row, fv, theme, hits);
        }
        return;
    }
    let bar = theme.style(Role::StatusBar);
    let w = usize::from(rect.width);
    hits.push(rect, HitTarget::Status);

    // Left: the filter field, then the counts.
    let mut spans = Vec::new();
    let mut cursor = None;
    let mut used = 0;
    if let Some(fv) = &m.filter {
        let (s, c) = filter_spans(fv, rect.width, theme);
        used = s.iter().map(|s| text::width(&s.content)).sum::<usize>();
        if used <= w {
            let field = Rect { width: used as u16, ..rect };
            hits.push(field, HitTarget::Filter);
            spans = s;
            cursor = c;
        } else {
            used = 0;
        }
    }
    let mut counts = if used == 0 { format!(" {}", m.counts) } else { m.counts.clone() };
    if !m.hidden.is_empty() {
        counts.push(' ');
        counts.push_str(&m.hidden);
    }
    if !m.selected.is_empty() {
        counts.push_str(" · ");
        counts.push_str(&m.selected);
    }
    let counts = text::truncate_end(&counts, w - used);
    used += text::width(&counts);
    spans.push(Span::styled(counts, bar));
    f.render_widget(Paragraph::new(Line::from(spans)).style(bar), rect);
    if let Some(c) = cursor.filter(|&c| c < rect.width) {
        f.set_cursor_position(Position { x: rect.x + c, y: rect.y });
        hits.set_caret(Position { x: rect.x + c, y: rect.y });
    }

    // Right: smart folder, free space, view mode, sort, hint; the last ones go first when
    // it is narrow.
    let mut right: Vec<&str> = [&m.smart, &m.free, &m.mode, &m.sort, &m.hint]
        .into_iter()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    while !right.is_empty() {
        let s = format!("{} ", right.join(" · "));
        let sw = text::width(&s);
        if sw + 2 <= w - used {
            let r = Rect { x: rect.right() - sw as u16, width: sw as u16, ..rect };
            f.render_widget(Paragraph::new(s).style(bar), r);
            break;
        }
        right.pop();
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    use super::*;

    fn vm() -> StatusVm {
        StatusVm {
            visible: true,
            counts: "3 of 10 items".into(),
            hidden: String::new(),
            selected: "1 selected  (2 KB)".into(),
            smart: String::new(),
            free: "12 GB free".into(),
            mode: "Details".into(),
            sort: "Name ↑".into(),
            hint: "Ctrl+Q quit".into(),
            filter: None,
        }
    }

    fn draw(m: &StatusVm, w: u16, h: u16, status: bool) -> (Vec<String>, Terminal<TestBackend>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        let rect = if status { Rect::new(0, h - 1, w, 1) } else { Rect::new(0, h, 0, 0) };
        t.draw(|f| draw_status(f, rect, m, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        (lines, t, hits)
    }

    #[test]
    fn counts_and_right_side() {
        let (lines, _, hits) = draw(&vm(), 90, 1, true);
        assert_eq!(
            lines[0].trim_end(),
            " 3 of 10 items · 1 selected  (2 KB)           12 GB free · Details · Name ↑ · Ctrl+Q quit"
        );
        assert_eq!(hits.at(3, 0), Some(HitTarget::Status));
        // Narrow: the hint, then the sort go first; the counts stay.
        let (lines, _, _) = draw(&vm(), 70, 1, true);
        assert_eq!(lines[0].trim_end(), " 3 of 10 items · 1 selected  (2 KB)     12 GB free · Details · Name ↑");
        let (lines, _, _) = draw(&vm(), 20, 1, true);
        assert_eq!(lines[0], " 3 of 10 items · 1 …");
        // Hidden counts follow the item count.
        let m = StatusVm { hidden: "· 2 hidden".into(), selected: String::new(), ..vm() };
        let (lines, _, _) = draw(&m, 40, 1, true);
        assert!(lines[0].starts_with(" 3 of 10 items · 2 hidden "), "{:?}", lines[0]);
    }

    #[test]
    fn filter_field_with_cursor() {
        let m =
            StatusVm { filter: Some(FilterVm { label: "Filter".into(), text: "ab".into(), cursor: Some(2) }), ..vm() };
        let (lines, mut t, hits) = draw(&m, 60, 1, true);
        // " Filter " + a 20-cell box + " " + counts.
        assert!(lines[0].starts_with(" Filter ab                   3 of 10 items"), "{:?}", lines[0]);
        let buf = t.backend().buffer().clone();
        // The bar is reversed, the box is not, the cursor cell is.
        assert!(buf[(0, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(8, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(10, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(11, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(t.get_cursor_position().unwrap(), Position { x: 10, y: 0 });
        assert_eq!(hits.at(12, 0), Some(HitTarget::Filter));
        assert_eq!(hits.at(40, 0), Some(HitTarget::Status));
        // The cursor inside the text reverses the character under it.
        let mid =
            StatusVm { filter: Some(FilterVm { label: "Filter".into(), text: "abc".into(), cursor: Some(1) }), ..vm() };
        let (lines, t, _) = draw(&mid, 60, 1, true);
        assert!(lines[0].starts_with(" Filter abc                  3 of 10 items"), "{:?}", lines[0]);
        let buf = t.backend().buffer();
        assert!(!buf[(8, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(9, 0)].modifier.contains(Modifier::REVERSED) && buf[(9, 0)].symbol() == "b");
        assert!(!buf[(10, 0)].modifier.contains(Modifier::REVERSED));
        // A set filter without the field open: the text, no cursor cell.
        let m = StatusVm { filter: Some(FilterVm { cursor: None, ..m.filter.clone().unwrap() }), ..vm() };
        let (lines, t, _) = draw(&m, 60, 1, true);
        assert!(lines[0].starts_with(" Filter ab 3 of 10 items"), "{:?}", lines[0]);
        assert!(!t.backend().buffer()[(9, 0)].modifier.contains(Modifier::REVERSED));
        assert!(t.backend().buffer()[(10, 0)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn hidden_status_line_still_shows_an_open_filter() {
        let open = FilterVm { label: "Filter".into(), text: "x".into(), cursor: Some(1) };
        let m = StatusVm { visible: false, filter: Some(open.clone()), ..vm() };
        let (lines, _, hits) = draw(&m, 30, 3, false);
        assert!(lines[2].starts_with(" Filter x"), "{lines:?}");
        assert!(lines[0].trim().is_empty() && lines[1].trim().is_empty());
        assert_eq!(hits.at(3, 2), Some(HitTarget::Filter));
        // Closed: nothing at all.
        let m = StatusVm { visible: false, filter: Some(FilterVm { cursor: None, ..open }), ..vm() };
        let (lines, _, hits) = draw(&m, 30, 3, false);
        assert!(lines.iter().all(|l| l.trim().is_empty()) && hits.is_empty());
    }

    #[test]
    fn the_smart_folder_segment_leads_the_right_side() {
        let m = StatusVm { smart: "Photos · 12 files".into(), free: String::new(), ..vm() };
        let (lines, _, _) = draw(&m, 100, 1, true);
        assert!(lines[0].trim_end().ends_with("Photos · 12 files · Details · Name ↑ · Ctrl+Q quit"), "{:?}", lines[0]);
        // Narrow: it stays while the others go.
        let (lines, _, _) = draw(&m, 60, 1, true);
        assert!(lines[0].trim_end().ends_with("Photos · 12 files"), "{:?}", lines[0]);
    }

    #[test]
    fn tiny_widths_never_panic() {
        let full = StatusVm {
            hidden: "· 2 hidden".into(),
            filter: Some(FilterVm { label: "Filter".into(), text: "日本語".into(), cursor: Some(6) }),
            ..vm()
        };
        for m in [vm(), full.clone(), StatusVm { visible: false, ..full }] {
            for w in 0..=24 {
                for status in [true, false] {
                    let h = 2;
                    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                    let rect = if status { Rect::new(0, 1, w, 1) } else { Rect::new(0, h, w, 0) };
                    t.draw(|f| draw_status(f, rect, &m, &TuiTheme::plain(), &mut Hits::default())).unwrap();
                }
            }
        }
    }
}
