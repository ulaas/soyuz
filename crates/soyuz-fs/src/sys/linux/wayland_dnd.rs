//! Wayland drag and drop on the window toolkit's own connection.
//!
//! winit 0.30 implements no `wl_data_device`, so files dragged from a Wayland file
//! manager never reach egui, and nothing can be dragged out. This module attaches to
//! winit's `wl_display` in guest mode (`Backend::from_foreign_display`: never closes the
//! connection), creates its **own** event queue, binds `wl_seat` +
//! `wl_data_device_manager`, gets a `wl_data_device` (and a `wl_pointer`, only to learn
//! the serial of the button press a drag-out needs) and dispatches that queue on a
//! background thread with the multi-queue read protocol (`prepare_read` → `poll` →
//! `read`), which libwayland coordinates with winit's reader. winit's queue is never
//! dispatched here. All setup happens on that thread too (no round trip on the UI thread).
//!
//! * **Drop target**: drags over the window's `wl_surface` (matched by proxy pointer) are
//!   accepted when the offer carries `text/uri-list` (or `x-special/gnome-copied-files`),
//!   with the copy action only (see `dragout::drop_actions`). On drop the list is
//!   received through a pipe on a short-lived thread, then the offer is finished.
//! * **Drag source** ([`start_drag`]): a `wl_data_source` offering `text/uri-list`,
//!   GNOME's list and plain text, started with `wl_data_device.start_drag` and the
//!   serial of the primary-button press seen by our own `wl_pointer` (the compositor
//!   sends the same serial to every pointer object of the seat). A drag that comes back
//!   over the window (our private marker type in the offer) stays a compositor drag, but
//!   is reported as `DropTargetEvent::Own*` so the app resumes its internal drag (a drop
//!   there is an internal drop).

use std::collections::HashMap;
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy as _, QueueHandle, WEnum};

use crate::dragout::{
    DND_ACTION_COPY, DND_ACTION_MOVE, DROPEFFECT_NONE, DragReport, DropSink, DropTargetEvent, MIME_GNOME_FILES,
    MIME_URI_LIST, Outcome, drop_actions, gnome_copied_files, parse_uri_list, pick_mime, plain_text, uri_list,
    wayland_drag_report,
};

/// The running thread: a wake pipe, its command channel and join handle.
struct Running {
    wake: OwnedFd,
    commands: Sender<Command>,
    thread: std::thread::JoinHandle<()>,
}

enum Command {
    StartDrag { paths: Vec<PathBuf>, report: Sender<DragReport> },
    CancelDrag,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
/// The primary button is held on our surface (per our own `wl_pointer`).
static BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
/// A drag can be started (the thread is up with a data device and a pointer).
static DRAG_READY: AtomicBool = AtomicBool::new(false);
/// The data device exists (the drop target works).
static TARGET_READY: AtomicBool = AtomicBool::new(false);

/// Largest file list read from a drop (a few hundred thousand paths).
const MAX_LIST_BYTES: usize = 64 * 1024 * 1024;
/// How long a drop source may take to send its list.
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);
const BTN_LEFT: u32 = 0x110;
const WAKE_STOP: u8 = 1;
const WAKE_COMMAND: u8 = 2;

/// Our private MIME type: tells our own drop target that a drag is ours.
fn marker_mime() -> String {
    format!("application/x-soyuz-drag-{}", std::process::id())
}

