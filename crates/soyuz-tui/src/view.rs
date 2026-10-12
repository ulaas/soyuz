//! The minimal pane view of Phase T1: a path line, the focused pane's rows, toasts, a
//! status line and the dialog fallback.
//!
//! [`ViewModel::from_core`] turns the core's state into plain data (no ratatui types,
//! easy to test); [`draw`] renders it. Plain theme for now (T2 brings colours): the
//! cursor row is reversed, selected rows carry a `*`, directories a trailing `/` and
//! symlinks to files a trailing `@`. All names pass through [`crate::dialog::safe`], so
//! a file name can never write escape sequences to the terminal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::toasts::ToastKind;
use soyuz_appcore::state::pane::ListingStatus;
use soyuz_appcore::tr;
use soyuz_core::{EntryKind, Location};

use crate::dialog::{DialogView, safe};

/// What kind of item a row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Dir,
    /// A symlink to a file (links to folders count as [`RowKind::Dir`]).
    Link,
    File,
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Display name (extension hidden when `[general] show_extensions` is off).
    pub name: String,
    pub kind: RowKind,
    /// Formatted size (empty for folders).
    pub size: String,
    pub cursor: bool,
    pub selected: bool,
}

impl Row {
    /// The name with its kind marker (`docs/`, `link@`).
    pub fn label(&self) -> String {
        match self.kind {
            RowKind::Dir => format!("{}/", self.name),
            RowKind::Link => format!("{}@", self.name),
            RowKind::File => self.name.clone(),
        }
    }
}

/// A toast line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastLine {
    pub kind: ToastKind,
    pub text: String,
}

/// Everything one frame draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewModel {
    /// The focused pane's location.
    pub path: String,
    /// The rows in view (from `top`).
    pub rows: Vec<Row>,
    /// Index (in the pane's visible order) of the first row shown.
    pub top: usize,
    /// Items in the listing (after filters).
    pub total: usize,
    /// Shown instead of rows: loading, empty, failed, not available.
    pub message: Option<String>,
    /// "12 items · 2 selected".
    pub counts: String,
    /// Key hint on the status line.
    pub hint: String,
    /// Newest last.
    pub toasts: Vec<ToastLine>,
    /// The top-most core prompt.
    pub dialog: Option<DialogView>,
}

/// Toasts shown at once (the newest).
pub const MAX_TOASTS: usize = 3;

/// Rows the list gets in a terminal `height` rows tall with `toasts` toast lines (path
/// line and status line take one row each).
pub fn list_rows(height: u16, toasts: usize) -> usize {
    usize::from(height).saturating_sub(2 + toasts.min(MAX_TOASTS)).max(1)
}

/// The first row to show so the cursor stays in view, starting from last frame's `top`.
pub fn scroll_top(prev_top: usize, cursor: Option<usize>, total: usize, rows: usize) -> usize {
    let rows = rows.max(1);
    let mut top = prev_top.min(total.saturating_sub(rows));
    if let Some(c) = cursor {
        if c < top {
            top = c;
        } else if c >= top + rows {
            top = c + 1 - rows;
        }
    }
    top
}

/// The location as a path line.
fn path_line(loc: &Location) -> String {
    match loc {
        Location::Dir(p) => safe(&soyuz_fs::strip_verbatim(p).display().to_string()),
        other => safe(&soyuz_appcore::state::crumbs::location_title(other)),
    }
}

