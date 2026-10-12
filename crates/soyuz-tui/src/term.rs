//! Terminal setup and restore: raw mode, the alternate screen, bracketed paste, mouse
//! capture (`[tui] mouse`), kitty keyboard enhancement (`[tui] keyboard_protocol`) and the
//! saved window title.
//!
//! [`TerminalGuard`] enables the modes and restores them on drop. What is enabled is also
//! recorded in a process-wide bit set, so [`restore_terminal`] can undo it from anywhere,
//! exactly once (the panic hook calls it: `soyuz_appcore::services::crash::set_tui_hooks`).
//!
//! Keyboard enhancement support must be queried with
//! [`crossterm::terminal::supports_keyboard_enhancement`] by the caller *before* any
//! thread reads terminal events (the query reads the terminal's reply from the same input
//! stream) and passed to [`TerminalGuard::enter`].

use std::io::{self, Write as _};
use std::sync::atomic::{AtomicU8, Ordering};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use soyuz_core::config::{KeyboardProtocol, TuiSection};

/// Save the window title on the terminal's title stack (XTWINOPS 22).
const PUSH_TITLE: &[u8] = b"\x1b[22;0t";
/// Restore the saved window title (XTWINOPS 23).
const POP_TITLE: &[u8] = b"\x1b[23;0t";

const RAW: u8 = 1 << 0;
const ALT: u8 = 1 << 1;
const PASTE: u8 = 1 << 2;
const MOUSE: u8 = 1 << 3;
const KEYBOARD: u8 = 1 << 4;
const TITLE: u8 = 1 << 5;

/// What is currently enabled (bits above).
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// The kitty keyboard flags to push, if any. Only disambiguation and alternate keys:
/// no release / repeat events and no "all keys as escape codes", so plain typing and the
/// key adapter stay simple.
pub fn keyboard_flags(
    protocol: KeyboardProtocol,
    supported: bool,
    is_windows: bool,
) -> Option<KeyboardEnhancementFlags> {
    let on = match protocol {
        KeyboardProtocol::Auto => supported && !is_windows,
        KeyboardProtocol::Kitty => supported,
        KeyboardProtocol::Legacy => false,
    };
    on.then_some(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS)
}

/// One restore step, in the order [`restore_steps`] gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    PopKeyboard,
    DisableMouse,
    DisablePaste,
    LeaveAlt,
    DisableRaw,
    ShowCursor,
    PopTitle,
}

/// Undo `enabled` in reverse order of setup (pure).
fn restore_steps(enabled: u8) -> Vec<Step> {
    let mut out = Vec::new();
    let has = |bit: u8| enabled & bit != 0;
    if has(KEYBOARD) {
        out.push(Step::PopKeyboard);
    }
    if has(MOUSE) {
        out.push(Step::DisableMouse);
    }
    if has(PASTE) {
        out.push(Step::DisablePaste);
    }
    if has(ALT) {
        out.push(Step::LeaveAlt);
    }
    if has(RAW) {
        out.push(Step::DisableRaw);
    }
    if enabled != 0 {
        out.push(Step::ShowCursor);
    }
    if has(TITLE) {
        out.push(Step::PopTitle);
    }
    out
}

fn run(step: Step) -> io::Result<()> {
    let mut out = io::stdout();
    match step {
        Step::PopKeyboard => execute!(out, PopKeyboardEnhancementFlags),
        Step::DisableMouse => execute!(out, DisableMouseCapture),
        Step::DisablePaste => execute!(out, DisableBracketedPaste),
        Step::LeaveAlt => execute!(out, LeaveAlternateScreen),
        Step::DisableRaw => crossterm::terminal::disable_raw_mode(),
        Step::ShowCursor => execute!(out, crossterm::cursor::Show),
        Step::PopTitle => out.write_all(POP_TITLE).and_then(|()| out.flush()),
    }
}

/// Restore the terminal to how it was before [`TerminalGuard::enter`]. Idempotent (only
/// the first call after a setup does anything), never panics, and takes no locks besides
/// stdout's (re-entrant, so it is safe in a panic hook on the TUI thread).
pub fn restore_terminal() {
    for step in restore_steps(ENABLED.swap(0, Ordering::AcqRel)) {
        // Best effort: keep undoing the rest even when one step fails.
        let _ = run(step);
    }
}

