//! `soyuz-term`: embedded terminal (alacritty_terminal backend + egui widget).
//! No soyuz-* dependency, so it builds and tests in isolation.
//!
//! The backend ([`Terminal`], [`TermPalette`], [`TermColor`]) is toolkit-free: it wakes
//! the host UI through a [`Wake`] callback. The egui widget ([`TerminalView`],
//! [`TermTheme`], [`egui_wake`]) is behind the cargo feature `egui`.
//!
//! Usage:
//! ```no_run
//! # fn demo(wake: soyuz_term::Wake) -> Result<(), soyuz_term::TermError> {
//! use soyuz_term::{Terminal, TermSettings, TermEvent};
//! // egui: `soyuz_term::egui_wake(ctx.clone())`.
//! let term = Terminal::spawn(TermSettings { cwd: "/tmp".into(), ..Default::default() }, wake, 1)?;
//! // every frame:
//! for ev in term.events().try_iter() {
//!     if let TermEvent::CwdChanged(dir) = ev { /* sync the file pane */ let _ = dir; }
//! }
//! # Ok(()) }
//! ```
#![deny(unsafe_code)]

mod backend;
pub mod color;
mod encode;
#[cfg(any(all(test, feature = "egui"), feature = "fuzzing"))]
pub mod fuzzing;
#[cfg(feature = "egui")]
mod keys;
pub mod osc7;
mod pty;
#[cfg(feature = "egui")]
mod render;
#[cfg_attr(not(feature = "egui"), allow(dead_code))]
mod selection;
#[cfg(feature = "egui")]
pub mod theme;
// The only `unsafe` in the crate: forwarding alacritty's `unsafe fn register`.
#[allow(unsafe_code)]
mod tty_tee;
#[cfg_attr(not(feature = "egui"), allow(dead_code))]
mod url;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::TermMode;
#[cfg(feature = "egui")]
use egui::{FontId, Ui};

pub use color::{TermColor, TermPalette};
pub use pty::has_control_chars;
#[cfg(feature = "egui")]
pub use theme::TermTheme;

/// Asks the host UI to redraw; called from the PTY thread (coalesced: at most once per
/// drawn frame for output, plus title / bell / exit / clipboard events).
pub type Wake = std::sync::Arc<dyn Fn() + Send + Sync>;

/// A [`Wake`] that requests an egui repaint.
#[cfg(feature = "egui")]
pub fn egui_wake(ctx: egui::Context) -> Wake {
    std::sync::Arc::new(move || ctx.request_repaint())
}

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
    /// OSC 52 store request from program output. Untrusted: the host decides whether it
    /// reaches the clipboard (Soyuz: `terminal.osc52`, ask by default). OSC 52 loads are
    /// always answered with an empty string.
    Clipboard(String),
}

/// Minimum quiet time before `send_cd` may type into the shell.
const CD_IDLE: Duration = Duration::from_millis(300);

/// A running terminal session. `Send`; owned by the UI thread.
pub struct Terminal {
    events_rx: crossbeam_channel::Receiver<TermEvent>,
    backend: backend::Backend,
    #[cfg(feature = "egui")]
    view: parking_lot::Mutex<render::ViewState>,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("id", &self.backend.id)
            .field("pid", &self.backend.pid)
            .field("alive", &self.is_alive())
            .finish_non_exhaustive()
    }
}

