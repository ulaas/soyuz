//! Single instance: starting `soyuz` while a Soyuz window runs (Start menu, taskbar,
//! `soyuz <path>`, "Open with") hands the request to the running process, which opens a
//! window or a tab (`[workspace] launch`); the launcher then exits. An extra process
//! costs 140–350 MB, an in-process window a few MB.
//!
//! * Endpoint: its own pipe / socket (the `soyuz_mcp::pipe` transport, current user only),
//!   not the CLI pipe (that one is bound lazily, optional, and served by the terminal UI
//!   too). The name is scoped by a stable hash of the config dir, the Windows session
//!   (named pipes are machine-global; Remote Desktop), elevation and the unix display
//!   ([`EndpointKey`]), so a portable copy, an elevated window or another desktop session
//!   each have their own primary.
//! * Election: every launcher tries to bind before it opens a window; the one that binds
//!   is the primary. A loser connects; while the winner has not bound its socket yet (or
//!   its pipe is busy) it retries for [`ELECTION`].
//! * Protocol (JSON lines): the launcher sends a hello (protocol, version, git commit) and
//!   a `launch` request with absolute paths; the window answers `queued` at once (with its
//!   pid and whether it is still starting), then `applied` once the UI took it, or
//!   `closing` / `rejected` / `expired`. Anything but `applied` makes the launcher run as
//!   its own process (`Startup::secondary`: no session writes).
//! * The UI side ([`pump`]) opens the window / tab, merges launches that arrive within
//!   [`MERGE`] (Explorer starts one process per selected folder) into tabs of one window,
//!   and raises the window with [`WindowCmd::Raise`] (the launcher granted the foreground).
//! * [`shutdown`] drops the endpoint as soon as quitting starts, so a launch during exit
//!   (or "Install & restart") starts a fresh process instead of talking to a dying one.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use soyuz_core::Location;
use soyuz_core::config::LaunchMode;

use crate::core::AppCore;
use crate::host::{Host, WindowCmd, WindowId as HostWindow};
use crate::models::windows::WinOp;
use crate::services::mailbox::Wake;
use crate::state::pane::Pane;
use crate::state::workspace::{Tab, TabId};
use crate::state::workspace_windows::WindowId;

/// Wire protocol version (hello `soyuz_instance`).
pub const PROTOCOL: u64 = 1;
/// How long a launcher keeps trying to bind or reach the primary.
pub const ELECTION: Duration = Duration::from_secs(2);
/// How long a launcher waits for `queued` after sending its request.
pub const QUEUE_WAIT: Duration = Duration::from_secs(3);
/// Launches closer together than this become tabs of one window.
pub const MERGE: Duration = Duration::from_millis(300);
/// Dev hook: allow the handoff in scripted runs (`SOYUZ_SCRIPT` / `SOYUZ_WINDOW_POS`);
/// neither the launcher nor the window touches the foreground then.
pub const TEST_ENV: &str = "SOYUZ_INSTANCE_TEST";

/// How long the launcher waits for `applied` (and the window keeps the request): a
/// window that is still starting needs its first frame (≥ 355 ms after `main`, more on a
/// cold start).
pub fn apply_wait(starting: bool) -> Duration {
    Duration::from_secs(if starting { 15 } else { 5 })
}

// ---------------------------------------------------------------------------
// Endpoint name
// ---------------------------------------------------------------------------

/// What scopes the instance endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointKey {
    /// Normalised config dir ([`normalize_dir`]).
    pub config_dir: String,
    /// Windows session id.
    pub session: Option<u32>,
    pub elevated: bool,
    /// unix: `WAYLAND_DISPLAY` / `DISPLAY` (one primary per desktop).
    pub display: String,
}

impl EndpointKey {
    pub fn current() -> EndpointKey {
        let display = if cfg!(unix) {
            let var = |k: &str| std::env::var(k).unwrap_or_default();
            format!("{}|{}", var("WAYLAND_DISPLAY"), var("DISPLAY"))
        } else {
            String::new()
        };
        EndpointKey {
            config_dir: normalize_dir(&soyuz_core::config::config_dir()),
            session: soyuz_fs::instance::session_id(),
            elevated: soyuz_fs::instance::is_elevated(),
            display,
        }
    }

