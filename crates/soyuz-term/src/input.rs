//! Toolkit-free terminal input encoding: keys ([`TermKey`] + [`TermMods`] → xterm
//! CSI / SS3 sequences, Ctrl+letter, Alt = ESC prefix), mouse reports (X10 / UTF-8 /
//! SGR), paste framing and focus reports, against the program's [`Modes`]. The egui
//! widget maps its key events onto these, so both front ends send the same bytes.
//!
//! Printable text is *not* encoded here: a front end sends typed text verbatim (with an
//! ESC prefix when Alt acts as Meta, see [`ALT_IS_META`]). [`encode_key`] only handles
//! keys that produce no text, plus Ctrl / Alt combinations.

use alacritty_terminal::term::TermMode;

pub use crate::encode::{focus_report, paste_bytes};

/// A key, independent of any UI toolkit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TermKey {
    /// A letter (`a`-`z`; an upper-case letter implies Shift), a digit or a symbol key
    /// (`[`, `\`, `]`, `/`, `-`, …). `' '` is the same as [`TermKey::Space`].
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    /// F1-F20 (others are not encoded).
    F(u8),
    Enter,
    Tab,
    Backspace,
    Escape,
    Space,
}

/// Held modifiers. (Command / Super is not a terminal modifier: front ends keep those
/// chords for themselves.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TermMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl TermMods {
    pub const NONE: TermMods = TermMods { shift: false, alt: false, ctrl: false };

    pub const fn new(shift: bool, alt: bool, ctrl: bool) -> Self {
        TermMods { shift, alt, ctrl }
    }

    pub fn is_none(self) -> bool {
        self == TermMods::NONE
    }
}

/// The terminal modes a program has switched on that change how input is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modes {
    /// The alternate screen (full-screen programs: no scrollback).
    pub alt_screen: bool,
    /// DECCKM: cursor keys send SS3 sequences.
    pub app_cursor: bool,
    /// LNM: Enter sends CR LF.
    pub line_feed_new_line: bool,
    pub bracketed_paste: bool,
    /// Mouse reporting of clicks (1000).
    pub mouse_click: bool,
    /// Mouse reporting of drags (1002).
    pub mouse_drag: bool,
    /// Mouse reporting of every motion (1003).
    pub mouse_motion: bool,
    /// SGR mouse encoding (1006).
    pub sgr_mouse: bool,
    /// UTF-8 mouse encoding (1005).
    pub utf8_mouse: bool,
    /// Focus in / out reports (1004).
    pub focus_in_out: bool,
    /// Wheel → arrow keys in the alternate screen (1007).
    pub alternate_scroll: bool,
}

impl Modes {
    /// Any mouse reporting mode is on: clicks, drags and the wheel belong to the program.
    pub fn mouse_any(self) -> bool {
        self.mouse_click || self.mouse_drag || self.mouse_motion
    }

    pub(crate) fn from_term_mode(m: TermMode) -> Self {
        Modes {
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            app_cursor: m.contains(TermMode::APP_CURSOR),
            line_feed_new_line: m.contains(TermMode::LINE_FEED_NEW_LINE),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            mouse_click: m.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: m.contains(TermMode::MOUSE_DRAG),
            mouse_motion: m.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: m.contains(TermMode::SGR_MOUSE),
            utf8_mouse: m.contains(TermMode::UTF8_MOUSE),
            focus_in_out: m.contains(TermMode::FOCUS_IN_OUT),
            alternate_scroll: m.contains(TermMode::ALTERNATE_SCROLL),
        }
    }
}

/// Whether Alt acts as Meta (ESC prefix). On macOS Option composes characters instead.
pub const ALT_IS_META: bool = !cfg!(target_os = "macos");

/// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn mod_param(m: TermMods) -> u8 {
    1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl)
}

