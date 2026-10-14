//! crossterm events → the toolkit-free [`InputFrame`] the core dispatches.
//!
//! The output follows the egui adapter (`soyuz-app/src/ui/input.rs`): a key press becomes
//! a [`InputEvent::Chord`] and, when it types something, a following [`InputEvent::Text`].
//! The order matters: `keys::sequence::feed_frame` swallows the `Text` after a consumed
//! plain chord, so a letter bound to a command never reaches type-ahead.
//!
//! Rules:
//! - Key releases are dropped (Windows and the kitty protocol report them); presses and
//!   repeats are kept.
//! - `Char(c)` becomes the chord of its key: letters by their upper-case [`KeyName`] (an
//!   upper-case letter or Shift adds `SHIFT`, so `G` is `Shift+G` like in egui), digits,
//!   and the US-layout punctuation keys. Shifted punctuation maps to Shift + its US key
//!   (`:` is `Shift+;`, `?` is `Shift+/`, `+` is `Plus`), the way the presets spell them.
//!   Characters without a key name (`!`, `é`, `ş`, …) are text only.
//! - Text follows the chord unless Ctrl, Alt or Super/Meta is held.
//! - Windows reports AltGr as Ctrl+Alt. There, Ctrl+Alt+`c` with `c` not an ASCII letter
//!   or digit is AltGr typing (`@ { \ €` on German / Turkish layouts) and becomes text only.
//!   Ctrl+Alt+letter/digit stays a chord (`Ctrl+Alt+N`), because the console reports the
//!   key's base character for real Ctrl+Alt chords. The price: Ctrl+Alt+punctuation
//!   (`Ctrl+Alt+=`) cannot be told apart from AltGr and types instead.
//! - Named keys (arrows, Home / End, PageUp / PageDown, Enter, Esc, Backspace, Delete,
//!   Insert, Tab, F1–F24) are chords; `BackTab` is `Shift+Tab`.
//! - `Event::Paste` (bracketed paste) is text (the file list runs Paste for it before
//!   conversion, `crate::input::list`); mouse, focus and resize events are
//!   [`InputEvent::Other`]. crossterm has no mouse back / forward buttons, so
//!   [`InputFrame::back`] / [`InputFrame::forward`] stay false.
//! - Super (and kitty's Meta / Hyper as Super / Alt) map to `CMD` / `ALT`, then
//!   [`KeyChord::normalized`] applies `[keys] cmd_is_ctrl` like the GUI.
//!
//! Legacy terminal aliases (no kitty keyboard protocol): Ctrl+I arrives as Tab, Ctrl+M as
//! Enter and Ctrl+[ as Esc. They are the same bytes, so they always map to the plain key.
//! Ctrl+H arrives as `0x08`, which crossterm reports as Ctrl+H; terminals that send `0x08`
//! for Backspace or Ctrl+Backspace therefore produce Ctrl+H. Ctrl+Space and Ctrl+2 send
//! NUL (Ctrl+Space); Ctrl+\ ] ^ _ (and Ctrl+4…7) arrive as Ctrl+4…Ctrl+7; Ctrl+Shift+letter
//! is Ctrl+letter. `crate::overlay` binds sendable alternatives for the defaults hit by this.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use soyuz_appcore::devscript::KeyTok;
use soyuz_appcore::keys::input::{InputEvent, InputFrame};
use soyuz_core::config::{KeyChord, KeyName, Mods};

/// The modifiers that stop a key from typing text.
const COMMAND_MODS: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::META)
    .union(KeyModifiers::HYPER);

/// One crossterm event → zero or more core input events (in order: chord, then text).
/// `cmd_is_ctrl`: `[keys] cmd_is_ctrl`; `is_windows`: apply the AltGr rule
/// (`cfg!(windows)` in the app, a parameter so tests cover both).
pub fn convert(ev: &Event, cmd_is_ctrl: bool, is_windows: bool) -> Vec<InputEvent> {
    match ev {
        Event::Key(k) => convert_key(k, cmd_is_ctrl, is_windows),
        Event::Paste(s) if !s.is_empty() => vec![InputEvent::Text(s.clone())],
        Event::Paste(_) => Vec::new(),
        Event::FocusGained | Event::FocusLost | Event::Mouse(_) | Event::Resize(..) => vec![InputEvent::Other],
    }
}