impl Terminal {
    /// Spawn the shell in a PTY (ConPTY on Windows, openpty on unix). `wake` is called
    /// (from the PTY thread) when the terminal needs a redraw. `id` must be unique per
    /// live terminal (it keys the widget's egui `Id`).
    pub fn spawn(settings: TermSettings, wake: Wake, id: u64) -> Result<Self, TermError> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let backend = backend::Backend::spawn(&settings, wake, id, tx)?;
        Ok(Terminal {
            events_rx: rx,
            backend,
            #[cfg(feature = "egui")]
            view: parking_lot::Mutex::new(render::ViewState::default()),
        })
    }

    /// Colors reported to programs that query them (OSC 4 / 10 / 11 / 12). The egui
    /// widget sets them from its [`TermTheme`] every frame; other front ends call this.
    pub fn set_palette(&self, palette: &TermPalette) {
        self.backend.shared.set_colors(palette.fg, palette.bg, palette.cursor, &palette.ansi);
    }

    /// Call once the host has drawn the terminal: re-arms output wakeups, which are
    /// coalesced until the next draw. The egui widget does this itself.
    pub fn mark_drawn(&self) {
        self.backend.shared.repaint_pending.store(false, Ordering::Release);
    }

    /// Write raw bytes to the PTY.
    pub fn write(&self, bytes: &[u8]) {
        self.backend.shared.write(Cow::Owned(bytes.to_vec()));
    }

    /// User input: scrolls the viewport back to the bottom, then writes.
    pub(crate) fn write_input(&self, bytes: Vec<u8>) {
        {
            let mut t = self.backend.term.lock();
            if t.grid().display_offset() != 0 {
                t.scroll_display(Scroll::Bottom);
            }
        }
        self.backend.shared.write(Cow::Owned(bytes));
    }

    /// Bracketed-paste aware.
    pub fn paste(&self, text: &str) {
        let bracketed = self.backend.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.write_input(encode::paste_bytes(text, bracketed));
    }

    /// No-op when unchanged.
    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        self.backend.resize(cols, rows, cell_w, cell_h);
    }

    /// Types `cd "<dir>"` when the shell is idle at a prompt; `false`
    /// when refused. Paths containing control characters are always refused: typed into a
    /// shell, a newline or escape sequence in a (crafted) folder name would run commands.
    pub fn send_cd(&self, dir: &Path) -> bool {
        if has_control_chars(&dir.to_string_lossy()) {
            return false;
        }
        if !self.is_alive() || self.idle_for() < CD_IDLE {
            return false;
        }
        {
            let term = self.backend.term.lock();
            if term.mode().contains(TermMode::ALT_SCREEN) {
                return false;
            }
            let (left, right) = backend::cursor_line_text(&term);
            if !pty::looks_like_prompt(&left, &right) {
                return false;
            }
        }
        self.write_input(pty::cd_command(self.backend.kind, dir).into_bytes());
        true
    }

    /// Scroll the viewport; positive = up into the scrollback.
    pub fn scroll(&self, lines: i32) {
        self.backend.term.lock().scroll_display(Scroll::Delta(lines));
        self.backend.shared.force_repaint();
    }

    pub fn clear_scrollback(&self) {
        let mut t = self.backend.term.lock();
        t.grid_mut().clear_history();
        t.selection = None;
        drop(t);
        self.backend.shared.force_repaint();
    }

    pub fn title(&self) -> String {
        self.backend.shared.title.lock().clone()
    }

    pub fn is_alive(&self) -> bool {
        self.backend.shared.alive.load(Ordering::Acquire)
    }

    /// Last known working directory: what the shell reported (OSC 7 or the title hook,
    /// latest wins) unless `/proc` (Linux) shows the shell moved since, else the spawn cwd.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.backend.shared.poll_proc_cwd();
        self.backend.shared.cwd.lock().clone()
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.backend.pid
    }

    pub fn events(&self) -> &crossbeam_channel::Receiver<TermEvent> {
        &self.events_rx
    }

    pub fn selected_text(&self) -> Option<String> {
        self.backend.term.lock().selection_to_string()
    }

    pub fn clear_selection(&self) {
        self.backend.term.lock().selection = None;
    }

    /// Terminate the child (closes the PTY) and stop the event loop.
    pub fn kill(&self) {
        self.backend.shutdown();
    }

    pub fn mode_alt_screen(&self) -> bool {
        self.backend.term.lock().mode().contains(TermMode::ALT_SCREEN)
    }

    /// Time since the last PTY output.
    pub fn idle_for(&self) -> Duration {
        self.backend.shared.idle_for()
    }

    /// Visible screen text, rows joined with `\n` (trailing blanks trimmed). Also the
    /// value of the terminal's accessibility node.
    pub fn screen_text(&self) -> String {
        use alacritty_terminal::grid::Dimensions;
        use alacritty_terminal::index::{Column, Line};
        let t = self.backend.term.lock();
        let grid = t.grid();
        let mut out = String::new();
        for line in 0..t.screen_lines() {
            let row = &grid[Line(line as i32)];
            let s: String = (0..t.columns()).map(|c| row[Column(c)].c).collect();
            out.push_str(s.trim_end());
            out.push('\n');
        }
        out
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.backend.shutdown();
    }
}

