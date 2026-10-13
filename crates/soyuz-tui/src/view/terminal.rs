//! The view of the embedded terminal pane (Phase T5, `[tui] shell_mode = "pane"`): drawn into
//! `Regions::bottom` (shared with the jobs panel by tab; [`shown`] while the core's bottom
//! panel shows its Terminal tab), `[tui] terminal_rows` high ([`rows`]).
//!
//! ```text
//! ─ * pwsh x ─ o build x ─ [+] ──────── Esc at the prompt: back … ─ busy ─ ^> ─
//!  The terminal wants to set your clipboard: "secret"  [A] Allow once  [S] …  [D] Deny
//! PS C:\work> echo hi
//! hi
//! PS C:\work> _
//! ```
//!
//! Row 0 is the session strip (the GUI's mini-tabs: a status dot — alive, busy, exited —,
//! the label, a close mark; the new-session button; the "busy" hint when the shell refused
//! a `cd`; the external-terminal button), row 1 the OSC 52 prompt bar while a program
//! asks to set the clipboard (`terminal.osc52 = "ask"`), the rest the grid. The grid comes
//! from `soyuz_term::Terminal::snapshot` with the colours already resolved against the
//! terminal palette (`TuiTheme::terminal_palette`: `[terminal] theme`, `auto` = the design
//! tokens like the GUI) and reaches the screen through `[tui] color_mode`
//! (`TuiTheme::term_style`): truecolor as is, 256 colours by the nearest cube entry, 16
//! colours and mono by palette slot. Programs still see `TERM=xterm-256color`; the pane
//! quantises when it draws. Wide characters take two cells (the spacer is reset), the
//! selection is the palette's selection colour over the cell (reversed in 16 colours and
//! mono), the cursor a cursor-coloured cell plus the real terminal cursor while the pane
//! has the keyboard, an underline otherwise. While scrolled back, the top-right corner
//! shows how far (`^ 120`).
//!
//! The jobs panel shares the region: with `[layout] activity_panel = "bottom"` the two are
//! tabs of one panel; in the other placements the one opened last wins (Alt+J from the
//! terminal shows the jobs, Ctrl+` from the jobs the terminal).
//!
//! [`prepare`] (step 1 of a frame) applies the strip menu's commands
//! (`models::context_menu::take_terminal_tab_cmds`), starts a session when the shown pane
//! has none (like the GUI's `body`), resizes the active PTY to the grid rect every frame
//! (a no-op when unchanged), sets its palette and focus, and re-arms its output wake-up
//! (`mark_drawn`): the snapshot [`TerminalVm::from_core`] takes right after is what the
//! frame shows. Click targets: `TermTab`, `TermTabClose`, `TermNew`, `TermExternal`,
//! `TermClip(0..3)` and one `TermCell { row: 0, col: 0 }` over the whole grid (the cell is
//! computed from the pointer: `crate::input::terminal`).

use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::{Host, ThemePref, WindowCmd, WindowId};
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::context_menu::{apply_terminal_tab, take_terminal_tab_cmds};
use soyuz_appcore::models::terminal::{self as term, clipboard_preview, current_owner};
use soyuz_appcore::tr;
use soyuz_core::config::ActivityPlacement;
use soyuz_term::{CursorShape, GridSnapshot, GridStyle, TermPalette, Wake};

use crate::dialog::safe;
use crate::icons::{self, IconSet};
use crate::input::{InputState, TuiFocus};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{ColorMode, Role, TuiTheme, term_blend};
use crate::view::text;

/// Cells a session label takes at most in the strip.
pub const LABEL_MAX: usize = 20;
/// One session tab of the strip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionTabVm {
    pub id: u64,
    /// Label (terminal-safe).
    pub label: String,
    pub alive: bool,
    /// The shell refused a `cd` lately.
    pub busy: bool,
    pub active: bool,
}

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalVm {
    /// The pane is shown (else nothing is drawn).
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
    /// The session strip.
    pub tabs: Vec<SessionTabVm>,
    /// A tab being renamed: its id, the visible part of the text and the cursor column.
    pub renaming: Option<(u64, String, u16)>,
    /// The active session refused a `cd` lately.
    pub busy: bool,
    /// Key hints while focused.
    pub hints: String,
    /// The pending OSC 52 write's preview (the prompt bar is shown).
    pub clip: Option<String>,
    /// The active session's grid.
    pub grid: Option<GridSnapshot>,
    /// The active session's shell could not start.
    pub error: Option<String>,
    /// The palette the grid was resolved against (for the 16-colour and mono mapping).
    pub palette: TermPalette,
}

