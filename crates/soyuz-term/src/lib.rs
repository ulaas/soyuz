//! `soyuz-term`: embedded terminal (alacritty_terminal backend + egui widget).
//! No soyuz-* dependency, so it builds and tests in isolation (PLAN §2.7).
//!
//! **M0 status:** public types are final; [`Terminal::spawn`] returns an error and the
//! [`TerminalView`] paints a placeholder until M8.
#![deny(unsafe_code)]

mod backend;
mod keys;
#[allow(unsafe_code)]
mod pty;
mod render;
mod selection;
pub mod theme;
mod url;

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{FontId, Ui};

pub use theme::TermTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug)]
pub struct TermSettings {
    /// `None` = auto (Windows: pwsh → powershell → cmd; unix: `$SHELL` → `/bin/sh`).
    pub shell: Option<String>,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Lines of scrollback (default 10 000).
    pub scrollback: usize,
    pub cursor: CursorStyle,
    /// Inject a prompt hook that reports the cwd via the window title.
    pub shell_integration: bool,
}

impl Default for TermSettings {
    fn default() -> Self {
        TermSettings {
            shell: None,
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env: Vec::new(),
            scrollback: 10_000,
            cursor: CursorStyle::Block,
            shell_integration: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TermError {
    #[error("shell not found: {0}")]
    ShellNotFound(String),
    #[error("pty: {0}")]
    Pty(std::io::Error),
    #[error("event loop: {0}")]
    EventLoop(std::io::Error),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TermEvent {
    Title(String),
    Bell,
    Exited(Option<i32>),
    CwdChanged(PathBuf),
    /// OSC 52 store.
    Clipboard(String),
}

/// A running terminal session. `Send`; owned by the UI thread.
pub struct Terminal {
    events_rx: crossbeam_channel::Receiver<TermEvent>,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal").finish_non_exhaustive()
    }
}

impl Terminal {
    /// Spawn the shell in a PTY (ConPTY on Windows). M0: always `Err(Pty(Unsupported))`.
    pub fn spawn(settings: TermSettings, ctx: egui::Context, id: u64) -> Result<Self, TermError> {
        let _ = (settings, ctx, id);
        Err(TermError::Pty(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "embedded terminal is not implemented yet (M8)",
        )))
    }

    pub fn write(&self, bytes: &[u8]) {
        let _ = bytes;
        todo!("M8: Terminal::write")
    }

    /// Bracketed-paste aware.
    pub fn paste(&self, text: &str) {
        let _ = text;
        todo!("M8: Terminal::paste")
    }

    /// No-op when unchanged.
    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        let _ = (cols, rows, cell_w, cell_h);
        todo!("M8: Terminal::resize")
    }

    /// Types `cd "<dir>"` when the shell is idle at a prompt (PLAN §2.7 rule); `false`
    /// when refused.
    pub fn send_cd(&self, dir: &Path) -> bool {
        let _ = dir;
        todo!("M8: Terminal::send_cd")
    }

    pub fn scroll(&self, lines: i32) {
        let _ = lines;
        todo!("M8: Terminal::scroll")
    }

    pub fn clear_scrollback(&self) {
        todo!("M8: Terminal::clear_scrollback")
    }

    pub fn title(&self) -> String {
        todo!("M8: Terminal::title")
    }

    pub fn is_alive(&self) -> bool {
        todo!("M8: Terminal::is_alive")
    }

    pub fn cwd(&self) -> Option<PathBuf> {
        todo!("M8: Terminal::cwd")
    }

    pub fn child_pid(&self) -> Option<u32> {
        todo!("M8: Terminal::child_pid")
    }

    pub fn events(&self) -> &crossbeam_channel::Receiver<TermEvent> {
        &self.events_rx
    }

    pub fn selected_text(&self) -> Option<String> {
        todo!("M8: Terminal::selected_text")
    }

    pub fn clear_selection(&self) {
        todo!("M8: Terminal::clear_selection")
    }

    pub fn kill(&self) {
        todo!("M8: Terminal::kill")
    }

    pub fn mode_alt_screen(&self) -> bool {
        todo!("M8: Terminal::mode_alt_screen")
    }

    /// Time since the last PTY output.
    pub fn idle_for(&self) -> Duration {
        todo!("M8: Terminal::idle_for")
    }
}

/// The egui widget showing a [`Terminal`].
pub struct TerminalView<'a> {
    term: &'a Terminal,
    focused: bool,
    theme: Option<&'a TermTheme>,
    font: Option<FontId>,
}

#[derive(Debug)]
pub struct TermResponse {
    pub response: egui::Response,
    pub wants_focus: bool,
    pub link_clicked: Option<String>,
    /// Esc pressed on an empty prompt line (app may return focus to the file list).
    pub escape_on_empty_line: bool,
}

impl<'a> TerminalView<'a> {
    pub fn new(term: &'a Terminal) -> Self {
        TerminalView { term, focused: false, theme: None, font: None }
    }

    pub fn focused(self, focused: bool) -> Self {
        TerminalView { focused, ..self }
    }

    pub fn theme(self, theme: &'a TermTheme) -> Self {
        TerminalView { theme: Some(theme), ..self }
    }

    pub fn font(self, font: FontId) -> Self {
        TerminalView { font: Some(font), ..self }
    }

    pub fn show(self, ui: &mut Ui) -> TermResponse {
        render::placeholder(ui, self.term, self.focused, self.theme, self.font)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_reports_unavailable() {
        let r = Terminal::spawn(TermSettings::default(), egui::Context::default(), 1);
        assert!(matches!(r, Err(TermError::Pty(_))));
    }

    #[test]
    fn themes_and_traits() {
        assert_ne!(TermTheme::dark(), TermTheme::light());
        assert_eq!(TermTheme::default(), TermTheme::dark());
        fn assert_send<T: Send>() {}
        assert_send::<Terminal>();
        assert_eq!(TermSettings::default().scrollback, 10_000);
    }
}