impl ViewModel {
    /// Build the frame's data from the core: `rows` list rows, last frame's `prev_top`.
    pub fn from_core(core: &AppCore, rows: usize, prev_top: usize) -> ViewModel {
        let pane = core.ws.pane();
        let total = pane.visible.len();
        let cursor = pane.cursor_pos();
        let top = scroll_top(prev_top, cursor, total, rows);
        let show_ext = core.config.general.show_extensions;
        let size_style = core.config.general.size_style;
        let list: Vec<Row> = (top..total.min(top + rows))
            .filter_map(|i| Some((i, pane.entry_at(i)?, pane.key_at(i)?)))
            .map(|(i, e, key)| {
                let kind = if e.is_dir_like() {
                    RowKind::Dir
                } else if matches!(e.kind, EntryKind::Symlink { .. }) {
                    RowKind::Link
                } else {
                    RowKind::File
                };
                let name = if show_ext || kind == RowKind::Dir { e.name.as_str() } else { e.stem() };
                Row {
                    name: safe(name),
                    kind,
                    size: if kind == RowKind::Dir {
                        String::new()
                    } else {
                        soyuz_core::size::format_size(e.size, size_style)
                    },
                    cursor: cursor == Some(i),
                    selected: pane.selection.contains(key),
                }
            })
            .collect();
        let message = match &pane.status {
            ListingStatus::Loading { .. } if total == 0 => Some(tr!("tui-loading").to_string()),
            ListingStatus::Failed(e) => Some(tr!("tui-failed", error = safe(&e.to_string()))),
            ListingStatus::Virtual => Some(tr!("tui-virtual").to_string()),
            _ if total == 0 && pane.is_loading() => Some(tr!("tui-loading").to_string()),
            _ if total == 0 && pane.filter_active() => Some(tr!("tui-no-matches").to_string()),
            _ if total == 0 => Some(tr!("tui-empty").to_string()),
            _ => None,
        };
        let mut counts = tr!("tui-items", count = total);
        let selected = pane.selection.len();
        if selected > 0 {
            counts.push_str(" · ");
            counts.push_str(&tr!("tui-selected", count = selected));
        }
        let live: Vec<_> = core.ui.toasts.live.iter().filter(|l| l.closing.is_none()).collect();
        let toasts = live[live.len().saturating_sub(MAX_TOASTS)..]
            .iter()
            .map(|l| {
                let mut text = safe(&l.toast.title);
                if let Some(b) = &l.toast.body {
                    text.push_str(" — ");
                    text.push_str(&safe(b));
                }
                ToastLine { kind: l.toast.kind, text }
            })
            .collect();
        ViewModel {
            path: path_line(&pane.location),
            rows: list,
            top,
            total,
            message,
            counts,
            hint: tr!("tui-hint").to_string(),
            toasts,
            dialog: core.dialogs.top().map(crate::dialog::fallback),
        }
    }
}

/// Width of the size column.
const SIZE_W: u16 = 11;

fn toast_tag(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Info => "[i]",
        ToastKind::Success => "[ok]",
        ToastKind::Warning => "[!]",
        ToastKind::Error => "[x]",
    }
}

/// Draw a frame.
pub fn draw(f: &mut Frame<'_>, m: &ViewModel) {
    let area = f.area();
    let toast_h = m.toasts.len().min(MAX_TOASTS) as u16;
    let [path_area, list_area, toast_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(toast_h),
        Constraint::Length(1),
    ])
    .areas(area);

    f.render_widget(Paragraph::new(m.path.as_str()).style(Style::new().add_modifier(Modifier::BOLD)), path_area);

    if let Some(msg) = &m.message
        && m.rows.is_empty()
    {
        f.render_widget(Paragraph::new(msg.as_str()).alignment(Alignment::Center), list_area);
    }
    for (i, row) in m.rows.iter().enumerate().take(usize::from(list_area.height)) {
        let r = Rect { y: list_area.y + i as u16, height: 1, ..list_area };
        let size_w = SIZE_W.min(r.width / 3);
        let [name_r, size_r] = Layout::horizontal([Constraint::Min(1), Constraint::Length(size_w)]).areas(r);
        let mark = if row.selected { "* " } else { "  " };
        f.render_widget(Paragraph::new(format!("{mark}{}", row.label())), name_r);
        f.render_widget(Paragraph::new(format!("{} ", row.size)).alignment(Alignment::Right), size_r);
        let mut style = Style::new();
        if row.selected {
            style = style.add_modifier(Modifier::BOLD);
        }
        if row.cursor {
            style = style.add_modifier(Modifier::REVERSED);
        }
        f.buffer_mut().set_style(r, style);
    }

    let lines: Vec<Line<'_>> = m
        .toasts
        .iter()
        .map(|t| {
            let line = Line::from(format!("{} {}", toast_tag(t.kind), t.text));
            if matches!(t.kind, ToastKind::Warning | ToastKind::Error) {
                line.style(Style::new().add_modifier(Modifier::BOLD))
            } else {
                line
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), toast_area);

    let bar = Style::new().add_modifier(Modifier::REVERSED);
    f.render_widget(Paragraph::new(format!(" {}", m.counts)).style(bar), status_area);
    f.render_widget(Paragraph::new(format!("{} ", m.hint)).alignment(Alignment::Right).style(bar), status_area);

    if let Some(d) = &m.dialog {
        draw_dialog(f, area, d);
    }
}

