//! The terminal UI's text clipboard: the local clipboard and OSC 52 (`[tui] osc52`).
//!
//! `TuiHost::copy_text` calls [`copy`]:
//! * the **local** clipboard is written directly where the core has a text path for it
//!   (`services::os_clipboard::set_text`: Windows `CF_UNICODETEXT` through raw user32,
//!   Linux the native Wayland / X11 selection when the desktop offers one).
//!   Elsewhere there is none, so OSC 52 carries the text. Over SSH the local clipboard belongs to the remote machine and is skipped.
//! * **OSC 52** asks the terminal to set the clipboard of the machine it runs on
//!   (Windows Terminal, kitty, WezTerm, iTerm2, foot, Alacritty, xterm with
//!   `allowWindowOps`, tmux with `set-clipboard`): `always`, `never`, or `auto` = over SSH
//!   (`SSH_TTY` / `SSH_CONNECTION` / `SSH_CLIENT`) or when the local clipboard could not
//!   be set ([`send_osc52`]). Inside tmux (`TMUX`) the sequence is wrapped in tmux's DCS
//!   passthrough, inside GNU screen (`STY`) in screen's (in chunks, screen limits a DCS
//!   string); texts whose encoding exceeds [`MAX_OSC52_BASE64`] bytes are not sent (many
//!   terminals drop or truncate longer ones).
//!
//! The file clipboard (Copy / Cut / Paste of files) is the core's
//! (`services::os_clipboard::set_files` / `with_files`), as in the GUI.

use soyuz_core::config::TuiOsc52;

/// The largest base64 payload sent with OSC 52 (xterm's and hterm's limit is about
/// 100 000 bytes).
pub const MAX_OSC52_BASE64: usize = 100_000;

/// GNU screen passes at most this many bytes per DCS string: longer sequences go out in
/// several.
const SCREEN_CHUNK: usize = 76;

/// Where the terminal UI runs (read from the environment once, at start).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// Over SSH: the local clipboard is the remote machine's.
    pub ssh: bool,
    /// Inside tmux.
    pub tmux: bool,
    /// Inside GNU screen.
    pub screen: bool,
}

impl Env {
    /// The process environment.
    pub fn detect() -> Env {
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        Env { ssh: set("SSH_TTY") || set("SSH_CONNECTION") || set("SSH_CLIENT"), tmux: set("TMUX"), screen: set("STY") }
    }
}

/// Whether a copy writes OSC 52 under `policy`. `local_ok`: the local clipboard took the
/// text.
pub fn send_osc52(policy: TuiOsc52, env: &Env, local_ok: bool) -> bool {
    match policy {
        TuiOsc52::Always => true,
        TuiOsc52::Never => false,
        TuiOsc52::Auto => env.ssh || !local_ok,
    }
}

/// Copy `text`: set the local clipboard (when `local` and not over SSH), then return the
/// OSC 52 bytes to write to the terminal, if any.
pub fn copy(text: &str, policy: TuiOsc52, env: &Env, local: bool) -> Option<Vec<u8>> {
    let local_ok = local && !env.ssh && set_local(text);
    if send_osc52(policy, env, local_ok) { osc52(text, env) } else { None }
}

/// The local clipboard (Windows; Linux with a native Wayland / X11 backend; `false` elsewhere).
fn set_local(text: &str) -> bool {
    soyuz_appcore::services::os_clipboard::set_text(text)
}

/// The base64 of `text` (crossterm's OSC 52 encoder, without its framing).
fn base64(text: &str) -> String {
    let seq = String::from_utf8(crate::host::osc52_bytes(text)).unwrap_or_default();
    seq.strip_prefix("\x1b]52;c;").and_then(|s| s.strip_suffix("\x1b\\")).unwrap_or_default().to_string()
}

