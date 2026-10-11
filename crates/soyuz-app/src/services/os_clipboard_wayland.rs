//! Wayland file clipboard through `ext-data-control-v1` (wlroots-style compositors,
//! niri, KWin, …) on a connection of our own, so it works without a focused surface and
//! independently of the toolkit's clipboard.
//!
//! * [`set`]: a data-control source offering every format is set as the selection and
//!   served by a background thread until the compositor cancels it (another client took
//!   the clipboard) or [`Owner::release`] is called. Each `send` is written on its own
//!   short-lived thread with a deadline, so a reader that never reads cannot pin it.
//! * [`read`]: a short-lived connection gets the current selection offer and receives
//!   the wanted types through pipes.
//! * [`clear`]: sets an empty selection.
//!
//! Every wait on the compositor is bounded (`prepare_read` + `poll` with a deadline), so a
//! hung compositor costs a timeout, never a stuck thread.
//!
//! Compositors without the protocol (GNOME's Mutter) make [`available`] false; the
//! caller then tries X11 (XWayland) or the configured fallback tools.

use std::collections::HashMap;
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use std::time::{Duration, Instant};
use wayland_client::backend::WaylandError;

use wayland_client::protocol::{wl_callback, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self as device, ExtDataControlDeviceV1 as Device},
    ext_data_control_manager_v1::ExtDataControlManagerV1 as Manager,
    ext_data_control_offer_v1::{self as offer, ExtDataControlOfferV1 as Offer},
    ext_data_control_source_v1::{self as source, ExtDataControlSourceV1 as Source},
};

use super::SelectionOwner;

/// How long a read waits for the selection owner to deliver one type.
const READ_TIMEOUT: Duration = Duration::from_millis(500);
/// How long one exchange with the compositor (connect + registry, a roundtrip) may take.
const SERVER_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long a `send` to another client may take before it is abandoned.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest clipboard body read (file lists are far smaller).
const MAX_READ: usize = 64 << 20;

