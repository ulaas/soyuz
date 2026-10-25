//! Keys and mouse of the embedded terminal pane (Phase T5, `[tui] shell_mode = "pane"`).
//!
//! The pane has the keyboard ([`has_focus`]) while the core says the terminal owns it
//! (`models::terminal::owns_keyboard`: `ToggleTerminal` Ctrl+` / Alt+`, `FocusTerminal`
//! Alt+T, "Open terminal here", a click on the pane) and the pane is shown. Then every key
//! goes to the shell, encoded by `soyuz_term::input::encode_key` (cursor / function keys,
//! Ctrl and Alt combinations, the program's cursor-key and newline modes; plain letters,
//! digits and other characters are sent as typed text, Alt as an Esc prefix except on
//! macOS), with these exceptions, in order:
//!
//! | Key | Does |
//! |---|---|
//! | `a` / `s` / `d` / Esc while the OSC 52 bar is open | allow once / always for this session / deny (other keys wait) |
//! | typing, Enter, Esc while a tab is renamed | edit the name, keep it, cancel |
//! | a passthrough chord bound in `KeyContext::Terminal` | the key map's action (Ctrl+` / Alt+` toggles, Ctrl+Tab / Ctrl+PageDown next tab, Ctrl+Shift+letter / Alt+Shift+letter, Alt+P, Alt+J jobs, Alt+E editor, F12 sidebar: `models::terminal::terminal_passthrough` + `overlay::terminal_passthrough_tui`) |
//! | Shift+PageUp / Shift+PageDown (not in the alternate screen) | scroll the scrollback a page |
//! | Esc at an empty prompt, `[terminal] escape_returns_focus` | give the keyboard back to the file list |
//! | Ctrl+Shift+C / Alt+Shift+C | copy the selection |
//! | Ctrl+C with a selection | copy it and clear it (without one: interrupt, `0x03`) |
//! | Ctrl+Shift+V / Alt+Shift+V | paste the local clipboard's text (Windows, [`is_paste_chord`]) |
//!
//! A bracketed paste from the terminal (`Event::Paste`) is pasted into the shell (as a
//! bracketed paste when the program asked for one). Key releases are dropped.
//!
//! Mouse ([`on_mouse`]): on the strip, a tab activates (double-click renames, middle
//! closes, right opens the session menu `models::context_menu::terminal_tab`), its close
//! mark closes, `[+]` starts a session, the arrow opens an external terminal; the bar's
//! buttons answer the OSC 52 request. On the grid a press takes the keyboard; when the
//! program turned mouse reporting on (and Shift is not held) presses, releases, drags and
//! the wheel go to it (`Terminal::report_mouse`); otherwise left press / drag selects (1, 2,
//! 3 clicks: characters, words, lines; Shift+click extends), Ctrl+click opens a link (web
//! links in the browser through `models::terminal::open_web_link`, copied with a toast over
//! SSH; paths in a pane: `models::terminal::open_path_link`), the
//! wheel scrolls the scrollback (arrow keys in the alternate screen with alternate scroll).
//! While the pane has the keyboard (or a selection drag runs) it captures the mouse: a
//! press anywhere else gives the keyboard back and goes on to its target.

use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Host;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::context_menu::{TermTabCmd, apply_terminal_tab, terminal_tab};
use soyuz_appcore::models::terminal::{self as term, current_owner};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::config::KeyContext;
use soyuz_term::input::{ALT_IS_META, encode_key};
use soyuz_term::{ClickTracker, MouseButton as TermButton, TermKey, TermMods, TermPalette, Terminal};

use super::InputState;
use super::menu::Runs;
use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::line_edit::EditOutcome;
use crate::widgets::{LineEdit, Menu};
use ratatui::layout::Rect;

/// Lines one wheel notch scrolls the scrollback.
pub const WHEEL_LINES: i32 = 3;