/// The egui widget showing a [`Terminal`].
///
/// ```no_run
/// # fn demo(term: &soyuz_term::Terminal, ui: &mut egui::Ui) {
/// let r = soyuz_term::TerminalView::new(term).focused(true).show(ui);
/// if let Some(url) = r.link_clicked { let _ = url; /* open it */ }
/// # }
/// ```
#[cfg(feature = "egui")]
pub struct TerminalView<'a> {
    term: &'a Terminal,
    focused: bool,
    theme: Option<&'a TermTheme>,
    font: Option<FontId>,
}

#[cfg(feature = "egui")]
#[derive(Debug)]
pub struct TermResponse {
    pub response: egui::Response,
    pub wants_focus: bool,
    pub link_clicked: Option<String>,
    /// Esc pressed on an empty prompt line (app may return focus to the file list).
    pub escape_on_empty_line: bool,
}

#[cfg(feature = "egui")]
impl<'a> TerminalView<'a> {
    pub fn new(term: &'a Terminal) -> Self {
        TerminalView { term, focused: false, theme: None, font: None }
    }

    /// Whether the terminal owns keyboard input (the app tracks focus; see `wants_focus`).
    pub fn focused(self, focused: bool) -> Self {
        TerminalView { focused, ..self }
    }

    pub fn theme(self, theme: &'a TermTheme) -> Self {
        TerminalView { theme: Some(theme), ..self }
    }

    /// Font (size) of the grid; default is the style's `TextStyle::Monospace`.
    pub fn font(self, font: FontId) -> Self {
        TerminalView { font: Some(font), ..self }
    }

