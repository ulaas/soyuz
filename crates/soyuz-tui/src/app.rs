//! The terminal UI's frame loop.
//!
//! [`run`] is the production entry (`soyuz --tui`): it queries the keyboard protocol,
//! installs the legacy keymap overlay, enters TUI mode ([`TerminalGuard`]), registers the
//! panic hooks, starts the input reader and the signal watcher, and drives [`App`] with
//! [`run_loop`] until the user quits. Tests drive the same loop with ratatui's
//! `TestBackend`, a scripted [`EventSource`] and a [`TermIo`] that touches no terminal.
//!
//! One loop iteration:
//! 1. the close request of the previous frame ([`TuiHost::take_close`]) goes through
//!    [`AppCore::handle_close_request`]; the app quits when no `CancelClose` arrived
//!    during that call;
//! 2. one frame ([`App::frame`]) with the input that arrived since the last one, running
//!    the core phases in the GUI's order (`SoyuzApp::frame`): begin_frame,
//!    pump_services, pump_jobs, pump_background, apply_cmds, input, apply_cmds, draw
//!    (prepare_visible_panes first), `ops::fileops::end_frame`, apply_cmds,
//!    update_title, end_frame; then the host's escape sequences (title, OSC 52) are
//!    written;
//! 3. wait for input, a wake-up of the core, the earliest `redraw_after` deadline or the
//!    250 ms idle tick (toasts expire, spinners move).
//!
//! In raw mode Ctrl+C and Ctrl+Z are ordinary keys: Copy and Undo with the default key
//! map. The process is ended from outside with `SIGTERM` / `SIGHUP` (see
//! [`crate::signals`]), which saves first.

use std::io::{self, Write as _};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use soyuz_appcore::core::{AppCore, CMD_ROUNDS, Focus, FrontendKind, Startup};
use soyuz_appcore::host::Host as _;
use soyuz_appcore::models::frontend::FrontendCmd;
use soyuz_appcore::models::properties::MetaDialog;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::services::crash;
use soyuz_appcore::{ops, tr};
use soyuz_core::config::{Config, KeyContext, TuiSection};

use crate::host::TuiHost;
use crate::term::TerminalGuard;
use crate::view::{self, ViewModel};

/// Something for the frame loop.
#[derive(Debug)]
pub enum TuiEvent {
    /// Terminal input (keys, paste, mouse, resize).
    Term(Event),
    /// Save and quit now: `SIGTERM` / `SIGHUP` / `SIGINT`, the console window closing,
    /// or the input stream failing.
    Terminate,
    /// `SIGCONT`: re-enter TUI mode and repaint.
    Resume,
}

/// Where the loop gets its events. Production: [`ChannelSource`] (input reader thread +
/// signals); tests: a script that looks at the app before each event.
pub trait EventSource {
    /// Wait up to `timeout` for events, also returning early when the core asks for a
    /// frame (`redraw`). Returns what arrived (possibly nothing).
    fn next(&mut self, app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent>;
}

/// Events from a channel: the input reader thread ([`spawn_reader`]) and the signal
/// watcher ([`crate::signals::install`]) send to it.
pub struct ChannelSource {
    pub rx: Receiver<TuiEvent>,
}

impl EventSource for ChannelSource {
    fn next(&mut self, _app: &App, redraw: &Receiver<()>, timeout: Duration) -> Vec<TuiEvent> {
        let mut out = Vec::new();
        crossbeam_channel::select! {
            recv(self.rx) -> ev => match ev {
                Ok(ev) => out.push(ev),
                // Every sender is gone: nothing can arrive any more.
                Err(_) => out.push(TuiEvent::Terminate),
            },
            recv(redraw) -> _ => {}
            default(timeout) => {}
        }
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
        }
        out
    }
}