/// What this part keeps between frames (`InputState::terminal`).
#[derive(Debug, Default)]
pub struct State {
    /// The grid's rect in the last frame (`view::terminal::prepare`).
    pub grid: Rect,
    /// The palette the grid is resolved against (`view::terminal::prepare`).
    pub palette: Option<TermPalette>,
    /// Cursor and scroll of the tab rename field.
    pub rename: LineEdit,
    /// The rename field's visible text and cursor column (`view::terminal::prepare`).
    pub rename_view: Option<(String, u16)>,
    /// Multi-click detection on the grid.
    clicks: ClickTracker,
    /// Time base of `clicks`.
    epoch: Option<Instant>,
    /// A left-button selection drag runs.
    selecting: bool,
    /// The last left press on a tab, for double-click detection.
    tab_press: Option<(u64, Instant)>,
    /// `(terminal, jobs panel)` shown in the last frame (`view::terminal::prepare` lets
    /// the one opened last keep the shared bottom region).
    pub last_shown: (bool, bool),
}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(core: &AppCore, _st: &InputState) -> bool {
    term::owns_keyboard(core) && crate::view::terminal::shown(core)
}

/// The active session of the focused tab.
fn active(core: &AppCore) -> Option<&term::Session> {
    let id = core.ui.terminal.active_id(current_owner(core))?;
    core.ui.terminal.session(id)
}

fn active_term(core: &AppCore) -> Option<&Terminal> {
    active(core).and_then(|s| s.term.as_ref())
}

/// The modifiers of a crossterm key or mouse event.
pub fn term_mods(m: KeyModifiers) -> TermMods {
    TermMods::new(
        m.contains(KeyModifiers::SHIFT),
        m.intersects(KeyModifiers::ALT | KeyModifiers::META),
        m.contains(KeyModifiers::CONTROL),
    )
}

/// The terminal key of a crossterm key event (`None`: nothing a terminal sends, such as
/// Caps Lock or media keys). Raw control characters (`0x01`–`0x1A`, some Windows
/// consoles) become Ctrl + their letter, NUL is Ctrl+Space, Shift+Tab (`BackTab`) is Tab
/// with Shift.
pub fn term_key(k: &KeyEvent) -> Option<(TermKey, TermMods)> {
    let mut mods = term_mods(k.modifiers);
    let key = match k.code {
        KeyCode::Char(c) => match c as u32 {
            0x01..=0x1A => {
                mods.ctrl = true;
                TermKey::Char((b'a' + (c as u8) - 1) as char)
            }
            _ if c == ' ' => TermKey::Space,
            _ if c.is_control() => return None,
            _ => TermKey::Char(c),
        },
        KeyCode::Null => {
            mods.ctrl = true;
            TermKey::Space
        }
        KeyCode::Backspace => TermKey::Backspace,
        KeyCode::Enter => TermKey::Enter,
        KeyCode::Left => TermKey::Left,
        KeyCode::Right => TermKey::Right,
        KeyCode::Up => TermKey::Up,
        KeyCode::Down => TermKey::Down,
        KeyCode::Home => TermKey::Home,
        KeyCode::End => TermKey::End,
        KeyCode::PageUp => TermKey::PageUp,
        KeyCode::PageDown => TermKey::PageDown,
        KeyCode::Tab => TermKey::Tab,
        KeyCode::BackTab => {
            mods.shift = true;
            TermKey::Tab
        }
        KeyCode::Delete => TermKey::Delete,
        KeyCode::Insert => TermKey::Insert,
        KeyCode::Esc => TermKey::Escape,
        KeyCode::F(n) => TermKey::F(n),
        _ => return None,
    };
    Some((key, mods))
}

/// The bytes a key press sends to the shell: the encoded key, else its typed text
/// (`None`: nothing, e.g. Ctrl + a symbol with no control code).
pub fn key_bytes(k: &KeyEvent, modes: &soyuz_term::Modes) -> Option<Vec<u8>> {
    let (key, mods) = term_key(k)?;
    if let Some(b) = encode_key(key, mods, modes) {
        return Some(b);
    }
    let KeyCode::Char(c) = k.code else { return None };
    if c.is_control() {
        return None;
    }
    // Ctrl + a key without a control code sends nothing; Ctrl+Alt is AltGr typing on
    // Windows (encode_key leaves it to the text), Alt is Option typing on macOS.
    if mods.ctrl && !(mods.alt && cfg!(windows)) {
        return None;
    }
    let mut text = String::new();
    if mods.alt && ALT_IS_META && !mods.ctrl {
        text.push('\x1b');
    }
    // kitty's "report all keys" sends Shift+a as `a` + Shift.
    if mods.shift && c.is_lowercase() {
        text.extend(c.to_uppercase());
    } else {
        text.push(c);
    }
    Some(text.into_bytes())
}