    /// Stable 64-bit FNV-1a hash (the same in every build, unlike `DefaultHasher`).
    pub fn hash(&self) -> u64 {
        let session = self.session.map(|s| s.to_string()).unwrap_or_default();
        let parts = [self.config_dir.as_str(), &session, if self.elevated { "1" } else { "0" }, &self.display];
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for p in parts {
            for b in p.bytes().chain([0]) {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }
}

/// A directory as one string per place: on Windows without the `\\?\` prefix, with
/// backslashes, no trailing separator and lowercased (paths are case-insensitive there).
pub fn normalize_dir(dir: &Path) -> String {
    let s = dir.to_string_lossy();
    if cfg!(windows) {
        let s = s.replace('/', "\\");
        let s = match s.strip_prefix(r"\\?\UNC\") {
            Some(rest) => format!(r"\\{rest}"),
            None => s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s),
        };
        let t = s.trim_end_matches('\\');
        // Keep the separator of a drive root (`c:\`).
        let t = if t.ends_with(':') { format!("{t}\\") } else { t.to_string() };
        t.to_lowercase()
    } else {
        let t = s.trim_end_matches('/');
        if t.is_empty() { "/".to_string() } else { t.to_string() }
    }
}

/// The endpoint for this user, config dir, session, elevation and display.
pub fn endpoint_name() -> String {
    soyuz_mcp::pipe::user_pipe_name(&format!("soyuz-instance-{:016x}", EndpointKey::current().hash()))
}

// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------

/// Who is talking: the running window and the launcher must be the same build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub version: String,
    pub sha: String,
}

impl Ident {
    pub fn this_build() -> Ident {
        Ident { version: env!("CARGO_PKG_VERSION").to_string(), sha: crate::GIT_SHA.to_string() }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    soyuz_instance: u64,
    version: String,
    sha: String,
    pid: u32,
}

/// A launch handed to the running window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchRequest {
    /// The first path (a file opens its folder with the file selected).
    pub open: Option<PathBuf>,
    /// Further paths (more tabs).
    #[serde(default)]
    pub extra: Vec<PathBuf>,
    /// Linux startup notification (`XDG_ACTIVATION_TOKEN` / `DESKTOP_STARTUP_ID`).
    #[serde(default)]
    pub activation_token: Option<String>,
}

impl LaunchRequest {
    /// The command-line paths made absolute against the launcher's working directory (no
    /// I/O; remote URLs stay as they are), plus the activation token from the environment.
    pub fn from_args(open: Option<&Path>, extra: &[PathBuf]) -> LaunchRequest {
        let token = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
        LaunchRequest {
            open: open.map(absolutize),
            extra: extra.iter().map(|p| absolutize(p)).collect(),
            activation_token: token,
        }
    }

    fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.open.iter().chain(&self.extra)
    }
}

/// `p` made absolute without touching the disk; remote URLs (`sftp://…`) unchanged.
pub fn absolutize(p: &Path) -> PathBuf {
    if crate::services::remote::is_remote_path(p) {
        return p.to_path_buf();
    }
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

#[derive(Debug, Serialize, Deserialize)]
struct RequestLine {
    op: String,
    #[serde(flatten)]
    req: LaunchRequest,
}

/// The window's answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Queued { pid: u32, starting: bool },
    Applied,
    Closing,
    Rejected { reason: String },
    Expired,
}

fn line<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string(v).unwrap_or_default();
    s.push('\n');
    s
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// A launch waiting for the UI.
struct Pending {
    targets: Vec<(Location, Option<String>)>,
    deadline: Instant,
    reply: Sender<Event>,
}

struct Shared {
    ident: Ident,
    queue: Mutex<Vec<Pending>>,
    wake: Mutex<Option<Wake>>,
    /// The UI pumped at least once.
    started: AtomicBool,
    /// Quitting: new requests get `closing` (set under the queue lock).
    closing: AtomicBool,
}

impl Shared {
    fn push(&self, p: Pending) -> Result<(), Pending> {
        {
            let mut q = self.queue.lock();
            if self.closing.load(Ordering::SeqCst) {
                return Err(p);
            }
            q.push(p);
        }
        if let Some(w) = self.wake.lock().clone() {
            w();
        }
        Ok(())
    }

