//! Screen-reader mode (Phase T8): `[tui] screen_reader`, the palette's "Toggle
//! screen-reader mode" (`Action::ToggleScreenReader`) and the run-only `--screen-reader`
//! override (`App::sr_override`, never written to the config).
//!
//! When it is on ([`active`]):
//! - the theme says so (`TuiTheme::screen_reader`: no box lines, the ASCII icon set) and
//!   the views draw their linear, box-free variants: no borders, splitters, icons or
//!   images; `*` marks selected rows, folders get a trailing `/`, no `>` markers; overlays
//!   (dialogs, menus, the palette) clear the rows they sit on, so no row mixes their text
//!   with the panes'. Whatever box drawing, block or braille character is still on the
//!   screen after the views drew is blanked ([`strip`]);
//! - the layout (`Facts::screen_reader`, `layout::compute_screen_reader`) draws only the
//!   focused pane, plus the "Other pane: <path>" line ([`draw`], `Regions::other_pane`,
//!   `ViewModel::other_pane`); the status line always takes the bottom row;
//! - each view records the focused element's cell (`Hits::set_caret`) and [`draw`] puts
//!   the terminal's cursor there ([`caret`]), so a reader that follows the cursor (Orca,
//!   NVDA review, magnifiers) reads that line;
//! - the status line is one sentence that starts with the focused item
//!   ([`status_sentence`]: "b.txt, file, 1 KB, 2 of 3; 12 items; 12 GB free; Details,
//!   sorted by Name, ascending"), or the newest toast ("Warning: title. body") while one
//!   is live: toasts get no boxes ([`status_line`]). When the line changes it is rewritten
//!   in full after the frame ([`after_frame`]): a reader that speaks changed text
//!   (Narrator, NVDA) then reads the whole line, not the suffix ratatui's diff wrote. The
//!   row is drawn in the terminal's own colours (`Style::reset`) so the rewrite matches
//!   the buffer cell for cell;
//! - toasts last at least their unscaled duration ([`toast_dt`]).

use std::io;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::toasts::ToastKind;
use soyuz_appcore::tr;
use soyuz_core::config::TuiSection;

use crate::app::TermIo;
use crate::dialog::safe;
use crate::input::TuiFocus;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;
use crate::view::palette::PaletteLine;
use crate::view::{Row, RowKind, ToastLine, ViewModel, text};
use crate::widgets::MenuItem;
use crate::widgets::form::FieldVmKind;

/// Screen-reader mode is on: the setting, or the run-only `--screen-reader` override.
pub fn active(tui: &TuiSection, run_override: bool) -> bool {
    tui.screen_reader || run_override
}

/// What screen-reader mode keeps between frames (`App::a11y`).
#[derive(Debug, Default)]
pub struct State {
    /// The status line written last ([`after_frame`]).
    last_status: String,
}

/// The "Other pane: <path>" text: the folders of the tab's shown panes that are not drawn
/// (screen-reader mode, when the layout gave the line a row).
pub fn other_pane(core: &AppCore, regions: &Regions) -> Option<String> {
    if regions.other_pane.is_empty() {
        return None;
    }
    let tab = core.ws.tab();
    let others: Vec<String> = tab
        .panes
        .iter()
        .enumerate()
        .take(tab.shown())
        .filter(|(i, _)| *i != tab.focused)
        .map(|(_, p)| safe(&p.location.display()))
        .collect();
    (!others.is_empty()).then(|| tr!("tui-sr-other-pane", path = others.join("; ")))
}

/// A list row read out: "b.txt, file, 1 KB, 2 of 3" (", selected" when it is).
fn item(row: &Row, total: usize) -> String {
    let kind = match row.kind {
        RowKind::Dir => tr!("tui-sr-folder"),
        RowKind::Link => tr!("tui-sr-link"),
        RowKind::File => tr!("tui-sr-file"),
    };
    let (pos, total) = (row.index + 1, total.max(row.index + 1));
    let mut s = if row.size.is_empty() {
        tr!("tui-sr-item", name = row.name.as_str(), kind = kind, pos = pos, total = total)
    } else {
        tr!(
            "tui-sr-item-size",
            name = row.name.as_str(),
            kind = kind,
            size = row.size.as_str(),
            pos = pos,
            total = total
        )
    };
    if row.selected {
        s.push_str(", ");
        s.push_str(tr!("tui-sr-selected"));
    }
    s
}