/// Read terminal events on a thread (`crossterm::event::read`) into `tx`. Start it only
/// after `crossterm::terminal::supports_keyboard_enhancement` was queried: the query
/// reads the terminal's answer from the same input stream.
pub fn spawn_reader(tx: Sender<TuiEvent>) -> io::Result<()> {
    std::thread::Builder::new().name("soyuz-tui-input".into()).spawn(move || {
        loop {
            match crossterm::event::read() {
                Ok(ev) => {
                    if tx.send(TuiEvent::Term(ev)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    log::error!("tui: reading terminal input failed: {e}");
                    let _ = tx.send(TuiEvent::Terminate);
                    break;
                }
            }
        }
    })?;
    Ok(())
}

/// The terminal besides ratatui's drawing: raw escape sequences and mode changes.
pub trait TermIo {
    /// Write escape sequences (window title, OSC 52) after a frame.
    fn write_raw(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// `[tui]` changed: re-apply mouse capture / keyboard protocol.
    fn reapply(&mut self, cfg: &TuiSection) -> io::Result<()>;
    /// Continued after a stop: re-enter raw mode, the alternate screen and the modes.
    fn resume(&mut self, cfg: &TuiSection) -> io::Result<()>;
}

/// The real terminal (stdout), owning the [`TerminalGuard`].
pub struct StdoutIo {
    pub guard: TerminalGuard,
}

impl TermIo for StdoutIo {
    fn write_raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut out = io::stdout();
        out.write_all(bytes)?;
        out.flush()
    }

    fn reapply(&mut self, cfg: &TuiSection) -> io::Result<()> {
        self.guard.reapply(cfg)
    }

    fn resume(&mut self, cfg: &TuiSection) -> io::Result<()> {
        self.guard.resume(cfg)
    }
}

/// No terminal (tests): records what would be written.
#[derive(Default)]
pub struct NullIo {
    pub written: Vec<u8>,
    pub reapplied: usize,
    pub resumed: usize,
}

impl TermIo for NullIo {
    fn write_raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.written.extend_from_slice(bytes);
        Ok(())
    }

    fn reapply(&mut self, _cfg: &TuiSection) -> io::Result<()> {
        self.reapplied += 1;
        Ok(())
    }

    fn resume(&mut self, _cfg: &TuiSection) -> io::Result<()> {
        self.resumed += 1;
        Ok(())
    }
}

/// The idle tick: the loop draws at least this often.
pub const IDLE_TICK: Duration = Duration::from_millis(250);
/// Frames driven only by core wake-ups are at least this far apart (no busy loop while
/// a job reports progress).
pub const MIN_FRAME: Duration = Duration::from_millis(16);
/// `save_all` interval while running (the GUI's `auto_save_interval`).
pub const SAVE_EVERY: Duration = Duration::from_secs(30);

/// How the loop ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopEnd {
    /// The user quit (the close request went through).
    Quit,
    /// Terminated from outside (signal, console closed, input gone): already saved.
    Terminated,
}

/// The terminal UI: the core plus what only the terminal frontend keeps.
pub struct App {
    pub core: AppCore,
    pub host: TuiHost,
    /// First list row shown (scroll position of the focused pane).
    top: usize,
    /// The model of the last frame drawn.
    view: Option<ViewModel>,
    deadline: Option<Instant>,
    last_frame: Instant,
    last_save: Instant,
}

impl App {
    /// Build the core for the terminal UI (`startup.frontend` is forced to the TUI: own
    /// session file, no FileManager1, no other windows).
    pub fn new(config: Config, mut startup: Startup) -> App {
        startup.frontend = FrontendKind::Tui;
        let host = TuiHost::new();
        let mut core = AppCore::new(config, startup, host.wake(), soyuz_appcore::services::preview_gfx::no_upload());
        core.renderer = "tui";
        App { core, host, top: 0, view: None, deadline: None, last_frame: Instant::now(), last_save: Instant::now() }
    }

    /// The model of the last frame drawn.
    pub fn view(&self) -> Option<&ViewModel> {
        self.view.as_ref()
    }

    /// Key context of the frame's input (the core's prompts own the keyboard when open).
    pub fn key_context(&self) -> KeyContext {
        if self.core.dialogs.is_open() { KeyContext::Dialog } else { KeyContext::FileList }
    }

    /// Apply queued commands (bounded rounds, like the core's); the frontend requests are
    /// handled after every command, as in `SoyuzApp::apply_cmds`.
    pub fn apply_cmds(&mut self) {
        for _ in 0..CMD_ROUNDS {
            if self.core.cmds.is_empty() {
                break;
            }
            for c in std::mem::take(&mut self.core.cmds) {
                ops::apply(&mut self.core, c, &self.host);
                self.drain_frontend();
            }
        }
        self.drain_frontend();
    }

    fn unavailable(&mut self, feature: &str) {
        self.core.ui.toasts.push(Toast::info(tr!("tui-not-available")).body(feature));
    }

