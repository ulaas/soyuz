//! alacritty_terminal wiring: `Term` in `Arc<FairMutex<..>>`, `EventLoop` + its sender,
//! the `Listener` (shared state with an `OnceLock<EventLoopSender>` so replies such as
//! `PtyWrite` can be written back to the PTY) and `GridSize: Dimensions`.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, State};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Osc52, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle as VteCursorStyle, Rgb};
use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::pty::{self, ShellKind};
use crate::tty_tee::TeePty;
use crate::{CursorStyle, TermError, TermEvent, TermSettings, TermTheme};

/// `WindowSize` does not implement `Dimensions`; this does (no scrollback: `Term` adds it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GridSize {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// State shared between the UI thread (`Terminal`) and the PTY thread (`Listener`).
/// Never lock the term from inside these methods: the PTY thread calls them while
/// holding the term lock.
pub(crate) struct Shared {
    ctx: egui::Context,
    events_tx: Sender<TermEvent>,
    sender: OnceLock<EventLoopSender>,
    pub alive: AtomicBool,
    exited_sent: AtomicBool,
    /// Set when a repaint was requested and the UI has not drawn since (coalesces Wakeups).
    pub repaint_pending: AtomicBool,
    start: Instant,
    last_output_ms: AtomicU64,
    pub title: Mutex<String>,
    default_title: String,
    pub cwd: Mutex<Option<PathBuf>>,
    /// The shell reported its cwd itself (OSC 7 or the title hook); `/proc` polling is
    /// then no longer consulted.
    pub cwd_reported: AtomicBool,
    pub window_size: Mutex<WindowSize>,
    colors: Mutex<ColorAnswers>,
}

/// Colors reported for OSC 4/10/11/12 queries.
struct ColorAnswers {
    palette: [egui::Color32; 256],
    fg: egui::Color32,
    bg: egui::Color32,
    cursor: egui::Color32,
}

impl ColorAnswers {
    fn new(theme: &TermTheme) -> Self {
        ColorAnswers { palette: theme.palette256(), fg: theme.fg, bg: theme.bg, cursor: theme.cursor }
    }
}

impl Shared {
    fn repaint(&self) {
        if !self.repaint_pending.swap(true, Ordering::AcqRel) {
            self.ctx.request_repaint();
        }
    }

    pub fn force_repaint(&self) {
        self.ctx.request_repaint();
    }

    fn send(&self, ev: TermEvent) {
        let _ = self.events_tx.send(ev);
    }

    /// Queue bytes for the PTY (no-op before the event loop exists / after shutdown).
    pub fn write(&self, bytes: Cow<'static, [u8]>) {
        if bytes.is_empty() {
            return;
        }
        if let Some(s) = self.sender.get() {
            let _ = s.send(Msg::Input(bytes));
        }
    }

    pub fn send_msg(&self, msg: Msg) {
        if let Some(s) = self.sender.get() {
            let _ = s.send(msg);
        }
    }

    pub fn idle_for(&self) -> Duration {
        let now = self.start.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.last_output_ms.load(Ordering::Acquire)))
    }

    pub fn mark_exited(&self, code: Option<i32>) {
        self.alive.store(false, Ordering::Release);
        if !self.exited_sent.swap(true, Ordering::AcqRel) {
            self.send(TermEvent::Exited(code));
        }
    }

    pub fn set_cwd(&self, path: PathBuf) {
        let changed = {
            let mut cwd = self.cwd.lock();
            if cwd.as_ref() == Some(&path) {
                false
            } else {
                *cwd = Some(path.clone());
                true
            }
        };
        if changed {
            self.send(TermEvent::CwdChanged(path));
        }
    }

    /// A cwd reported by the shell (OSC 7 from the PTY tee, or the title hook). The
    /// latest report wins.
    pub fn report_cwd(&self, path: PathBuf) {
        self.cwd_reported.store(true, Ordering::Release);
        self.set_cwd(path);
    }

    pub fn set_theme(&self, theme: &TermTheme) {
        let mut c = self.colors.lock();
        if c.fg != theme.fg || c.bg != theme.bg || c.cursor != theme.cursor || c.palette[..16] != theme.ansi {
            *c = ColorAnswers::new(theme);
        }
    }

    fn color_answer(&self, index: usize) -> Rgb {
        let c = self.colors.lock();
        let col = match index {
            0..=255 => c.palette[index],
            256 => c.fg,
            257 => c.bg,
            258 => c.cursor,
            _ => c.fg,
        };
        Rgb { r: col.r(), g: col.g(), b: col.b() }
    }

    fn set_title(&self, title: String) {
        let title = match pty::parse_cwd_title(&title) {
            Some(path) => {
                let shown = path.to_string_lossy().into_owned();
                self.report_cwd(path);
                shown
            }
            None => title,
        };
        let changed = {
            let mut t = self.title.lock();
            if *t == title {
                false
            } else {
                t.clone_from(&title);
                true
            }
        };
        if changed {
            self.send(TermEvent::Title(title));
        }
    }
}

