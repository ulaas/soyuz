//! X11 file clipboard: the `CLIPBOARD` selection over a pure-Rust x11rb connection
//! (used in X11 sessions, and under Wayland compositors without data-control).
//!
//! * [`set`]: a hidden window owns `CLIPBOARD` on a background thread and answers
//!   `SelectionRequest`s (`TARGETS` plus every format) until `SelectionClear`.
//!   Payloads are file lists, far below the maximum request size, so `INCR` is never
//!   needed when serving (larger bodies are refused).
//! * [`read`]: `ConvertSelection` into a property of a hidden window, with `INCR`
//!   transfers handled, bounded by a timeout.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const READ_TIMEOUT: Duration = Duration::from_millis(500);

/// A connection and a hidden 1×1 input-only window.
struct Session {
    conn: RustConnection,
    win: Window,
}

impl Session {
    fn open(events: EventMask) -> Result<Session, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| format!("x11: {e}"))?;
        let root = conn.setup().roots.get(screen).ok_or("x11: no screen")?.root;
        let win = conn.generate_id().map_err(|e| format!("x11: {e}"))?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(events),
        )
        .map_err(|e| format!("x11: {e}"))?;
        Ok(Session { conn, win })
    }

    fn atom(&self, name: &str) -> Result<Atom, String> {
        self.conn
            .intern_atom(false, name.as_bytes())
            .map_err(|e| format!("x11: {e}"))?
            .reply()
            .map(|r| r.atom)
            .map_err(|e| format!("x11: {e}"))
    }

    fn atom_name(&self, atom: Atom) -> Option<String> {
        let r = self.conn.get_atom_name(atom).ok()?.reply().ok()?;
        String::from_utf8(r.name).ok()
    }
}

/// A selection owner: the session, `CLIPBOARD`, `TARGETS` and the target table.
type Owner = (Session, Atom, Atom, Vec<(Atom, Vec<u8>)>);

/// An X server is reachable.
pub(super) fn available() -> bool {
    match x11rb::connect(None) {
        Ok(_) => true,
        Err(e) => {
            log::info!("clipboard: x11: {e}");
            false
        }
    }
}

