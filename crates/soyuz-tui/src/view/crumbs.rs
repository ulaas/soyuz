//! The location line: the focused pane's breadcrumbs, or the address field while it is
//! edited (with the completion popup below it).
//!
//! Breadcrumbs are `soyuz_appcore::state::crumbs::crumbs` segments joined by separators;
//! when they do not fit, the leading segments give way to an ellipsis (the folder you are
//! in always stays, emphasised). Each segment is a click target (`HitTarget::Crumb(i)`,
//! `i` indexing [`CrumbsVm::segments`]); the rest of the line is `HitTarget::Address`.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::state::crumbs::{self, CrumbKind};

use super::text;
use crate::dialog::safe;
use crate::icons::{IconSet, glyph};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};

/// The location line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrumbsVm {
    /// Breadcrumb labels, root first (terminal-safe).
    pub segments: Vec<String>,
    /// The address field, while it is edited.
    pub edit: Option<AddressVm>,
}

/// The address field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressVm {
    /// The typed text (terminal-safe).
    pub text: String,
    /// Cursor, a byte offset in `text`.
    pub cursor: usize,
    /// The field owns the keyboard (no dialog or palette above it): the cursor shows.
    pub focused: bool,
    /// Folder completions of the typed text.
    pub completions: Vec<String>,
    /// The highlighted completion.
    pub selected: Option<usize>,
}

impl CrumbsVm {
    /// One segment showing `label` (tests, places without crumbs).
    pub fn plain(label: &str) -> CrumbsVm {
        CrumbsVm { segments: vec![label.to_string()], edit: None }
    }

    /// The model of the focused pane's location; `address`: the TUI's address field state
    /// (its cursor).
    pub fn from_core(core: &AppCore, address: &crate::input::address::State) -> CrumbsVm {
        let loc = &core.ws.pane().location;
        let home = soyuz_fs::home();
        let mut segments: Vec<String> =
            crumbs::crumbs(loc, Some(&home)).into_iter().map(|c| segment_label(&c.label, c.kind)).collect();
        if segments.is_empty() {
            segments.push(safe(&crumbs::location_title(loc)));
        }
        let a = &core.ui.address;
        let edit = a.editing.then(|| {
            let comp = crumbs::completion_cached(&core.services, &a.text);
            let raw_cursor = address.cursor.filter(|&c| a.text.is_char_boundary(c));
            // `safe` may change the text (control characters); the cursor then goes to
            // the end.
            let text = safe(&a.text);
            let cursor = match raw_cursor {
                Some(c) if text == a.text => c,
                _ => text.len(),
            };
            AddressVm {
                text,
                cursor,
                focused: !core.dialogs.is_open() && !core.palette.open,
                completions: comp.names.iter().map(|n| safe(n)).collect(),
                selected: a.completion.filter(|&s| s < comp.names.len()),
            }
        });
        CrumbsVm { segments, edit }
    }
}

/// A crumb's label for the terminal.
fn segment_label(label: &str, kind: CrumbKind) -> String {
    match kind {
        // A remote location's server reads like a URI (`user@host:`).
        CrumbKind::Server => safe(&format!("{label}:")),
        _ => safe(label),
    }
}

/// Between two segments.
fn separator(set: IconSet) -> String {
    format!(" {} ", glyph(IconId::CaretRight, set))
}

/// The segments that fit into `width` cells: `(first shown, cut)`. Leading segments give
/// way first (`cut`: an ellipsis and a separator go first); the last one always stays.
fn fit(segments: &[String], width: usize, set: IconSet) -> (usize, bool) {
    let sep = text::width(&separator(set));
    let total: usize = segments.iter().map(|s| text::width(s)).sum::<usize>() + sep * segments.len().saturating_sub(1);
    if total <= width {
        return (0, false);
    }
    let ell = text::width(text::ELLIPSIS) + sep;
    let mut used = ell;
    let mut first = segments.len();
    for i in (0..segments.len()).rev() {
        let w = text::width(&segments[i]) + if i + 1 < segments.len() { sep } else { 0 };
        if used + w > width && first < segments.len() {
            break;
        }
        used += w;
        first = i;
    }
    (first.min(segments.len().saturating_sub(1)), true)
}