    /// Fills the available rect, resizes the PTY to fit, handles input and paints.
    pub fn show(self, ui: &mut Ui) -> TermResponse {
        render::show(ui, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// No UI: the backend runs without any toolkit.
    fn noop_wake() -> Wake {
        std::sync::Arc::new(|| {})
    }

    #[test]
    fn missing_shell_is_reported() {
        let s = TermSettings { shell: Some("definitely-not-a-shell-xyz".into()), ..Default::default() };
        let r = Terminal::spawn(s, noop_wake(), 1);
        assert!(matches!(r, Err(TermError::ShellNotFound(_))));
    }

    #[test]
    fn themes_and_traits() {
        #[cfg(feature = "egui")]
        {
            assert_ne!(TermTheme::dark(), TermTheme::light());
            assert_eq!(TermTheme::default(), TermTheme::dark());
        }
        fn assert_send<T: Send>() {}
        assert_send::<Terminal>();
        assert_eq!(TermSettings::default().scrollback, 10_000);
    }

    fn wait_for(term: &Terminal, what: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if term.screen_text().contains(what) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn pty_smoke_echo_hi() {
        let (shell, args) = if cfg!(windows) {
            ("cmd.exe", vec!["/c".to_owned(), "echo hi".to_owned()])
        } else {
            ("sh", vec!["-c".to_owned(), "echo hi".to_owned()])
        };
        let settings = TermSettings {
            shell: Some(shell.into()),
            args,
            shell_integration: false,
            cwd: std::env::temp_dir(),
            ..Default::default()
        };
        // Toolkit-free host: count wakeups instead of repainting a UI.
        let wakes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = wakes.clone();
        let wake: Wake = std::sync::Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let term = Terminal::spawn(settings, wake, 7).expect("spawn");
        term.set_palette(&TermPalette::light());
        assert!(term.child_pid().is_some());
        let found = wait_for(&term, "hi", Duration::from_secs(15));
        assert!(found, "grid did not contain `hi`:\n{}", term.screen_text());
        // The child exits on its own.
        let start = Instant::now();
        let mut exited = false;
        while start.elapsed() < Duration::from_secs(15) && !exited {
            exited = term.events().try_iter().any(|e| matches!(e, TermEvent::Exited(_)));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(exited, "no Exited event");
        assert!(!term.is_alive());
        assert!(wakes.load(Ordering::Relaxed) > 0, "the wake callback was never called");
        term.mark_drawn();
    }

    /// The child starts in the requested folder.
    #[cfg(unix)]
    #[test]
    fn pty_child_runs_in_cwd() {
        let dir = tempfile_dir("pwd");
        let settings = TermSettings {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), "pwd; sleep 0.2".into()],
            shell_integration: false,
            cwd: dir.clone(),
            ..Default::default()
        };
        let term = Terminal::spawn(settings, noop_wake(), 9).expect("spawn");
        let want = std::fs::canonicalize(&dir).unwrap();
        let found = wait_for(&term, &want.to_string_lossy(), Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found, "grid did not contain {}:\n{}", want.display(), term.screen_text());
    }

    /// A fresh folder under the temp dir (no `tempfile` dependency here).
    #[cfg(unix)]
    fn tempfile_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("soyuz-term-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Waits for a `CwdChanged(want)` event, then checks `cwd()` agrees.
    #[cfg(unix)]
    fn wait_for_cwd(term: &Terminal, want: &Path) {
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < Duration::from_secs(10) && !seen.iter().any(|p: &PathBuf| p == want) {
            std::thread::sleep(Duration::from_millis(20));
            seen.extend(term.events().try_iter().filter_map(|e| match e {
                TermEvent::CwdChanged(p) => Some(p),
                _ => None,
            }));
        }
        assert!(seen.iter().any(|p| p == want), "no CwdChanged({}); saw {seen:?}", want.display());
        assert_eq!(term.cwd().as_deref(), Some(want));
    }

    /// OSC 7 from program output (dropped by alacritty's parser) is seen by the PTY tee,
    /// including a percent-encoded path with spaces, and overrides `/proc` polling.
    #[cfg(unix)]
    #[test]
    fn pty_osc7_sets_cwd() {
        let base = tempfile_dir("osc7");
        let dir = base.join("a b%c");
        std::fs::create_dir_all(&dir).unwrap();
        let encoded = dir.to_string_lossy().replace('%', "%25").replace(' ', "%20");
        // Split across two printf calls so the sequence may straddle reads; ST terminator.
        let script = format!(
            r#"printf '\033]7;file://localhost%s' '{encoded}'; sleep 0.05; printf '\033\\'; echo done; sleep 0.5"#
        );
        let settings = TermSettings {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), script],
            shell_integration: false,
            cwd: base.clone(),
            ..Default::default()
        };
        let term = Terminal::spawn(settings, noop_wake(), 11).expect("spawn");
        wait_for_cwd(&term, &dir);
        // OSC 7 is invisible on screen.
        assert!(wait_for(&term, "done", Duration::from_secs(10)), "{}", term.screen_text());
        assert!(!term.screen_text().contains("file://"), "{}", term.screen_text());
        term.kill();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// One stray OSC 7 (a nested shell, a `cat`ted file) must not stop `/proc` tracking:
    /// once the shell moves without reporting, `/proc` wins again.
    #[cfg(target_os = "linux")]
    #[test]
    fn pty_stray_osc7_does_not_stop_proc_tracking() {
        let base = tempfile_dir("stray");
        let dir = base.join("later");
        std::fs::create_dir_all(&dir).unwrap();
        let script = format!(r#"printf '\033]7;file:///usr\007'; sleep 0.5; cd '{}'; sleep 3"#, dir.display());
        let settings = TermSettings {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), script],
            shell_integration: false,
            cwd: base.clone(),
            ..Default::default()
        };
        let term = Terminal::spawn(settings, noop_wake(), 13).expect("spawn");
        let start = Instant::now();
        let mut seen_usr = false;
        while start.elapsed() < Duration::from_secs(10) {
            let c = term.cwd();
            seen_usr |= c.as_deref() == Some(Path::new("/usr"));
            if seen_usr && c.as_deref() == Some(dir.as_path()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen_usr, "the OSC 7 report was not taken");
        assert_eq!(term.cwd().as_deref(), Some(dir.as_path()), "/proc did not win after the shell moved");
        term.kill();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// OSC 7 and the title hook interleave: the last report wins.
    #[cfg(unix)]
    #[test]
    fn pty_osc7_and_title_hook_latest_wins() {
        let script = r#"printf '\033]7;file:///tmp\007'; sleep 0.3; printf '\033]0;soyuz-cwd:/usr\007'; sleep 0.3; printf '\033]7;file:///var\007'; sleep 0.5"#;
        let settings = TermSettings {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), script.into()],
            shell_integration: false,
            cwd: PathBuf::from("/"),
            ..Default::default()
        };
        let term = Terminal::spawn(settings, noop_wake(), 12).expect("spawn");
        let want: Vec<PathBuf> = ["/tmp", "/usr", "/var"].iter().map(PathBuf::from).collect();
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < Duration::from_secs(10) && seen.last() != want.last() {
            std::thread::sleep(Duration::from_millis(20));
            seen.extend(term.events().try_iter().filter_map(|e| match e {
                TermEvent::CwdChanged(p) => Some(p),
                _ => None,
            }));
        }
        assert_eq!(seen, want);
        assert_eq!(term.cwd().as_deref(), Some(Path::new("/var")));
        term.kill();
    }

    /// The terminal answers a cursor-position query (`ESC[6n`) through `PtyWrite`.
    #[cfg(unix)]
    #[test]
    fn pty_write_reply_reaches_shell() {
        let script = r#"stty raw -echo; printf '\033[6n'; r=$(dd bs=1 count=6 2>/dev/null); stty sane; case "$r" in *R*) echo GOT-REPLY;; esac; sleep 0.2"#;
        let settings = TermSettings {
            shell: Some("sh".into()),
            args: vec!["-c".into(), script.into()],
            shell_integration: false,
            ..Default::default()
        };
        let term = Terminal::spawn(settings, noop_wake(), 8).expect("spawn");
        assert!(wait_for(&term, "GOT-REPLY", Duration::from_secs(10)), "{}", term.screen_text());
    }

    #[test]
    fn interactive_default_shell_send_cd() {
        send_cd_roundtrip(None, 9, "default");
    }

    #[test]
    fn interactive_alt_shell_send_cd() {
        let alt = if cfg!(windows) { "cmd.exe" } else { "bash" };
        if pty::which(alt).is_some() {
            send_cd_roundtrip(Some(alt), 10, alt);
        }
    }

    fn send_cd_roundtrip(shell: Option<&str>, id: u64, dir_tag: &str) {
        let settings = TermSettings { cwd: std::env::temp_dir(), shell: shell.map(Into::into), ..Default::default() };
        let term = Terminal::spawn(settings, noop_wake(), id).expect("spawn shell");
        assert!(term.is_alive());
        // Wait for a prompt, then for the shell to go quiet.
        let start = Instant::now();
        let mut at_prompt = false;
        while start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(100));
            let t = term.backend.term.lock();
            let (l, r) = backend::cursor_line_text(&t);
            drop(t);
            if pty::looks_like_prompt(&l, &r) && term.idle_for() >= CD_IDLE {
                at_prompt = true;
                break;
            }
        }
        if !at_prompt {
            eprintln!("no recognizable prompt; skipping send_cd check:\n{}", term.screen_text());
            return;
        }
        let target = std::env::temp_dir().join(format!("soyuz term test {dir_tag}"));
        std::fs::create_dir_all(&target).unwrap();
        assert!(term.send_cd(&target));
        // With shell integration the new cwd is reported back (cmd/pwsh/bash/zsh/fish).
        let start = Instant::now();
        let mut reported = None;
        while start.elapsed() < Duration::from_secs(10) && reported.is_none() {
            std::thread::sleep(Duration::from_millis(50));
            reported = term.events().try_iter().find_map(|e| match e {
                TermEvent::CwdChanged(p) if p.ends_with(target.file_name().unwrap()) => Some(p),
                _ => None,
            });
        }
        eprintln!(
            "kind {:?}, reported {reported:?}, title {:?}
{}",
            term.backend.kind,
            term.title(),
            term.screen_text()
        );
        if term.backend.kind != pty::ShellKind::Posix {
            assert!(
                reported.is_some(),
                "cwd not reported:
{}",
                term.screen_text()
            );
            assert_eq!(term.cwd().as_deref().map(|p| p.ends_with(target.file_name().unwrap())), Some(true));
        }
        term.kill();
        assert!(!term.is_alive());
        let _ = std::fs::remove_dir(&target);
    }
}