/// A batch of crossterm events (one frame's worth) as an [`InputFrame`]. `command_held`
/// follows the modifiers of the last key press, like egui's "currently held" modifiers
/// (AltGr typing does not count).
pub fn frame(events: &[Event], cmd_is_ctrl: bool, is_windows: bool) -> InputFrame {
    let out = events.iter().flat_map(|ev| convert(ev, cmd_is_ctrl, is_windows)).collect();
    let mut f = InputFrame::new(out);
    let last_key = events.iter().rev().find_map(|ev| match ev {
        Event::Key(k) if k.kind != KeyEventKind::Release => Some(k),
        _ => None,
    });
    f.command_held = last_key.is_some_and(|k| k.modifiers.intersects(COMMAND_MODS) && !is_altgr(k, is_windows));
    f
}

/// Windows AltGr typing: Ctrl+Alt plus a character that is not an ASCII letter or digit.
fn is_altgr(k: &KeyEvent, is_windows: bool) -> bool {
    is_windows
        && k.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && matches!(k.code, KeyCode::Char(c) if !c.is_ascii_alphanumeric() && !c.is_control())
}

fn mods(m: KeyModifiers) -> Mods {
    let mut out = Mods::empty();
    if m.contains(KeyModifiers::CONTROL) {
        out |= Mods::CTRL;
    }
    if m.contains(KeyModifiers::SHIFT) {
        out |= Mods::SHIFT;
    }
    if m.intersects(KeyModifiers::ALT | KeyModifiers::META) {
        out |= Mods::ALT;
    }
    if m.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER) {
        out |= Mods::CMD;
    }
    out
}

fn convert_key(k: &KeyEvent, cmd_is_ctrl: bool, is_windows: bool) -> Vec<InputEvent> {
    if k.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let m = mods(k.modifiers);
    let chord = |mods: Mods, key: KeyName| InputEvent::Chord(KeyChord::new(mods, key).normalized(cmd_is_ctrl));
    let named = match k.code {
        KeyCode::Backspace => Some((m, KeyName::Backspace)),
        KeyCode::Enter => Some((m, KeyName::Enter)),
        KeyCode::Left => Some((m, KeyName::ArrowLeft)),
        KeyCode::Right => Some((m, KeyName::ArrowRight)),
        KeyCode::Up => Some((m, KeyName::ArrowUp)),
        KeyCode::Down => Some((m, KeyName::ArrowDown)),
        KeyCode::Home => Some((m, KeyName::Home)),
        KeyCode::End => Some((m, KeyName::End)),
        KeyCode::PageUp => Some((m, KeyName::PageUp)),
        KeyCode::PageDown => Some((m, KeyName::PageDown)),
        KeyCode::Tab => Some((m, KeyName::Tab)),
        KeyCode::BackTab => Some((m | Mods::SHIFT, KeyName::Tab)),
        KeyCode::Delete => Some((m, KeyName::Delete)),
        KeyCode::Insert => Some((m, KeyName::Insert)),
        KeyCode::Esc => Some((m, KeyName::Escape)),
        KeyCode::F(n) => function_key(n).map(|key| (m, key)),
        // Some terminals report Ctrl+Space as NUL.
        KeyCode::Null => Some((m | Mods::CTRL, KeyName::Space)),
        KeyCode::Char(c) => return convert_char(c, k.modifiers, m, cmd_is_ctrl, is_windows),
        _ => None,
    };
    match named {
        Some((mods, key)) => vec![chord(mods, key)],
        None => vec![InputEvent::Other],
    }
}