/// The focused element read out, for the start of the status sentence (`None` for text
/// fields, whose typing must not make the line change with every key, and for panels
/// that place the cursor themselves).
fn lead(m: &ViewModel, focus: Option<TuiFocus>) -> Option<String> {
    match focus.unwrap_or(TuiFocus::List) {
        TuiFocus::List => {
            let p = m.focused_pane()?;
            if let Some(pl) = &p.places {
                return pl.rows.iter().find(|r| r.cursor).map(|r| r.name.clone()).or_else(|| Some(pl.title.clone()));
            }
            p.rows().find(|r| r.cursor).map(|r| item(r, p.total)).or_else(|| p.message.clone())
        }
        TuiFocus::Sidebar => m.sidebar.lines.iter().find(|l| l.cursor).map(|l| l.label.clone()),
        TuiFocus::Palette => {
            let row = m.palette.lines.iter().find_map(|l| match l {
                PaletteLine::Item(r) if r.selected => Some(r.title.iter().map(|(s, _)| s.as_str()).collect()),
                _ => None,
            });
            row.or_else(|| m.palette.lines.is_empty().then(|| m.palette.empty.clone()))
        }
        TuiFocus::Menu => {
            let mut level = m.menu.menu.as_ref();
            let mut label = None;
            while let Some(menu) = level {
                if let Some(MenuItem::Entry(e)) = menu.cursor.and_then(|c| menu.items.get(c)) {
                    label = Some(e.label.clone());
                }
                level = menu.sub.as_deref();
            }
            label
        }
        TuiFocus::Dialog => {
            let d = m.dialog.as_ref()?;
            let part = match d.fields.get(d.focus) {
                Some(f) => match &f.kind {
                    FieldVmKind::Text { .. } => Some(f.label.clone()),
                    FieldVmKind::Check(on) => {
                        let state = if *on { tr!("tui-sr-checked") } else { tr!("tui-sr-unchecked") };
                        Some(format!("{}, {state}", f.label))
                    }
                    FieldVmKind::Radio { options, selected } => {
                        Some(format!("{}, {}", f.label, options.get(*selected).map_or("", String::as_str)))
                    }
                },
                None => d.focus.checked_sub(d.fields.len()).and_then(|b| d.buttons.get(b)).map(|b| b.label.clone()),
            };
            Some(match part {
                // "Delete 3 items? Delete", not "…?: Delete".
                Some(p) if d.title.ends_with(['?', '!', '.', ':']) => format!("{} {p}", d.title),
                Some(p) if !d.title.is_empty() => format!("{}: {p}", d.title),
                Some(p) => p,
                None => d.title.clone(),
            })
        }
        TuiFocus::Settings => {
            let s = &m.settings;
            if s.popup.is_some() {
                return None;
            }
            if s.nav_focused {
                return s.nav.get(s.nav_cursor).cloned();
            }
            let l = s.lines.iter().find(|l| l.cursor && l.edit_cursor.is_none())?;
            Some(if l.value.is_empty() { l.label.clone() } else { format!("{}, {}", l.label, l.value) })
        }
        TuiFocus::Jobs => {
            let j = &m.jobs;
            j.cursor.checked_sub(j.first).and_then(|k| j.items.get(k)).map(|i| i.title.clone())
        }
        // The media player (Phase V3b): its state in words.
        TuiFocus::Preview => player_words(&m.preview.body),
        TuiFocus::QuickLook => player_words(&m.quicklook.preview.body),
        _ => None,
    }
}

/// The player's status in words when `body` is its body ("Playing, 0:12 of 1:30, volume
/// 80 %").
fn player_words(body: &crate::view::preview::Body) -> Option<String> {
    match body {
        crate::view::preview::Body::Player { status, .. } => Some(status.words()),
        _ => None,
    }
}

/// The sort read out: "sorted by Name, ascending" (from the status line's "Name ↑").
fn sort_words(sort: &str) -> Option<String> {
    let (column, dir) = if let Some(c) = sort.strip_suffix('↑') {
        (c, tr!("tui-sr-ascending"))
    } else if let Some(c) = sort.strip_suffix('↓') {
        (c, tr!("tui-sr-descending"))
    } else {
        return (!sort.is_empty()).then(|| sort.to_string());
    };
    Some(tr!("tui-sr-sorted", column = column.trim(), dir = dir))
}

