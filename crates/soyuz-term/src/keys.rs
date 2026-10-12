//! egui `Key` + `Modifiers` + `TermMode` → bytes: maps egui's keys onto the toolkit-free
//! [`crate::input`] encoder (xterm CSI/SS3 tables, Ctrl+letter, Alt = ESC prefix, mouse
//! reports, paste framing), so the GUI and the terminal UI send the same bytes.
//!
//! Printable text is *not* encoded here: it arrives as `egui::Event::Text` and is sent
//! verbatim (with an ESC prefix when Alt is held). `encode_key` only handles keys that
//! produce no text event, plus Ctrl/Alt combinations.

use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};

use crate::input::{self, Modes, TermKey, TermMods};

/// The toolkit-free key for an egui key (`None`: not a terminal key).
pub(crate) fn term_key(key: Key) -> Option<TermKey> {
    use Key::*;
    let letters = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    if let Some(i) = letters.iter().position(|&x| x == key) {
        return Some(TermKey::Char(char::from(b'a' + i as u8)));
    }
    let digits = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
    if let Some(i) = digits.iter().position(|&x| x == key) {
        return Some(TermKey::Char(char::from(b'0' + i as u8)));
    }
    let functions = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
        F25, F26, F27, F28, F29, F30, F31, F32, F33, F34, F35,
    ];
    if let Some(i) = functions.iter().position(|&x| x == key) {
        return Some(TermKey::F(i as u8 + 1));
    }
    Some(match key {
        ArrowUp => TermKey::Up,
        ArrowDown => TermKey::Down,
        ArrowLeft => TermKey::Left,
        ArrowRight => TermKey::Right,
        Home => TermKey::Home,
        End => TermKey::End,
        Insert => TermKey::Insert,
        Delete => TermKey::Delete,
        PageUp => TermKey::PageUp,
        PageDown => TermKey::PageDown,
        Enter => TermKey::Enter,
        Tab => TermKey::Tab,
        Backspace => TermKey::Backspace,
        Escape => TermKey::Escape,
        Space => TermKey::Space,
        OpenBracket => TermKey::Char('['),
        CloseBracket => TermKey::Char(']'),
        Backslash => TermKey::Char('\\'),
        Slash => TermKey::Char('/'),
        Minus => TermKey::Char('-'),
        Backtick => TermKey::Char('`'),
        Comma => TermKey::Char(','),
        Period => TermKey::Char('.'),
        Semicolon => TermKey::Char(';'),
        Colon => TermKey::Char(':'),
        Quote => TermKey::Char('\''),
        Equals => TermKey::Char('='),
        Plus => TermKey::Char('+'),
        Pipe => TermKey::Char('|'),
        Questionmark => TermKey::Char('?'),
        Exclamationmark => TermKey::Char('!'),
        OpenCurlyBracket => TermKey::Char('{'),
        CloseCurlyBracket => TermKey::Char('}'),
        _ => return None,
    })
}

/// The terminal modifiers of egui modifiers (Command / Super is not one).
pub(crate) fn term_mods(m: Modifiers) -> TermMods {
    TermMods { shift: m.shift, alt: m.alt, ctrl: m.ctrl }
}

/// Whether Alt acts as Meta (ESC prefix). On macOS Option composes characters instead.
pub(crate) const ALT_IS_META: bool = input::ALT_IS_META;

/// Encode a key press. `None` = not handled here (text events / app shortcuts).
pub(crate) fn encode_key(key: Key, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    if m.mac_cmd {
        return None; // Cmd shortcuts belong to the app.
    }
    input::encode_key(term_key(key)?, term_mods(m), &Modes::from_term_mode(mode))
}

/// True when `encode_key` produced bytes for a key that *also* yields a text event
/// (Alt+letter/digit/space), so the following `Event::Text` must be dropped.
pub(crate) fn produces_text_too(key: Key, m: Modifiers) -> bool {
    term_key(key).is_some_and(|k| input::produces_text_too(k, term_mods(m)))
}