    fn take(&self) -> Vec<Pending> {
        std::mem::take(&mut *self.queue.lock())
    }

    fn close(&self) {
        let drained = {
            let mut q = self.queue.lock();
            self.closing.store(true, Ordering::SeqCst);
            std::mem::take(&mut *q)
        };
        for p in drained {
            let _ = p.reply.send(Event::Closing);
        }
    }
}

/// The bound instance endpoint (the primary holds it for its whole run).
pub struct Listener {
    shared: Arc<Shared>,
    server: Option<soyuz_mcp::pipe::PipeServer>,
}

impl Listener {
    /// Bind `name` for this build; fails when another process serves it.
    pub fn bind(name: &str) -> std::io::Result<Listener> {
        Listener::bind_as(name, Ident::this_build())
    }

    fn bind_as(name: &str, ident: Ident) -> std::io::Result<Listener> {
        let shared = Arc::new(Shared {
            ident,
            queue: Mutex::new(Vec::new()),
            wake: Mutex::new(None),
            started: AtomicBool::new(false),
            closing: AtomicBool::new(false),
        });
        let s = shared.clone();
        let server = soyuz_mcp::pipe::PipeServer::serve(name, move |stream| serve_connection(&s, stream))?;
        Ok(Listener { shared, server: Some(server) })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shared.close();
        // Unbind (the accept loop ends, the name is free for the next primary).
        drop(self.server.take());
    }
}

/// One launcher connection (its own thread): hello, request, `queued`, then the UI's answer.
fn serve_connection<S: std::io::Read + Write + Clone>(shared: &Shared, stream: S) {
    let mut out = stream.clone();
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let Ok(hello) = serde_json::from_str::<Hello>(first.trim()) else {
        log::warn!("instance: connection without a hello line; closing");
        return;
    };
    let reject = |out: &mut S, reason: String| {
        log::info!("instance: refused a launch from pid {}: {reason}", hello.pid);
        let _ = out.write_all(line(&Event::Rejected { reason }).as_bytes());
    };
    if hello.soyuz_instance != PROTOCOL {
        return reject(&mut out, format!("protocol {} (this window speaks {PROTOCOL})", hello.soyuz_instance));
    }
    if hello.version != shared.ident.version || hello.sha != shared.ident.sha {
        return reject(
            &mut out,
            format!(
                "another build ({} {} here, {} {} launching)",
                shared.ident.version, shared.ident.sha, hello.version, hello.sha
            ),
        );
    }
    let mut second = String::new();
    if reader.read_line(&mut second).unwrap_or(0) == 0 {
        return;
    }
    let req = match serde_json::from_str::<RequestLine>(second.trim()) {
        Ok(r) if r.op == "launch" => r.req,
        Ok(r) => return reject(&mut out, format!("unknown op {}", r.op)),
        Err(e) => return reject(&mut out, format!("bad request: {e}")),
    };
    // Resolve on this thread (stat calls stay off the UI thread).
    let targets: Vec<_> = req.paths().filter_map(|p| crate::core::startup_target(p)).collect();
    let starting = !shared.started.load(Ordering::SeqCst);
    let wait = apply_wait(starting);
    let (tx, rx) = crossbeam_channel::bounded(1);
    let pending = Pending { targets, deadline: Instant::now() + wait, reply: tx };
    if shared.push(pending).is_err() {
        let _ = out.write_all(line(&Event::Closing).as_bytes());
        return;
    }
    log::info!("instance: queued a launch from pid {} ({} path(s))", hello.pid, req.paths().count());
    if out.write_all(line(&Event::Queued { pid: std::process::id(), starting }).as_bytes()).is_err() {
        return; // the launcher is gone; the UI's answer has nowhere to go
    }
    let ev = rx.recv_timeout(wait + Duration::from_secs(1)).unwrap_or(Event::Expired);
    let _ = out.write_all(line(&ev).as_bytes());
}

static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);

/// Keep `l` as this process's endpoint (the primary, before the window opens).
pub fn install(l: Listener) {
    *LISTENER.lock() = Some(l);
}