/// Draw the location line into `rect`.
pub fn draw_crumbs(f: &mut Frame<'_>, rect: Rect, m: &CrumbsVm, theme: &TuiTheme, hits: &mut Hits) {
    if rect.is_empty() {
        return;
    }
    hits.push(rect, HitTarget::Address);
    if let Some(e) = &m.edit {
        draw_field(f, rect, e, theme);
        return;
    }
    let width = usize::from(rect.width);
    let (first, cut) = fit(&m.segments, width, theme.icons());
    // Screen-reader mode (T8): a slash, not an arrow a reader calls "greater than" (the
    // same width, so `fit` holds).
    let sep = if theme.screen_reader() { " / ".to_string() } else { separator(theme.icons()) };
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut x = rect.x;
    let right = rect.right();
    let push = |spans: &mut Vec<Span<'static>>, x: &mut u16, s: String, role: Role| -> Rect {
        let w = text::width(&s).min(usize::from(right - *x)) as u16;
        let r = Rect::new(*x, rect.y, w, 1);
        *x += w;
        let style = theme.style(role);
        spans.push(Span::styled(s, style));
        r
    };
    if cut {
        push(&mut spans, &mut x, text::ELLIPSIS.to_string(), Role::Muted);
        push(&mut spans, &mut x, sep.clone(), Role::Faint);
    }
    let last = m.segments.len().saturating_sub(1);
    for (i, label) in m.segments.iter().enumerate().skip(first) {
        let room = usize::from(right - x);
        if room == 0 {
            break;
        }
        let current = i == last;
        // Only the current folder can be too wide on its own: cut it in the middle.
        let shown = if current { text::truncate_middle(label, room) } else { text::truncate_end(label, room) };
        // The current folder is emphasised (`Title`: bold even in monochrome).
        let r = push(&mut spans, &mut x, shown, if current { Role::Title } else { Role::Muted });
        hits.push(r, HitTarget::Crumb(i));
        if !current {
            let room = usize::from(right - x);
            // Screen-reader mode: no second slash after a root that ends in one ("/",
            // "C:\"); blanks of the same width.
            let root = label.ends_with('/') || label.ends_with('\\');
            let sep = if theme.screen_reader() && root { " ".repeat(sep.len()) } else { sep.clone() };
            push(&mut spans, &mut x, text::truncate_end(&sep, room), Role::Faint);
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), rect);
}

/// The address field: the text scrolled so the cursor shows.
fn draw_field(f: &mut Frame<'_>, rect: Rect, e: &AddressVm, theme: &TuiTheme) {
    let width = usize::from(rect.width);
    let col = text::width(&e.text[..e.cursor.min(e.text.len())]);
    // Scroll so the cursor (and one cell after it) is visible.
    let scroll = (col + 1).saturating_sub(width);
    let shown = skip_cols(&e.text, scroll);
    let shown = text::fit(&shown, width);
    f.render_widget(Paragraph::new(shown).style(theme.style(Role::Input)), rect);
    if e.focused {
        let at = field_caret(rect, e);
        if let Some(cell) = f.buffer_mut().cell_mut(at) {
            cell.modifier.insert(Modifier::REVERSED);
        }
        f.set_cursor_position(at);
    }
}

/// The cell of the address field's cursor in `rect` (the text scrolled as [`draw_field`]
/// scrolls it).
fn field_caret(rect: Rect, e: &AddressVm) -> Position {
    let width = usize::from(rect.width);
    let col = text::width(&e.text[..e.cursor.min(e.text.len())]);
    let scroll = (col + 1).saturating_sub(width);
    Position { x: rect.x + (col - scroll).min(width.saturating_sub(1)) as u16, y: rect.y }
}