    /// Carry out (or decline) the core's requests for the frontend.
    pub fn drain_frontend(&mut self) {
        for c in std::mem::take(&mut self.core.frontend) {
            let feature = match c {
                FrontendCmd::ReleaseFocus | FrontendCmd::CloseSettings => continue,
                FrontendCmd::CrashReports(reports) => {
                    if let Some(newest) = reports.first() {
                        let path = soyuz_fs::strip_verbatim(&newest.path).display().to_string();
                        let path = crate::dialog::safe(&path);
                        self.core.ui.toasts.push(Toast::warning(tr!("tui-crashed", path = path)).sticky());
                    }
                    // Offered once, like the GUI's "Keep" (the reports stay on disk).
                    crash::request(crash::RecoveryAction::KeepReports(reports));
                    continue;
                }
                FrontendCmd::RecoveredJobs(recovered) => {
                    // Left in the journal for the GUI to resume or roll back.
                    let count: usize = recovered.iter().map(|r| r.jobs.len()).sum();
                    self.core.ui.toasts.push(Toast::warning(tr!("tui-recovered-jobs", count = count)).sticky());
                    continue;
                }
                FrontendCmd::EditorUnsaved(p) => {
                    // There is no editor view to change a document in: quitting saves
                    // what is open (then quits), closing one document keeps it open.
                    use soyuz_appcore::editors::io::Answer;
                    p.answer(if p.target.is_none() { Answer::Save } else { Answer::Cancel });
                    tr!("tui-feature-editor")
                }
                FrontendCmd::OpenSettings(_) => tr!("tui-feature-settings"),
                FrontendCmd::SaveWorkspace(_) | FrontendCmd::ManageWorkspaces => tr!("tui-feature-workspaces"),
                FrontendCmd::TabHistory { .. } => tr!("tui-feature-history"),
                FrontendCmd::CloseWindowPrompt { .. } => tr!("tui-feature-windows"),
                FrontendCmd::FolderProperties(_) | FrontendCmd::Properties { .. } => tr!("tui-feature-properties"),
                FrontendCmd::UnlockDevice { .. } | FrontendCmd::OfferReadOnlyMount { .. } => {
                    tr!("tui-feature-devices")
                }
                FrontendCmd::Smart(_) => tr!("tui-feature-smart"),
                FrontendCmd::Remote(_) => tr!("tui-feature-remote"),
                FrontendCmd::Archive(_) => tr!("tui-feature-archive"),
                FrontendCmd::About => tr!("tui-feature-about"),
                FrontendCmd::Confirm { .. } => tr!("tui-feature-confirm"),
                FrontendCmd::SelectByPattern(_) => tr!("tui-feature-select-pattern"),
                FrontendCmd::ChooseColumns(_) => tr!("tui-feature-columns"),
                FrontendCmd::OpenWith(_) => tr!("tui-feature-open-with"),
                FrontendCmd::MetaDialog(MetaDialog::BatchRename) => tr!("tui-feature-batch-rename"),
                FrontendCmd::MetaDialog(MetaDialog::Attributes) => tr!("tui-feature-attributes"),
                FrontendCmd::MetaDialog(MetaDialog::Link(_)) => tr!("tui-feature-links"),
            };
            self.unavailable(feature);
        }
    }

    /// Core states the terminal UI has no view for yet: reset them (so the keyboard
    /// keeps working) and say so.
    fn reset_unsupported(&mut self) {
        if self.core.palette.open {
            soyuz_appcore::palette::close(&mut self.core);
            self.unavailable(tr!("tui-feature-palette"));
        }
        if self.core.ui.address.editing {
            self.core.ui.address.editing = false;
            self.core.ui.address.request_focus = false;
            self.unavailable(tr!("tui-feature-address"));
        }
        if self.core.ui.settings_open {
            self.core.ui.settings_open = false;
            self.unavailable(tr!("tui-feature-settings"));
        }
        if self.core.ui.quicklook.open {
            soyuz_appcore::models::quicklook::close(&mut self.core);
            self.unavailable(tr!("tui-feature-quicklook"));
        }
        if soyuz_appcore::models::terminal::owns_keyboard(&self.core) {
            self.core.ui.terminal.focused = false;
            self.unavailable(tr!("tui-feature-terminal"));
        }
        if self.core.ui.focus == Focus::Sidebar {
            self.core.ui.focus = Focus::List;
            self.unavailable(tr!("tui-feature-sidebar"));
        }
    }