impl TerminalVm {
    pub fn from_core(core: &AppCore, input: &InputState, _regions: &Regions) -> TerminalVm {
        if !shown(core) {
            return TerminalVm::default();
        }
        let t = &core.ui.terminal;
        let owner = current_owner(core);
        let active = t.active_id(owner);
        let tabs = t
            .visible_ids(owner)
            .into_iter()
            .filter_map(|id| t.session(id))
            .map(|s| SessionTabVm {
                id: s.id,
                label: safe(&s.label()),
                alive: s.is_alive(),
                busy: s.is_busy(),
                active: Some(s.id) == active,
            })
            .collect();
        let renaming = t
            .renaming
            .as_ref()
            .and_then(|(id, _)| input.terminal.rename_view.as_ref().map(|(s, c)| (*id, safe(s), *c)));
        let session = active.and_then(|id| t.session(id));
        let palette = input.terminal.palette.clone().unwrap_or_default();
        let grid = session.and_then(|s| s.term.as_ref()).map(|term| term.snapshot(&palette, GridStyle::default()));
        let error = session.filter(|s| s.term.is_none()).map(|s| {
            let e = s.error.clone().unwrap_or_else(|| tr!("panels-terminal-unknown-error").to_string());
            safe(&tr!("panels-terminal-shell-failed", error = e))
        });
        let focused = crate::input::focus(core, input) == TuiFocus::Terminal;
        let clip = t.clip_request.as_ref().map(|text| {
            let p = clipboard_preview(text);
            if p.is_empty() { tr!("panels-terminal-clip-empty").to_string() } else { safe(&p) }
        });
        TerminalVm {
            visible: true,
            focused,
            tabs,
            renaming,
            busy: session.is_some_and(term::Session::is_busy),
            hints: if focused { tr!("tui-terminal-hints").to_string() } else { String::new() },
            clip,
            grid,
            error,
            palette,
        }
    }
}

/// It is drawn this frame: the bottom panel shows its Terminal tab and the terminal
/// actions do not suspend to a shell instead (`[tui] shell_mode`).
pub fn shown(core: &AppCore) -> bool {
    core.ui.bottom.terminal_shown() && !term::suspend_mode(core)
}

/// Rows the pane wants (`[tui] terminal_rows`, clamped); `Facts::bottom_rows` while
/// [`shown`].
pub fn rows(core: &AppCore) -> u16 {
    core.config.tui.terminal_rows_clamped()
}

/// The grid's rect in the pane's `area`: below the strip (and the prompt bar, `clip`).
pub fn grid_rect(area: Rect, clip: bool) -> Rect {
    let top = 1 + u16::from(clip);
    if area.height <= top {
        return Rect::new(area.x, area.bottom(), area.width, 0);
    }
    Rect { y: area.y + top, height: area.height - top, ..area }
}

/// A [`Host`] for the core calls `prepare` makes (starting a session needs the wake-up
/// its output redraws with): the core's own wake-up.
struct WakeHost(Wake);

impl Host for WakeHost {
    fn wake(&self) -> Wake {
        self.0.clone()
    }
    fn redraw(&self) {
        (self.0)();
    }
    fn redraw_after(&self, _after: Duration) {
        (self.0)();
    }
    fn copy_text(&self, _text: String) {}
    fn set_theme(&self, _pref: ThemePref) {}
    fn window(&self, _id: WindowId, _cmd: WindowCmd) {}
    fn config_changed(&self, _old: &soyuz_core::config::Config, _new: &soyuz_core::config::Config) {}
}