/// `s` without its first `cols` cells (whole graphemes; a wide one cut in half is
/// dropped).
fn skip_cols(s: &str, cols: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let mut x = 0;
    let mut out = String::new();
    for g in s.graphemes(true) {
        if x >= cols {
            out.push_str(g);
        }
        x += text::width(g);
    }
    out
}

/// Completion rows shown at most.
pub const POPUP_ROWS: usize = soyuz_appcore::state::crumbs::COMPLETION_LIMIT;

/// The completion popup below the field `rect`, inside `area`. Drawn after the panes, so
/// it lies on top of them.
pub fn draw_completions(f: &mut Frame<'_>, rect: Rect, area: Rect, m: &CrumbsVm, theme: &TuiTheme, hits: &mut Hits) {
    let Some(e) = &m.edit else { return };
    // Recorded here, after the panes are drawn (the field is drawn before them): the
    // focused field's caret (screen-reader mode, `Hits::set_caret`).
    if e.focused && !rect.is_empty() {
        hits.set_caret(field_caret(rect, e));
    }
    if e.completions.is_empty() || !e.focused || rect.is_empty() {
        return;
    }
    let below = area.bottom().saturating_sub(rect.bottom());
    let rows = e.completions.len().min(POPUP_ROWS).min(usize::from(below));
    if rows == 0 {
        return;
    }
    let hint = "Tab";
    let tab_row = e.selected.unwrap_or(0);
    let widest = e.completions.iter().map(|n| text::width(n) + 1).max().unwrap_or(0);
    let want = (widest + 2 + text::width(hint) + 2).max(20);
    let w = want.min(usize::from(rect.width)) as u16;
    let popup = Rect::new(rect.x, rect.bottom(), w, rows as u16).intersection(area);
    f.render_widget(Clear, popup);
    for (i, name) in e.completions.iter().take(rows).enumerate() {
        let row = Rect::new(popup.x, popup.y + i as u16, popup.width, 1);
        let inner = usize::from(row.width);
        let hint_w = if i == tab_row && inner > text::width(hint) + 4 { text::width(hint) + 1 } else { 0 };
        let label = text::fit(&format!(" {}{}", name, glyph(IconId::Folder, theme.icons())), inner - hint_w);
        let role = if e.selected == Some(i) { Role::PaletteSelected } else { Role::Input };
        let mut spans = vec![Span::styled(label, theme.style(role))];
        if hint_w > 0 {
            spans.push(Span::styled(format!("{hint} "), theme.style(Role::KeyHint)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(theme.style(role)), row);
    }
    hits.push(popup, HitTarget::Address);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn segs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn draw(m: &CrumbsVm, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer, Hits, Option<Position>) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        let mut cursor = None;
        t.draw(|f| {
            let area = f.area();
            let rect = Rect::new(0, 0, w, 1.min(h));
            draw_crumbs(f, rect, m, &TuiTheme::plain(), &mut hits);
            draw_completions(f, rect, area, m, &TuiTheme::plain(), &mut hits);
        })
        .unwrap();
        if m.edit.as_ref().is_some_and(|e| e.focused) {
            cursor = t.get_cursor_position().ok();
        }
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, buf, hits, cursor)
    }

    #[test]
    fn segments_with_separators_and_targets() {
        let m = CrumbsVm { segments: segs(&["C:", "Users", "me"]), edit: None };
        let (lines, buf, hits, _) = draw(&m, 40, 1);
        assert_eq!(lines[0], "C: > Users > me");
        assert_eq!(hits.at(0, 0), Some(HitTarget::Crumb(0)));
        assert_eq!(hits.at(6, 0), Some(HitTarget::Crumb(1)));
        assert_eq!(hits.at(13, 0), Some(HitTarget::Crumb(2)));
        // Separators and the empty rest of the line open the field.
        assert_eq!(hits.at(3, 0), Some(HitTarget::Address));
        assert_eq!(hits.at(30, 0), Some(HitTarget::Address));
        // The current folder is emphasised, the others are not.
        assert!(buf[(13, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buf[(6, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn long_paths_lose_leading_segments() {
        let m = CrumbsVm { segments: segs(&["C:", "Users", "someone", "projects", "soyuz"]), edit: None };
        let (lines, _, hits, _) = draw(&m, 24, 1);
        assert_eq!(lines[0], "… > projects > soyuz");
        assert_eq!(hits.at(4, 0), Some(HitTarget::Crumb(3)));
        assert_eq!(hits.at(16, 0), Some(HitTarget::Crumb(4)));
        // Not even the current folder fits: it is cut in the middle.
        let m = CrumbsVm { segments: segs(&["/", "a_very_long_folder_name"]), edit: None };
        let (lines, _, _, _) = draw(&m, 14, 1);
        assert_eq!(text::width(&lines[0]), 14, "{:?}", lines[0]);
        assert!(lines[0].starts_with("… > "), "{:?}", lines[0]);
        // Every width down to zero draws without panicking.
        for w in 0..30 {
            let _ = draw(&CrumbsVm { segments: segs(&["/", "日本語", "a", "b"]), edit: None }, w, 1);
        }
    }

    #[test]
    fn field_with_cursor_and_popup() {
        let e = AddressVm {
            text: "/data/su".into(),
            cursor: 8,
            focused: true,
            completions: segs(&["sub", "sun"]),
            selected: Some(1),
        };
        let m = CrumbsVm { segments: segs(&["/", "data"]), edit: Some(e.clone()) };
        let (lines, buf, hits, cursor) = draw(&m, 30, 5);
        assert_eq!(lines[0], "/data/su");
        assert_eq!(cursor, Some(Position { x: 8, y: 0 }));
        assert!(buf[(8, 0)].modifier.contains(Modifier::REVERSED));
        assert!(lines[1].starts_with(" sub/"), "{:?}", lines[1]);
        assert!(lines[2].starts_with(" sun/") && lines[2].ends_with("Tab"), "{:?}", lines[2]);
        assert!(buf[(1, 2)].modifier.contains(Modifier::REVERSED), "the highlighted completion");
        assert_eq!(lines[3], "");
        assert_eq!(hits.at(2, 1), Some(HitTarget::Address));
        // A long text scrolls so the cursor stays visible.
        let long = AddressVm { text: format!("/{}", "x".repeat(40)), cursor: 41, ..e.clone() };
        let (lines, _, _, cursor) = draw(&CrumbsVm { segments: vec![], edit: Some(long) }, 10, 1);
        assert_eq!(lines[0], "xxxxxxxxx");
        assert_eq!(cursor, Some(Position { x: 9, y: 0 }));
        // Without the keyboard (a dialog above): no cursor, no popup.
        let idle = AddressVm { focused: false, ..e };
        let (lines, _, _, cursor) = draw(&CrumbsVm { segments: vec![], edit: Some(idle) }, 30, 5);
        assert_eq!((lines[1].as_str(), cursor), ("", None));
        // Tiny sizes.
        for w in 0..12 {
            for h in 0..4 {
                let _ = draw(&m, w, h);
            }
        }
    }

    #[test]
    fn fit_keeps_the_tail() {
        let s = segs(&["aaaa", "bbbb", "cccc"]);
        let a = IconSet::Ascii;
        assert_eq!(fit(&s, 100, a), (0, false));
        assert_eq!(fit(&s, 18, a), (0, false));
        assert_eq!(fit(&s, 17, a), (1, true));
        assert_eq!(fit(&s, 5, a), (2, true));
        assert_eq!(fit(&[], 0, a), (0, false));
    }
}