/// Stop serving the endpoint (quitting started / the process exits): queued launches are
/// answered `closing` and later launchers start their own process.
pub fn shutdown() {
    let l = LISTENER.lock().take();
    if l.is_some() {
        log::info!("instance: endpoint closed");
    }
    drop(l);
}

/// Launches are waiting for [`pump`] (see `AppCore::needs_ui_pass`).
pub fn pending() -> bool {
    active().is_some_and(|s| !s.queue.lock().is_empty())
}

fn active() -> Option<Arc<Shared>> {
    LISTENER.lock().as_ref().map(|l| l.shared.clone())
}

// ---------------------------------------------------------------------------
// UI side
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PumpState {
    /// The last window a launch opened: (when, its first tab).
    recent: Option<(Instant, TabId)>,
    /// A window to raise once it exists: (its first tab, give up at).
    raise: Option<(TabId, Instant)>,
}

static PUMP: Mutex<PumpState> = Mutex::new(PumpState { recent: None, raise: None });

/// Every window of this process is minimized (set by the frontend each frame).
static NONE_SHOWN: AtomicBool = AtomicBool::new(false);

/// The frontend shows no window right now (all minimized). A launch then opens as a tab
/// of an existing window, which is restored: eframe can run a UI pass for a hidden
/// window but cannot create a new one from it.
pub fn set_none_shown(none: bool) {
    NONE_SHOWN.store(none, Ordering::Relaxed);
}

/// Per-frame pump (UI thread, no secondary window swapped in): apply queued launches.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    let Some(shared) = active() else { return };
    if !app.workspaces.current.is_main() {
        return;
    }
    {
        let mut w = shared.wake.lock();
        if w.is_none() {
            *w = Some(host.wake());
        }
    }
    shared.started.store(true, Ordering::SeqCst);
    let mut st = PUMP.lock();
    if let Some((tab, until)) = st.raise {
        match window_of_tab(app, tab) {
            Some(id) => {
                st.raise = None;
                host.window(HostWindow::Window(id), WindowCmd::Raise);
            }
            None if Instant::now() > until => st.raise = None,
            None => host.redraw_after(Duration::from_millis(16)),
        }
    }
    for p in shared.take() {
        let now = Instant::now();
        if now > p.deadline {
            let _ = p.reply.send(Event::Expired);
            continue;
        }
        apply(app, host, &mut st, p.targets, now);
        let _ = p.reply.send(Event::Applied);
    }
}

fn apply(
    app: &mut AppCore,
    host: &dyn Host,
    st: &mut PumpState,
    targets: Vec<(Location, Option<String>)>,
    now: Instant,
) {
    let targets = if targets.is_empty() { vec![(crate::core::start_location(&app.config), None)] } else { targets };
    // A burst of launches (one process per selected folder): tabs of the first one's window.
    if let Some((at, tab)) = st.recent
        && now.duration_since(at) < MERGE
        && add_to_window_of(app, tab, &targets)
    {
        st.recent = Some((now, tab));
        log::info!("instance: launch merged into the previous launch's window");
        return;
    }
    let none_shown = NONE_SHOWN.load(Ordering::Relaxed);
    let mode = effective_mode(app.config.workspace.launch, none_shown);
    if mode != app.config.workspace.launch {
        log::info!("instance: every window is minimized; the launch opens as a tab");
    }
    match mode {
        LaunchMode::NewTab => {
            let owner = app.workspaces.owner;
            let id = if crate::models::windows::index(app, owner).is_some() { owner } else { WindowId::MAIN };
            crate::models::windows::with_window(app, id, |app| open_tabs(app, &targets));
            host.window(HostWindow::Window(id), WindowCmd::Raise);
            st.recent = None;
            log::info!("instance: launch opened {} tab(s) in window {}", targets.len(), id.0);
        }
        LaunchMode::NewWindow | LaunchMode::NewProcess => {
            let tabs: Vec<Tab> = targets.into_iter().map(|(loc, sel)| make_tab(app, loc, sel)).collect();
            let first = tabs[0].id;
            let size = app.workspaces.current_size();
            log::info!("instance: launch opens a new window ({} tab(s))", tabs.len());
            app.workspaces.ops.push(WinOp::OpenTabs { tabs, group: None, pos: None, size });
            st.recent = Some((now, first));
            st.raise = Some((first, now + Duration::from_secs(3)));
            host.redraw();
        }
    }
}