/// The status line as one sentence, starting with the focused item ("b.txt, file, 1 KB,
/// 2 of 3; 12 items, 2 selected (1.5 KB); 12 GB free; Details, sorted by Name,
/// ascending"). `focus`: who has the keyboard (`Regions::focus`; `None`: the list).
pub fn status_sentence(m: &ViewModel, focus: Option<TuiFocus>) -> String {
    let s = &m.status;
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = lead(m, focus).filter(|l| !l.is_empty()) {
        parts.push(l);
    }
    let mut counts = s.counts.clone();
    for extra in [s.hidden.trim_start_matches(['·', ' ']), s.selected.as_str()] {
        if !extra.is_empty() {
            if !counts.is_empty() {
                counts.push_str(", ");
            }
            counts.push_str(&extra.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    parts.push(counts);
    parts.push(s.smart.clone());
    parts.push(s.free.clone());
    let view = match sort_words(&s.sort) {
        Some(sort) if !s.mode.is_empty() => format!("{}, {sort}", s.mode),
        Some(sort) => sort,
        None => s.mode.clone(),
    };
    parts.push(view);
    parts.retain(|p| !p.is_empty());
    parts.join("; ")
}

/// A toast read out: "Warning: title. body. Undo (Ctrl+Z)".
fn toast_text(t: &ToastLine) -> String {
    let kind = match t.kind {
        ToastKind::Info => tr!("tui-sr-info"),
        ToastKind::Success => tr!("tui-sr-success"),
        ToastKind::Warning => tr!("tui-sr-warning"),
        ToastKind::Error => tr!("tui-sr-error"),
    };
    let mut s = format!("{kind}: {}", t.title);
    for more in [&t.body, &t.action].into_iter().flatten() {
        s.push_str(". ");
        s.push_str(more);
    }
    s
}

/// What the status row shows in screen-reader mode: the newest toast while one is live,
/// else the status sentence; `None` while the filter field is open there (the status
/// view draws it).
pub fn status_line(m: &ViewModel, focus: Option<TuiFocus>) -> Option<String> {
    if m.status.filter.as_ref().is_some_and(|f| f.cursor.is_some()) {
        return None;
    }
    Some(match m.toasts.lines.last() {
        Some(t) => toast_text(t),
        None => status_sentence(m, focus),
    })
}

/// A box drawing (U+2500–U+257F), block (U+2580–U+259F) or braille (U+2800–U+28FF,
/// spinners) character: none is left on a screen-reader screen ([`strip`]).
pub fn is_box_char(c: char) -> bool {
    matches!(c, '\u{2500}'..='\u{259F}' | '\u{2800}'..='\u{28FF}')
}

/// Blank every cell of `buf` that shows a [`is_box_char`] character (lines become
/// spaces, bars and spinners go).
pub fn strip(buf: &mut Buffer) {
    for cell in buf.content.iter_mut() {
        if cell.symbol().chars().any(is_box_char) {
            cell.set_symbol(" ");
        }
    }
}

/// `s` written alone on the row `r` in the terminal's own colours (fitted to it).
fn write_line(buf: &mut Buffer, r: Rect, s: &str) {
    let r = r.intersection(buf.area);
    if r.is_empty() {
        return;
    }
    for x in r.left()..r.right() {
        buf[(x, r.y)].reset();
    }
    let s = text::truncate_end(s, usize::from(r.width));
    buf.set_stringn(r.x, r.y, &s, usize::from(r.width), Style::reset());
}

/// Where the terminal's cursor goes: the caret the views recorded (`Hits::set_caret`),
/// except while a panel that places the cursor itself has the keyboard (the editors, the
/// AI composer, the embedded terminal: `None`, their own cursor stays) and for the panels
/// that record none (the preview, Quick Look, the full-screen tools: their first row).
pub fn caret(regions: &Regions, m: &ViewModel, hits: &Hits) -> Option<Position> {
    let corner = |r: Rect| (!r.is_empty()).then_some(Position { x: r.x, y: r.y });
    match regions.focus {
        Some(TuiFocus::Editor | TuiFocus::Ai | TuiFocus::Terminal) => None,
        Some(TuiFocus::Preview) => corner(regions.preview),
        Some(TuiFocus::QuickLook) => corner(regions.area),
        Some(TuiFocus::Tool) if !m.compare.visible => corner(regions.area),
        _ => hits.caret(),
    }
}

/// Draw screen-reader mode's own parts after every view (does nothing outside it): blank
/// the remaining box characters, write the "Other pane: <path>" line and the status line
/// ([`status_line`]), then put the terminal's cursor on the caret ([`caret`]).
pub fn draw(f: &mut Frame<'_>, regions: &Regions, m: &ViewModel, theme: &TuiTheme, hits: &mut Hits) {
    if !theme.screen_reader() {
        return;
    }
    let buf = f.buffer_mut();
    strip(buf);
    if let Some(o) = &m.other_pane {
        write_line(buf, regions.other_pane, o);
    }
    if !regions.status.is_empty()
        && let Some(s) = status_line(m, regions.focus)
    {
        write_line(buf, regions.status, &s);
    }
    if let Some(at) = caret(regions, m, hits).filter(|p| f.area().contains(*p)) {
        f.set_cursor_position(at);
    }
}

/// The bytes that rewrite the status row `r` with `line` (already fitted): save the
/// cursor, move to the row, the terminal's own colours, the text, erase to the end of the
/// line, restore the cursor.
pub fn rewrite(r: Rect, line: &str) -> String {
    format!("\x1b7\x1b[{};{}H\x1b[0m{line}\x1b[K\x1b8", r.y + 1, r.x + 1)
}

/// After a frame was drawn and flushed: in screen-reader mode, rewrite the status line in
/// full when its text changed (through `io.write_raw`, [`rewrite`]: the cursor is saved
/// and restored around it, so it stays where the frame put it).
pub fn after_frame(
    state: &mut State,
    on: bool,
    m: &ViewModel,
    regions: &Regions,
    _hits: &Hits,
    io: &mut dyn TermIo,
) -> io::Result<()> {
    let r = regions.status;
    let line = if on && !r.is_empty() { status_line(m, regions.focus) } else { None };
    let Some(line) = line else {
        state.last_status.clear();
        return Ok(());
    };
    let line = text::truncate_end(&line, usize::from(r.width));
    if line == state.last_status {
        return Ok(());
    }
    io.write_raw(rewrite(r, &line).as_bytes())?;
    state.last_status = line;
    Ok(())
}

/// The time a frame takes off the toasts' timers. `scale` is `[appearance]
/// toast_time_scale`, already applied to each toast's duration when it was pushed: in
/// screen-reader mode a toast lasts at least its unscaled duration, so a reader can finish
/// speaking it (a shorter scale is undone; a longer one stays).
pub fn toast_dt(dt: f32, scale: f32, on: bool) -> f32 {
    if on && scale.is_finite() && scale > 0.0 && scale < 1.0 { dt * scale } else { dt }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_or_the_run_override_turns_it_on() {
        let mut tui = TuiSection::default();
        assert!(!active(&tui, false));
        assert!(active(&tui, true));
        tui.screen_reader = true;
        assert!(active(&tui, false));
    }

    #[test]
    fn toasts_keep_at_least_their_unscaled_duration() {
        // Outside screen-reader mode the frame time counts in full.
        assert_eq!(toast_dt(0.25, 0.5, false), 0.25);
        // A half-length toast timer counts down at half speed: the full duration.
        assert_eq!(toast_dt(0.25, 0.5, true), 0.125);
        // Longer toasts stay longer; odd scales change nothing.
        assert_eq!(toast_dt(0.25, 2.0, true), 0.25);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(toast_dt(0.25, bad, true), 0.25);
        }
    }

    #[test]
    fn box_characters_are_blanked() {
        let mut buf = Buffer::with_lines(["╭─╮ a│b █▀ ⠋ é→日"]);
        strip(&mut buf);
        let s: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(!s.chars().any(is_box_char), "{s:?}");
        assert_eq!(s, "    a b      é→日 ");
        assert!(!is_box_char('*') && !is_box_char('/') && is_box_char('│') && is_box_char('▐'));
    }

    #[test]
    fn sort_and_rewrite_sequences() {
        assert_eq!(sort_words("Name ↑").as_deref(), Some("sorted by Name, ascending"));
        assert_eq!(sort_words("Size ↓").as_deref(), Some("sorted by Size, descending"));
        assert_eq!(sort_words(""), None);
        assert_eq!(rewrite(Rect::new(0, 23, 80, 1), "hi"), "\x1b7\x1b[24;1H\x1b[0mhi\x1b[K\x1b8");
    }
}