pub(crate) fn start(display: usize, surface: usize, sink: DropSink) -> io::Result<()> {
    if display == 0 || surface == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no wl_display / wl_surface"));
    }
    stop();
    // SAFETY: `display` is the live `wl_display*` of the window's connection (the caller's
    // contract: valid until `stop`). Guest mode never disconnects it, and `stop` releases
    // our objects and joins the thread before the caller lets the connection go.
    let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    // SAFETY: `surface` is the live `wl_surface*` of the same connection (same contract).
    // `from_ptr` checks the proxy's interface name; the id only borrows the proxy (winit
    // keeps managing it) and is used for requests while the window exists.
    let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface as *mut _) }
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "not a wl_surface"))?;
    let surface_proxy = WlSurface::from_id(&conn, surface_id).map_err(io::Error::other)?;
    let (wake_rx, wake_tx) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
    let (cmd_tx, cmd_rx) = unbounded();
    let thread = std::thread::Builder::new()
        .name("soyuz-wl-dnd".into())
        .spawn(move || {
            let mut queue: EventQueue<State> = conn.new_event_queue();
            let state = State {
                conn: conn.clone(),
                qh: queue.handle(),
                surface,
                surface_proxy,
                sink: Arc::new(sink),
                seat: None,
                manager: None,
                device: None,
                pointer: None,
                offers: HashMap::new(),
                current: None,
                pointer_on_us: false,
                press_serial: None,
                drag: None,
            };
            match setup(&mut queue, state) {
                Ok(state) => run(queue, state, wake_rx, cmd_rx),
                Err(e) => log::warn!("Wayland drag and drop unavailable: {e}"),
            }
            DRAG_READY.store(false, Ordering::Relaxed);
            TARGET_READY.store(false, Ordering::Relaxed);
            BUTTON_DOWN.store(false, Ordering::Relaxed);
        })
        .map_err(io::Error::other)?;
    *RUNNING.lock() = Some(Running { wake: wake_tx, commands: cmd_tx, thread });
    Ok(())
}

/// Bind the globals and create the data device (two round trips on our queue).
fn setup(queue: &mut EventQueue<State>, mut state: State) -> io::Result<State> {
    let _registry = state.conn.display().get_registry(&state.qh, ());
    queue.roundtrip(&mut state).map_err(io::Error::other)?;
    let (Some(seat), Some(manager)) = (&state.seat, &state.manager) else {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "compositor has no wl_seat / wl_data_device_manager"));
    };
    state.device = Some(manager.get_data_device(seat, &state.qh, ()));
    queue.roundtrip(&mut state).map_err(io::Error::other)?;
    log::info!(
        "Wayland drag and drop ready (wl_data_device_manager v{}, wl_seat v{}, pointer {})",
        state.manager.as_ref().map_or(0, |m| m.version()),
        state.seat.as_ref().map_or(0, |s| s.version()),
        state.pointer.is_some()
    );
    DRAG_READY.store(state.pointer.is_some(), Ordering::Relaxed);
    TARGET_READY.store(true, Ordering::Relaxed);
    Ok(state)
}

pub(crate) fn stop() {
    let Some(r) = RUNNING.lock().take() else { return };
    let _ = rustix::io::write(&r.wake, &[WAKE_STOP]);
    let _ = r.thread.join();
}

/// The thread is up and a drag-out can be started (a pointer exists).
pub(crate) fn drag_ready() -> bool {
    DRAG_READY.load(Ordering::Relaxed)
}

/// The drop target thread is running (the window is a Wayland window).
pub(crate) fn running() -> bool {
    RUNNING.lock().is_some()
}

/// The primary button is held on our window (our own `wl_pointer`'s view).
pub(crate) fn button_down() -> bool {
    BUTTON_DOWN.load(Ordering::Relaxed)
}

fn send(cmd: Command) -> io::Result<()> {
    let g = RUNNING.lock();
    let Some(r) = g.as_ref() else {
        return Err(io::Error::new(io::ErrorKind::NotConnected, "Wayland drag and drop is not running"));
    };
    r.commands.send(cmd).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Wayland drag thread gone"))?;
    rustix::io::write(&r.wake, &[WAKE_COMMAND])?;
    Ok(())
}

/// Start a drag-out of `paths` (the primary button must be held on our window). The
/// report arrives on the returned channel exactly once.
pub(crate) fn start_drag(paths: Vec<PathBuf>) -> io::Result<Receiver<DragReport>> {
    if !drag_ready() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "no Wayland pointer / data device"));
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    send(Command::StartDrag { paths, report: tx })?;
    Ok(rx)
}