/// Own `CLIPBOARD` with `data`, served on a background thread until another client
/// takes the selection. The flag turns false then.
pub(super) fn set(data: Vec<(String, Vec<u8>)>) -> Result<Arc<AtomicBool>, String> {
    let (tx, rx) = crossbeam_channel::bounded::<Result<Arc<AtomicBool>, String>>(1);
    std::thread::Builder::new()
        .name("soyuz-clipboard-x11".into())
        .spawn(move || {
            let ready = (|| -> Result<Owner, String> {
                let s = Session::open(EventMask::NO_EVENT)?;
                let clipboard = s.atom("CLIPBOARD")?;
                let targets = s.atom("TARGETS")?;
                let mut table = Vec::with_capacity(data.len());
                for (name, body) in data {
                    table.push((s.atom(&name)?, body));
                }
                s.conn.set_selection_owner(s.win, clipboard, x11rb::CURRENT_TIME).map_err(|e| format!("x11: {e}"))?;
                let owner = s
                    .conn
                    .get_selection_owner(clipboard)
                    .map_err(|e| format!("x11: {e}"))?
                    .reply()
                    .map_err(|e| format!("x11: {e}"))?
                    .owner;
                if owner != s.win {
                    return Err("x11: could not own the clipboard".into());
                }
                Ok((s, clipboard, targets, table))
            })();
            let (s, clipboard, targets, table) = match ready {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            let alive = Arc::new(AtomicBool::new(true));
            let _ = tx.send(Ok(alive.clone()));
            let max = s.conn.maximum_request_bytes().saturating_sub(1024);
            loop {
                let ev = match s.conn.wait_for_event() {
                    Ok(ev) => ev,
                    Err(e) => {
                        log::debug!("clipboard: x11 owner: {e}");
                        break;
                    }
                };
                match ev {
                    Event::SelectionClear(e) if e.selection == clipboard => break,
                    Event::SelectionRequest(e) => {
                        // Obsolete clients pass no property: use the target as the name.
                        let mut prop = if e.property == x11rb::NONE { e.target } else { e.property };
                        let ok = if e.selection != clipboard {
                            false
                        } else if e.target == targets {
                            let mut atoms: Vec<u32> = table.iter().map(|(a, _)| *a).collect();
                            atoms.push(targets);
                            s.conn
                                .change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &atoms)
                                .is_ok()
                        } else if let Some((_, body)) = table.iter().find(|(a, _)| *a == e.target) {
                            body.len() <= max
                                && s.conn.change_property8(PropMode::REPLACE, e.requestor, prop, e.target, body).is_ok()
                        } else {
                            false
                        };
                        if !ok {
                            prop = x11rb::NONE;
                        }
                        let notify = SelectionNotifyEvent {
                            response_type: SELECTION_NOTIFY_EVENT,
                            sequence: 0,
                            time: e.time,
                            requestor: e.requestor,
                            selection: e.selection,
                            target: e.target,
                            property: prop,
                        };
                        let _ = s.conn.send_event(false, e.requestor, EventMask::NO_EVENT, notify);
                        let _ = s.conn.flush();
                    }
                    _ => {}
                }
            }
            alive.store(false, Ordering::SeqCst);
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(2)).map_err(|_| "x11: the server did not answer".to_string())?
}

/// Wait (bounded by `deadline`) for an event matching `want`.
fn wait_event(s: &Session, deadline: Instant, mut want: impl FnMut(&Event) -> bool) -> Option<Event> {
    loop {
        match s.conn.poll_for_event() {
            Ok(Some(ev)) if want(&ev) => return Some(ev),
            Ok(Some(_)) => continue,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => return None,
        }
    }
}

/// Convert `CLIPBOARD` to `target` and read the property (`INCR` included).
fn convert(s: &Session, clipboard: Atom, target: Atom, prop: Atom, incr: Atom) -> Option<Vec<u8>> {
    s.conn.convert_selection(s.win, clipboard, target, prop, x11rb::CURRENT_TIME).ok()?;
    s.conn.flush().ok()?;
    let deadline = Instant::now() + READ_TIMEOUT;
    let ev = wait_event(s, deadline, |e| matches!(e, Event::SelectionNotify(n) if n.requestor == s.win))?;
    let Event::SelectionNotify(n) = ev else { return None };
    if n.property == x11rb::NONE {
        return None;
    }
    let r = s.conn.get_property(true, s.win, prop, AtomEnum::ANY, 0, u32::MAX / 4).ok()?.reply().ok()?;
    if r.type_ != incr {
        return Some(r.value);
    }
    // INCR: the owner sends chunks, each announced by a PropertyNotify(NewValue); an
    // empty chunk ends the transfer.
    s.conn.flush().ok()?;
    let mut out = Vec::new();
    let deadline = Instant::now() + READ_TIMEOUT * 4;
    loop {
        wait_event(
            s,
            deadline,
            |e| matches!(e, Event::PropertyNotify(p) if p.window == s.win && p.atom == prop && p.state == Property::NEW_VALUE),
        )?;
        let chunk = s.conn.get_property(true, s.win, prop, AtomEnum::ANY, 0, u32::MAX / 4).ok()?.reply().ok()?;
        s.conn.flush().ok()?;
        if chunk.value.is_empty() {
            return Some(out);
        }
        out.extend_from_slice(&chunk.value);
    }
}

/// Run `f` with the `CLIPBOARD` targets and a reader for them.
pub(super) fn read<R>(f: impl FnOnce(&[String], &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> R) -> Result<R, String> {
    let s = Session::open(EventMask::PROPERTY_CHANGE)?;
    let clipboard = s.atom("CLIPBOARD")?;
    let targets_atom = s.atom("TARGETS")?;
    let incr = s.atom("INCR")?;
    let prop = s.atom("SOYUZ_CLIPBOARD")?;
    let owner = s.conn.get_selection_owner(clipboard).map_err(|e| format!("x11: {e}"))?.reply();
    if owner.is_ok_and(|o| o.owner == x11rb::NONE) {
        let mut none = |_: &str| None;
        return Ok(f(&[], &mut none));
    }
    let types: Vec<String> = convert(&s, clipboard, targets_atom, prop, incr)
        .map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).filter_map(|a| s.atom_name(a)).collect())
        .unwrap_or_default();
    let mut receive = |mime: &str| -> Option<Vec<u8>> {
        let target = s.atom(mime).ok()?;
        convert(&s, clipboard, target, prop, incr)
    };
    Ok(f(&types, &mut receive))
}

/// Give up `CLIPBOARD` (whoever owns it).
pub(super) fn clear() -> Result<(), String> {
    let s = Session::open(EventMask::NO_EVENT)?;
    let clipboard = s.atom("CLIPBOARD")?;
    s.conn.set_selection_owner(x11rb::NONE, clipboard, x11rb::CURRENT_TIME).map_err(|e| format!("x11: {e}"))?;
    s.conn.sync().map_err(|e| format!("x11: {e}"))?;
    Ok(())
}
