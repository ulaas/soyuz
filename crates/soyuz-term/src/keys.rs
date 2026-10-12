//! egui `Key` + `Modifiers` + `TermMode` → bytes (xterm CSI/SS3 tables, Ctrl+letter,
//! Alt = ESC prefix), mouse reports (X10 / UTF-8 / SGR) and paste framing.
//!
//! Printable text is *not* encoded here: it arrives as `egui::Event::Text` and is sent
//! verbatim (with an ESC prefix when Alt is held). `encode_key` only handles keys that
//! produce no text event, plus Ctrl/Alt combinations.

use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};

/// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn mod_param(m: Modifiers) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl)
}

fn letter(key: Key) -> Option<u8> {
    use Key::*;
    let k = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    k.iter().position(|&x| x == key).map(|i| b'a' + i as u8)
}

fn digit(key: Key) -> Option<u8> {
    use Key::*;
    let k = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
    k.iter().position(|&x| x == key).map(|i| b'0' + i as u8)
}

/// Control byte for Ctrl + a non-letter key (xterm conventions).
fn ctrl_symbol(key: Key) -> Option<u8> {
    Some(match key {
        Key::Space | Key::Num2 => 0x00,
        Key::OpenBracket | Key::Num3 => 0x1b,
        Key::Backslash | Key::Num4 => 0x1c,
        Key::CloseBracket | Key::Num5 => 0x1d,
        Key::Num6 => 0x1e,
        Key::Slash | Key::Minus | Key::Num7 => 0x1f,
        Key::Num8 => 0x7f,
        _ => return None,
    })
}

fn with_alt(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(bytes.len() + 1);
    if alt {
        v.push(0x1b);
    }
    v.extend_from_slice(bytes);
    v
}

/// Whether Alt acts as Meta (ESC prefix). On macOS Option composes characters instead.
pub(crate) const ALT_IS_META: bool = !cfg!(target_os = "macos");

/// Encode a key press. `None` = not handled here (text events / app shortcuts).
pub(crate) fn encode_key(key: Key, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    if m.mac_cmd {
        return None; // Cmd shortcuts belong to the app.
    }
    let mp = mod_param(m);
    let modded = mp > 1;
    let alt = m.alt && ALT_IS_META;

    // Cursor keys / Home / End: CSI or SS3 (application cursor mode) final byte.
    let cursor_final = match key {
        Key::ArrowUp => Some(b'A'),
        Key::ArrowDown => Some(b'B'),
        Key::ArrowRight => Some(b'C'),
        Key::ArrowLeft => Some(b'D'),
        Key::Home => Some(b'H'),
        Key::End => Some(b'F'),
        _ => None,
    };
    if let Some(f) = cursor_final {
        let f = f as char;
        return Some(if modded {
            format!("\x1b[1;{mp}{f}").into_bytes()
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{f}").into_bytes()
        } else {
            format!("\x1b[{f}").into_bytes()
        });
    }

    // `CSI n ~` keys.
    let tilde = match key {
        Key::Insert => Some(2),
        Key::Delete => Some(3),
        Key::PageUp => Some(5),
        Key::PageDown => Some(6),
        Key::F5 => Some(15),
        Key::F6 => Some(17),
        Key::F7 => Some(18),
        Key::F8 => Some(19),
        Key::F9 => Some(20),
        Key::F10 => Some(21),
        Key::F11 => Some(23),
        Key::F12 => Some(24),
        Key::F13 => Some(25),
        Key::F14 => Some(26),
        Key::F15 => Some(28),
        Key::F16 => Some(29),
        Key::F17 => Some(31),
        Key::F18 => Some(32),
        Key::F19 => Some(33),
        Key::F20 => Some(34),
        _ => None,
    };
    if let Some(n) = tilde {
        return Some(if modded { format!("\x1b[{n};{mp}~") } else { format!("\x1b[{n}~") }.into_bytes());
    }

    // F1-F4: SS3 P..S.
    let pf = match key {
        Key::F1 => Some('P'),
        Key::F2 => Some('Q'),
        Key::F3 => Some('R'),
        Key::F4 => Some('S'),
        _ => None,
    };
    if let Some(f) = pf {
        return Some(if modded { format!("\x1b[1;{mp}{f}") } else { format!("\x1bO{f}") }.into_bytes());
    }

    match key {
        Key::Enter => {
            let nl: &[u8] = if mode.contains(TermMode::LINE_FEED_NEW_LINE) { b"\r\n" } else { b"\r" };
            Some(with_alt(alt, nl))
        }
        Key::Tab => {
            if m.ctrl {
                None // Ctrl+Tab is a tab-switch shortcut in the app.
            } else if m.shift {
                Some(b"\x1b[Z".to_vec())
            } else {
                Some(with_alt(alt, b"\t"))
            }
        }
        Key::Backspace => Some(with_alt(alt, if m.ctrl { b"\x08" } else { b"\x7f" })),
        Key::Escape => Some(with_alt(alt, b"\x1b")),
        _ => {
            if let Some(l) = letter(key) {
                if m.ctrl {
                    // On Windows Ctrl+Alt is AltGr: leave it to the text event.
                    if m.alt && cfg!(windows) {
                        return None;
                    }
                    return Some(with_alt(alt, &[l - b'a' + 1]));
                }
                if alt {
                    let c = if m.shift { l.to_ascii_uppercase() } else { l };
                    return Some(vec![0x1b, c]);
                }
                return None;
            }
            if m.ctrl {
                if m.alt && cfg!(windows) {
                    return None;
                }
                return ctrl_symbol(key).map(|b| with_alt(alt, &[b]));
            }
            if alt && !m.shift {
                if let Some(d) = digit(key) {
                    return Some(vec![0x1b, d]);
                }
                if key == Key::Space {
                    return Some(b"\x1b ".to_vec());
                }
            }
            None
        }
    }
}

/// True when `encode_key` produced bytes for a key that *also* yields a text event
/// (Alt+letter/digit/space), so the following `Event::Text` must be dropped.
pub(crate) fn produces_text_too(key: Key, m: Modifiers) -> bool {
    m.alt && ALT_IS_META && !m.ctrl && (letter(key).is_some() || digit(key).is_some() || key == Key::Space)
}

/// Mouse button codes for reports.
pub(crate) const MOUSE_LEFT: u8 = 0;
pub(crate) const MOUSE_MIDDLE: u8 = 1;
pub(crate) const MOUSE_RIGHT: u8 = 2;
pub(crate) const MOUSE_NONE: u8 = 3;
pub(crate) const MOUSE_WHEEL_UP: u8 = 64;
pub(crate) const MOUSE_WHEEL_DOWN: u8 = 65;

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
    let mut cb = button;
    if m.shift {
        cb += 4;
    }
    if m.alt {
        cb += 8;
    }
    if m.ctrl {
        cb += 16;
    }
    if motion {
        cb += 32;
    }
    let (x, y) = (col + 1, line + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let f = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{cb};{x};{y}{f}").into_bytes());
    }
    if !pressed && button < 64 {
        // X10/normal: release is button 3 (keeps modifier/motion bits).
        cb = (cb & !0b11) | 3;
    }
    let mut out = vec![0x1b, b'[', b'M', 32 + cb];
    for v in [x, y] {
        let v = v + 32;
        if mode.contains(TermMode::UTF8_MOUSE) {
            if v > 2047 {
                return None;
            }
            let mut buf = [0; 4];
            out.extend_from_slice(char::from_u32(v as u32)?.encode_utf8(&mut buf).as_bytes());
        } else {
            if v > 255 {
                return None;
            }
            out.push(v as u8);
        }
    }
    Some(out)
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