    /// Keys while a core prompt is open go to the dialog fallback only (like the GUI,
    /// where the modal owns the keyboard).
    fn dialog_input(&mut self, events: &[Event]) {
        for ev in events {
            let Event::Key(k) = ev else { continue };
            let Some(top) = self.core.dialogs.top_mut() else { break };
            if let crate::dialog::Outcome::Close(cmd) = crate::dialog::on_key(top, k) {
                if let Some(c) = cmd {
                    self.core.cmds.push(c);
                }
                self.core.dialogs.close_top();
            }
        }
    }

    fn input(&mut self, events: &[Event]) {
        // Prompts that went away by themselves close first.
        while let Some(top) = self.core.dialogs.top_mut() {
            if !crate::dialog::stale(top) {
                break;
            }
            self.core.dialogs.close_top();
        }
        let kctx = self.key_context();
        if kctx == KeyContext::Dialog {
            self.dialog_input(events);
            return;
        }
        let mut frame = crate::keys::frame(events, self.core.config.keys.cmd_is_ctrl, cfg!(windows));
        self.core.input(&self.host, &mut frame, kctx, false);
    }

    /// Count down the toasts' timers; drop expired and dismissed ones.
    fn tick_toasts(&mut self, dt: f32) {
        self.core.ui.toasts.live.retain_mut(|l| {
            if l.closing.is_some() {
                return false;
            }
            match l.remaining.as_mut() {
                Some(r) => {
                    *r -= dt;
                    *r > 0.0
                }
                None => true,
            }
        });
    }

    /// One frame with this frame's terminal `events`.
    pub fn frame<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        events: &[Event],
        io: &mut dyn TermIo,
    ) -> io::Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        if crash::take_needs_full_redraw() {
            terminal.clear().map_err(io::Error::other)?;
        }
        if self.host.take_modes_changed()
            && let Err(e) = io.reapply(&self.core.config.tui)
        {
            log::warn!("tui: cannot re-apply terminal modes: {e}");
        }
        self.core.begin_frame();
        self.core.pump_services(&self.host);
        self.core.pump_jobs(&self.host);
        self.core.pump_background(&self.host);
        self.apply_cmds();
        self.input(events);
        self.apply_cmds();
        self.reset_unsupported();
        self.tick_toasts(dt);

        let size = terminal.size().map_err(io::Error::other)?;
        let toasts = self.core.ui.toasts.live.iter().filter(|l| l.closing.is_none()).count();
        let rows = view::list_rows(size.height, toasts);
        {
            let pane = self.core.ws.pane_mut();
            pane.page_rows = rows;
            pane.viewport_h = rows as f32;
            // Cursor keys move linearly through the list (no GUI grid layout here).
            pane.layout = None;
        }
        self.core.prepare_visible_panes();
        let model = ViewModel::from_core(&self.core, rows, self.top);
        self.top = model.top;
        terminal.draw(|f| view::draw(f, &model)).map_err(io::Error::other)?;
        self.view = Some(model);

        ops::fileops::end_frame(&mut self.core, &self.host);
        self.apply_cmds();
        self.core.update_title(&self.host);
        self.core.end_frame();
        let out = self.host.take_output();
        if !out.is_empty() {
            io.write_raw(&out)?;
        }
        if self.last_save.elapsed() >= SAVE_EVERY {
            self.last_save = Instant::now();
            self.core.save_all();
        }
        Ok(())
    }

    /// Step 1 of an iteration: the previous frame's close request. Returns whether the
    /// app quits now.
    pub fn close_step(&mut self) -> bool {
        let close = self.host.take_close();
        let _ = self.host.take_close_cancelled();
        self.core.handle_close_request(&self.host, close);
        close && !self.host.take_close_cancelled()
    }

    /// How long the loop may wait for the next event.
    fn timeout(&mut self, now: Instant) -> Duration {
        if let Some(d) = self.host.take_deadline() {
            self.deadline = Some(self.deadline.map_or(d, |cur| cur.min(d)));
        }
        match self.deadline {
            Some(d) if d <= now => {
                self.deadline = None;
                Duration::ZERO
            }
            Some(d) => (d - now).min(IDLE_TICK),
            None => IDLE_TICK,
        }
    }

    /// Everything is written: session, view memory, settings, palette memory, tags.
    pub fn finish(&mut self) {
        self.core.save_all();
        self.core.on_exit();
    }
}

