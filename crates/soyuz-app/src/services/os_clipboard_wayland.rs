//! Wayland file clipboard through `ext-data-control-v1` (wlroots-style compositors,
//! niri, KWin, …) on a connection of our own, so it works without a focused surface and
//! independently of the toolkit's clipboard.
//!
//! * [`set`]: a data-control source offering every format is set as the selection and
//!   served by a background thread (each `send` is written on its own short-lived thread)
//!   until the compositor cancels it, i.e. until another client takes the clipboard.
//! * [`read`]: a short-lived connection gets the current selection offer and receives
//!   the wanted types through pipes.
//! * [`clear`]: sets an empty selection.
//!
//! Compositors without the protocol (GNOME's Mutter) make [`available`] false; the
//! caller then tries X11 (XWayland) or the configured fallback tools.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self as device, ExtDataControlDeviceV1 as Device},
    ext_data_control_manager_v1::ExtDataControlManagerV1 as Manager,
    ext_data_control_offer_v1::{self as offer, ExtDataControlOfferV1 as Offer},
    ext_data_control_source_v1::{self as source, ExtDataControlSourceV1 as Source},
};

/// How long a read waits for the selection owner to deliver one type.
const READ_TIMEOUT: Duration = Duration::from_millis(500);

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
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
                // source (and with it every later request).
                let _ = std::thread::Builder::new().name("soyuz-clip-send".into()).spawn(move || {
                    let mut f = std::fs::File::from(fd);
                    if let Some(b) = body {
                        let _ = f.write_all(&b);
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
    fn open() -> Result<Session, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| format!("wayland: {e}"))?;
        let qh = queue.handle();
        let manager: Manager =
            globals.bind(&qh, 1..=1, ()).map_err(|_| "the compositor has no ext-data-control-v1".to_string())?;
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|_| "wayland: no seat".to_string())?;
        let device = manager.get_data_device(&seat, &qh, ());
        let mut state = State::default();
        queue.roundtrip(&mut state).map_err(|e| format!("wayland: {e}"))?;
        Ok(Session { conn, queue, manager, device, state })
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.queue.roundtrip(&mut self.state).map(drop).map_err(|e| format!("wayland: {e}"))
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
/// client replaces it. Returns once the compositor accepted it, with a flag that turns
/// false when the selection is lost.
pub(super) fn set(data: Vec<(String, Vec<u8>)>) -> Result<Arc<AtomicBool>, String> {
    let (tx, rx) = crossbeam_channel::bounded::<Result<Arc<AtomicBool>, String>>(1);
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
            if let Err(e) = s.roundtrip() {
                let _ = tx.send(Err(e));
                return;
            }
            let alive = Arc::new(AtomicBool::new(!s.state.cancelled));
            let _ = tx.send(Ok(alive.clone()));
            while !s.state.cancelled && !s.state.finished {
                if let Err(e) = s.queue.blocking_dispatch(&mut s.state) {
                    log::debug!("clipboard: wayland source: {e}");
                    break;
                }
                // Offers for our own (or later) selections are not needed here.
                if let Some(o) = s.state.selection.take() {
                    s.state.offers.remove(&o.id());
                    o.destroy();
                }
            }
            alive.store(false, Ordering::SeqCst);
            source.destroy();
            s.device.destroy();
            let _ = s.conn.flush();
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(2)).map_err(|_| "wayland: the compositor did not answer".to_string())?
}

/// Run `f` with the current selection's MIME types and a reader for them.
pub(super) fn read<R>(f: impl FnOnce(&[String], &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> R) -> Result<R, String> {
    let mut s = Session::open()?;
    if !s.state.got_selection {
        s.roundtrip()?;
    }
    let Some(sel) = s.state.selection.clone() else {
        let mut none = |_: &str| None;
        return Ok(f(&[], &mut none));
    };
    let types = s.state.offers.get(&sel.id()).cloned().unwrap_or_default();
    let conn = s.conn.clone();
    let mut receive = |mime: &str| -> Option<Vec<u8>> {
        let (mut reader, writer) = std::io::pipe().ok()?;
        sel.receive(mime.to_string(), writer.as_fd());
        conn.flush().ok()?;
        drop(writer);
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("soyuz-clip-recv".into())
            .spawn(move || {
                let mut out = Vec::new();
                let _ = tx.send(reader.read_to_end(&mut out).ok().map(|_| out));
            })
            .ok()?;
        rx.recv_timeout(READ_TIMEOUT).ok().flatten()
    };
    let r = f(&types, &mut receive);
    sel.destroy();
    s.device.destroy();
    let _ = s.conn.flush();
    Ok(r)
}

/// Empty the clipboard.
pub(super) fn clear() -> Result<(), String> {
    let mut s = Session::open()?;
    s.device.set_selection(None);
    s.roundtrip()?;
    s.device.destroy();
    let _ = s.conn.flush();
    Ok(())
}