pub(crate) fn cancel_drag() {
    let _ = send(Command::CancelDrag);
}

/// Dispatch loop: our queue only, until the wake pipe says stop or the connection fails.
fn run(mut queue: EventQueue<State>, mut state: State, wake: OwnedFd, commands: Receiver<Command>) {
    loop {
        if let Err(e) = queue.dispatch_pending(&mut state) {
            log::warn!("Wayland drag and drop stopped: {e}");
            return;
        }
        let _ = queue.flush();
        let Some(guard) = queue.prepare_read() else { continue };
        let (conn_ready, wake_ready) = match poll_in(&[guard.connection_fd(), wake.as_fd()], -1) {
            Ok(r) => (r[0], r[1]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => (false, false),
            Err(e) => {
                log::warn!("Wayland drag and drop poll: {e}");
                return;
            }
        };
        if wake_ready {
            drop(guard); // cancels the prepared read
            let mut buf = [0u8; 64];
            let n = rustix::io::read(&wake, &mut buf).unwrap_or(0);
            if n == 0 || buf[..n].contains(&WAKE_STOP) {
                state.shutdown();
                let _ = queue.flush();
                return;
            }
            while let Ok(cmd) = commands.try_recv() {
                state.command(cmd);
            }
            continue;
        }
        if conn_ready && let Err(e) = guard.read() {
            log::warn!("Wayland drag and drop stopped: {e}");
            return;
        }
    }
}

#[derive(Default)]
struct OfferInfo {
    mimes: Vec<String>,
    source_actions: u32,
}

/// The drag currently over our surface.
struct Current {
    offer: WlDataOffer,
    mime: Option<&'static str>,
    /// Our own drag-out, back over the window.
    own: bool,
    pos: (f64, f64),
}

/// Our drag-out in progress.
struct DragOut {
    source: WlDataSource,
    paths: Arc<Vec<PathBuf>>,
    report: Sender<DragReport>,
    /// Last `wl_data_source.action`.
    action: u32,
}

struct State {
    conn: Connection,
    qh: QueueHandle<State>,
    /// Our `wl_surface*`.
    surface: usize,
    surface_proxy: WlSurface,
    sink: Arc<DropSink>,
    seat: Option<WlSeat>,
    manager: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    pointer: Option<WlPointer>,
    /// Offers announced (`data_offer`) and not yet entered / set as selection.
    offers: HashMap<ObjectId, OfferInfo>,
    current: Option<Current>,
    pointer_on_us: bool,
    /// Serial of the primary-button press on our surface while it is held.
    press_serial: Option<u32>,
    drag: Option<DragOut>,
}

impl State {
    fn shutdown(&mut self) {
        self.finish_drag(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE));
        if let Some(c) = self.current.take() {
            c.offer.destroy();
        }
        if let Some(p) = self.pointer.take()
            && p.version() >= 3
        {
            p.release();
        }
        if let Some(d) = self.device.take()
            && d.version() >= 2
        {
            d.release();
        }
        if let Some(s) = self.seat.take()
            && s.version() >= 5
        {
            s.release();
        }
    }

    fn emit(&self, ev: DropTargetEvent) {
        (**self.sink)(ev);
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::CancelDrag => self.finish_drag(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE)),
            Command::StartDrag { paths, report } => {
                self.finish_drag(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE));
                let (Some(manager), Some(device), Some(serial)) = (&self.manager, &self.device, self.press_serial)
                else {
                    log::debug!("Wayland drag-out: no button press to start from");
                    let _ = report.send(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE));
                    return;
                };
                let source = manager.create_data_source(&self.qh, ());
                for m in [MIME_URI_LIST, MIME_GNOME_FILES, "text/plain;charset=utf-8", "UTF8_STRING", "text/plain"] {
                    source.offer(m.to_owned());
                }
                source.offer(marker_mime());
                if source.version() >= 3 {
                    source.set_actions(DndAction::from_bits_truncate(DND_ACTION_COPY | DND_ACTION_MOVE));
                }
                device.start_drag(Some(&source), &self.surface_proxy, None, serial);
                let _ = self.conn.flush();
                log::debug!("Wayland drag-out of {} item(s) started", paths.len());
                self.drag = Some(DragOut { source, paths: Arc::new(paths), report, action: 0 });
            }
        }
    }

    fn finish_drag(&mut self, report: DragReport) {
        if let Some(d) = self.drag.take() {
            d.source.destroy();
            let _ = self.conn.flush();
            let _ = d.report.send(report);
        }
    }

    fn enter(&mut self, serial: u32, surface_ptr: usize, x: f64, y: f64, offer: Option<WlDataOffer>) {
        if let Some(c) = self.current.take() {
            c.offer.destroy();
        }
        let Some(offer) = offer else { return };
        let info = self.offers.remove(&offer.id()).unwrap_or_default();
        let marker = marker_mime();
        let ours = info.mimes.contains(&marker);
        if surface_ptr != self.surface || (ours && self.drag.is_none()) {
            // Another surface of ours (a secondary viewport), or a stale drag of ours: not
            // a target.
            offer.accept(serial, None);
            offer.destroy();
            return;
        }
        if ours {
            // Our own drag-out came back over the window: it stays a compositor drag (the
            // button grab is the compositor's now), but the app treats it as its internal
            // drag again; a drop here is an internal drop.
            offer.accept(serial, Some(marker));
            if offer.version() >= 3 {
                let copy = DndAction::from_bits_truncate(DND_ACTION_COPY);
                offer.set_actions(copy, copy);
            }
            self.current = Some(Current { offer, mime: None, own: true, pos: (x, y) });
            self.emit(DropTargetEvent::OwnEnter { x, y });
            return;
        }
        let mime = pick_mime(&info.mimes);
        let actions = mime.and_then(|_| drop_actions(info.source_actions));
        offer.accept(serial, actions.and(mime).map(str::to_owned));
        if offer.version() >= 3 {
            let (all, preferred) = actions.unwrap_or((0, 0));
            offer.set_actions(DndAction::from_bits_truncate(all), DndAction::from_bits_truncate(preferred));
        }
        let mime = actions.and(mime);
        self.current = Some(Current { offer, mime, own: false, pos: (x, y) });
        if mime.is_some() {
            self.emit(DropTargetEvent::Enter { x, y });
        }
    }

    fn drop_here(&mut self) {
        let Some(c) = self.current.take() else { return };
        if c.own {
            let (x, y) = c.pos;
            self.emit(DropTargetEvent::OwnDrop { x, y });
            if c.offer.version() >= 3 {
                c.offer.finish();
            }
            c.offer.destroy();
            return;
        }
        let Some(mime) = c.mime else {
            c.offer.destroy();
            return;
        };
        let (x, y) = c.pos;
        self.emit(DropTargetEvent::Drop { x, y });
        let (rx, tx) = match rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("drop: pipe failed: {e}");
                c.offer.destroy();
                self.emit(DropTargetEvent::Files(Vec::new()));
                return;
            }
        };
        c.offer.receive(mime.to_owned(), tx.as_fd());
        drop(tx);
        let _ = self.conn.flush();
        let sink = Arc::clone(&self.sink);
        let conn = self.conn.clone();
        let offer = c.offer;
        let spawned = std::thread::Builder::new().name("soyuz-wl-drop".into()).spawn(move || {
            let paths = match read_list(rx) {
                Ok(text) => parse_uri_list(&text),
                Err(e) => {
                    log::warn!("drop: reading {mime} failed: {e}");
                    Vec::new()
                }
            };
            if offer.version() >= 3 && !paths.is_empty() {
                offer.finish();
            }
            offer.destroy();
            let _ = conn.flush();
            log::debug!("Wayland drop: {} path(s)", paths.len());
            (**sink)(DropTargetEvent::Files(paths));
        });
        if let Err(e) = spawned {
            log::warn!("drop: reader thread failed: {e}");
            self.emit(DropTargetEvent::Files(Vec::new()));
        }
    }

    /// `wl_data_source.send`: write the requested form of the list on a short thread.
    fn serve(&self, mime: &str, fd: OwnedFd) {
        let Some(d) = &self.drag else { return };
        let paths = Arc::clone(&d.paths);
        let cut = d.action & DND_ACTION_MOVE != 0;
        let body = match mime {
            MIME_URI_LIST => uri_list(&paths),
            MIME_GNOME_FILES => gnome_copied_files(&paths, cut),
            "text/plain;charset=utf-8" | "UTF8_STRING" | "text/plain" => plain_text(&paths),
            _ => return, // dropping `fd` closes it: the target reads nothing
        };
        let _ = std::thread::Builder::new().name("soyuz-wl-send".into()).spawn(move || {
            let mut f = std::fs::File::from(fd);
            if let Err(e) = f.write_all(body.as_bytes()) {
                log::debug!("drag-out: writing the list failed: {e}");
            }
        });
    }
}