fn convert_char(c: char, raw: KeyModifiers, m: Mods, cmd_is_ctrl: bool, is_windows: bool) -> Vec<InputEvent> {
    // A raw control character (some Windows consoles): Ctrl + its letter.
    let (c, m) = match c as u32 {
        0x01..=0x1A => ((b'a' + (c as u8) - 1) as char, m | Mods::CTRL),
        _ => (c, m),
    };
    if c.is_control() {
        return vec![InputEvent::Other];
    }
    let altgr = is_windows && raw.contains(KeyModifiers::CONTROL | KeyModifiers::ALT) && !c.is_ascii_alphanumeric();
    if altgr {
        return vec![InputEvent::Text(c.to_string())];
    }
    let types = !m.intersects(Mods::CTRL | Mods::ALT | Mods::CMD);
    let mut out = Vec::with_capacity(2);
    if let Some((extra, key)) = char_key(c) {
        let mods = match key {
            // `+` is shifted on most layouts; the presets bind plain `Plus`.
            KeyName::Plus => m - Mods::SHIFT,
            _ => m | extra,
        };
        out.push(InputEvent::Chord(KeyChord::new(mods, key).normalized(cmd_is_ctrl)));
    }
    if types {
        // kitty's "report all keys" sends Shift+a as `a` + SHIFT.
        let text = if m.contains(Mods::SHIFT) && c.is_lowercase() { c.to_uppercase().collect() } else { c.to_string() };
        out.push(InputEvent::Text(text));
    }
    if out.is_empty() {
        out.push(InputEvent::Other);
    }
    out
}

/// The key a character is typed with (US layout), plus the modifier it implies.
fn char_key(c: char) -> Option<(Mods, KeyName)> {
    use KeyName as N;
    const NONE: Mods = Mods::empty();
    const SHIFT: Mods = Mods::SHIFT;
    if c.is_ascii_alphabetic() {
        let up = c.to_ascii_uppercase();
        let key = KeyName::parse(up.encode_utf8(&mut [0; 4]))?;
        return Some((if c.is_ascii_uppercase() { SHIFT } else { NONE }, key));
    }
    if c.is_ascii_digit() {
        return KeyName::parse(c.encode_utf8(&mut [0; 4])).map(|k| (NONE, k));
    }
    Some(match c {
        ' ' => (NONE, N::Space),
        '`' => (NONE, N::Backtick),
        '~' => (SHIFT, N::Backtick),
        '-' => (NONE, N::Minus),
        '_' => (SHIFT, N::Minus),
        '=' => (NONE, N::Equals),
        '+' => (NONE, N::Plus),
        ',' => (NONE, N::Comma),
        '<' => (SHIFT, N::Comma),
        '.' => (NONE, N::Period),
        '>' => (SHIFT, N::Period),
        '/' => (NONE, N::Slash),
        '?' => (SHIFT, N::Slash),
        '\\' => (NONE, N::Backslash),
        '|' => (SHIFT, N::Backslash),
        ';' => (NONE, N::Semicolon),
        ':' => (SHIFT, N::Semicolon),
        '\'' => (NONE, N::Quote),
        '"' => (SHIFT, N::Quote),
        '[' => (NONE, N::OpenBracket),
        '{' => (SHIFT, N::OpenBracket),
        ']' => (NONE, N::CloseBracket),
        '}' => (SHIFT, N::CloseBracket),
        _ => return None,
    })
}

fn function_key(n: u8) -> Option<KeyName> {
    use KeyName as N;
    const F: [KeyName; 24] = [
        N::F1,
        N::F2,
        N::F3,
        N::F4,
        N::F5,
        N::F6,
        N::F7,
        N::F8,
        N::F9,
        N::F10,
        N::F11,
        N::F12,
        N::F13,
        N::F14,
        N::F15,
        N::F16,
        N::F17,
        N::F18,
        N::F19,
        N::F20,
        N::F21,
        N::F22,
        N::F23,
        N::F24,
    ];
    F.get((n as usize).checked_sub(1)?).copied()
}

// ---- Phase T8: `SOYUZ_SCRIPT` `keys:` ----