/// The dialog fallback: a bordered box in the middle with the body and the key hints.
fn draw_dialog(f: &mut Frame<'_>, area: Rect, d: &DialogView) {
    let width = area.width.saturating_sub(4).clamp(1, 72);
    let inner_w = usize::from(width.saturating_sub(2)).max(1);
    // Wrapped body lines (rough count: by characters), one blank line, the hints.
    let body_h: usize = d.body.iter().map(|l| l.chars().count().div_ceil(inner_w).max(1)).sum();
    let height = ((body_h + 2) as u16 + 2).min(area.height);
    let r = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    f.render_widget(Clear, r);
    let mut lines: Vec<Line<'_>> = d.body.iter().map(|l| Line::from(l.as_str())).collect();
    lines.push(Line::from(""));
    lines.push(Line::from(d.hints.as_str()).style(Style::new().add_modifier(Modifier::BOLD)));
    let block = Block::new().borders(Borders::ALL).title(format!(" {} ", d.title));
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), r);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn screen(m: &ViewModel, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, m)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect();
        (lines, buf)
    }

    fn row(name: &str, kind: RowKind, cursor: bool, selected: bool) -> Row {
        Row {
            name: name.into(),
            kind,
            size: if kind == RowKind::Dir { String::new() } else { "1 KiB".into() },
            cursor,
            selected,
        }
    }

    fn model() -> ViewModel {
        ViewModel {
            path: "/data".into(),
            rows: vec![
                row("sub", RowKind::Dir, false, false),
                row("a.txt", RowKind::File, true, false),
                row("b.txt", RowKind::File, false, true),
            ],
            top: 0,
            total: 3,
            message: None,
            counts: "3 items · 1 selected".into(),
            hint: "Ctrl+Q quit".into(),
            toasts: Vec::new(),
            dialog: None,
        }
    }

    #[test]
    fn list_with_cursor_and_selection() {
        let (lines, buf) = screen(&model(), 40, 8);
        assert_eq!(lines[0], "/data");
        assert_eq!(lines[1], "  sub/");
        assert!(lines[2].starts_with("  a.txt") && lines[2].ends_with("1 KiB"), "{:?}", lines[2]);
        assert!(lines[3].starts_with("* b.txt"), "{:?}", lines[3]);
        assert!(lines[7].starts_with(" 3 items · 1 selected") && lines[7].ends_with("Ctrl+Q quit"), "{:?}", lines[7]);
        // The cursor row is reversed across the whole width, the others are not.
        assert!(buf[(0, 2)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(39, 2)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(0, 1)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(2, 3)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn empty_message_and_toasts() {
        let mut m = model();
        m.rows.clear();
        m.total = 0;
        m.message = Some("This folder is empty".into());
        m.toasts = vec![
            ToastLine { kind: ToastKind::Info, text: "Copied".into() },
            ToastLine { kind: ToastKind::Error, text: "Failed — why".into() },
        ];
        let (lines, buf) = screen(&m, 40, 8);
        assert!(lines[1].contains("This folder is empty"), "{lines:?}");
        assert_eq!(lines[5], "[i] Copied");
        assert_eq!(lines[6], "[x] Failed — why");
        assert!(buf[(0, 6)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn dialog_fallback_box() {
        let mut m = model();
        m.dialog = Some(DialogView {
            title: "Move “a.txt” to the trash?".into(),
            body: vec!["You can restore it from the trash.".into()],
            hints: "Enter/Y confirm · N/Esc cancel".into(),
        });
        let (lines, _) = screen(&m, 60, 12);
        let all = lines.join("\n");
        assert!(all.contains("Move “a.txt” to the trash?"), "{all}");
        assert!(all.contains("You can restore it from the trash."), "{all}");
        assert!(all.contains("Enter/Y confirm · N/Esc cancel"), "{all}");
    }

    #[test]
    fn scrolling_keeps_the_cursor_in_view() {
        assert_eq!(scroll_top(0, Some(3), 100, 10), 0);
        assert_eq!(scroll_top(0, Some(10), 100, 10), 1);
        assert_eq!(scroll_top(50, Some(10), 100, 10), 10);
        assert_eq!(scroll_top(95, None, 100, 10), 90);
        assert_eq!(scroll_top(5, Some(0), 3, 10), 0);
        assert_eq!(list_rows(24, 0), 22);
        assert_eq!(list_rows(24, 5), 19);
        assert_eq!(list_rows(1, 0), 1);
    }
}