/// Step 1 of a frame (`view::prepare`): see the module docs.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let host = WakeHost(core.wake.clone());
    for c in take_terminal_tab_cmds() {
        apply_terminal_tab(core, &host, c);
    }
    share_bottom(core, input);
    let st = &mut input.terminal;
    if !shown(core) {
        // `shell_mode` switched to suspend while the pane had the keyboard: give it back.
        if core.ui.terminal.focused && term::suspend_mode(core) {
            core.ui.terminal.focused = false;
        }
        st.grid = Rect::default();
        st.rename_view = None;
        for s in &core.ui.terminal.sessions {
            if let Some(t) = &s.term {
                t.set_focused(false);
            }
        }
        return;
    }
    term::ensure_session(core, &host);
    let palette = TuiTheme::cached(core, false).terminal_palette(&core.config.terminal.theme);
    let area = regions.bottom;
    let grid = grid_rect(area, core.ui.terminal.clip_request.is_some());
    st.grid = grid;
    let focused = crate::input::focus(core, input) == TuiFocus::Terminal;
    let st = &mut input.terminal;
    let active = core.ui.terminal.active_id(current_owner(core));
    let (cw, ch) = core.config.tui.cell_pixels_clamped();
    for s in &core.ui.terminal.sessions {
        let Some(t) = &s.term else { continue };
        if Some(s.id) != active {
            t.set_focused(false);
            continue;
        }
        if grid.width >= 2 && grid.height >= 1 {
            t.resize(grid.width, grid.height, cw, ch);
        }
        t.set_palette(&palette);
        t.set_focused(focused);
        t.mark_drawn();
    }
    st.palette = Some(palette);
    // The rename field: its visible part and cursor (a tab label is LABEL_MAX wide).
    st.rename_view = match &core.ui.terminal.renaming {
        Some((_, text)) => Some(st.rename.view(text, LABEL_MAX as u16)),
        None => None,
    };
}

/// The jobs panel and the terminal share `Regions::bottom`. With the activity panel in the
/// bottom placement they are tabs of one panel; in the right / floating placements (the
/// GUI's side panel and window) both can be open in the core, and here the one opened last
/// wins: the other closes (and gives up the keyboard).
fn share_bottom(core: &mut AppCore, input: &mut InputState) {
    let b = &mut core.ui.bottom;
    let (term_now, jobs_now) = (b.terminal_shown(), b.activity_shown());
    if term_now && jobs_now && b.activity_placement != ActivityPlacement::Bottom {
        let (term_before, jobs_before) = input.terminal.last_shown;
        if jobs_before && !term_before {
            b.activity_open = false;
            input.jobs.focused = false;
        } else {
            b.open = false;
            core.ui.terminal.focused = false;
        }
        (core.wake)();
    }
    input.terminal.last_shown = (core.ui.bottom.terminal_shown(), core.ui.bottom.activity_shown());
}

/// `s` written at `(x, y)`, cut at `end`; returns the next column.
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
pub fn draw_terminal(f: &mut Frame<'_>, area: Rect, m: &TerminalVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    let mut cursor: Option<Position> = None;
    {
        let buf = f.buffer_mut();
        buf.set_style(area, theme.style(Role::Text));
        draw_strip(buf, area, m, theme, hits, &mut cursor);
        let grid = grid_rect(area, m.clip.is_some());
        if let Some(preview) = &m.clip
            && area.height >= 2
        {
            draw_clip(buf, Rect { y: area.y + 1, height: 1, ..area }, preview, theme, hits);
        }
        if !grid.is_empty() {
            hits.push(grid, HitTarget::TermCell { row: 0, col: 0 });
            if let Some(g) = &m.grid {
                let c = draw_grid(buf, grid, g, &m.palette, m.focused, theme);
                if cursor.is_none() {
                    cursor = c;
                }
            } else if let Some(e) = &m.error {
                let y = grid.y + grid.height.saturating_sub(1) / 2;
                let s = text::truncate_end(e, usize::from(grid.width));
                let x = grid.x + (grid.width - text::width(&s) as u16) / 2;
                buf.set_string(x, y, &s, theme.style(Role::Danger));
            }
        }
    }
    if let Some(p) = cursor {
        f.set_cursor_position(p);
    }
}

