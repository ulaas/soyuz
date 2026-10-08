//! Entry points for the `fuzz/` targets (cargo feature `fuzzing`): untrusted terminal
//! output (escape sequences, titles used for cwd detection), key / mouse / paste encoding
//! and URL detection. Each function must never panic for any input.

use alacritty_terminal::Term;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use egui::{Key, Modifiers};

use crate::backend::GridSize;

/// Feed raw PTY output through the VT parser into a small terminal, then read the grid.
pub fn feed_escapes(bytes: &[u8]) {
    let grid = GridSize { cols: 40, lines: 12 };
    let mut term = Term::new(Config::default(), &grid, VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, bytes);
    let _ = term.renderable_content().display_iter.count();
    let _ = term.mode();
}

/// Cwd detection from a window title (`user@host: /path`, `C:\path>` ...).
pub fn cwd_from_title(title: &str) -> Option<std::path::PathBuf> {
    crate::pty::parse_cwd_title(title)
}

/// Encode one key / mouse / focus event derived from `data` (key index, modifier bits,
/// terminal mode bits, mouse position).
pub fn encode_input(data: &[u8]) {
    let b = |i: usize| data.get(i).copied().unwrap_or(0);
    let key = Key::ALL[b(0) as usize % Key::ALL.len()];
    let m = b(1);
    let mods =
        Modifiers { alt: m & 1 != 0, ctrl: m & 2 != 0, shift: m & 4 != 0, mac_cmd: m & 8 != 0, command: m & 16 != 0 };
    let mode = TermMode::from_bits_truncate(u32::from_le_bytes([b(2), b(3), b(4), b(5)]));
    let _ = crate::keys::encode_key(key, mods, mode);
    let _ = crate::keys::produces_text_too(key, mods);
    let (col, line) = (usize::from(b(7)) * 37, usize::from(b(8)) * 53);
    let _ = crate::keys::encode_mouse(b(6), m & 32 != 0, m & 64 != 0, col, line, mods, mode);
    let _ = crate::keys::focus_report(m & 128 != 0);
}

/// Paste encoding (bracketed and plain) and URL detection on arbitrary text.
pub fn paste_and_urls(text: &str) {
    let br = crate::keys::paste_bytes(text, true);
    // A bracketed paste can never terminate the bracket early.
    let inner = &br[6..br.len() - 6];
    assert!(!inner.windows(6).any(|w| w == b"\x1b[201~"), "paste escaped its bracket");
    let _ = crate::keys::paste_bytes(text, false);
    for u in crate::url::find_urls(text) {
        let _ = u;
    }
    let _ = crate::pty::has_control_chars(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        feed_escapes(b"\x1b[2J\x1b[31mred\x1b]0;title\x07\x1b[?1049h\x1b[200~x\x1b[201~\x1bP+q\x1b\\");
        feed_escapes(&[0x1b, b'[', 0xff, 0x00, b'9', b'9', b'9', b'9', b'9', b'9', b'9', b'9', b'm']);
        encode_input(&[3, 0xff, 0xff, 0xff, 0xff, 0xff, 1, 255, 255]);
        encode_input(&[]);
        paste_and_urls("see https://example.com/x?y=1 \x1b[201~ and\r\nmore");
        let _ = cwd_from_title("user@host: ~/src");
    }
}