/// A `keys:` token → the crossterm events a terminal would send for it (the inverse of
/// [`convert`]: [`convert`] turns them back into the same chord). Text is a key press of
/// its character (Shift for an ASCII upper-case letter, like the Windows console), `\n`
/// is Enter and `\t` Tab; `{Copy}` / `{Cut}` are Ctrl+C / Ctrl+X; `{Paste:text}` is a
/// bracketed paste (`Event::Paste`).
pub fn to_crossterm(tok: &KeyTok) -> Vec<Event> {
    let press = |code, m| vec![Event::Key(KeyEvent::new(code, m))];
    match tok {
        KeyTok::Text('\n') => press(KeyCode::Enter, KeyModifiers::NONE),
        KeyTok::Text('\t') => press(KeyCode::Tab, KeyModifiers::NONE),
        KeyTok::Text(c) => {
            press(KeyCode::Char(*c), if c.is_ascii_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE })
        }
        KeyTok::Chord(chord) => {
            let (code, m) = chord_event(*chord);
            press(code, m)
        }
        KeyTok::Copy => press(KeyCode::Char('c'), KeyModifiers::CONTROL),
        KeyTok::Cut => press(KeyCode::Char('x'), KeyModifiers::CONTROL),
        KeyTok::Paste(s) => vec![Event::Paste(s.clone())],
    }
}