/// The core chord of key event `ev`, if it is one.
fn chord_of(app: &App, ev: &Event) -> Option<soyuz_core::config::KeyChord> {
    crate::keys::convert(ev, app.core.config.keys.cmd_is_ctrl, cfg!(windows)).into_iter().find_map(|e| match e {
        InputEvent::Chord(c) => Some(c),
        _ => None,
    })
}

/// This frame's events while it has the focus.
pub fn route(app: &mut App, events: &[Event]) {
    for ev in events {
        if !has_focus(&app.core, &app.input) {
            break;
        }
        match ev {
            Event::Key(k) if k.kind == KeyEventKind::Release => {}
            Event::Key(k) => key(app, ev, k),
            Event::Paste(text) => {
                if app.core.ui.terminal.renaming.is_some() {
                    rename_input(app, ev);
                } else if let Some(t) = active_term(&app.core) {
                    t.paste(text);
                }
            }
            _ => {}
        }
    }
}

/// Answer the pending OSC 52 request: `0` allow once, `1` always (this session), `2` deny.
fn answer(app: &mut App, i: u8) {
    term::answer_clip(&mut app.core, &app.host, i < 2, i == 1);
}

fn key(app: &mut App, ev: &Event, k: &KeyEvent) {
    let plain = !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::META);
    // 1. The OSC 52 bar waits for its answer.
    if app.core.ui.terminal.clip_request.is_some() {
        match k.code {
            KeyCode::Char('a' | 'A') if plain => answer(app, 0),
            KeyCode::Char('s' | 'S') if plain => answer(app, 1),
            KeyCode::Char('d' | 'D') | KeyCode::Esc if plain => answer(app, 2),
            _ => {}
        }
        return;
    }
    // 2. A tab's name is being edited.
    if app.core.ui.terminal.renaming.is_some() {
        rename_input(app, ev);
        return;
    }
    // 3. The app's chords.
    if let Some(c) = chord_of(app, ev)
        && (term::terminal_passthrough(c) || crate::overlay::terminal_passthrough_tui(c))
        && app.core.keymap.lookup(KeyContext::Terminal, c).is_some()
    {
        super::core_input(app, std::slice::from_ref(ev), KeyContext::Terminal, true);
        return;
    }
    let Some(t) = active_term(&app.core) else { return };
    let modes = t.modes();
    let m = k.modifiers;
    let (shift, ctrl, alt) = (
        m.contains(KeyModifiers::SHIFT),
        m.contains(KeyModifiers::CONTROL),
        m.intersects(KeyModifiers::ALT | KeyModifiers::META),
    );
    // 4. Scrollback.
    if shift && !ctrl && !alt && !modes.alt_screen && matches!(k.code, KeyCode::PageUp | KeyCode::PageDown) {
        t.scroll_page(k.code == KeyCode::PageUp);
        return;
    }
    // 5. Esc at an empty prompt gives the keyboard back.
    if k.code == KeyCode::Esc && plain && !shift && app.core.config.terminal.escape_returns_focus && t.at_prompt() {
        term::unfocus(&mut app.core);
        return;
    }
    // 6. Copy.
    let c_key = matches!(k.code, KeyCode::Char('c' | 'C'));
    if c_key && shift && (ctrl || alt) {
        copy_selection(app, false);
        return;
    }
    if c_key && ctrl && !alt && t.selected_text().is_some_and(|s| !s.is_empty()) {
        copy_selection(app, true);
        return;
    }
    // 6b. Paste the local clipboard's text (Windows: the console sends no paste for it).
    if is_paste_chord(k)
        && let Some(text) = soyuz_appcore::services::os_clipboard::get_text().filter(|s| !s.is_empty())
    {
        t.select_clear();
        t.paste(&text);
        return;
    }
    // 7. The shell.
    if let Some(bytes) = key_bytes(k, &modes) {
        t.select_clear();
        t.input(bytes);
    }
}

