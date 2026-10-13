//! The terminal UI's frame loop.
//!
//! [`run`] is the production entry (`soyuz --tui`): it queries the keyboard protocol,
//! installs the key map overlay (`crate::overlay`: the TUI defaults, plus the legacy
//! alternatives without the kitty protocol and the Windows Terminal stand-ins inside Windows
//! Terminal), enters TUI mode ([`TerminalGuard`]), registers the
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
//!    pump_services, pump_jobs, pump_background, apply_cmds, input
//!    ([`crate::input::route`], after closing stale dialogs), apply_cmds,
//!    [`crate::tools::drain`] (also right after pump_background), `editors::pump`, draw
//!    (prepare_visible_panes, [`Regions::compute`], [`view::prepare`],
//!    [`ViewModel::from_core`], [`view::draw`]), `ops::fileops::end_frame`, apply_cmds,
//!    update_title, end_frame; then the host's escape sequences (title, OSC 52) are
//!    written;
//! 3. a suspend the frame asked for ([`App::request_suspend`]) runs now, between two
//!    frames ([`crate::suspend::run`]);
//! 4. wait for input, a wake-up of the core, the earliest `redraw_after` deadline or the
//!    250 ms idle tick (toasts expire, spinners move).
//!
//! In raw mode Ctrl+C and Ctrl+Z are ordinary keys: Copy and Undo with the default key
//! map. The process is ended from outside with `SIGTERM` / `SIGHUP` (see
//! [`crate::signals`]), which saves first.

use std::io::{self, Write as _};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use soyuz_appcore::core::{AppCore, CMD_ROUNDS, FrontendKind, Startup};
use soyuz_appcore::host::Host as _;
use soyuz_appcore::models::frontend::SuspendRequest;
use soyuz_appcore::ops;
use soyuz_appcore::services::crash;
use soyuz_core::config::{Config, KeyContext, TuiSection};

use crate::host::TuiHost;
use crate::input::{InputState, TuiFocus};
use crate::layout::{Facts, Hits, Regions};
use crate::suspend::{ChildRunner, FinishedEdit, InputGate};
use crate::term::TerminalGuard;
use crate::theme::TuiTheme;
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

/// How long the input reader waits for input before it looks at its gate again.
pub const READ_POLL: Duration = Duration::from_millis(50);

/// Read terminal events on a thread (`crossterm::event::poll` + `read`) into `tx`. Start it
/// only after `crossterm::terminal::supports_keyboard_enhancement` was queried: the query
/// reads the terminal's answer from the same input stream.
///
/// The thread parks on `gate` while a child program owns the terminal
/// ([`crate::suspend::InputGate`]): it never sits in a blocking `read`, so it cannot take
/// the child's keystrokes. Once let go, it drops the input that piled up meanwhile (it was
/// meant for the child), except resizes.
pub fn spawn_reader(tx: Sender<TuiEvent>, gate: Arc<InputGate>) -> io::Result<()> {
    gate.attach_reader();
    let g = Arc::clone(&gate);
    let spawned = std::thread::Builder::new().name("soyuz-tui-input".into()).spawn(move || {
        read_events(&tx, &g);
        g.detach_reader();
    });
    if let Err(e) = spawned {
        gate.detach_reader();
        return Err(e);
    }
    Ok(())
}

/// The input reader's loop; returns when the loop is gone or reading failed.
fn read_events(tx: &Sender<TuiEvent>, gate: &InputGate) {
    let failed = |e: io::Error| {
        log::error!("tui: reading terminal input failed: {e}");
        let _ = tx.send(TuiEvent::Terminate);
    };
    loop {
        if gate.park() {
            // Stale: typed while the child had the terminal.
            loop {
                match crossterm::event::poll(Duration::ZERO) {
                    Ok(true) => match crossterm::event::read() {
                        Ok(ev @ Event::Resize(..)) => {
                            if tx.send(TuiEvent::Term(ev)).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => return failed(e),
                    },
                    Ok(false) => break,
                    Err(e) => return failed(e),
                }
            }
        }
        match crossterm::event::poll(READ_POLL) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => return failed(e),
        }
        match crossterm::event::read() {
            Ok(ev) => {
                if tx.send(TuiEvent::Term(ev)).is_err() {
                    return;
                }
            }
            Err(e) => return failed(e),
        }
    }
}