/// The OSC 52 sequence for `text` in `env` (wrapped for tmux / screen); `None` when the
/// text is too large.
pub fn osc52(text: &str, env: &Env) -> Option<Vec<u8>> {
    let b64 = base64(text);
    if b64.len() > MAX_OSC52_BASE64 {
        log::warn!("tui: {} bytes are too many for OSC 52; not sent", text.len());
        return None;
    }
    let out = if env.tmux {
        // tmux passthrough: `ESC P tmux; <sequence, every ESC doubled> ESC \`.
        format!("\x1bPtmux;\x1b\x1b]52;c;{b64}\x07\x1b\\")
    } else if env.screen {
        // screen passthrough: the sequence split into DCS strings of SCREEN_CHUNK bytes.
        let seq = format!("\x1b]52;c;{b64}\x07");
        let mut s = String::with_capacity(seq.len() + seq.len() / SCREEN_CHUNK * 4 + 4);
        for chunk in seq.as_bytes().chunks(SCREEN_CHUNK) {
            s.push_str("\x1bP");
            // The sequence is ASCII: every chunk is valid UTF-8.
            s.push_str(std::str::from_utf8(chunk).unwrap_or_default());
            s.push_str("\x1b\\");
        }
        s
    } else {
        format!("\x1b]52;c;{b64}\x1b\\")
    };
    Some(out.into_bytes())
}

/// `TuiOsc52` as stored in the host (an atomic byte).
pub fn to_byte(p: TuiOsc52) -> u8 {
    match p {
        TuiOsc52::Auto => 0,
        TuiOsc52::Always => 1,
        TuiOsc52::Never => 2,
    }
}

/// The inverse of [`to_byte`] (unknown bytes are `Auto`).
pub fn from_byte(b: u8) -> TuiOsc52 {
    match b {
        1 => TuiOsc52::Always,
        2 => TuiOsc52::Never,
        _ => TuiOsc52::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: Env = Env { ssh: false, tmux: false, screen: false };
    const SSH: Env = Env { ssh: true, tmux: false, screen: false };

    #[test]
    fn policies() {
        assert!(send_osc52(TuiOsc52::Always, &LOCAL, true));
        assert!(!send_osc52(TuiOsc52::Never, &SSH, false));
        // Auto: over SSH, or when the local clipboard did not take the text.
        assert!(send_osc52(TuiOsc52::Auto, &SSH, true));
        assert!(send_osc52(TuiOsc52::Auto, &LOCAL, false));
        assert!(!send_osc52(TuiOsc52::Auto, &LOCAL, true));
        for p in TuiOsc52::ALL {
            assert_eq!(from_byte(to_byte(p)), p);
        }
    }

    #[test]
    fn copy_without_local_clipboard() {
        // `local: false` never touches the system clipboard.
        assert_eq!(copy("hi", TuiOsc52::Auto, &LOCAL, false).unwrap(), b"\x1b]52;c;aGk=\x1b\\");
        assert_eq!(copy("hi", TuiOsc52::Always, &SSH, false).unwrap(), b"\x1b]52;c;aGk=\x1b\\");
        assert!(copy("hi", TuiOsc52::Never, &SSH, false).is_none());
    }

    #[test]
    fn plain_and_wrapped_sequences() {
        assert_eq!(osc52("hi", &LOCAL).unwrap(), b"\x1b]52;c;aGk=\x1b\\");
        assert_eq!(osc52("", &LOCAL).unwrap(), b"\x1b]52;c;\x1b\\");
        let tmux = Env { tmux: true, ..LOCAL };
        assert_eq!(osc52("hi", &tmux).unwrap(), b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\");
        let screen = Env { screen: true, ..LOCAL };
        let s = String::from_utf8(osc52(&"x".repeat(200), &screen).unwrap()).unwrap();
        let chunks: Vec<&str> = s.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert!(chunks.len() > 1 && chunks.iter().all(|c| c.starts_with("\x1bP") && c.len() <= SCREEN_CHUNK + 2));
        let joined: String = chunks.iter().map(|c| &c[2..]).collect();
        assert!(joined.starts_with("\x1b]52;c;eHh4") && joined.ends_with('\x07'));
        // Unicode survives the encoding.
        assert_eq!(base64("ş"), "xZ8=");
    }

    #[test]
    fn size_cap() {
        let max_text = MAX_OSC52_BASE64 / 4 * 3;
        assert!(osc52(&"a".repeat(max_text), &LOCAL).is_some());
        assert!(osc52(&"a".repeat(max_text + 3), &LOCAL).is_none());
    }
}