/// `Char` normalised: `' '` → `Space`, an upper-case letter → lower case + Shift.
fn normalize(key: TermKey, m: TermMods) -> (TermKey, TermMods) {
    match key {
        TermKey::Char(' ') => (TermKey::Space, m),
        TermKey::Char(c) if c.is_ascii_uppercase() => {
            (TermKey::Char(c.to_ascii_lowercase()), TermMods { shift: true, ..m })
        }
        k => (k, m),
    }
}

fn letter(key: TermKey) -> Option<u8> {
    match key {
        TermKey::Char(c) if c.is_ascii_lowercase() => Some(c as u8),
        _ => None,
    }
}

fn digit(key: TermKey) -> Option<u8> {
    match key {
        TermKey::Char(c) if c.is_ascii_digit() => Some(c as u8),
        _ => None,
    }
}

/// Control byte for Ctrl + a non-letter key (xterm conventions).
fn ctrl_symbol(key: TermKey) -> Option<u8> {
    Some(match key {
        TermKey::Space | TermKey::Char('2') => 0x00,
        TermKey::Char('[' | '3') => 0x1b,
        TermKey::Char('\\' | '4') => 0x1c,
        TermKey::Char(']' | '5') => 0x1d,
        TermKey::Char('6') => 0x1e,
        TermKey::Char('/' | '-' | '7') => 0x1f,
        TermKey::Char('8') => 0x7f,
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

/// Encode a key press. `None` = not handled here (typed text, or a chord the front end
/// keeps, such as Ctrl+Tab).
pub fn encode_key(key: TermKey, mods: TermMods, modes: &Modes) -> Option<Vec<u8>> {
    let (key, m) = normalize(key, mods);
    let mp = mod_param(m);
    let modded = mp > 1;
    let alt = m.alt && ALT_IS_META;

    // Cursor keys / Home / End: CSI or SS3 (application cursor mode) final byte.
    let cursor_final = match key {
        TermKey::Up => Some('A'),
        TermKey::Down => Some('B'),
        TermKey::Right => Some('C'),
        TermKey::Left => Some('D'),
        TermKey::Home => Some('H'),
        TermKey::End => Some('F'),
        _ => None,
    };
    if let Some(f) = cursor_final {
        return Some(if modded {
            format!("\x1b[1;{mp}{f}").into_bytes()
        } else if modes.app_cursor {
            format!("\x1bO{f}").into_bytes()
        } else {
            format!("\x1b[{f}").into_bytes()
        });
    }

    // `CSI n ~` keys.
    let tilde = match key {
        TermKey::Insert => Some(2),
        TermKey::Delete => Some(3),
        TermKey::PageUp => Some(5),
        TermKey::PageDown => Some(6),
        TermKey::F(n) => match n {
            5 => Some(15),
            6 => Some(17),
            7 => Some(18),
            8 => Some(19),
            9 => Some(20),
            10 => Some(21),
            11 => Some(23),
            12 => Some(24),
            13 => Some(25),
            14 => Some(26),
            15 => Some(28),
            16 => Some(29),
            17 => Some(31),
            18 => Some(32),
            19 => Some(33),
            20 => Some(34),
            _ => None,
        },
        _ => None,
    };
    if let Some(n) = tilde {
        return Some(if modded { format!("\x1b[{n};{mp}~") } else { format!("\x1b[{n}~") }.into_bytes());
    }

    match key {
        // F1-F4: SS3 P..S.
        TermKey::F(n @ 1..=4) => {
            let f = char::from(b'P' + (n - 1));
            Some(if modded { format!("\x1b[1;{mp}{f}") } else { format!("\x1bO{f}") }.into_bytes())
        }
        TermKey::F(_) => None,
        TermKey::Enter => {
            let nl: &[u8] = if modes.line_feed_new_line { b"\r\n" } else { b"\r" };
            Some(with_alt(alt, nl))
        }
        TermKey::Tab => {
            if m.ctrl {
                None // Ctrl+Tab is a tab-switch shortcut in the app.
            } else if m.shift {
                Some(b"\x1b[Z".to_vec())
            } else {
                Some(with_alt(alt, b"\t"))
            }
        }
        TermKey::Backspace => Some(with_alt(alt, if m.ctrl { b"\x08" } else { b"\x7f" })),
        TermKey::Escape => Some(with_alt(alt, b"\x1b")),
        _ => {
            if let Some(l) = letter(key) {
                if m.ctrl {
                    // On Windows Ctrl+Alt is AltGr: leave it to the text.
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
                if key == TermKey::Space {
                    return Some(b"\x1b ".to_vec());
                }
            }
            None
        }
    }
}

/// True when [`encode_key`] produced bytes for a key that *also* yields typed text
/// (Alt+letter / digit / space): the front end must drop that text.
pub fn produces_text_too(key: TermKey, mods: TermMods) -> bool {
    let (key, m) = normalize(key, mods);
    m.alt && ALT_IS_META && !m.ctrl && (letter(key).is_some() || digit(key).is_some() || key == TermKey::Space)
}

/// A mouse button (or wheel step) for [`encode_mouse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// Motion with no button held.
    None,
    WheelUp,
    WheelDown,
}

impl MouseButton {
    /// The xterm button code.
    pub(crate) const fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
            MouseButton::WheelUp => 64,
            MouseButton::WheelDown => 65,
        }
    }
}

/// Encode a mouse report. `col` / `row` are 0-based viewport cells. `None` when the
/// position cannot be encoded (beyond column 223 without UTF-8 / SGR encoding).
pub fn encode_mouse(
    button: MouseButton,
    pressed: bool,
    motion: bool,
    col: usize,
    row: usize,
    mods: TermMods,
    modes: &Modes,
) -> Option<Vec<u8>> {
    encode_mouse_code(button.code(), pressed, motion, col, row, mods, modes)
}

pub(crate) fn encode_mouse_code(
    button: u8,
    pressed: bool,
    motion: bool,
    col: usize,
    line: usize,
    m: TermMods,
    modes: &Modes,
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
    if modes.sgr_mouse {
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
        if modes.utf8_mouse {
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

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: TermMods = TermMods::NONE;
    const CTRL: TermMods = TermMods::new(false, false, true);
    const ALT: TermMods = TermMods::new(false, true, false);
    const SHIFT: TermMods = TermMods::new(true, false, false);

    fn enc(key: TermKey, m: TermMods) -> Option<Vec<u8>> {
        encode_key(key, m, &Modes::default())
    }
    fn s(b: &[u8]) -> Option<Vec<u8>> {
        Some(b.to_vec())
    }

    #[test]
    fn modes_from_term_mode() {
        let m = Modes::from_term_mode(
            TermMode::ALT_SCREEN | TermMode::APP_CURSOR | TermMode::BRACKETED_PASTE | TermMode::SGR_MOUSE,
        );
        assert!(m.alt_screen && m.app_cursor && m.bracketed_paste && m.sgr_mouse);
        assert!(!m.mouse_any() && !m.focus_in_out && !m.line_feed_new_line);
        let m = Modes::from_term_mode(TermMode::MOUSE_DRAG | TermMode::FOCUS_IN_OUT | TermMode::UTF8_MOUSE);
        assert!(m.mouse_any() && m.mouse_drag && !m.mouse_click && m.focus_in_out && m.utf8_mouse);
        assert_eq!(Modes::from_term_mode(TermMode::empty()), Modes::default());
    }

    #[test]
    fn cursor_and_editing_keys() {
        let app = Modes { app_cursor: true, ..Modes::default() };
        for (key, f) in [(TermKey::Up, 'A'), (TermKey::Down, 'B'), (TermKey::Right, 'C'), (TermKey::Left, 'D')] {
            assert_eq!(enc(key, NONE), Some(format!("\x1b[{f}").into_bytes()));
            assert_eq!(encode_key(key, NONE, &app), Some(format!("\x1bO{f}").into_bytes()));
            assert_eq!(encode_key(key, CTRL, &app), Some(format!("\x1b[1;5{f}").into_bytes()));
        }
        assert_eq!(enc(TermKey::Home, NONE), s(b"\x1b[H"));
        assert_eq!(enc(TermKey::End, SHIFT), s(b"\x1b[1;2F"));
        assert_eq!(enc(TermKey::Left, TermMods::new(true, true, true)), s(b"\x1b[1;8D"));
        assert_eq!(enc(TermKey::PageUp, NONE), s(b"\x1b[5~"));
        assert_eq!(enc(TermKey::Delete, SHIFT), s(b"\x1b[3;2~"));
        assert_eq!(enc(TermKey::Insert, NONE), s(b"\x1b[2~"));
        assert_eq!(enc(TermKey::Enter, NONE), s(b"\r"));
        let lnm = Modes { line_feed_new_line: true, ..Modes::default() };
        assert_eq!(encode_key(TermKey::Enter, NONE, &lnm), s(b"\r\n"));
        assert_eq!(enc(TermKey::Tab, NONE), s(b"\t"));
        assert_eq!(enc(TermKey::Tab, SHIFT), s(b"\x1b[Z"));
        assert_eq!(enc(TermKey::Tab, CTRL), None);
        assert_eq!(enc(TermKey::Backspace, NONE), s(b"\x7f"));
        assert_eq!(enc(TermKey::Backspace, CTRL), s(b"\x08"));
        assert_eq!(enc(TermKey::Escape, NONE), s(b"\x1b"));
    }

    #[test]
    fn function_keys() {
        let want: [&[u8]; 20] = [
            b"\x1bOP",
            b"\x1bOQ",
            b"\x1bOR",
            b"\x1bOS",
            b"\x1b[15~",
            b"\x1b[17~",
            b"\x1b[18~",
            b"\x1b[19~",
            b"\x1b[20~",
            b"\x1b[21~",
            b"\x1b[23~",
            b"\x1b[24~",
            b"\x1b[25~",
            b"\x1b[26~",
            b"\x1b[28~",
            b"\x1b[29~",
            b"\x1b[31~",
            b"\x1b[32~",
            b"\x1b[33~",
            b"\x1b[34~",
        ];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(enc(TermKey::F(i as u8 + 1), NONE).as_deref(), Some(*w), "F{}", i + 1);
        }
        assert_eq!(enc(TermKey::F(1), SHIFT), s(b"\x1b[1;2P"));
        assert_eq!(enc(TermKey::F(5), CTRL), s(b"\x1b[15;5~"));
        assert_eq!(enc(TermKey::F(0), NONE), None);
        assert_eq!(enc(TermKey::F(21), NONE), None);
    }

    #[test]
    fn ctrl_and_alt_combinations() {
        assert_eq!(enc(TermKey::Char('a'), CTRL), s(&[1]));
        assert_eq!(enc(TermKey::Char('c'), CTRL), s(&[3]));
        assert_eq!(enc(TermKey::Char('z'), CTRL), s(&[26]));
        // An upper-case letter is the letter with Shift: Ctrl+Shift+L is still FF.
        assert_eq!(enc(TermKey::Char('L'), CTRL), s(&[12]));
        assert_eq!(enc(TermKey::Space, CTRL), s(&[0]));
        assert_eq!(enc(TermKey::Char(' '), CTRL), s(&[0]));
        assert_eq!(enc(TermKey::Char('2'), CTRL), s(&[0]));
        assert_eq!(enc(TermKey::Char('['), CTRL), s(&[0x1b]));
        assert_eq!(enc(TermKey::Char('\\'), CTRL), s(&[0x1c]));
        assert_eq!(enc(TermKey::Char(']'), CTRL), s(&[0x1d]));
        assert_eq!(enc(TermKey::Char('6'), CTRL), s(&[0x1e]));
        assert_eq!(enc(TermKey::Char('/'), CTRL), s(&[0x1f]));
        assert_eq!(enc(TermKey::Char('-'), CTRL), s(&[0x1f]));
        assert_eq!(enc(TermKey::Char('8'), CTRL), s(&[0x7f]));
        assert_eq!(enc(TermKey::Char('`'), CTRL), None);
        // Plain letters, digits and space are typed text.
        assert_eq!(enc(TermKey::Char('a'), NONE), None);
        assert_eq!(enc(TermKey::Char('A'), NONE), None);
        assert_eq!(enc(TermKey::Space, NONE), None);
        assert_eq!(enc(TermKey::Char('1'), NONE), None);
        if ALT_IS_META {
            assert_eq!(enc(TermKey::Char('b'), ALT), s(b"\x1bb"));
            assert_eq!(enc(TermKey::Char('b'), TermMods::new(true, true, false)), s(b"\x1bB"));
            assert_eq!(enc(TermKey::Char('B'), ALT), s(b"\x1bB"));
            assert_eq!(enc(TermKey::Char('1'), ALT), s(b"\x1b1"));
            assert_eq!(enc(TermKey::Space, ALT), s(b"\x1b "));
            assert_eq!(enc(TermKey::Backspace, ALT), s(b"\x1b\x7f"));
            assert_eq!(enc(TermKey::Enter, ALT), s(b"\x1b\r"));
            assert!(produces_text_too(TermKey::Char('b'), ALT));
            assert!(produces_text_too(TermKey::Char(' '), ALT));
            assert!(!produces_text_too(TermKey::Char('b'), CTRL));
            assert!(!produces_text_too(TermKey::Up, ALT));
            if cfg!(windows) {
                // Ctrl+Alt is AltGr on Windows: the composed character arrives as text.
                assert_eq!(enc(TermKey::Char('a'), TermMods::new(false, true, true)), None);
            } else {
                assert_eq!(enc(TermKey::Char('a'), TermMods::new(false, true, true)), s(b"\x1b\x01"));
            }
        } else {
            assert_eq!(enc(TermKey::Char('b'), ALT), None);
            assert!(!produces_text_too(TermKey::Char('b'), ALT));
        }
    }

    #[test]
    fn mouse_reports() {
        let sgr = Modes { sgr_mouse: true, mouse_click: true, ..Modes::default() };
        assert_eq!(encode_mouse(MouseButton::Left, true, false, 0, 0, NONE, &sgr), s(b"\x1b[<0;1;1M"));
        assert_eq!(encode_mouse(MouseButton::Left, false, false, 9, 4, NONE, &sgr), s(b"\x1b[<0;10;5m"));
        assert_eq!(encode_mouse(MouseButton::Right, true, false, 0, 0, CTRL, &sgr), s(b"\x1b[<18;1;1M"));
        assert_eq!(encode_mouse(MouseButton::WheelUp, true, false, 2, 3, NONE, &sgr), s(b"\x1b[<64;3;4M"));
        assert_eq!(encode_mouse(MouseButton::WheelDown, true, false, 2, 3, SHIFT, &sgr), s(b"\x1b[<69;3;4M"));
        assert_eq!(encode_mouse(MouseButton::None, true, true, 0, 0, NONE, &sgr), s(b"\x1b[<35;1;1M"));
        let x10 = Modes { mouse_click: true, ..Modes::default() };
        assert_eq!(encode_mouse(MouseButton::Left, true, false, 0, 0, NONE, &x10), s(&[0x1b, b'[', b'M', 32, 33, 33]));
        assert_eq!(
            encode_mouse(MouseButton::Middle, false, false, 1, 1, NONE, &x10),
            s(&[0x1b, b'[', b'M', 35, 34, 34])
        );
        assert_eq!(encode_mouse(MouseButton::Left, true, false, 300, 0, NONE, &x10), None);
        let utf8 = Modes { utf8_mouse: true, ..x10 };
        let out = encode_mouse(MouseButton::Left, true, false, 300, 0, NONE, &utf8).unwrap();
        assert_eq!(&out[4..], "\u{14d}!".as_bytes());
    }

    #[test]
    fn paste_and_focus() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb");
        assert_eq!(paste_bytes("x", true), b"\x1b[200~x\x1b[201~");
        assert_eq!(focus_report(true), b"\x1b[I");
        assert_eq!(focus_report(false), b"\x1b[O");
    }
}