/// The terminal besides ratatui's drawing: raw escape sequences and mode changes.
pub trait TermIo {
    /// Write escape sequences (window title, OSC 52) after a frame.
    fn write_raw(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// `[tui]` changed: re-apply mouse capture / keyboard protocol.
    fn reapply(&mut self, cfg: &TuiSection) -> io::Result<()>;
    /// Continued after a stop: re-enter raw mode, the alternate screen and the modes.
    fn resume(&mut self, cfg: &TuiSection) -> io::Result<()>;
    /// Leave TUI mode for a child program (`crate::suspend`); [`TermIo::resume`] comes
    /// back.
    fn suspend(&mut self) -> io::Result<()>;
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

    fn suspend(&mut self) -> io::Result<()> {
        self.guard.suspend()
    }
}

/// No terminal (tests): records what would be written.
#[derive(Default)]
pub struct NullIo {
    pub written: Vec<u8>,
    pub reapplied: usize,
    pub resumed: usize,
    pub suspended: usize,
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

    fn suspend(&mut self) -> io::Result<()> {
        self.suspended += 1;
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
    /// Input state kept between frames (`crate::input`).
    pub(crate) input: InputState,
    /// Click targets of the last frame drawn.
    pub(crate) hits: Hits,
    /// The screen split of the last frame drawn.
    regions: Regions,
    /// The model of the last frame drawn.
    view: Option<ViewModel>,
    deadline: Option<Instant>,
    last_frame: Instant,
    last_save: Instant,
    /// What a terminal paste (`Event::Paste`) into the file list does: the core's Paste
    /// (`crate::input::list::paste`). Tests replace it, so they never read the system
    /// clipboard.
    pub list_paste: fn(&mut App),
    /// A suspend asked for this frame (`FrontendCmd::Suspend`), run by [`run_loop`]
    /// between two frames.
    pub(crate) pending_suspend: Option<SuspendRequest>,
    /// Runs the child of a suspend (`crate::suspend::default_runner`). Tests replace it,
    /// so they never start a real shell or editor.
    pub child_runner: ChildRunner,
    /// Parks the input reader while a child owns the terminal.
    pub(crate) input_gate: Arc<InputGate>,
    /// Editor runs for `EditPurpose::Scratch` that finished, until their owner takes them
    /// ([`App::take_finished_edit`]).
    pub(crate) finished_edits: Vec<FinishedEdit>,
}

impl App {
    /// Build the core for the terminal UI (`startup.frontend` is forced to the TUI: own
    /// session file, no FileManager1, no other windows).
    ///
    /// Installs the TUI's own default keys (`crate::overlay::tui_defaults`) before the core
    /// builds its key map, unless [`run`] installed another overlay (they all include them)
    /// first: the hook is process-wide and set once.
    pub fn new(config: Config, mut startup: Startup) -> App {
        soyuz_appcore::keys::set_keymap_overlay(crate::overlay::tui_defaults);
        startup.frontend = FrontendKind::Tui;
        let host = TuiHost::new();
        host.set_osc52(config.tui.osc52);
        // The terminal UI's texture store (`crate::images`).
        let uploader = crate::images::install(&config, host.wake());
        let mut core = AppCore::new(config, startup, host.wake(), uploader);
        core.renderer = "tui";
        App {
            core,
            host,
            input: InputState::default(),
            hits: Hits::default(),
            regions: Regions::default(),
            view: None,
            deadline: None,
            last_frame: Instant::now(),
            last_save: Instant::now(),
            list_paste: crate::input::list::paste,
            pending_suspend: None,
            child_runner: crate::suspend::default_runner,
            input_gate: Arc::new(InputGate::default()),
            finished_edits: Vec::new(),
        }
    }

    /// Leave the screen for a shell / command / editor after this frame (the first
    /// request of a frame wins).
    pub fn request_suspend(&mut self, req: SuspendRequest) {
        self.pending_suspend.get_or_insert(req);
    }

    /// A suspend is waiting for the end of the frame.
    pub fn suspend_pending(&self) -> bool {
        self.pending_suspend.is_some()
    }

    /// The gate that parks the input reader thread during a suspend (shared with it).
    pub fn input_gate(&self) -> Arc<InputGate> {
        Arc::clone(&self.input_gate)
    }

    /// The finished editor run of scratch edit `token`, if it is done (once).
    pub fn take_finished_edit(&mut self, token: u64) -> Option<FinishedEdit> {
        let i = self.finished_edits.iter().position(|e| e.token == token)?;
        Some(self.finished_edits.remove(i))
    }

    /// The model of the last frame drawn.
    pub fn view(&self) -> Option<&ViewModel> {
        self.view.as_ref()
    }

    /// The screen split of the last frame drawn.
    pub fn regions(&self) -> &Regions {
        &self.regions
    }

    /// Click targets of the last frame drawn.
    pub fn hits(&self) -> &Hits {
        &self.hits
    }

    /// Open a terminal UI dialog on top of every open dialog (`crate::input::dialogs`).
    pub fn push_dialog(&mut self, d: crate::input::dialogs::TuiDialog) {
        self.input.dialogs.tui.push(d);
    }