/// Row 0: the session tabs, the new-session button, hints, busy, external terminal.
fn draw_strip(
    buf: &mut Buffer,
    area: Rect,
    m: &TerminalVm,
    theme: &TuiTheme,
    hits: &mut Hits,
    cursor: &mut Option<Position>,
) {
    let y = area.y;
    let end = area.right();
    let line = crate::borders::lines(theme.borders()).horizontal;
    let line_style = theme.style(if m.focused { Role::BorderFocused } else { Role::Border });
    for x in area.x..end {
        buf.set_string(x, y, line, line_style);
    }
    let set = theme.icons();
    let ascii = set == IconSet::Ascii || theme.ascii_only();
    let close = icons::glyph(IconId::X, set);
    let external = icons::glyph(IconId::ArrowSquareOut, set);
    // Right side first (it must stay visible): external button, then the busy hint.
    let ext = format!(" {external} ");
    let ext_w = text::width(&ext) as u16;
    let mut right = end;
    if ext_w + 2 <= area.width {
        let x = end - ext_w - 1;
        put(buf, x, y, end, &ext, theme.style(Role::Muted));
        hits.push(Rect::new(x, y, ext_w, 1), HitTarget::TermExternal);
        right = x.saturating_sub(1);
    }
    let mut x = area.x + 1.min(area.width);
    for tab in &m.tabs {
        if x >= right {
            break;
        }
        let dot = match (tab.alive, tab.busy, ascii) {
            (false, _, true) => "x",
            (false, _, false) => "✕",
            (true, _, true) => "*",
            (true, _, false) => "●",
        };
        let dot_role = if !tab.alive {
            Role::Danger
        } else if tab.busy {
            Role::Warning
        } else {
            Role::Success
        };
        let role = if tab.active { Role::TabActive } else { Role::TabInactive };
        let start = x;
        x = put(buf, x, y, right, " ", theme.style(role));
        x = put(buf, x, y, right, dot, theme.style(dot_role));
        x = put(buf, x, y, right, " ", theme.style(role));
        match m.renaming.as_ref().filter(|(id, ..)| *id == tab.id) {
            Some((_, shown, col)) => {
                let field_end = (x + LABEL_MAX as u16).min(right);
                let style = theme.style(Role::Input);
                for fx in x..field_end {
                    buf.set_string(fx, y, " ", style);
                }
                put(buf, x, y, field_end, shown, style);
                if m.focused && x + col < field_end {
                    *cursor = Some(Position { x: x + col, y });
                }
                x = field_end;
            }
            None => {
                let label = text::truncate_end(&tab.label, LABEL_MAX);
                x = put(buf, x, y, right, &label, theme.style(role));
            }
        }
        x = put(buf, x, y, right, " ", theme.style(role));
        hits.push(Rect::new(start, y, x - start, 1), HitTarget::TermTab(tab.id));
        let cx = x;
        x = put(buf, x, y, right, close, theme.style(Role::Muted));
        hits.push(Rect::new(cx, y, x - cx, 1), HitTarget::TermTabClose(tab.id));
        x = put(buf, x, y, right, " ", theme.style(role));
        x = (x + 1).min(right);
    }
    if x + 3 <= right {
        let nx = put(buf, x, y, right, "[+]", theme.style(Role::Button));
        hits.push(Rect::new(x, y, nx - x, 1), HitTarget::TermNew);
        x = nx + 1;
    }
    // Busy hint, then the key hints, right-aligned before the external button.
    let mut r = right;
    if m.busy {
        let busy = format!(" {} ", tr!("panels-terminal-busy"));
        let w = text::width(&busy) as u16;
        if x + w < r {
            r -= w;
            put(buf, r, y, r + w, &busy, theme.style(Role::Warning));
        }
    }
    if !m.hints.is_empty() {
        let hints = format!(" {} ", m.hints);
        let w = text::width(&hints) as u16;
        if x + w < r {
            put(buf, r - w, y, r, &hints, theme.style(Role::KeyHint));
        }
    }
}

/// The OSC 52 prompt bar: the request's preview and its three answers.
fn draw_clip(buf: &mut Buffer, row: Rect, preview: &str, theme: &TuiTheme, hits: &mut Hits) {
    let (y, end) = (row.y, row.right());
    buf.set_style(row, theme.style(Role::StatusBar));
    let buttons = [
        (0u8, tr!("tui-terminal-clip-allow-once")),
        (1, tr!("tui-terminal-clip-always")),
        (2, tr!("tui-terminal-clip-deny")),
    ];
    let labels: Vec<String> = buttons.iter().map(|(_, l)| format!("[{l}]")).collect();
    let buttons_w: u16 = labels.iter().map(|l| text::width(l) as u16 + 1).sum();
    // The buttons win the room; the title and preview take what is left.
    let mut x = row.x;
    let text_end = end.saturating_sub(buttons_w).max(row.x);
    x = put(buf, x, y, text_end, &format!(" {} ", tr!("panels-terminal-clip-title")), theme.style(Role::Warning));
    put(buf, x, y, text_end, &format!("\"{preview}\""), theme.style(Role::StatusBar));
    let mut bx = text_end;
    for ((i, _), label) in buttons.iter().zip(&labels) {
        if bx >= end {
            break;
        }
        let nx = put(buf, bx, y, end, label, theme.style(Role::Button));
        hits.push(Rect::new(bx, y, nx - bx, 1), HitTarget::TermClip(*i));
        bx = nx + 1;
    }
}