#[derive(Default)]
struct State {
    /// MIME types announced per offer.
    offers: HashMap<wayland_client::backend::ObjectId, Vec<String>>,
    /// The current clipboard selection (`None` = empty).
    selection: Option<Offer>,
    /// A `selection` event arrived.
    got_selection: bool,
    /// The device is gone (seat removed).
    finished: bool,
    /// What a source serves.
    data: Arc<Vec<(String, Vec<u8>)>>,
    /// The source was replaced by another selection.
    cancelled: bool,
    /// Globals announced by the registry: `(name, interface, version)`.
    globals: Vec<(u32, String, u32)>,
    /// `wl_display.sync` callbacks answered so far.
    synced: u64,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => state.globals.push((name, interface, version)),
            wl_registry::Event::GlobalRemove { name } => state.globals.retain(|g| g.0 != name),
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.synced += 1;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<Manager, ()> for State {
    fn event(_: &mut Self, _: &Manager, _: <Manager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<Device, ()> for State {
    fn event(state: &mut Self, _: &Device, event: device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            device::Event::Selection { id } => {
                if let Some(old) = state.selection.take()
                    && id.as_ref().is_none_or(|n| n.id() != old.id())
                {
                    state.offers.remove(&old.id());
                    old.destroy();
                }
                state.selection = id;
                state.got_selection = true;
            }
            device::Event::PrimarySelection { id } => {
                // Not used: drop the offer unless it is also the clipboard selection.
                if let Some(o) = id
                    && state.selection.as_ref().is_none_or(|s| s.id() != o.id())
                {
                    state.offers.remove(&o.id());
                    o.destroy();
                }
            }
            device::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, Device, [
        device::EVT_DATA_OFFER_OPCODE => (Offer, ()),
    ]);
}

impl Dispatch<Offer, ()> for State {
    fn event(state: &mut Self, o: &Offer, event: offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let offer::Event::Offer { mime_type } = event {
            state.offers.entry(o.id()).or_default().push(mime_type);
        }
    }
}

impl Dispatch<Source, ()> for State {
    fn event(state: &mut Self, _: &Source, event: source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            source::Event::Send { mime_type, fd } => {
                let body = state.data.iter().find(|(t, _)| *t == mime_type).map(|(_, b)| b.clone());
                // Write on a helper thread: a reader that never reads must not stall the
                // source (and with it every later request); the write gives up at a
                // deadline, so the helper cannot block forever either.
                let _ = std::thread::Builder::new().name("soyuz-clip-send".into()).spawn(move || {
                    let mut f = std::fs::File::from(fd);
                    if let Some(b) = body
                        && let Err(e) = soyuz_fs::fdwait::write_all_until(&mut f, &b, Instant::now() + SEND_TIMEOUT)
                    {
                        log::debug!("clipboard: wayland send {mime_type}: {e}");
                    }
                });
            }
            source::Event::Cancelled => state.cancelled = true,
            _ => {}
        }
    }
}

/// A connection with the data-control manager, a seat and its data device.
struct Session {
    conn: Connection,
    queue: EventQueue<State>,
    manager: Manager,
    device: Device,
    state: State,
}

impl Session {
    /// Connect, bind the data-control manager and a seat and get the data device, all
    /// within [`SERVER_TIMEOUT`].
    fn open() -> Result<Session, String> {
        let deadline = Instant::now() + SERVER_TIMEOUT;
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
        let queue = conn.new_event_queue::<State>();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut s = SessionInit { conn, queue, state: State::default() };
        s.roundtrip(deadline)?;
        let find = |iface: &str| s.state.globals.iter().find(|g| g.1 == iface).map(|g| (g.0, g.2));
        let (m_name, _) = find(Manager::interface().name).ok_or("the compositor has no ext-data-control-v1")?;
        let (seat_name, _) = find(wl_seat::WlSeat::interface().name).ok_or("wayland: no seat")?;
        let manager: Manager = registry.bind(m_name, 1, &qh, ());
        let seat: wl_seat::WlSeat = registry.bind(seat_name, 1, &qh, ());
        let device = manager.get_data_device(&seat, &qh, ());
        s.roundtrip(deadline)?;
        let SessionInit { conn, queue, state } = s;
        Ok(Session { conn, queue, manager, device, state })
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        dispatch_roundtrip(&self.conn, &mut self.queue, &mut self.state, Instant::now() + SERVER_TIMEOUT)
    }

    /// Dispatch events until `done` holds, `deadline` passes (`Ok(false)`) or the
    /// connection fails.
    fn dispatch_until(&mut self, deadline: Instant, done: impl Fn(&State) -> bool) -> Result<bool, String> {
        dispatch_until(&self.conn, &mut self.queue, &mut self.state, deadline, done)
    }
}

/// The connection while the globals are being collected.
struct SessionInit {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
}

impl SessionInit {
    fn roundtrip(&mut self, deadline: Instant) -> Result<(), String> {
        dispatch_roundtrip(&self.conn, &mut self.queue, &mut self.state, deadline)
    }
}

/// `wl_display.sync` and wait for its callback, bounded by `deadline`.
fn dispatch_roundtrip(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    deadline: Instant,
) -> Result<(), String> {
    let want = state.synced + 1;
    conn.display().sync(&queue.handle(), ());
    if dispatch_until(conn, queue, state, deadline, |s| s.synced >= want)? {
        Ok(())
    } else {
        Err("wayland: the compositor did not answer".into())
    }
}

/// Dispatch until `done(state)`, `deadline` (`Ok(false)`) or an error.
fn dispatch_until(
    conn: &Connection,
    queue: &mut EventQueue<State>,
    state: &mut State,
    deadline: Instant,
    done: impl Fn(&State) -> bool,
) -> Result<bool, String> {
    let err = |e: &dyn std::fmt::Display| format!("wayland: {e}");
    loop {
        queue.dispatch_pending(state).map_err(|e| err(&e))?;
        if done(state) {
            return Ok(true);
        }
        match conn.flush() {
            Ok(()) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(err(&e)),
        }
        let Some(guard) = queue.prepare_read() else { continue };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        let ready = soyuz_fs::fdwait::poll(&[(guard.connection_fd(), false)], Some(left)).map_err(|e| err(&e))?;
        if !ready[0] {
            drop(guard);
            continue;
        }
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(err(&e)),
        }
    }
}

/// The compositor offers `ext-data-control-v1` (one roundtrip; call off the UI thread).
pub(super) fn available() -> bool {
    match Session::open() {
        Ok(_) => true,
        Err(e) => {
            log::info!("clipboard: {e}");
            false
        }
    }
}

/// Make `data` the clipboard selection, served by a background thread until another
/// client replaces it or the owner is released. Returns once the compositor accepted it.
/// A thread whose answer arrives after the caller gave up drops the selection again, so
/// no owner is left behind that the caller does not know about.
pub(super) fn set(data: Vec<(String, Vec<u8>)>) -> Result<SelectionOwner, String> {
    // Rendezvous channel: a successful send means the caller has the owner.
    let (tx, rx) = crossbeam_channel::bounded::<Result<SelectionOwner, String>>(0);
    std::thread::Builder::new()
        .name("soyuz-clipboard-wl".into())
        .spawn(move || {
            let mut s = match Session::open() {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let qh = s.queue.handle();
            s.state.data = Arc::new(data);
            let source = s.manager.create_data_source(&qh, ());
            for (mime, _) in s.state.data.iter() {
                source.offer(mime.clone());
            }
            s.device.set_selection(Some(&source));
            let owner = SelectionOwner::default();
            let ok = match s.roundtrip() {
                Ok(()) => {
                    owner.alive.store(!s.state.cancelled, Ordering::SeqCst);
                    tx.send(Ok(owner.clone())).is_ok()
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    false
                }
            };
            drop(tx);
            if !ok {
                log::debug!("clipboard: wayland source abandoned (the caller gave up)");
            }
            while ok && !s.state.cancelled && !s.state.finished && !owner.release.load(Ordering::SeqCst) {
                let deadline = Instant::now() + Duration::from_millis(250);
                if let Err(e) = s.dispatch_until(deadline, |st| st.cancelled || st.finished) {
                    log::debug!("clipboard: wayland source: {e}");
                    break;
                }
                // Offers for our own (or later) selections are not needed here.
                if let Some(o) = s.state.selection.take() {
                    s.state.offers.remove(&o.id());
                    o.destroy();
                }
            }
            owner.alive.store(false, Ordering::SeqCst);
            // Destroying the source unsets the selection only while it is still ours.
            source.destroy();
            s.device.destroy();
            let _ = s.conn.flush();
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(SERVER_TIMEOUT + Duration::from_millis(500))
        .map_err(|_| "wayland: the compositor did not answer".to_string())?
}

/// Run `f` on a session with the current selection's MIME types and a reader for them.
fn read_in<R>(
    s: &mut Session,
    f: impl FnOnce(&[String], &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> R,
) -> Result<(R, Option<Offer>), String> {
    if !s.state.got_selection {
        s.roundtrip()?;
    }
    let Some(sel) = s.state.selection.clone() else {
        let mut none = |_: &str| None;
        return Ok((f(&[], &mut none), None));
    };
    let types = s.state.offers.get(&sel.id()).cloned().unwrap_or_default();
    let conn = s.conn.clone();
    let mut receive = |mime: &str| -> Option<Vec<u8>> {
        let (mut reader, writer) = std::io::pipe().ok()?;
        sel.receive(mime.to_string(), writer.as_fd());
        conn.flush().ok()?;
        drop(writer);
        soyuz_fs::fdwait::read_to_end_until(&mut reader, Instant::now() + READ_TIMEOUT, MAX_READ).ok()
    };
    let r = f(&types, &mut receive);
    Ok((r, Some(sel)))
}

fn close(s: Session, sel: Option<Offer>) {
    if let Some(sel) = sel {
        sel.destroy();
    }
    s.device.destroy();
    let _ = s.conn.flush();
}

/// Run `f` with the current selection's MIME types and a reader for them.
pub(super) fn read<R>(f: impl FnOnce(&[String], &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> R) -> Result<R, String> {
    let mut s = Session::open()?;
    let (r, sel) = read_in(&mut s, f)?;
    close(s, sel);
    Ok(r)
}

/// Read the selection with `f` and empty the clipboard when `clear(&result)` holds and
/// the selection is still the one that was read (no other client replaced it meanwhile).
pub(super) fn read_and_clear_if<R>(
    f: impl FnOnce(&[String], &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> R,
    clear: impl FnOnce(&R) -> bool,
) -> Result<R, String> {
    let mut s = Session::open()?;
    let (r, sel) = read_in(&mut s, f)?;
    if let Some(read) = &sel
        && clear(&r)
    {
        // Pick up a selection change that raced with the read.
        s.roundtrip()?;
        if s.state.selection.as_ref().is_some_and(|now| now.id() == read.id()) {
            s.device.set_selection(None);
            s.roundtrip()?;
        }
    }
    close(s, sel);
    Ok(r)
}

/// Empty the clipboard.
pub(super) fn clear() -> Result<(), String> {
    let mut s = Session::open()?;
    s.device.set_selection(None);
    s.roundtrip()?;
    close(s, None);
    Ok(())
}