/// Mouse button codes for reports.
pub(crate) const MOUSE_LEFT: u8 = input::MouseButton::Left.code();
pub(crate) const MOUSE_MIDDLE: u8 = input::MouseButton::Middle.code();
pub(crate) const MOUSE_RIGHT: u8 = input::MouseButton::Right.code();
pub(crate) const MOUSE_NONE: u8 = input::MouseButton::None.code();
pub(crate) const MOUSE_WHEEL_UP: u8 = input::MouseButton::WheelUp.code();
pub(crate) const MOUSE_WHEEL_DOWN: u8 = input::MouseButton::WheelDown.code();

/// Encode a mouse report. `col`/`line` are 0-based viewport cells.
pub(crate) fn encode_mouse(
    button: u8,
    pressed: bool,
    motion: bool,
    col: usize,
    line: usize,
    m: Modifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    input::encode_mouse_code(button, pressed, motion, col, line, term_mods(m), &Modes::from_term_mode(mode))
}

// Toolkit-free encoders, shared with non-egui front ends.
pub(crate) use crate::encode::{focus_report, paste_bytes};

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    fn m(shift: bool, alt: bool, ctrl: bool) -> Modifiers {
        Modifiers { alt, ctrl, shift, mac_cmd: false, command: ctrl }
    }
    fn enc(key: Key, mods: Modifiers) -> Option<Vec<u8>> {
        encode_key(key, mods, TermMode::default())
    }
    fn s(b: &[u8]) -> Option<Vec<u8>> {
        Some(b.to_vec())
    }

    #[test]
    fn cursor_keys_normal_and_app_mode() {
        let table = [(Key::ArrowUp, 'A'), (Key::ArrowDown, 'B'), (Key::ArrowRight, 'C'), (Key::ArrowLeft, 'D')];
        for (key, f) in table {
            assert_eq!(enc(key, NONE), Some(format!("\x1b[{f}").into_bytes()));
            let app = encode_key(key, NONE, TermMode::default() | TermMode::APP_CURSOR);
            assert_eq!(app, Some(format!("\x1bO{f}").into_bytes()));
            // Modifiers always use the CSI 1;m form, even in app mode.
            let ctrl = encode_key(key, m(false, false, true), TermMode::APP_CURSOR);
            assert_eq!(ctrl, Some(format!("\x1b[1;5{f}").into_bytes()));
        }
        assert_eq!(enc(Key::Home, NONE), s(b"\x1b[H"));
        assert_eq!(enc(Key::End, NONE), s(b"\x1b[F"));
        assert_eq!(enc(Key::ArrowLeft, m(true, false, false)), s(b"\x1b[1;2D"));
        assert_eq!(enc(Key::ArrowLeft, m(false, true, false)), s(b"\x1b[1;3D"));
        assert_eq!(enc(Key::ArrowLeft, m(true, true, true)), s(b"\x1b[1;8D"));
    }

    #[test]
    fn function_and_editing_keys() {
        let table: &[(Key, &[u8])] = &[
            (Key::F1, b"\x1bOP"),
            (Key::F2, b"\x1bOQ"),
            (Key::F3, b"\x1bOR"),
            (Key::F4, b"\x1bOS"),
            (Key::F5, b"\x1b[15~"),
            (Key::F6, b"\x1b[17~"),
            (Key::F7, b"\x1b[18~"),
            (Key::F8, b"\x1b[19~"),
            (Key::F9, b"\x1b[20~"),
            (Key::F10, b"\x1b[21~"),
            (Key::F11, b"\x1b[23~"),
            (Key::F12, b"\x1b[24~"),
            (Key::Insert, b"\x1b[2~"),
            (Key::Delete, b"\x1b[3~"),
            (Key::PageUp, b"\x1b[5~"),
            (Key::PageDown, b"\x1b[6~"),
            (Key::Enter, b"\r"),
            (Key::Tab, b"\t"),
            (Key::Backspace, b"\x7f"),
            (Key::Escape, b"\x1b"),
        ];
        for (key, want) in table {
            assert_eq!(enc(*key, NONE).as_deref(), Some(*want), "{key:?}");
        }
        assert_eq!(enc(Key::F1, m(true, false, false)), s(b"\x1b[1;2P"));
        assert_eq!(enc(Key::F5, m(false, false, true)), s(b"\x1b[15;5~"));
        assert_eq!(enc(Key::Delete, m(true, false, false)), s(b"\x1b[3;2~"));
        assert_eq!(enc(Key::Tab, m(true, false, false)), s(b"\x1b[Z"));
        assert_eq!(enc(Key::Tab, m(false, false, true)), None);
        assert_eq!(enc(Key::Backspace, m(false, false, true)), s(b"\x08"));
        assert_eq!(encode_key(Key::Enter, NONE, TermMode::LINE_FEED_NEW_LINE), s(b"\r\n"));
        assert_eq!(enc(Key::F35, NONE), None);
    }

    #[test]
    fn ctrl_and_alt_combinations() {
        let ctrl = m(false, false, true);
        assert_eq!(enc(Key::A, ctrl), s(&[1]));
        assert_eq!(enc(Key::C, ctrl), s(&[3]));
        assert_eq!(enc(Key::D, ctrl), s(&[4]));
        assert_eq!(enc(Key::Z, ctrl), s(&[26]));
        assert_eq!(enc(Key::L, m(true, false, true)), s(&[12]));
        assert_eq!(enc(Key::Space, ctrl), s(&[0]));
        assert_eq!(enc(Key::OpenBracket, ctrl), s(&[0x1b]));
        assert_eq!(enc(Key::Backslash, ctrl), s(&[0x1c]));
        assert_eq!(enc(Key::CloseBracket, ctrl), s(&[0x1d]));
        assert_eq!(enc(Key::Slash, ctrl), s(&[0x1f]));
        assert_eq!(enc(Key::Backtick, ctrl), None);
        // Plain letters and space are text events.
        assert_eq!(enc(Key::A, NONE), None);
        assert_eq!(enc(Key::Space, NONE), None);
        assert_eq!(enc(Key::A, m(true, false, false)), None);
        if ALT_IS_META {
            let alt = m(false, true, false);
            assert_eq!(enc(Key::B, alt), s(b"\x1bb"));
            assert_eq!(enc(Key::B, m(true, true, false)), s(b"\x1bB"));
            assert_eq!(enc(Key::Num1, alt), s(b"\x1b1"));
            assert_eq!(enc(Key::Backspace, alt), s(b"\x1b\x7f"));
            assert_eq!(enc(Key::Enter, alt), s(b"\x1b\r"));
            assert!(produces_text_too(Key::B, alt));
            assert!(!produces_text_too(Key::B, ctrl));
            if !cfg!(windows) {
                assert_eq!(enc(Key::A, m(false, true, true)), s(b"\x1b\x01"));
            }
        }
        let cmd = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };
        assert_eq!(enc(Key::A, cmd), None);
    }

    #[test]
    fn mouse_reports() {
        let sgr = TermMode::SGR_MOUSE | TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(encode_mouse(MOUSE_LEFT, true, false, 0, 0, NONE, sgr), s(b"\x1b[<0;1;1M"));
        assert_eq!(encode_mouse(MOUSE_LEFT, false, false, 9, 4, NONE, sgr), s(b"\x1b[<0;10;5m"));
        assert_eq!(encode_mouse(MOUSE_RIGHT, true, false, 0, 0, m(false, false, true), sgr), s(b"\x1b[<18;1;1M"));
        assert_eq!(encode_mouse(MOUSE_WHEEL_UP, true, false, 2, 3, NONE, sgr), s(b"\x1b[<64;3;4M"));
        assert_eq!(encode_mouse(MOUSE_NONE, true, true, 0, 0, NONE, sgr), s(b"\x1b[<35;1;1M"));
        let x10 = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(encode_mouse(MOUSE_LEFT, true, false, 0, 0, NONE, x10), s(&[0x1b, b'[', b'M', 32, 33, 33]));
        assert_eq!(encode_mouse(MOUSE_MIDDLE, false, false, 1, 1, NONE, x10), s(&[0x1b, b'[', b'M', 35, 34, 34]));
        assert_eq!(encode_mouse(MOUSE_LEFT, true, false, 300, 0, NONE, x10), None);
        let utf8 = x10 | TermMode::UTF8_MOUSE;
        let out = encode_mouse(MOUSE_LEFT, true, false, 300, 0, NONE, utf8).unwrap();
        assert_eq!(&out[4..], "\u{14d}!".as_bytes());
    }
}