/// Ctrl+Shift+V (Alt+Shift+V on terminals that drop Shift with Ctrl): paste the local
/// clipboard's text (`os_clipboard::get_text`: Windows, Linux with a native backend; else
/// the key goes on to the shell and the terminal emulator's own paste arrives as
/// `Event::Paste`).
pub fn is_paste_chord(k: &KeyEvent) -> bool {
    let m = k.modifiers;
    matches!(k.code, KeyCode::Char('v' | 'V'))
        && m.contains(KeyModifiers::SHIFT)
        && m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// Copy the active session's selection (`clear`: then drop it).
fn copy_selection(app: &mut App, clear: bool) {
    let Some(t) = active_term(&app.core) else { return };
    let Some(text) = t.selected_text().filter(|s| !s.is_empty()) else { return };
    if clear {
        t.select_clear();
    }
    app.host.copy_text(text);
}

/// A key or paste for the tab rename field.
fn rename_input(app: &mut App, ev: &Event) {
    let Some((id, mut text)) = app.core.ui.terminal.renaming.take() else { return };
    let mut frame = crate::keys::frame(std::slice::from_ref(ev), app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    match app.input.terminal.rename.feed(&mut text, &mut frame) {
        EditOutcome::Submit => term::rename(&mut app.core, id, &text),
        EditOutcome::Cancel => {}
        _ => app.core.ui.terminal.renaming = Some((id, text)),
    }
}

/// Start renaming session `id`.
fn start_rename(app: &mut App, id: u64) {
    apply_terminal_tab(&mut app.core, &app.host, TermTabCmd::Rename(id));
    if let Some((_, text)) = &app.core.ui.terminal.renaming {
        app.input.terminal.rename = LineEdit::at_end(text);
    }
}

/// A mouse event over one of its click targets (`input::mouse` dispatches by target);
/// returns whether it was taken.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    let left_down = ev.kind == MouseEventKind::Down(MouseButton::Left);
    match target {
        HitTarget::TermTab(id) => {
            match ev.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let now = Instant::now();
                    let interval = super::mouse::double_click_interval(&app.core.config);
                    let double = app
                        .input
                        .terminal
                        .tab_press
                        .is_some_and(|(t, at)| t == id && now.saturating_duration_since(at) <= interval);
                    app.input.terminal.tab_press = if double { None } else { Some((id, now)) };
                    term::activate(&mut app.core, id);
                    if double {
                        start_rename(app, id);
                    }
                }
                MouseEventKind::Down(MouseButton::Middle) => term::close_session(&mut app.core, id),
                MouseEventKind::Down(MouseButton::Right) => {
                    term::activate(&mut app.core, id);
                    let mut runs = Runs::default();
                    let items = super::menu::rows(terminal_tab(id), &mut runs);
                    app.input.menu.runs = runs;
                    app.input.menu.header = None;
                    super::menu::open(app, Menu::new(items, (ev.column, ev.row)));
                }
                _ => {}
            }
            true
        }
        HitTarget::TermTabClose(id) => {
            if left_down {
                term::close_session(&mut app.core, id);
            }
            true
        }
        HitTarget::TermNew => {
            if left_down {
                term::new_session(&mut app.core, &app.host);
            }
            true
        }
        HitTarget::TermExternal => {
            if left_down && let Some(id) = active(&app.core).map(|s| s.id) {
                term::open_external(&mut app.core, id);
            }
            true
        }
        HitTarget::TermClip(i) => {
            if left_down {
                answer(app, i);
            }
            true
        }
        HitTarget::TermCell { .. } => {
            grid_mouse(app, ev);
            true
        }
        _ => {
            // Captured: a drag that left the grid keeps selecting; a press elsewhere gives
            // the keyboard back and goes on to its target.
            if app.input.terminal.selecting
                && matches!(ev.kind, MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left))
            {
                grid_mouse(app, ev);
                return true;
            }
            if matches!(ev.kind, MouseEventKind::Down(_)) {
                if let Some((id, text)) = app.core.ui.terminal.renaming.clone() {
                    term::rename(&mut app.core, id, &text);
                }
                app.core.ui.terminal.focused = false;
                app.input.terminal.selecting = false;
            }
            false
        }
    }
}

/// The grid cell under `(x, y)` (clamped into the grid).
fn cell_at(grid: Rect, x: u16, y: u16) -> (usize, usize) {
    let col = x.clamp(grid.x, grid.right().saturating_sub(1)) - grid.x;
    let row = y.clamp(grid.y, grid.bottom().saturating_sub(1)) - grid.y;
    (usize::from(row), usize::from(col))
}