    /// Who owns the keyboard now (`crate::input::focus`).
    pub fn focus(&self) -> TuiFocus {
        crate::input::focus(&self.core, &self.input)
    }

    /// Key context of the frame's input (the core's prompts own the keyboard when open).
    pub fn key_context(&self) -> KeyContext {
        crate::input::key_context(self.focus()).0
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

    /// Carry out (or decline) the core's requests for the frontend (`crate::frontend`).
    pub fn drain_frontend(&mut self) {
        crate::frontend::drain(self);
    }

    fn input(&mut self, events: &[Event]) {
        // Dialogs that went away by themselves close first.
        crate::input::dialogs::remove_stale(self);
        crate::input::route(self, events);
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
        crate::tools::drain(self);
        self.apply_cmds();
        self.input(events);
        // Settings edits made in dialogs over the settings editor (tables, lists, the
        // shortcut recorder) apply in the same frame, not when the dialog closes.
        crate::input::settings::sync(self);
        self.apply_cmds();
        // Tool commands the input queued (menus, actions) apply in the same frame.
        crate::tools::drain(self);
        // The editors' worker results (loads, saves, answers to "Save changes?"), every
        // frame like the GUI's editor panel: a quit that saves first goes on from here.
        soyuz_appcore::editors::pump(&mut self.core, &self.host);
        self.tick_toasts(dt);

        let size = terminal.size().map_err(io::Error::other)?;
        self.core.prepare_visible_panes();
        let area = Rect::new(0, 0, size.width, size.height);
        let regions = Regions::compute(area, &self.core.config.tui, &Facts::from_core(&self.core));
        view::prepare(&mut self.core, &mut self.input, &regions);
        crate::input::jobs::tick(&self.core, &self.host);
        crate::input::compare::tick(&self.input.compare, &self.host);
        crate::input::screens::tick(&self.input.screens, &self.host);
        crate::input::dialogs::approval_mcp::tick(&self.core, &self.host);
        let model = ViewModel::from_core(&self.core, &self.input, &regions);
        let theme = TuiTheme::cached(&self.core, self.host.take_palette_dirty());
        let hits = &mut self.hits;
        crate::images::begin_frame(&self.core.config);
        terminal
            .draw(|f| {
                let area = f.area();
                f.buffer_mut().set_style(area, theme.base());
                view::draw(f, &model, &regions, &theme, hits);
                // After everything (overlays too): repair images drawn over.
                crate::images::end_frame(f);
            })
            .map_err(io::Error::other)?;
        self.regions = regions;
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
        if let Some(req) = app.pending_suspend.take() {
            crate::suspend::run(app, terminal, io, req)?;
        }
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
/// claimed the crash journal (`services::crash::startup`). Installs the key map overlay
/// (`crate::overlay::Overlay::pick`) before the core builds its key map: the TUI's default
/// keys, plus the legacy alternatives when the kitty keyboard protocol is not used (never on
/// Windows), plus the Windows Terminal stand-ins when `WT_SESSION` is set.
/// After the loop the terminal is restored and an "Install & restart" update restarts the
/// process (`services::updates`).
pub fn run(config: Config, startup: Startup) -> ExitCode {
    soyuz_fs::register_ui_thread();
    // Before any thread reads terminal input: the query reads the terminal's answer.
    let keyboard_supported = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    let kitty = crate::term::keyboard_flags(config.tui.keyboard_protocol, keyboard_supported, cfg!(windows)).is_some();
    let in_wt = crate::overlay::in_windows_terminal(|k| std::env::var_os(k));
    let overlay = crate::overlay::Overlay::pick(kitty, cfg!(windows), in_wt);
    log::info!("tui: key map overlay {overlay:?}");
    soyuz_appcore::keys::set_keymap_overlay(overlay.function());
    let guard = match TerminalGuard::enter(&config.tui, keyboard_supported) {
        Ok(g) => g,
        Err(e) => {
            crate::term::restore_terminal();
            let _ = writeln!(io::stderr(), "soyuz: cannot set up the terminal: {e}");
            return ExitCode::FAILURE;
        }
    };
    crash::set_tui_hooks(restore_for_panic, std::thread::current().id());
    // The graphics protocol query (`[tui] images = "auto"`, unix only), before the input
    // reader: the answer arrives on the input stream.
    crate::images::startup(&config.tui);

    let (tx, rx) = crossbeam_channel::unbounded();
    crate::signals::install(tx.clone());
    let gate = Arc::new(InputGate::default());
    if let Err(e) = spawn_reader(tx, Arc::clone(&gate)) {
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
    app.input_gate = gate;
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