/// The `EventListener` handed to alacritty (clone per owner).
#[derive(Clone)]
pub(crate) struct Listener(pub Arc<Shared>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let s = &self.0;
        match event {
            Event::Wakeup => {
                s.last_output_ms.store(s.start.elapsed().as_millis() as u64, Ordering::Release);
                s.repaint();
            }
            Event::Title(t) => {
                s.set_title(t);
                s.force_repaint();
            }
            Event::ResetTitle => {
                s.set_title(s.default_title.clone());
                s.force_repaint();
            }
            Event::Bell => {
                s.send(TermEvent::Bell);
                s.force_repaint();
            }
            Event::ChildExit(status) => {
                s.mark_exited(status.code());
                s.force_repaint();
            }
            Event::Exit => {
                s.mark_exited(None);
                s.force_repaint();
            }
            // Terminal replies (DSR, DA, …) must go back to the PTY.
            Event::PtyWrite(text) => s.write(Cow::Owned(text.into_bytes())),
            Event::ClipboardStore(_, text) => {
                s.send(TermEvent::Clipboard(text));
                s.force_repaint();
            }
            // The system clipboard is never exposed to the shell.
            Event::ClipboardLoad(_, fmt) => s.write(Cow::Owned(fmt("").into_bytes())),
            Event::ColorRequest(index, fmt) => s.write(Cow::Owned(fmt(s.color_answer(index)).into_bytes())),
            Event::TextAreaSizeRequest(fmt) => {
                let size = *s.window_size.lock();
                s.write(Cow::Owned(fmt(size).into_bytes()));
            }
            Event::CursorBlinkingChange => s.repaint(),
            Event::MouseCursorDirty => {}
        }
    }
}

type LoopHandle = JoinHandle<(EventLoop<TeePty, Listener>, State)>;

/// The running session.
pub(crate) struct Backend {
    pub term: Arc<FairMutex<Term<Listener>>>,
    pub shared: Arc<Shared>,
    join: Mutex<Option<LoopHandle>>,
    pub pid: Option<u32>,
    pub kind: ShellKind,
    pub id: u64,
    /// Last (cols, rows, cell_w, cell_h) applied.
    size: Mutex<(u16, u16, u16, u16)>,
}

const INITIAL: (u16, u16, u16, u16) = (80, 24, 8, 16);

fn window_size((cols, rows, cw, ch): (u16, u16, u16, u16)) -> WindowSize {
    WindowSize { num_cols: cols, num_lines: rows, cell_width: cw, cell_height: ch }
}

impl Backend {
    pub fn spawn(
        settings: &TermSettings,
        ctx: egui::Context,
        id: u64,
        events_tx: Sender<TermEvent>,
    ) -> Result<Backend, TermError> {
        let cmd = pty::shell_command(settings)?;
        let default_title = std::path::Path::new(&cmd.program)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| cmd.program.clone());

        let shared = Arc::new(Shared {
            ctx,
            events_tx,
            sender: OnceLock::new(),
            alive: AtomicBool::new(true),
            exited_sent: AtomicBool::new(false),
            repaint_pending: AtomicBool::new(false),
            start: Instant::now(),
            last_output_ms: AtomicU64::new(0),
            title: Mutex::new(default_title.clone()),
            default_title,
            cwd: Mutex::new(cmd.cwd.clone()),
            cwd_reported: AtomicBool::new(false),
            window_size: Mutex::new(window_size(INITIAL)),
            colors: Mutex::new(ColorAnswers::new(&TermTheme::default())),
        });
        let listener = Listener(shared.clone());

        let shape = match settings.cursor {
            CursorStyle::Block => CursorShape::Block,
            CursorStyle::Beam => CursorShape::Beam,
            CursorStyle::Underline => CursorShape::Underline,
        };
        let config = term::Config {
            scrolling_history: settings.scrollback,
            default_cursor_style: VteCursorStyle { shape, blinking: false },
            // We encode keys the legacy xterm way; advertising kitty would make apps
            // expect CSI-u encoding.
            kitty_keyboard: false,
            osc52: Osc52::OnlyCopy,
            ..Default::default()
        };
        let grid = GridSize { cols: usize::from(INITIAL.0), lines: usize::from(INITIAL.1) };
        let term = Arc::new(FairMutex::new(Term::new(config, &grid, listener.clone())));

        // Windows: CreateProcessW parses the first token, so quote programs with spaces.
        let program = if cfg!(windows) && cmd.program.contains(' ') && !cmd.program.starts_with('"') {
            format!("\"{}\"", cmd.program)
        } else {
            cmd.program.clone()
        };
        let options = tty::Options {
            shell: Some(tty::Shell::new(program, cmd.args.clone())),
            working_directory: cmd.cwd.clone(),
            drain_on_exit: true,
            env: cmd.env.clone(),
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, window_size(INITIAL), id).map_err(TermError::Pty)?;
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(|p| p.get());
        #[cfg(not(windows))]
        let pid = Some(pty.child().id());

        // OSC 7 is dropped by alacritty's parser: observe the raw output on the way in.
        let pty = TeePty::new(pty, shared.clone());
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false).map_err(TermError::EventLoop)?;
        let _ = shared.sender.set(event_loop.channel());
        let join = event_loop.spawn();

        Ok(Backend { term, shared, join: Mutex::new(Some(join)), pid, kind: cmd.kind, id, size: Mutex::new(INITIAL) })
    }

    /// Apply a new grid size. No-op when unchanged.
    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        let new = (cols.max(2), rows.max(1), cell_w.max(1), cell_h.max(1));
        {
            let mut size = self.size.lock();
            if *size == new {
                return;
            }
            *size = new;
        }
        *self.shared.window_size.lock() = window_size(new);
        self.term.lock().resize(GridSize { cols: usize::from(new.0), lines: usize::from(new.1) });
        self.shared.send_msg(Msg::Resize(window_size(new)));
    }

    /// Stop the event loop; dropping the loop's `Pty` closes ConPTY / sends SIGHUP.
    pub fn shutdown(&self) {
        if let Some(join) = self.join.lock().take() {
            self.shared.send_msg(Msg::Shutdown);
            // Detach: the thread drops the event loop (and the PTY) when it returns.
            drop(join);
        }
        self.shared.mark_exited(None);
    }
}