/// A mouse event on the grid.
fn grid_mouse(app: &mut App, ev: &MouseEvent) {
    let grid = app.input.terminal.grid;
    if grid.is_empty() {
        return;
    }
    let (row, col) = cell_at(grid, ev.column, ev.row);
    if matches!(ev.kind, MouseEventKind::Down(_)) {
        app.core.ui.terminal.focused = true;
    }
    let Some(t) = active_term(&app.core) else { return };
    let mods = term_mods(ev.modifiers);
    let modes = t.modes();
    // The program's own mouse handling (Shift keeps it for the selection).
    if !mods.shift && modes.mouse_any() {
        let report = match ev.kind {
            MouseEventKind::Down(b) => Some((button(b), true, false)),
            MouseEventKind::Up(b) => Some((button(b), false, false)),
            MouseEventKind::Drag(b) => Some((button(b), true, true)),
            MouseEventKind::Moved => Some((TermButton::None, false, true)),
            MouseEventKind::ScrollUp => Some((TermButton::WheelUp, true, false)),
            MouseEventKind::ScrollDown => Some((TermButton::WheelDown, true, false)),
            _ => None,
        };
        if let Some((b, pressed, motion)) = report {
            t.report_mouse(b, pressed, motion, col, row, mods);
            return;
        }
    }
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) if mods.ctrl => {
            if let Some(link) = t.link_at(row, col) {
                let base = t.cwd().or_else(|| active(&app.core).and_then(|s| s.last_dir.clone()));
                open_link(app, &link.url, base.as_deref());
            }
        }
        MouseEventKind::Down(MouseButton::Left) if mods.shift => t.select_extend(row, col, false),
        MouseEventKind::Down(MouseButton::Left) => {
            let st = &mut app.input.terminal;
            let epoch = *st.epoch.get_or_insert_with(Instant::now);
            let clicks = st.clicks.register(epoch.elapsed().as_secs_f64(), (row as i32, col));
            t.select_begin(row, col, false, clicks);
            st.selecting = true;
        }
        MouseEventKind::Drag(MouseButton::Left) if app.input.terminal.selecting => t.select_extend(row, col, false),
        MouseEventKind::Up(MouseButton::Left) if app.input.terminal.selecting => {
            t.select_finish();
            app.input.terminal.selecting = false;
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let up = ev.kind == MouseEventKind::ScrollUp;
            if modes.alt_screen && modes.alternate_scroll {
                let key = if up { TermKey::Up } else { TermKey::Down };
                if let Some(b) = encode_key(key, TermMods::NONE, &modes) {
                    t.input(b.repeat(WHEEL_LINES as usize));
                }
            } else {
                t.scroll(if up { WHEEL_LINES } else { -WHEEL_LINES });
            }
        }
        _ => {}
    }
}

fn button(b: MouseButton) -> TermButton {
    match b {
        MouseButton::Left => TermButton::Left,
        MouseButton::Middle => TermButton::Middle,
        MouseButton::Right => TermButton::Right,
    }
}

/// Ctrl+click on a link: web links open in the browser where an opener exists (else they
/// are copied, with a toast), paths open in a pane or with their app
/// (`models::terminal::open_path_link`: never runs a program).
fn open_link(app: &mut App, url: &str, base: Option<&std::path::Path>) {
    if !term::is_web_link(url) {
        term::open_path_link(&mut app.core, url, base);
        return;
    }
    // Over SSH the browser would open on the remote machine.
    if crate::clipboard::Env::detect().ssh {
        app.host.copy_text(url.to_string());
        app.core.ui.toasts.push(Toast::info(tr!("tui-terminal-link-copied")).body(crate::dialog::safe(url)));
        return;
    }
    term::open_web_link(&mut app.core, url);
}

/// While `true`, every mouse event goes to [`on_mouse`] wherever it is (a drag that
/// leaves the targets, an overlay's capture); the target is `HitTarget::Overlay` when
/// nothing is under the pointer.
pub fn captures_mouse(core: &AppCore, st: &InputState) -> bool {
    has_focus(core, st) || st.terminal.selecting
}

#[cfg(test)]
mod tests {
    use soyuz_term::Modes;

    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn the_paste_chord() {
        let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert!(is_paste_chord(&k(KeyCode::Char('V'), cs)));
        assert!(is_paste_chord(&k(KeyCode::Char('v'), KeyModifiers::ALT | KeyModifiers::SHIFT)));
        assert!(!is_paste_chord(&k(KeyCode::Char('v'), KeyModifiers::CONTROL)), "Ctrl+V goes to the shell");
        assert!(!is_paste_chord(&k(KeyCode::Char('V'), KeyModifiers::SHIFT)), "typing V");
        assert!(!is_paste_chord(&k(KeyCode::Char('c'), cs)));
    }