/// Read the whole list from the source's end of the pipe (bounded in size and time).
fn read_list(fd: OwnedFd) -> io::Result<String> {
    let deadline = Instant::now() + RECEIVE_TIMEOUT;
    let mut file = std::fs::File::from(fd);
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "drop source did not send its data"));
        }
        let ms = left.as_millis().clamp(1, 60_000) as i32;
        match poll_in(&[file.as_fd()], ms) {
            Ok(r) if !r[0] => continue,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > MAX_LIST_BYTES {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "dropped file list too large"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// `poll(2)` for readability (or hang-up / error) on up to two descriptors; `timeout_ms`
/// < 0 waits forever. Returns which ones are ready.
pub(super) fn poll_in(fds: &[BorrowedFd<'_>], timeout_ms: i32) -> io::Result<[bool; 2]> {
    let mut p = [libc::pollfd { fd: -1, events: 0, revents: 0 }; 2];
    for (slot, fd) in p.iter_mut().zip(fds) {
        *slot = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    }
    let n = fds.len().min(2);
    // SAFETY: `p` holds `n` initialised `pollfd`s whose descriptors are borrowed (open) for
    // the duration of the call.
    let rc = unsafe { libc::poll(p.as_mut_ptr(), n as libc::nfds_t, timeout_ms) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok([p[0].revents != 0, p[1].revents != 0])
}

fn action_bits(a: WEnum<DndAction>) -> u32 {
    match a {
        WEnum::Value(v) => v.bits(),
        WEnum::Unknown(u) => u,
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ()));
                }
                "wl_data_device_manager" if state.manager.is_none() => {
                    state.manager = Some(registry.bind::<WlDataDeviceManager, _, _>(name, version.min(3), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event {
            let has = caps.contains(wl_seat::Capability::Pointer);
            if has && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            } else if !has && let Some(p) = state.pointer.take() {
                if p.version() >= 3 {
                    p.release();
                }
                state.press_serial = None;
                BUTTON_DOWN.store(false, Ordering::Relaxed);
            }
            if state.device.is_some() {
                DRAG_READY.store(state.pointer.is_some(), Ordering::Relaxed);
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(state: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => {
                state.pointer_on_us = surface.id().as_ptr() as usize == state.surface;
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_on_us = false;
                state.press_serial = None;
                BUTTON_DOWN.store(false, Ordering::Relaxed);
            }
            wl_pointer::Event::Button { serial, button: BTN_LEFT, state: WEnum::Value(s), .. } => {
                let pressed = s == wl_pointer::ButtonState::Pressed && state.pointer_on_us;
                state.press_serial = pressed.then_some(serial);
                BUTTON_DOWN.store(pressed, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if state.drag.as_ref().is_none_or(|d| d.source != *source) {
            return;
        }
        match event {
            wl_data_source::Event::Send { mime_type, fd } => state.serve(&mime_type, fd),
            wl_data_source::Event::Action { dnd_action } => {
                if let Some(d) = &mut state.drag {
                    d.action = action_bits(dnd_action);
                }
            }
            wl_data_source::Event::DndFinished => {
                let action = state.drag.as_ref().map_or(0, |d| d.action);
                state.finish_drag(wayland_drag_report(true, action));
            }
            wl_data_source::Event::Cancelled => {
                state.finish_drag(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), OfferInfo::default());
            }
            wl_data_device::Event::Enter { serial, surface, x, y, id } => {
                state.enter(serial, surface.id().as_ptr() as usize, x, y, id);
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(c) = &mut state.current {
                    c.pos = (x, y);
                    if c.own {
                        state.emit(DropTargetEvent::OwnMotion { x, y });
                    } else if c.mime.is_some() {
                        state.emit(DropTargetEvent::Motion { x, y });
                    }
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(c) = state.current.take() {
                    c.offer.destroy();
                    if c.own {
                        state.emit(DropTargetEvent::OwnLeave);
                    } else if c.mime.is_some() {
                        state.emit(DropTargetEvent::Leave);
                    }
                }
            }
            wl_data_device::Event::Drop => state.drop_here(),
            // The clipboard is handled elsewhere: just free the offer.
            wl_data_device::Event::Selection { id: Some(o) } => {
                state.offers.remove(&o.id());
                o.destroy();
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(info) = state.offers.get_mut(&offer.id()) else { return };
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if info.mimes.len() < 256 {
                    info.mimes.push(mime_type);
                }
            }
            wl_data_offer::Event::SourceActions { source_actions } => {
                info.source_actions = action_bits(source_actions);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayland_client::protocol::wl_compositor::WlCompositor;

    struct Probe {
        compositor: Option<WlCompositor>,
    }

    impl Dispatch<WlRegistry, ()> for Probe {
        fn event(
            state: &mut Self,
            registry: &WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global { name, interface, .. } = event
                && interface == "wl_compositor"
            {
                state.compositor = Some(registry.bind(name, 1, qh, ()));
            }
        }
    }
    wayland_client::delegate_noop!(Probe: ignore WlCompositor);
    wayland_client::delegate_noop!(Probe: ignore WlSurface);

    /// Needs a Wayland session; run with `SOYUZ_WAYLAND_TESTS=1`. A client of our own
    /// (standing in for winit) hands its display and a surface to the drop target, which
    /// binds the data device on its own queue and thread, then stops cleanly.
    #[test]
    fn binds_on_a_foreign_connection_and_stops() {
        if std::env::var_os("SOYUZ_WAYLAND_TESTS").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        let conn = Connection::connect_to_env().unwrap();
        let mut q = conn.new_event_queue::<Probe>();
        let _reg = conn.display().get_registry(&q.handle(), ());
        let mut probe = Probe { compositor: None };
        q.roundtrip(&mut probe).unwrap();
        let surface = probe.compositor.as_ref().unwrap().create_surface(&q.handle(), ());
        q.roundtrip(&mut probe).unwrap();
        let display = conn.backend().display_ptr() as usize;
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = Arc::clone(&events);
        start(display, surface.id().as_ptr() as usize, Box::new(move |e| ev.lock().push(e))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !TARGET_READY.load(Ordering::Relaxed) {
            assert!(Instant::now() < deadline, "data device not bound");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(running());
        assert!(!button_down());
        assert!(drag_ready(), "the seat has a pointer (drag-out possible)");
        // winit's side keeps working while our thread reads the shared socket.
        q.roundtrip(&mut probe).unwrap();
        stop();
        assert!(!running());
        assert!(!TARGET_READY.load(Ordering::Relaxed));
        assert!(events.lock().is_empty());
        q.roundtrip(&mut probe).unwrap();
    }
}