/// Run the loop until the user quits or the process is told to end. The caller then
/// calls [`App::finish`] (after [`LoopEnd::Quit`]; a terminate request saved already).
pub fn run_loop<B: Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    source: &mut dyn EventSource,
    io: &mut dyn TermIo,
) -> io::Result<LoopEnd>
where
    B::Error: Send + Sync + 'static,
{
    let mut events: Vec<Event> = Vec::new();
    loop {
        if app.close_step() {
            return Ok(LoopEnd::Quit);
        }
        app.frame(terminal, &events, io)?;
        events.clear();
        let started = Instant::now();
        let timeout = app.timeout(started);
        let mut got = source.next(app, app.host.redraw_rx(), timeout);
        // Only a wake-up of the core: keep frames apart.
        if got.is_empty() {
            let since = app.last_frame.elapsed();
            if since < MIN_FRAME {
                got = source.next(app, &crossbeam_channel::never(), MIN_FRAME - since);
            }
        }
        for ev in got {
            match ev {
                TuiEvent::Term(e) => events.push(e),
                TuiEvent::Terminate => {
                    // No `on_exit`: an "Install & restart" does not restart into a
                    // terminal that is going away.
                    app.core.save_all();
                    crate::signals::notify_saved();
                    return Ok(LoopEnd::Terminated);
                }
                TuiEvent::Resume => {
                    if let Err(e) = io.resume(&app.core.config.tui) {
                        log::warn!("tui: cannot resume the terminal: {e}");
                    }
                    terminal.clear().map_err(io::Error::other)?;
                }
            }
        }
    }
}

/// The panic hook's terminal restore (raw mode, alternate screen, console modes).
fn restore_for_panic() {
    crate::term::restore_terminal();
    soyuz_fs::console::restore();
}

/// `soyuz --tui`: run the terminal UI on this thread until the user quits.
///
/// The caller has prepared the console (`soyuz_fs::console::tui_console` on Windows),
/// checked `soyuz_fs::console::is_terminal_session`, set up logging to a file and
/// claimed the crash journal (`services::crash::startup`). Installs the legacy keymap
/// overlay (`crate::overlay`) when the kitty keyboard protocol is not used (never on
/// Windows), before the core builds its key map. After the loop the terminal is restored
/// and an "Install & restart" update restarts the process (`services::updates`).
pub fn run(config: Config, startup: Startup) -> ExitCode {
    soyuz_fs::register_ui_thread();
    // Before any thread reads terminal input: the query reads the terminal's answer.
    let keyboard_supported = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    let kitty = crate::term::keyboard_flags(config.tui.keyboard_protocol, keyboard_supported, cfg!(windows)).is_some();
    if !kitty && !cfg!(windows) {
        soyuz_appcore::keys::set_keymap_overlay(crate::overlay::apply);
    }
    let guard = match TerminalGuard::enter(&config.tui, keyboard_supported) {
        Ok(g) => g,
        Err(e) => {
            crate::term::restore_terminal();
            let _ = writeln!(io::stderr(), "soyuz: cannot set up the terminal: {e}");
            return ExitCode::FAILURE;
        }
    };
    crash::set_tui_hooks(restore_for_panic, std::thread::current().id());

    let (tx, rx) = crossbeam_channel::unbounded();
    crate::signals::install(tx.clone());
    if let Err(e) = spawn_reader(tx) {
        drop(guard);
        crash::clear_tui_hooks();
        let _ = writeln!(io::stderr(), "soyuz: cannot read terminal input: {e}");
        return ExitCode::FAILURE;
    }
    let mut terminal = match Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout())) {
        Ok(t) => t,
        Err(e) => {
            drop(guard);
            crash::clear_tui_hooks();
            let _ = writeln!(io::stderr(), "soyuz: cannot set up the terminal: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App::new(config, startup);
    let mut io_ = StdoutIo { guard };
    let mut source = ChannelSource { rx };
    let result = run_loop(&mut app, &mut terminal, &mut source, &mut io_);
    if let Ok(LoopEnd::Quit) | Err(_) = result {
        app.finish();
    }
    // Restore the terminal before anything else prints or a restart takes over.
    drop(terminal);
    drop(io_);
    crash::clear_tui_hooks();
    drop(app);
    if let Some(exe) = soyuz_appcore::services::updates::take_restart() {
        let e = soyuz_appcore::services::updates::exec_restart(&exe);
        log::error!("soyuz: restart failed: {e}");
        let _ = writeln!(io::stderr(), "soyuz: restart failed: {e}");
        return ExitCode::FAILURE;
    }
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("tui: {e}");
            let _ = writeln!(io::stderr(), "soyuz: {e}");
            ExitCode::FAILURE
        }
    }
}