    #[test]
    fn crossterm_keys_map_to_terminal_keys() {
        let none = KeyModifiers::NONE;
        assert_eq!(term_key(&k(KeyCode::Char('a'), none)), Some((TermKey::Char('a'), TermMods::NONE)));
        assert_eq!(term_key(&k(KeyCode::Char(' '), none)), Some((TermKey::Space, TermMods::NONE)));
        assert_eq!(
            term_key(&k(KeyCode::Char('\u{3}'), none)),
            Some((TermKey::Char('c'), TermMods::new(false, false, true)))
        );
        assert_eq!(term_key(&k(KeyCode::Null, none)), Some((TermKey::Space, TermMods::new(false, false, true))));
        assert_eq!(term_key(&k(KeyCode::BackTab, none)), Some((TermKey::Tab, TermMods::new(true, false, false))));
        assert_eq!(
            term_key(&k(KeyCode::F(5), KeyModifiers::ALT)),
            Some((TermKey::F(5), TermMods::new(false, true, false)))
        );
        assert_eq!(term_key(&k(KeyCode::Esc, none)), Some((TermKey::Escape, TermMods::NONE)));
        assert_eq!(term_key(&k(KeyCode::CapsLock, none)), None);
    }

    #[test]
    fn key_bytes_follow_xterm() {
        let m = Modes::default();
        let none = KeyModifiers::NONE;
        let b = |code, mods| key_bytes(&k(code, mods), &m);
        assert_eq!(b(KeyCode::Char('x'), none), Some(b"x".to_vec()));
        assert_eq!(b(KeyCode::Char('X'), KeyModifiers::SHIFT), Some(b"X".to_vec()));
        assert_eq!(b(KeyCode::Char('x'), KeyModifiers::SHIFT), Some(b"X".to_vec()), "kitty: Shift + lower case");
        assert_eq!(b(KeyCode::Char('ş'), none), Some("ş".as_bytes().to_vec()));
        assert_eq!(b(KeyCode::Char('c'), KeyModifiers::CONTROL), Some(vec![3]));
        assert_eq!(b(KeyCode::Enter, none), Some(b"\r".to_vec()));
        assert_eq!(b(KeyCode::Up, none), Some(b"\x1b[A".to_vec()));
        assert_eq!(b(KeyCode::Up, KeyModifiers::CONTROL), Some(b"\x1b[1;5A".to_vec()));
        assert_eq!(b(KeyCode::F(1), none), Some(b"\x1bOP".to_vec()));
        assert_eq!(b(KeyCode::Backspace, none), Some(b"\x7f".to_vec()));
        assert_eq!(b(KeyCode::Esc, none), Some(b"\x1b".to_vec()));
        if ALT_IS_META {
            assert_eq!(b(KeyCode::Char('b'), KeyModifiers::ALT), Some(b"\x1bb".to_vec()));
            assert_eq!(b(KeyCode::Char('.'), KeyModifiers::ALT), Some(b"\x1b.".to_vec()));
        } else {
            assert_eq!(b(KeyCode::Char('.'), KeyModifiers::ALT), Some(b".".to_vec()));
        }
        // Application cursor mode.
        let app_cursor = Modes { app_cursor: true, ..Modes::default() };
        assert_eq!(key_bytes(&k(KeyCode::Up, none), &app_cursor), Some(b"\x1bOA".to_vec()));
        // Windows AltGr typing (Ctrl+Alt + a symbol) is text there.
        let altgr = key_bytes(&k(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT), &m);
        if cfg!(windows) {
            assert_eq!(altgr, Some(b"@".to_vec()));
        }
    }

    #[test]
    fn cells_are_clamped_into_the_grid() {
        let g = Rect::new(2, 10, 20, 5);
        assert_eq!(cell_at(g, 2, 10), (0, 0));
        assert_eq!(cell_at(g, 7, 12), (2, 5));
        assert_eq!(cell_at(g, 0, 0), (0, 0));
        assert_eq!(cell_at(g, 100, 100), (4, 19));
    }
}