fn mark(bit: u8) {
    ENABLED.fetch_or(bit, Ordering::AcqRel);
}

fn unmark(bit: u8) {
    ENABLED.fetch_and(!bit, Ordering::AcqRel);
}

fn enabled(bit: u8) -> bool {
    ENABLED.load(Ordering::Acquire) & bit != 0
}

/// The terminal in TUI mode; restores it on drop.
pub struct TerminalGuard {
    keyboard_supported: bool,
}

impl TerminalGuard {
    /// Enter TUI mode. `keyboard_supported` is the result of
    /// [`crossterm::terminal::supports_keyboard_enhancement`], queried by the caller before
    /// any event-reading thread starts. On error, whatever was already enabled is restored.
    pub fn enter(cfg: &TuiSection, keyboard_supported: bool) -> io::Result<TerminalGuard> {
        let guard = TerminalGuard { keyboard_supported };
        let mut out = io::stdout();
        crossterm::terminal::enable_raw_mode()?;
        mark(RAW);
        execute!(out, EnterAlternateScreen)?;
        mark(ALT);
        // Not every terminal knows the title stack; unknown CSI sequences are ignored.
        if out.write_all(PUSH_TITLE).and_then(|()| out.flush()).is_ok() {
            mark(TITLE);
        }
        execute!(out, EnableBracketedPaste)?;
        mark(PASTE);
        guard.apply_modes(cfg)?;
        Ok(guard)
    }

    /// Re-apply the `[tui]` modes after a settings change (`TuiHost::take_modes_changed`).
    pub fn reapply(&mut self, cfg: &TuiSection) -> io::Result<()> {
        self.apply_modes(cfg)
    }

    fn apply_modes(&self, cfg: &TuiSection) -> io::Result<()> {
        let mut out = io::stdout();
        if cfg.mouse && !enabled(MOUSE) {
            execute!(out, EnableMouseCapture)?;
            mark(MOUSE);
        } else if !cfg.mouse && enabled(MOUSE) {
            unmark(MOUSE);
            execute!(out, DisableMouseCapture)?;
        }
        if enabled(KEYBOARD) {
            unmark(KEYBOARD);
            execute!(out, PopKeyboardEnhancementFlags)?;
        }
        if let Some(flags) = keyboard_flags(cfg.keyboard_protocol, self.keyboard_supported, cfg!(windows)) {
            match execute!(out, PushKeyboardEnhancementFlags(flags)) {
                Ok(()) => mark(KEYBOARD),
                // Not fatal: legacy key reading still works.
                Err(e) => log::warn!("tui: cannot enable the kitty keyboard protocol: {e}"),
            }
        }
        Ok(())
    }

    /// Whether the kitty keyboard flags are pushed right now (the key adapter may care).
    pub fn keyboard_enhanced(&self) -> bool {
        enabled(KEYBOARD)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_flags_truth_table() {
        let flags =
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
        use KeyboardProtocol::*;
        let cases = [
            (Auto, true, false, true),
            (Auto, true, true, false),
            (Auto, false, false, false),
            (Auto, false, true, false),
            (Kitty, true, false, true),
            (Kitty, true, true, true),
            (Kitty, false, false, false),
            (Kitty, false, true, false),
            (Legacy, true, false, false),
            (Legacy, true, true, false),
            (Legacy, false, false, false),
            (Legacy, false, true, false),
        ];
        for (p, supported, windows, on) in cases {
            assert_eq!(keyboard_flags(p, supported, windows), on.then_some(flags), "{p:?} {supported} {windows}");
        }
    }

    #[test]
    fn restore_runs_in_reverse_setup_order() {
        assert!(restore_steps(0).is_empty());
        assert_eq!(
            restore_steps(RAW | ALT | PASTE | MOUSE | KEYBOARD | TITLE),
            vec![
                Step::PopKeyboard,
                Step::DisableMouse,
                Step::DisablePaste,
                Step::LeaveAlt,
                Step::DisableRaw,
                Step::ShowCursor,
                Step::PopTitle
            ]
        );
        assert_eq!(restore_steps(RAW), vec![Step::DisableRaw, Step::ShowCursor]);
    }

    #[test]
    fn restore_without_setup_does_nothing() {
        // Nothing enabled in tests: must not touch the real terminal.
        restore_terminal();
        assert_eq!(ENABLED.load(Ordering::Acquire), 0);
    }
}