/// How a launch opens: as `[workspace] launch` says, except that a new window becomes a
/// tab (of a window that is then restored) while no window is shown.
fn effective_mode(configured: LaunchMode, none_shown: bool) -> LaunchMode {
    match configured {
        LaunchMode::NewWindow | LaunchMode::NewProcess if none_shown => LaunchMode::NewTab,
        mode => mode,
    }
}

fn make_tab(app: &mut AppCore, loc: Location, select: Option<String>) -> Tab {
    let loc = crate::services::remote::normalize(loc.clone()).unwrap_or(loc);
    let mut pane = Pane::new(loc.clone(), app.view_for(&loc));
    crate::ops::start_listing(&mut pane, &mut app.services, false);
    pane.pending_cursor = select;
    Tab::new(pane)
}

/// Open `targets` as tabs of the current workspace (`app.ws`), the first one in front.
fn open_tabs(app: &mut AppCore, targets: &[(Location, Option<String>)]) {
    let mut front = None;
    for (loc, select) in targets {
        crate::ops::open_in_new_tab(app, loc.clone(), true);
        app.ws.pane_mut().pending_cursor = select.clone();
        front.get_or_insert(app.ws.tabs[app.ws.active].id);
    }
    if let Some(i) = front.and_then(|id| app.ws.tab_index(id)) {
        app.ws.active = i;
    }
}

/// Run `f` with window `id`'s workspace swapped into `app.ws`.
/// The window that shows tab `tab`.
fn window_of_tab(app: &AppCore, tab: TabId) -> Option<WindowId> {
    if app.ws.tab_index(tab).is_some() {
        return Some(WindowId::MAIN);
    }
    app.workspaces.windows.iter().find(|w| w.ws.tab_index(tab).is_some()).map(|w| w.id)
}