/// The grid into `rect`; returns where the real cursor goes (focused, cursor in view).
fn draw_grid(
    buf: &mut Buffer,
    rect: Rect,
    g: &GridSnapshot,
    palette: &TermPalette,
    focused: bool,
    theme: &TuiTheme,
) -> Option<Position> {
    let rgb = matches!(theme.mode(), ColorMode::Truecolor | ColorMode::Ansi256);
    buf.set_style(rect, theme.term_style(palette.fg, g.default_bg, palette));
    let rows = g.rows.min(usize::from(rect.height));
    let cols = g.cols.min(usize::from(rect.width));
    let cursor = g.cursor.filter(|c| c.shape != CursorShape::Hidden && c.row < rows && c.col < cols);
    for r in 0..rows {
        let y = rect.y + r as u16;
        for (c, cell) in g.row(r).iter().enumerate().take(cols) {
            let x = rect.x + c as u16;
            let selected = g.is_selected(r, c);
            let at_cursor = cursor.is_some_and(|k| k.row == r && k.col == c);
            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if selected && rgb {
                bg = term_blend(palette.selection, bg);
            }
            let block = at_cursor && focused && matches!(cursor.map(|k| k.shape), Some(CursorShape::Block));
            if block && rgb {
                (fg, bg) = (g.default_bg, g.cursor_color);
            }
            let mut style = theme.term_style(fg, bg, palette);
            let mut mods = Modifier::empty();
            if cell.bold {
                mods |= Modifier::BOLD;
            }
            if cell.italic {
                mods |= Modifier::ITALIC;
            }
            if cell.underline {
                mods |= Modifier::UNDERLINED;
            }
            if cell.strike {
                mods |= Modifier::CROSSED_OUT;
            }
            if (selected || block) && !rgb {
                mods |= Modifier::REVERSED;
            }
            if at_cursor && !block {
                mods |= Modifier::UNDERLINED;
            }
            style = style.add_modifier(mods);
            let out = &mut buf[(x, y)];
            if cell.spacer {
                out.reset();
                out.set_style(style);
                continue;
            }
            // A wide character cut by the right edge shows as a blank.
            let ch = if cell.c.is_control() || (cell.wide && c + 1 >= cols) { ' ' } else { cell.c };
            if cell.zerowidth.is_empty() || ch == ' ' {
                out.set_char(ch);
            } else {
                let mut s = String::with_capacity(4 + cell.zerowidth.len());
                s.push(ch);
                s.extend(cell.zerowidth.chars().filter(|z| !z.is_control()));
                out.set_symbol(&s);
            }
            out.set_style(style);
        }
    }
    if g.display_offset > 0 && rect.width >= 4 {
        let arrow = if theme.ascii_only() || theme.icons() == IconSet::Ascii { "^" } else { "▲" };
        let s = format!(" {arrow} {} ", g.display_offset);
        let w = (text::width(&s) as u16).min(rect.width);
        put(buf, rect.right() - w, rect.y, rect.right(), &s, theme.style(Role::StatusBar));
    }
    let k = cursor.filter(|_| focused && g.display_offset == 0)?;
    Some(Position { x: rect.x + k.col as u16, y: rect.y + k.row as u16 })
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_term::{GridCell, GridCursor, TermColor};

    use super::*;

    fn grid(text: &[&str], cols: usize) -> GridSnapshot {
        let p = TermPalette::dark();
        let mut cells = Vec::new();
        for line in text {
            let mut row: Vec<GridCell> = Vec::new();
            for ch in line.chars() {
                let wide = text::width(&ch.to_string()) == 2;
                row.push(GridCell { c: ch, wide, fg: p.fg, bg: p.bg, ..GridCell::default() });
                if wide {
                    row.push(GridCell { c: ' ', spacer: true, fg: p.fg, bg: p.bg, ..GridCell::default() });
                }
            }
            row.resize(cols, GridCell { c: ' ', fg: p.fg, bg: p.bg, ..GridCell::default() });
            cells.extend(row);
        }
        GridSnapshot {
            cols,
            rows: text.len(),
            cells,
            cursor: Some(GridCursor { row: 1, col: 2, shape: CursorShape::Block }),
            default_bg: p.bg,
            cursor_color: p.cursor,
            selection: vec![(0, 0, 1)],
            ..GridSnapshot::default()
        }
    }

    fn vm() -> TerminalVm {
        TerminalVm {
            visible: true,
            focused: true,
            tabs: vec![
                SessionTabVm { id: 1, label: "pwsh".into(), alive: true, busy: false, active: true },
                SessionTabVm {
                    id: 2,
                    label: "a very long session label".into(),
                    alive: false,
                    busy: true,
                    active: false,
                },
            ],
            busy: true,
            hints: "Esc: back".into(),
            clip: Some("secret".into()),
            grid: Some(grid(&["hi 漢x", "$ "], 8)),
            palette: TermPalette::dark(),
            ..TerminalVm::default()
        }
    }

    fn draw(m: &TerminalVm, w: u16, h: u16, theme: &TuiTheme) -> (Terminal<TestBackend>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| {
            let area = f.area();
            draw_terminal(f, area, m, theme, &mut hits);
        })
        .unwrap();
        (t, hits)
    }

    fn row(t: &Terminal<TestBackend>, y: u16) -> String {
        let buf = t.backend().buffer();
        (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect()
    }

    #[test]
    fn draws_strip_clip_bar_and_grid() {
        let theme = TuiTheme::plain().with_icons(IconSet::Ascii);
        let (t, hits) = draw(&vm(), 120, 6, &theme);
        assert!(row(&t, 0).contains("pwsh"), "{}", row(&t, 0));
        assert!(row(&t, 0).contains("[+]"));
        assert!(row(&t, 1).contains("\"secret\""), "{}", row(&t, 1));
        // The spacer after a wide character is a reset cell (ratatui skips it when drawing).
        assert!(row(&t, 2).starts_with("hi 漢 x"), "{}", row(&t, 2));
        assert!(row(&t, 3).starts_with("$ "));
        let targets: Vec<HitTarget> = hits.iter().map(|(_, t)| *t).collect();
        for want in [
            HitTarget::TermTab(1),
            HitTarget::TermTabClose(1),
            HitTarget::TermTab(2),
            HitTarget::TermNew,
            HitTarget::TermExternal,
            HitTarget::TermClip(0),
            HitTarget::TermClip(1),
            HitTarget::TermClip(2),
            HitTarget::TermCell { row: 0, col: 0 },
        ] {
            assert!(targets.contains(&want), "{want:?} missing: {targets:?}");
        }
        // Mono: the selection and the block cursor are reversed.
        let buf = t.backend().buffer();
        assert!(buf[(0, 2)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(2, 3)].modifier.contains(Modifier::REVERSED));
        assert!(!buf[(5, 2)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn truecolor_cells_take_the_palette_colours() {
        let theme =
            TuiTheme::from_tokens(&soyuz_appcore::theme::tokens::ORBIT_DARK, ColorMode::Truecolor, IconSet::Ascii);
        let mut m = vm();
        m.clip = None;
        let (mut t, _) = draw(&m, 40, 4, &theme);
        assert_eq!(t.get_cursor_position().ok(), Some(Position { x: 2, y: 2 }));
        let buf = t.backend().buffer();
        let p = TermPalette::dark();
        let rgb = |c: TermColor| ratatui::style::Color::Rgb(c.r, c.g, c.b);
        // Unselected text: palette fg on bg; selected: the selection blended over bg.
        assert_eq!(buf[(5, 1)].fg, rgb(p.fg));
        assert_eq!(buf[(5, 1)].bg, rgb(p.bg));
        assert_eq!(buf[(0, 1)].bg, rgb(term_blend(p.selection, p.bg)));
        // The block cursor: cursor colour behind the default background.
        assert_eq!(buf[(2, 2)].bg, rgb(p.cursor));
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let theme = TuiTheme::plain();
        let mut scrolled = vm();
        if let Some(g) = scrolled.grid.as_mut() {
            g.display_offset = 120;
        }
        let mut renaming = vm();
        renaming.renaming = Some((1, "new name".into(), 3));
        let mut failed = vm();
        failed.grid = None;
        failed.error = Some("Couldn't start the shell: nope".into());
        for m in [vm(), scrolled, renaming, failed, TerminalVm::default()] {
            for w in 0..14 {
                for h in 0..5 {
                    draw(&m, w, h, &theme);
                }
            }
            draw(&m, 200, 40, &theme);
        }
    }

    #[test]
    fn grid_rect_leaves_the_strip_and_the_bar() {
        let a = Rect::new(0, 10, 30, 6);
        assert_eq!(grid_rect(a, false), Rect::new(0, 11, 30, 5));
        assert_eq!(grid_rect(a, true), Rect::new(0, 12, 30, 4));
        assert!(grid_rect(Rect::new(0, 0, 30, 1), false).is_empty());
        assert!(grid_rect(Rect::new(0, 0, 30, 2), true).is_empty());
    }
}