/// The key code and modifiers a terminal reports for `chord`.
fn chord_event(chord: KeyChord) -> (KeyCode, KeyModifiers) {
    use KeyName as N;
    let mut m = KeyModifiers::NONE;
    for (from, to) in [
        (Mods::CTRL, KeyModifiers::CONTROL),
        (Mods::SHIFT, KeyModifiers::SHIFT),
        (Mods::ALT, KeyModifiers::ALT),
        (Mods::CMD, KeyModifiers::SUPER),
    ] {
        if chord.mods.contains(from) {
            m |= to;
        }
    }
    let shift = chord.mods.contains(Mods::SHIFT);
    // A character key: its US-layout character, the shifted one with Shift where that
    // still names the same key (`Shift+;` is `:`).
    let ch = |plain: char, shifted: char| KeyCode::Char(if shift { shifted } else { plain });
    let code = match chord.key {
        N::Enter => KeyCode::Enter,
        N::Escape => KeyCode::Esc,
        N::Tab if shift => KeyCode::BackTab,
        N::Tab => KeyCode::Tab,
        N::Space => KeyCode::Char(' '),
        N::Backspace => KeyCode::Backspace,
        N::Delete => KeyCode::Delete,
        N::Insert => KeyCode::Insert,
        N::Home => KeyCode::Home,
        N::End => KeyCode::End,
        N::PageUp => KeyCode::PageUp,
        N::PageDown => KeyCode::PageDown,
        N::ArrowUp => KeyCode::Up,
        N::ArrowDown => KeyCode::Down,
        N::ArrowLeft => KeyCode::Left,
        N::ArrowRight => KeyCode::Right,
        N::Backtick => ch('`', '~'),
        N::Minus => ch('-', '_'),
        N::Plus => KeyCode::Char('+'),
        N::Equals => KeyCode::Char('='),
        N::Comma => ch(',', '<'),
        N::Period => ch('.', '>'),
        N::Slash => ch('/', '?'),
        N::Backslash => ch('\\', '|'),
        N::Semicolon => ch(';', ':'),
        N::Quote => ch('\'', '"'),
        N::OpenBracket => ch('[', '{'),
        N::CloseBracket => ch(']', '}'),
        key => {
            let name = key.as_str();
            match name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
                Some(n) => KeyCode::F(n),
                // A letter (upper-case with Shift) or a digit.
                None => {
                    let c = name.chars().next().unwrap_or(' ');
                    KeyCode::Char(if shift { c } else { c.to_ascii_lowercase() })
                }
            }
        }
    };
    (code, m)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crossterm::event::{KeyEventState, MouseEvent, MouseEventKind};
    use soyuz_appcore::keys::sequence::{Feed, Sequencer, feed_frame};
    use soyuz_core::config::{Action, KeyContext, KeyPreset, Keymap, defaults};

    use super::*;

    fn key(code: KeyCode, m: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, m))
    }

    fn kind(code: KeyCode, m: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent { code, modifiers: m, kind, state: KeyEventState::NONE })
    }

    fn ch(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn text(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
    }

    fn conv(ev: Event) -> Vec<InputEvent> {
        convert(&ev, true, false)
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    #[test]
    fn releases_are_dropped_repeats_kept() {
        assert!(conv(kind(KeyCode::Char('a'), NONE, KeyEventKind::Release)).is_empty());
        assert!(conv(kind(KeyCode::Down, NONE, KeyEventKind::Release)).is_empty());
        assert_eq!(conv(kind(KeyCode::Down, NONE, KeyEventKind::Repeat)), vec![ch("Down")]);
        assert_eq!(conv(kind(KeyCode::Char('j'), NONE, KeyEventKind::Repeat)), vec![ch("J"), text("j")]);
    }

    #[test]
    fn chord_comes_before_text() {
        assert_eq!(conv(key(KeyCode::Char('q'), NONE)), vec![ch("Q"), text("q")]);
        assert_eq!(conv(key(KeyCode::Char('7'), NONE)), vec![ch("7"), text("7")]);
        assert_eq!(conv(key(KeyCode::Char(' '), NONE)), vec![ch("Space"), text(" ")]);
    }

    #[test]
    fn uppercase_and_shift_add_shift() {
        // Unix terminals send `G` (crossterm adds SHIFT), Windows sends `G` + SHIFT.
        assert_eq!(conv(key(KeyCode::Char('G'), NONE)), vec![ch("Shift+G"), text("G")]);
        assert_eq!(conv(key(KeyCode::Char('G'), SHIFT)), vec![ch("Shift+G"), text("G")]);
        // kitty "report all keys": base key + SHIFT.
        assert_eq!(conv(key(KeyCode::Char('g'), SHIFT)), vec![ch("Shift+G"), text("G")]);
        // Ctrl+Shift+P (Windows console, kitty).
        assert_eq!(conv(key(KeyCode::Char('P'), CTRL | SHIFT)), vec![ch("Ctrl+Shift+P")]);
        assert_eq!(conv(key(KeyCode::Char('p'), CTRL | SHIFT)), vec![ch("Ctrl+Shift+P")]);
    }

    #[test]
    fn shifted_punctuation_uses_the_us_key() {
        assert_eq!(conv(key(KeyCode::Char(':'), NONE)), vec![ch("Shift+;"), text(":")]);
        assert_eq!(conv(key(KeyCode::Char(':'), SHIFT)), vec![ch("Shift+;"), text(":")]);
        assert_eq!(conv(key(KeyCode::Char('?'), SHIFT)), vec![ch("Shift+/"), text("?")]);
        assert_eq!(conv(key(KeyCode::Char('+'), SHIFT)), vec![ch("Plus"), text("+")]);
        assert_eq!(conv(key(KeyCode::Char('+'), CTRL | SHIFT)), vec![ch("Ctrl+Plus")]);
        assert_eq!(conv(key(KeyCode::Char('/'), NONE)), vec![ch("/"), text("/")]);
        // No key name: text only.
        assert_eq!(conv(key(KeyCode::Char('!'), SHIFT)), vec![text("!")]);
        assert_eq!(conv(key(KeyCode::Char('ş'), NONE)), vec![text("ş")]);
        assert_eq!(conv(key(KeyCode::Char('é'), CTRL)), vec![InputEvent::Other]);
    }

    #[test]
    fn ctrl_and_alt_letters_are_chords_without_text() {
        assert_eq!(conv(key(KeyCode::Char('t'), CTRL)), vec![ch("Ctrl+T")]);
        assert_eq!(conv(key(KeyCode::Char('p'), ALT)), vec![ch("Alt+P")]);
        assert_eq!(conv(key(KeyCode::Char('1'), ALT)), vec![ch("Alt+1")]);
        assert_eq!(conv(key(KeyCode::Char('n'), CTRL | ALT)), vec![ch("Ctrl+Alt+N")]);
        // A raw control character.
        assert_eq!(conv(key(KeyCode::Char('\x14'), NONE)), vec![ch("Ctrl+T")]);
        // Super is Cmd, normalised to Ctrl unless cmd_is_ctrl is off.
        assert_eq!(conv(key(KeyCode::Char('c'), KeyModifiers::SUPER)), vec![ch("Ctrl+C")]);
        assert_eq!(convert(&key(KeyCode::Char('c'), KeyModifiers::SUPER), false, false), vec![ch("Cmd+C")]);
    }

    #[test]
    fn altgr_on_windows_types_text() {
        let altgr = CTRL | ALT;
        for c in ['@', '{', '\\', '€', '~', '|'] {
            assert_eq!(convert(&key(KeyCode::Char(c), altgr), true, true), vec![text(&c.to_string())], "{c}");
        }
        // A real Ctrl+Alt+letter stays a chord on Windows.
        assert_eq!(convert(&key(KeyCode::Char('w'), altgr), true, true), vec![ch("Ctrl+Alt+W")]);
        // Not Windows: Ctrl+Alt+punctuation is a chord.
        assert_eq!(convert(&key(KeyCode::Char('\\'), altgr), true, false), vec![ch("Ctrl+Alt+\\")]);
        // AltGr text is not "command held", so it reaches type-ahead.
        let f = frame(&[key(KeyCode::Char('@'), altgr)], true, true);
        assert!(!f.command_held);
        assert_eq!(f.typed_text(), "@");
    }

    #[test]
    fn named_keys() {
        let cases = [
            (KeyCode::Up, NONE, "Up"),
            (KeyCode::Down, SHIFT, "Shift+Down"),
            (KeyCode::Left, ALT, "Alt+Left"),
            (KeyCode::Right, NONE, "Right"),
            (KeyCode::Home, NONE, "Home"),
            (KeyCode::End, CTRL, "Ctrl+End"),
            (KeyCode::PageUp, NONE, "PageUp"),
            (KeyCode::PageDown, CTRL, "Ctrl+PageDown"),
            (KeyCode::Enter, NONE, "Enter"),
            (KeyCode::Enter, ALT, "Alt+Enter"),
            (KeyCode::Esc, NONE, "Esc"),
            (KeyCode::Backspace, NONE, "Backspace"),
            (KeyCode::Delete, SHIFT, "Shift+Del"),
            (KeyCode::Insert, NONE, "Ins"),
            (KeyCode::Tab, NONE, "Tab"),
            (KeyCode::Tab, CTRL, "Ctrl+Tab"),
            (KeyCode::BackTab, NONE, "Shift+Tab"),
            (KeyCode::BackTab, SHIFT, "Shift+Tab"),
            (KeyCode::F(1), NONE, "F1"),
            (KeyCode::F(9), SHIFT, "Shift+F9"),
            (KeyCode::F(24), NONE, "F24"),
            (KeyCode::Null, NONE, "Ctrl+Space"),
        ];
        for (code, m, want) in cases {
            assert_eq!(conv(key(code, m)), vec![ch(want)], "{code:?} {m:?}");
        }
        assert_eq!(conv(key(KeyCode::F(25), NONE)), vec![InputEvent::Other]);
        assert_eq!(conv(key(KeyCode::CapsLock, NONE)), vec![InputEvent::Other]);
    }

    #[test]
    fn paste_mouse_focus_resize() {
        assert_eq!(conv(Event::Paste("a b\nc".into())), vec![text("a b\nc")]);
        assert!(conv(Event::Paste(String::new())).is_empty());
        assert_eq!(conv(Event::FocusGained), vec![InputEvent::Other]);
        assert_eq!(conv(Event::Resize(80, 24)), vec![InputEvent::Other]);
        let mouse = MouseEvent { kind: MouseEventKind::ScrollDown, column: 1, row: 1, modifiers: NONE };
        assert_eq!(conv(Event::Mouse(mouse)), vec![InputEvent::Other]);
    }

    #[test]
    fn frame_command_held_follows_the_last_key() {
        let f = frame(&[key(KeyCode::Char('a'), NONE), key(KeyCode::Char('x'), CTRL)], true, false);
        assert!(f.command_held);
        assert_eq!(f.events, vec![ch("A"), text("a"), ch("Ctrl+X")]);
        assert_eq!(f.consumed, vec![false; 3]);
        let f = frame(
            &[
                key(KeyCode::Char('x'), CTRL),
                key(KeyCode::Char('b'), NONE),
                kind(KeyCode::Char('x'), CTRL, KeyEventKind::Release),
            ],
            true,
            false,
        );
        assert!(!f.command_held);
        assert_eq!(f.typed_text(), "b");
        assert!(!f.back && !f.forward);
    }

    fn feed(km: &Keymap, events: &[Event]) -> (InputFrame, Vec<Feed>) {
        let mut f = frame(events, true, false);
        let mut seq = Sequencer::default();
        let out = feed_frame(&mut f, km, &mut seq, KeyContext::FileList, false, false, Instant::now());
        (f, out)
    }

    #[test]
    fn vim_bound_letters_consume_their_text() {
        let km = defaults::preset(KeyPreset::Vim);
        let (f, out) = feed(&km, &[key(KeyCode::Char('j'), NONE), key(KeyCode::Char('q'), NONE)]);
        assert_eq!(out, vec![Feed::Action(Action::CursorDown)]);
        assert_eq!(f.consumed, vec![true, true, false, false]);
        assert_eq!(f.typed_text(), "q");
        // `G` (Shift+G) jumps to the end; `:` opens the palette.
        let (f, out) = feed(&km, &[key(KeyCode::Char('G'), NONE), key(KeyCode::Char(':'), NONE)]);
        assert_eq!(out, vec![Feed::Action(Action::CursorEnd), Feed::Action(Action::CommandPalette)]);
        assert_eq!(f.typed_text(), "");
    }

    #[test]
    fn explorer_letters_reach_type_ahead() {
        let km = defaults::preset(KeyPreset::Explorer);
        let (f, out) = feed(&km, &[key(KeyCode::Char('r'), NONE), key(KeyCode::Char('E'), SHIFT)]);
        assert!(out.is_empty());
        assert_eq!(f.typed_text(), "rE");
        // A bound plain key (Space = Quick Look) swallows its text; Ctrl+T has none.
        let (f, out) = feed(&km, &[key(KeyCode::Char(' '), NONE), key(KeyCode::Char('t'), CTRL)]);
        assert_eq!(out, vec![Feed::Action(Action::QuickLook), Feed::Action(Action::NewTab)]);
        assert_eq!(f.consumed, vec![true, true, true]);
        // Paste is text.
        let (f, _) = feed(&km, &[Event::Paste("abc".into())]);
        assert_eq!(f.typed_text(), "abc");
    }

    /// `keys:` chords survive the round trip through crossterm events and back.
    #[test]
    fn script_chords_round_trip() {
        let mods = [
            Mods::empty(),
            Mods::CTRL,
            Mods::ALT,
            Mods::SHIFT,
            Mods::CMD,
            Mods::CTRL | Mods::SHIFT,
            Mods::ALT | Mods::SHIFT,
            Mods::CTRL | Mods::ALT,
        ];
        for &key in KeyName::ALL {
            for m in mods {
                // `+` is a shifted key on most layouts: the presets spell it without Shift.
                let want = if key == KeyName::Plus { m - Mods::SHIFT } else { m };
                let chord = KeyChord::new(m, key);
                let events = to_crossterm(&KeyTok::Chord(chord));
                assert_eq!(events.len(), 1, "{chord}");
                let back = convert(&events[0], false, false);
                assert_eq!(back.first(), Some(&InputEvent::Chord(KeyChord::new(want, key))), "{chord}: {events:?}");
            }
        }
    }

    #[test]
    fn script_text_and_clipboard_tokens() {
        let typed =
            |t: &[KeyTok]| frame(&t.iter().flat_map(to_crossterm).collect::<Vec<_>>(), true, false).typed_text();
        let typing: Vec<KeyTok> = "aB ş:日".chars().map(KeyTok::Text).collect();
        assert_eq!(typed(&typing), "aB ş:日");
        assert_eq!(to_crossterm(&KeyTok::Text('\n')), vec![key(KeyCode::Enter, NONE)]);
        assert_eq!(to_crossterm(&KeyTok::Copy), vec![key(KeyCode::Char('c'), CTRL)]);
        assert_eq!(to_crossterm(&KeyTok::Cut), vec![key(KeyCode::Char('x'), CTRL)]);
        assert_eq!(to_crossterm(&KeyTok::Paste("x y".into())), vec![Event::Paste("x y".into())]);
        // A whole spec, as the script parses it.
        let spec = soyuz_appcore::devscript::parse_keys("ab{Ctrl+Shift+P}{Shift+Tab}");
        let events: Vec<Event> = spec.iter().flat_map(to_crossterm).collect();
        assert_eq!(
            frame(&events, true, false).events,
            vec![ch("A"), text("a"), ch("B"), text("b"), ch("Ctrl+Shift+P"), ch("Shift+Tab")]
        );
    }
}