/// Add `targets` as tabs of the window holding `tab` (or of the queued window it opens
/// with). `false` when that window is gone.
fn add_to_window_of(app: &mut AppCore, tab: TabId, targets: &[(Location, Option<String>)]) -> bool {
    if let Some(id) = window_of_tab(app, tab) {
        crate::models::windows::with_window(app, id, |app| {
            let keep = app.ws.active;
            open_tabs(app, targets);
            app.ws.active = keep.min(app.ws.tabs.len().saturating_sub(1));
        });
        return true;
    }
    let made: Vec<Tab> = targets.iter().map(|(l, s)| make_tab(app, l.clone(), s.clone())).collect();
    for op in &mut app.workspaces.ops {
        if let WinOp::OpenTabs { tabs, .. } = op
            && tabs.first().is_some_and(|t| t.id == tab)
        {
            tabs.extend(made);
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Launcher side
// ---------------------------------------------------------------------------

/// What the launcher does after [`claim_or_hand_off`].
pub enum Handoff {
    /// The running window took the request: exit.
    Handed,
    /// We are the first: keep this endpoint ([`install`]) and open the window.
    Primary(Listener),
    /// Run as our own (secondary) process, for this reason.
    Own(String),
}

/// `[workspace] launch` from `config.toml`, read without migrating or writing anything
/// (the full load comes later, only if this process stays).
pub fn configured_mode() -> LaunchMode {
    match soyuz_core::config::load_or_default(&soyuz_core::config::default_path()) {
        Ok(l) => l.config.workspace.launch,
        Err(_) => LaunchMode::default(),
    }
}

/// Become the primary, or hand `req` to the one that is.
pub fn claim_or_hand_off(name: &str, req: &LaunchRequest) -> Handoff {
    claim_as(name, req, &Ident::this_build())
}

fn claim_as(name: &str, req: &LaunchRequest, ident: &Ident) -> Handoff {
    let mut deadline = Instant::now() + ELECTION;
    let mut retried_closing = false;
    let mut last: String;
    loop {
        match Listener::bind_as(name, ident.clone()) {
            Ok(l) => return Handoff::Primary(l),
            // Windows: a taken first instance reads as "access denied", like a squatted
            // name; unix: `AddrInUse`. Either way, ask whoever serves it.
            Err(e) => last = format!("bind: {e}"),
        }
        match soyuz_mcp::pipe::connect(name) {
            Ok(Some(stream)) => match send(stream, req, ident) {
                Sent::Applied => return Handoff::Handed,
                Sent::Own(why) => return Handoff::Own(why),
                Sent::Closing if !retried_closing => {
                    // The window is quitting: take over once it has let go of the name.
                    retried_closing = true;
                    deadline = Instant::now() + ELECTION;
                    last = "the running window is closing".into();
                }
                Sent::Closing => return Handoff::Own("the running window is closing".into()),
                Sent::Gone(why) => last = why,
            },
            // Not served (yet): the winner of a concurrent start may not have bound.
            Ok(None) => {}
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Handoff::Own(e.to_string()),
            Err(e) => last = format!("connect: {e}"),
        }
        if Instant::now() >= deadline {
            return Handoff::Own(last);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

enum Sent {
    Applied,
    Closing,
    /// Fall back to our own process (refused, timed out).
    Own(String),
    /// The connection ended without an answer (the window went away): try again.
    Gone(String),
}

fn send(stream: soyuz_mcp::pipe::ClientStream, req: &LaunchRequest, ident: &Ident) -> Sent {
    // Let the window take the foreground before it applies the request (Windows).
    if std::env::var_os(TEST_ENV).is_none()
        && let Some(pid) = soyuz_mcp::pipe::server_pid(&stream)
    {
        soyuz_fs::instance::allow_set_foreground(pid);
    }
    let hello = Hello {
        soyuz_instance: PROTOCOL,
        version: ident.version.clone(),
        sha: ident.sha.clone(),
        pid: std::process::id(),
    };
    let mut out = stream.clone();
    let text = line(&hello) + line(&RequestLine { op: "launch".into(), req: req.clone() }).as_str();
    if let Err(e) = out.write_all(text.as_bytes()) {
        return Sent::Gone(format!("send: {e}"));
    }
    let events = read_events(stream);
    let starting = match events.recv_timeout(QUEUE_WAIT) {
        Ok(Some(Event::Queued { starting, .. })) => starting,
        Ok(Some(ev)) => return answer(ev),
        Ok(None) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            return Sent::Gone("the window closed the connection".into());
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            return Sent::Own("the running window did not answer".into());
        }
    };
    match events.recv_timeout(apply_wait(starting)) {
        Ok(Some(ev)) => answer(ev),
        Ok(None) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            Sent::Gone("the window closed the connection".into())
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            Sent::Own(format!("the running window did not open it within {}s", apply_wait(starting).as_secs()))
        }
    }
}

fn answer(ev: Event) -> Sent {
    match ev {
        Event::Applied => Sent::Applied,
        Event::Closing => Sent::Closing,
        Event::Rejected { reason } => Sent::Own(format!("the running window refused: {reason}")),
        Event::Expired => Sent::Own("the running window was too busy to open it in time".into()),
        Event::Queued { .. } => Sent::Own("unexpected answer".into()),
    }
}

/// Events from the window, read on a helper thread (pipe reads have no timeout).
/// `None`: the connection ended.
fn read_events(stream: soyuz_mcp::pipe::ClientStream) -> Receiver<Option<Event>> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let _ = std::thread::Builder::new().name("soyuz-instance-read".into()).spawn(move || {
        let mut r = BufReader::new(stream);
        loop {
            let mut l = String::new();
            match r.read_line(&mut l) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
                Ok(_) => {
                    if let Ok(ev) = serde_json::from_str::<Event>(l.trim())
                        && tx.send(Some(ev)).is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(tag: &str) -> String {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        soyuz_mcp::pipe::user_pipe_name(&format!("soyuz-instance-test-{tag}-{}-{t:x}", std::process::id()))
    }

    /// Answer every queued launch with `ev` until `stop` (a stand-in for the UI pump).
    fn fake_ui(l: &Listener, ev: Event, stop: &AtomicBool) -> Vec<Vec<(Location, Option<String>)>> {
        let mut got = Vec::new();
        l.shared.started.store(true, Ordering::SeqCst);
        while !stop.load(Ordering::SeqCst) {
            for p in l.shared.take() {
                got.push(p.targets);
                let _ = p.reply.send(ev.clone());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn launch_round_trip() {
        let name = unique("rt");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let Handoff::Primary(l) = claim_or_hand_off(&name, &LaunchRequest::default()) else {
            panic!("the first launcher binds");
        };
        let stop = Arc::new(AtomicBool::new(false));
        let client = std::thread::spawn({
            let (name, file, stop) = (name.clone(), file.clone(), stop.clone());
            move || {
                let req = LaunchRequest { open: Some(file), ..Default::default() };
                let r = matches!(claim_or_hand_off(&name, &req), Handoff::Handed);
                stop.store(true, Ordering::SeqCst);
                r
            }
        });
        let got = fake_ui(&l, Event::Applied, &stop);
        assert!(client.join().unwrap(), "the second launcher hands over");
        // A file argument opens its folder with the file selected (never launches it).
        assert_eq!(got, vec![vec![(Location::Dir(dir.path().to_path_buf()), Some("a.txt".to_string()))]]);
        drop(l);
        // The name is free again: the next launcher becomes the primary.
        assert!(matches!(claim_or_hand_off(&name, &LaunchRequest::default()), Handoff::Primary(_)));
    }

    #[test]
    fn another_build_is_refused() {
        let name = unique("ver");
        let other = Ident { version: "0.0.1".into(), sha: "abc".into() };
        let _l = Listener::bind_as(&name, other).unwrap();
        match claim_or_hand_off(&name, &LaunchRequest::default()) {
            Handoff::Own(why) => assert!(why.contains("another build"), "{why}"),
            _ => panic!("a different build runs its own process"),
        }
    }

    #[test]
    fn closing_window_lets_the_launcher_take_over() {
        let name = unique("close");
        let l = Listener::bind(&name).unwrap();
        l.shared.started.store(true, Ordering::SeqCst);
        let client = std::thread::spawn({
            let name = name.clone();
            move || claim_or_hand_off(&name, &LaunchRequest::default())
        });
        // Wait for the request, then quit: it is answered `closing` and the name freed.
        while l.shared.queue.lock().is_empty() {
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(l);
        assert!(matches!(client.join().unwrap(), Handoff::Primary(_)), "the launcher became the primary");
    }

    #[test]
    fn requests_after_close_are_refused() {
        let s = Shared {
            ident: Ident::this_build(),
            queue: Mutex::new(Vec::new()),
            wake: Mutex::new(None),
            started: AtomicBool::new(true),
            closing: AtomicBool::new(false),
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        assert!(s.push(Pending { targets: Vec::new(), deadline: Instant::now(), reply: tx }).is_ok());
        s.close();
        assert_eq!(rx.try_recv(), Ok(Event::Closing), "queued launches hear `closing`");
        let (tx, _rx) = crossbeam_channel::bounded(1);
        assert!(s.push(Pending { targets: Vec::new(), deadline: Instant::now(), reply: tx }).is_err());
    }

    /// Several launchers at once (Explorer multi-select): exactly one primary, the others
    /// hand over to it.
    #[test]
    fn concurrent_launchers_elect_one_primary() {
        let name = unique("race");
        let n = 5;
        let barrier = Arc::new(std::sync::Barrier::new(n));
        let primaries = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let threads: Vec<_> = (0..n)
            .map(|_| {
                let (name, barrier, primaries, stop) = (name.clone(), barrier.clone(), primaries.clone(), stop.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    match claim_or_hand_off(&name, &LaunchRequest::default()) {
                        Handoff::Primary(l) => {
                            primaries.lock().push(());
                            let got = fake_ui(&l, Event::Applied, &stop);
                            Some(got.len())
                        }
                        Handoff::Handed => None,
                        Handoff::Own(why) => panic!("fell back: {why}"),
                    }
                })
            })
            .collect();
        let deadline = Instant::now() + Duration::from_secs(20);
        // Stop the primary's fake UI once every other launcher has its answer.
        let mut results = Vec::new();
        let mut pending: Vec<_> = threads.into_iter().map(Some).collect();
        while results.len() < n - 1 && Instant::now() < deadline {
            for t in pending.iter_mut() {
                if t.as_ref().is_some_and(|t| t.is_finished()) {
                    results.push(t.take().unwrap().join().unwrap());
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::SeqCst);
        for t in pending.into_iter().flatten() {
            results.push(t.join().unwrap());
        }
        assert_eq!(primaries.lock().len(), 1, "exactly one primary");
        assert_eq!(results.iter().flatten().copied().collect::<Vec<_>>(), vec![n - 1], "it served the others");
    }

    #[test]
    fn endpoint_key_hash_inputs() {
        let base = EndpointKey {
            config_dir: r"c:\users\a\appdata\roaming\soyuz".into(),
            session: Some(1),
            elevated: false,
            display: String::new(),
        };
        let h = base.hash();
        assert_eq!(h, base.clone().hash(), "stable");
        assert_ne!(h, EndpointKey { session: Some(2), ..base.clone() }.hash(), "session");
        assert_ne!(h, EndpointKey { elevated: true, ..base.clone() }.hash(), "elevation");
        assert_ne!(h, EndpointKey { display: "wayland-1|".into(), ..base.clone() }.hash(), "display");
        assert_ne!(h, EndpointKey { config_dir: "d:\\portable".into(), ..base.clone() }.hash(), "config dir");
        // FNV-1a is fixed: the same in every build and process.
        let empty = EndpointKey { config_dir: String::new(), session: None, elevated: false, display: String::new() };
        assert_eq!(empty.hash(), 0x5a24_9fd8_d6ab_fe9f);
        let name = endpoint_name();
        assert!(name.contains("soyuz-instance-"), "{name}");
    }

    #[test]
    fn config_dirs_normalise() {
        if cfg!(windows) {
            let a = normalize_dir(Path::new(r"\\?\C:\Users\A\AppData\Roaming\Soyuz\"));
            let b = normalize_dir(Path::new("c:/users/a/appdata/roaming/soyuz"));
            assert_eq!(a, b);
            assert_eq!(a, r"c:\users\a\appdata\roaming\soyuz");
            assert_eq!(normalize_dir(Path::new(r"\\?\UNC\srv\share\x")), r"\\srv\share\x");
            assert_eq!(normalize_dir(Path::new(r"D:\")), r"d:\");
        } else {
            assert_eq!(normalize_dir(Path::new("/home/a/.config/soyuz/")), "/home/a/.config/soyuz");
            assert_eq!(normalize_dir(Path::new("/")), "/");
        }
    }

    #[test]
    fn launch_paths_are_absolute() {
        let r = LaunchRequest::from_args(Some(Path::new("rel/dir")), &[PathBuf::from("sftp://host/x")]);
        let open = r.open.unwrap();
        assert!(open.is_absolute(), "{}", open.display());
        assert!(open.ends_with(Path::new("rel").join("dir")));
        assert_eq!(r.extra, vec![PathBuf::from("sftp://host/x")], "remote URLs stay as they are");
    }

    #[test]
    fn minimized_windows_take_launches_as_tabs() {
        assert_eq!(effective_mode(LaunchMode::NewWindow, false), LaunchMode::NewWindow);
        assert_eq!(effective_mode(LaunchMode::NewWindow, true), LaunchMode::NewTab);
        assert_eq!(effective_mode(LaunchMode::NewProcess, true), LaunchMode::NewTab);
        assert_eq!(effective_mode(LaunchMode::NewTab, true), LaunchMode::NewTab);
        assert_eq!(effective_mode(LaunchMode::NewTab, false), LaunchMode::NewTab);
    }

    #[test]
    fn wire_format() {
        let ev: Event = serde_json::from_str(r#"{"event":"queued","pid":7,"starting":true}"#).unwrap();
        assert_eq!(ev, Event::Queued { pid: 7, starting: true });
        let l: RequestLine = serde_json::from_str(r#"{"op":"launch","open":"/x"}"#).unwrap();
        assert_eq!(l.req.open, Some(PathBuf::from("/x")));
        assert!(l.req.extra.is_empty());
    }
}
