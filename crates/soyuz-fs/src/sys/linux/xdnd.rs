//! X11 XDND drag source (protocol version 5, <https://freedesktop.org/wiki/Specifications/XDND/>).
//!
//! Runs on its own X connection and thread, so the UI never blocks. The drag starts from
//! an internal egui drag that left the window with the button held: the button's
//! implicit grab stays with the app window (winit keeps getting the motion and the
//! release), so instead of grabbing, the pointer is followed with `QueryPointer` (~120 Hz).
//! The window under it is found by descending from the root with `TranslateCoordinates`
//! until a window with `XdndAware` (version ≥ 3) turns up; `XdndProxy` is honoured.
//!
//! Data is served from an unmapped `InputOnly` window owning `XdndSelection`:
//! `text/uri-list`, `x-special/gnome-copied-files`, `UTF8_STRING` / `text/plain`, plus
//! `TARGETS` and `DELETE` (a target that performed a move asks the source to delete).
//! No `INCR` transfers: lists above the server's request size are refused.
//!
//! No `unsafe` here: x11rb's pure-Rust connection.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, bounded};
use parking_lot::Mutex;
use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, KeyButMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use crate::dragout::{
    DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, DragReport, Outcome, gnome_copied_files,
    plain_text, uri_list, xdnd_report,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        XdndAware,
        XdndProxy,
        XdndSelection,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndActionCopy,
        XdndActionMove,
        XdndActionLink,
        XdndTypeList,
        TARGETS,
        UTF8_STRING,
        DELETE,
        NULL,
        URI_LIST: b"text/uri-list",
        GNOME_FILES: b"x-special/gnome-copied-files",
        TEXT_PLAIN_UTF8: b"text/plain;charset=utf-8",
        TEXT_PLAIN: b"text/plain",
        SOYUZ_TIME: b"_SOYUZ_DND_TIME",
    }
}

const XDND_VERSION: u32 = 5;
const POLL: Duration = Duration::from_millis(8);
/// How long a target may take between the drop and `XdndFinished` (it may show a menu).
const FINISH_TIMEOUT: Duration = Duration::from_secs(60);

fn io_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("X11: {e}"))
}

/// Cached connection for [`primary_button_down`].
static POINTER_CONN: Mutex<Option<(RustConnection, Window)>> = Mutex::new(None);

/// Whether the primary button is held, per `QueryPointer` on `$DISPLAY`.
pub(crate) fn primary_button_down() -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    let mut g = POINTER_CONN.lock();
    if g.is_none() {
        match x11rb::connect(None) {
            Ok((c, screen)) => {
                let root = c.setup().roots[screen].root;
                *g = Some((c, root));
            }
            Err(e) => {
                log::debug!("X11 pointer query: {e}");
                return false;
            }
        }
    }
    let Some((c, root)) = g.as_ref() else { return false };
    match c.query_pointer(*root).map_err(io_err).and_then(|ck| ck.reply().map_err(io_err)) {
        Ok(r) => u16::from(r.mask) & u16::from(KeyButMask::BUTTON1) != 0,
        Err(e) => {
            log::debug!("X11 pointer query: {e}");
            *g = None;
            false
        }
    }
}

/// Start a drag on a background thread (connecting and taking `XdndSelection` happen
/// there too, so the UI thread does no X round trips): the report channel (one message)
/// and the cancel flag (see `dragout::DragSession`).
pub(crate) fn start(
    paths: Vec<PathBuf>,
    preferred: u32,
    own_window: u32,
) -> io::Result<(Receiver<DragReport>, Arc<AtomicBool>)> {
    let (tx, rx) = bounded(1);
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancel);
    std::thread::Builder::new()
        .name("soyuz-xdnd".into())
        .spawn(move || {
            let report =
                Source::new(paths, preferred, own_window).and_then(|mut src| src.run(&stop)).unwrap_or_else(|e| {
                    log::warn!("X11 drag failed: {e}");
                    DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE)
                });
            log::debug!("X11 drag ended: {report:?}");
            let _ = tx.send(report);
        })
        .map_err(io::Error::other)?;
    Ok((rx, cancel))
}

impl Source {
    /// Connect, create the `InputOnly` window and own `XdndSelection`.
    fn new(paths: Vec<PathBuf>, preferred: u32, own_window: u32) -> io::Result<Source> {
        let (conn, screen) = x11rb::connect(None).map_err(io_err)?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn).map_err(io_err)?.reply().map_err(io_err)?;
        let win = conn.generate_id().map_err(io_err)?;
        conn.create_window(
            0,
            win,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(io_err)?;
        let types = [atoms.URI_LIST, atoms.GNOME_FILES, atoms.UTF8_STRING, atoms.TEXT_PLAIN_UTF8, atoms.TEXT_PLAIN];
        conn.change_property32(PropMode::REPLACE, win, atoms.XdndTypeList, AtomEnum::ATOM, &types).map_err(io_err)?;
        let time = server_time(&conn, win, &atoms);
        conn.set_selection_owner(win, atoms.XdndSelection, time).map_err(io_err)?;
        let owner = conn.get_selection_owner(atoms.XdndSelection).map_err(io_err)?.reply().map_err(io_err)?.owner;
        if owner != win {
            return Err(io::Error::other("X11: could not own XdndSelection"));
        }
        conn.flush().map_err(io_err)?;
        Ok(Source {
            conn,
            atoms,
            root,
            win,
            own: own_window,
            types,
            paths,
            preferred,
            base_time: time,
            started: Instant::now(),
            target: None,
            delete_requested: false,
        })
    }
}

/// A server timestamp: touch a property on our window and read the `PropertyNotify`.
fn server_time(conn: &RustConnection, win: Window, atoms: &Atoms) -> u32 {
    let touched = conn
        .change_property8(PropMode::APPEND, win, atoms.SOYUZ_TIME, AtomEnum::STRING, &[])
        .and_then(|_| conn.flush());
    if touched.is_err() {
        return CURRENT_TIME;
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match conn.poll_for_event() {
            Ok(Some(Event::PropertyNotify(e))) if e.window == win => return e.time,
            Ok(Some(_)) => {}
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(_) => break,
        }
    }
    CURRENT_TIME
}

/// The `XdndAware` window under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    /// The window the messages are about.
    window: Window,
    /// Where they are sent (`XdndProxy`, else `window`).
    dest: Window,
    version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Nothing,
    Own,
    Aware(Target),
}

struct Current {
    t: Target,
    /// `XdndStatus`: accepted + action atom.
    accepted: bool,
    action: u32,
    waiting_status: bool,
    /// Position to send once the pending status arrives.
    last_sent: Option<(i16, i16, u32)>,
}

struct Source {
    conn: RustConnection,
    atoms: Atoms,
    root: Window,
    win: Window,
    own: u32,
    types: [u32; 5],
    paths: Vec<PathBuf>,
    preferred: u32,
    base_time: u32,
    started: Instant,
    target: Option<Current>,
    delete_requested: bool,
}

impl Source {
    fn now(&self) -> u32 {
        if self.base_time == CURRENT_TIME {
            return CURRENT_TIME;
        }
        self.base_time.wrapping_add(self.started.elapsed().as_millis() as u32)
    }

    fn action_atom(&self, effect: u32) -> u32 {
        match effect {
            DROPEFFECT_MOVE => self.atoms.XdndActionMove,
            DROPEFFECT_LINK => self.atoms.XdndActionLink,
            _ => self.atoms.XdndActionCopy,
        }
    }

    fn effect_of(&self, atom: u32) -> u32 {
        let a = &self.atoms;
        if atom == a.XdndActionMove {
            DROPEFFECT_MOVE
        } else if atom == a.XdndActionLink {
            DROPEFFECT_LINK
        } else if atom == a.XdndActionCopy {
            DROPEFFECT_COPY
        } else {
            DROPEFFECT_NONE
        }
    }

    fn send(&self, t: &Target, kind: u32, data: [u32; 5]) -> io::Result<()> {
        let ev = ClientMessageEvent::new(32, t.window, kind, data);
        self.conn.send_event(false, t.dest, EventMask::NO_EVENT, ev).map_err(io_err)?;
        Ok(())
    }

    fn leave(&mut self) -> io::Result<()> {
        if let Some(c) = self.target.take() {
            self.send(&c.t, self.atoms.XdndLeave, [self.win, 0, 0, 0, 0])?;
        }
        Ok(())
    }

    fn property32(&self, w: Window, prop: u32) -> io::Result<Option<u32>> {
        let r = self.conn.get_property(false, w, prop, AtomEnum::ANY, 0, 1).map_err(io_err)?.reply().map_err(io_err)?;
        Ok(r.value32().and_then(|mut v| v.next()))
    }

    fn find(&self, x: i16, y: i16) -> io::Result<Hit> {
        let mut w = self.root;
        for _ in 0..32 {
            let r = self.conn.translate_coordinates(self.root, w, x, y).map_err(io_err)?.reply().map_err(io_err)?;
            if r.child == NONE {
                return Ok(Hit::Nothing);
            }
            w = r.child;
            if w == self.own {
                return Ok(Hit::Own);
            }
            if let Some(v) = self.property32(w, self.atoms.XdndAware)? {
                if v < 3 {
                    return Ok(Hit::Nothing);
                }
                // XdndProxy is only valid when the proxy points at itself.
                let dest = match self.property32(w, self.atoms.XdndProxy)? {
                    Some(p) if p != NONE && self.property32(p, self.atoms.XdndProxy).ok().flatten() == Some(p) => p,
                    _ => w,
                };
                return Ok(Hit::Aware(Target { window: w, dest, version: v.min(XDND_VERSION) }));
            }
        }
        Ok(Hit::Nothing)
    }

    /// The effect for the current modifiers (Ctrl copy, Shift move, both link).
    fn wanted(&self, mask: u16) -> u32 {
        let ctrl = mask & u16::from(KeyButMask::CONTROL) != 0;
        let shift = mask & u16::from(KeyButMask::SHIFT) != 0;
        match (ctrl, shift) {
            (true, true) => DROPEFFECT_LINK,
            (true, false) => DROPEFFECT_COPY,
            (false, true) => DROPEFFECT_MOVE,
            (false, false) => self.preferred,
        }
    }

    fn run(&mut self, cancel: &AtomicBool) -> io::Result<DragReport> {
        // Only a *return* counts: the pointer must have been seen off the window first
        // (rounding at the window edge must not bounce the drag back and forth).
        let mut left_own = false;
        loop {
            self.pump(&mut None)?;
            if cancel.load(Ordering::Relaxed) {
                self.leave()?;
                self.conn.flush().map_err(io_err)?;
                return Ok(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE));
            }
            let p = self.conn.query_pointer(self.root).map_err(io_err)?.reply().map_err(io_err)?;
            let mask = u16::from(p.mask);
            let down = mask & u16::from(KeyButMask::BUTTON1) != 0;
            let hit = self.find(p.root_x, p.root_y)?;
            match hit {
                Hit::Own if !left_own => {}
                Hit::Own => {
                    self.leave()?;
                    self.conn.flush().map_err(io_err)?;
                    return Ok(DragReport::plain(Outcome::Returned, DROPEFFECT_NONE));
                }
                Hit::Nothing => self.leave()?,
                Hit::Aware(t) => {
                    if self.target.as_ref().map(|c| c.t) != Some(t) {
                        self.leave()?;
                        let more = u32::from(self.types.len() > 3);
                        let data = [self.win, (t.version << 24) | more, self.types[0], self.types[1], self.types[2]];
                        self.send(&t, self.atoms.XdndEnter, data)?;
                        self.target =
                            Some(Current { t, accepted: false, action: 0, waiting_status: false, last_sent: None });
                    }
                }
            }
            left_own |= hit != Hit::Own;
            if !down {
                return self.release();
            }
            let action = self.action_atom(self.wanted(mask));
            let now = self.now();
            if let Some(c) = &self.target {
                let pos = (p.root_x, p.root_y, action);
                if !c.waiting_status && c.last_sent != Some(pos) {
                    let xy = ((p.root_x as u16 as u32) << 16) | (p.root_y as u16 as u32);
                    self.send(&c.t, self.atoms.XdndPosition, [self.win, 0, xy, now, action])?;
                    if let Some(c) = &mut self.target {
                        c.waiting_status = true;
                        c.last_sent = Some(pos);
                    }
                }
            }
            self.conn.flush().map_err(io_err)?;
            std::thread::sleep(POLL);
        }
    }

    /// The button went up: drop on an accepting target, else cancel.
    fn release(&mut self) -> io::Result<DragReport> {
        let accepted = self.target.as_ref().is_some_and(|c| c.accepted);
        if !accepted {
            self.leave()?;
            self.conn.flush().map_err(io_err)?;
            return Ok(DragReport::plain(Outcome::Cancelled, DROPEFFECT_NONE));
        }
        let Some(c) = self.target.take() else { unreachable!() };
        self.send(&c.t, self.atoms.XdndDrop, [self.win, 0, self.now(), 0, 0])?;
        self.conn.flush().map_err(io_err)?;
        let status_effect = self.effect_of(c.action);
        let deadline = Instant::now() + FINISH_TIMEOUT;
        let mut finished: Option<(bool, u32)> = None;
        while Instant::now() < deadline {
            let mut fin = Some(c.t.window);
            if let Some(f) = self.pump(&mut fin)? {
                let (ok, atom) = f;
                finished = Some(if c.t.version >= 5 { (ok, self.effect_of(atom)) } else { (true, status_effect) });
                break;
            }
            std::thread::sleep(POLL);
        }
        if finished.is_none() {
            log::warn!("X11 drag: no XdndFinished from window {:#x}", c.t.window);
        }
        Ok(xdnd_report(finished, self.delete_requested))
    }

    /// Handle pending events. `finished_from`: the window whose `XdndFinished` ends the
    /// drop (returned as `(accepted, action atom)`).
    fn pump(&mut self, finished_from: &mut Option<Window>) -> io::Result<Option<(bool, u32)>> {
        while let Some(ev) = self.conn.poll_for_event().map_err(io_err)? {
            match ev {
                Event::ClientMessage(m) if m.format == 32 => {
                    let d = m.data.as_data32();
                    if m.type_ == self.atoms.XdndStatus
                        && let Some(c) = &mut self.target
                        && d[0] == c.t.window
                    {
                        c.accepted = d[1] & 1 != 0;
                        c.action = if c.accepted { d[4] } else { 0 };
                        c.waiting_status = false;
                    } else if m.type_ == self.atoms.XdndFinished && Some(d[0]) == *finished_from {
                        return Ok(Some((d[1] & 1 != 0, d[2])));
                    }
                }
                Event::SelectionRequest(r) => self.serve(&r)?,
                // Someone else took XdndSelection: the drop is over for us.
                Event::SelectionClear(_) if finished_from.is_some() => return Ok(Some((false, NONE))),
                _ => {}
            }
        }
        Ok(None)
    }

    fn serve(&mut self, r: &SelectionRequestEvent) -> io::Result<()> {
        let a = self.atoms;
        let prop = if r.property == NONE { r.target } else { r.property };
        let max = self.conn.maximum_request_bytes().saturating_sub(64);
        let cut = self.target.as_ref().is_some_and(|c| c.action == a.XdndActionMove);
        let ok = if r.selection != a.XdndSelection {
            false
        } else if r.target == a.TARGETS {
            let mut t = self.types.to_vec();
            t.extend([a.TARGETS, a.DELETE]);
            self.conn.change_property32(PropMode::REPLACE, r.requestor, prop, AtomEnum::ATOM, &t).map_err(io_err)?;
            true
        } else if r.target == a.DELETE {
            self.delete_requested = true;
            self.conn.change_property8(PropMode::REPLACE, r.requestor, prop, a.NULL, &[]).map_err(io_err)?;
            true
        } else {
            let body = if r.target == a.URI_LIST {
                Some(uri_list(&self.paths))
            } else if r.target == a.GNOME_FILES {
                Some(gnome_copied_files(&self.paths, cut))
            } else if [a.UTF8_STRING, a.TEXT_PLAIN_UTF8, a.TEXT_PLAIN, u32::from(AtomEnum::STRING)].contains(&r.target)
            {
                Some(plain_text(&self.paths))
            } else {
                None
            };
            match body {
                Some(b) if b.len() <= max => {
                    self.conn
                        .change_property8(PropMode::REPLACE, r.requestor, prop, r.target, b.as_bytes())
                        .map_err(io_err)?;
                    true
                }
                Some(_) => {
                    log::warn!("X11 drag: file list too large for one transfer");
                    false
                }
                None => false,
            }
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: r.time,
            requestor: r.requestor,
            selection: r.selection,
            target: r.target,
            property: if ok { prop } else { NONE },
        };
        self.conn.send_event(false, r.requestor, EventMask::NO_EVENT, notify).map_err(io_err)?;
        self.conn.flush().map_err(io_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x11_tests() -> bool {
        std::env::var_os("SOYUZ_X11_TESTS").is_some() && std::env::var_os("DISPLAY").is_some()
    }

    /// Needs an X server; run with `SOYUZ_X11_TESTS=1`. A drag started while the button is
    /// up ends at once (cancelled: no target accepted anything).
    #[test]
    fn drag_without_button_ends_cancelled() {
        if !x11_tests() {
            return;
        }
        let (rx, cancel) = start(vec![PathBuf::from("/tmp/a b")], DROPEFFECT_COPY, 0).unwrap();
        cancel.store(true, Ordering::Relaxed);
        let r = rx.recv_timeout(Duration::from_secs(5)).expect("drag thread reports");
        assert_eq!(r.outcome, Outcome::Cancelled);
    }

    /// Needs an X server; run with `SOYUZ_X11_TESTS=1`. A second client converts
    /// `XdndSelection` like a drop target does and reads every offered form back.
    #[test]
    fn selection_serves_the_file_list() {
        if !x11_tests() {
            return;
        }
        let paths = vec![PathBuf::from("/tmp/a b/c.txt"), PathBuf::from("/tmp/ü")];
        let mut src = Source::new(paths.clone(), DROPEFFECT_COPY, 0).unwrap();
        let (c2, screen) = x11rb::connect(None).unwrap();
        let root = c2.setup().roots[screen].root;
        let a = Atoms::new(&c2).unwrap().reply().unwrap();
        let w2 = c2.generate_id().unwrap();
        c2.create_window(0, w2, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new()).unwrap();
        let prop = c2.intern_atom(false, b"_SOYUZ_TEST_PROP").unwrap().reply().unwrap().atom;
        let fetch = |src: &mut Source, target: u32| -> Option<(u32, Vec<u8>)> {
            c2.convert_selection(w2, a.XdndSelection, target, prop, CURRENT_TIME).unwrap();
            c2.flush().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                src.pump(&mut None).unwrap();
                if let Some(Event::SelectionNotify(n)) = c2.poll_for_event().unwrap() {
                    if n.property == NONE {
                        return None;
                    }
                    let r = c2.get_property(true, w2, prop, AtomEnum::ANY, 0, 1 << 20).unwrap().reply().unwrap();
                    return Some((r.type_, r.value));
                }
                assert!(Instant::now() < deadline, "no SelectionNotify");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        let (ty, uris) = fetch(&mut src, a.URI_LIST).unwrap();
        assert_eq!(ty, a.URI_LIST);
        assert_eq!(crate::dragout::parse_uri_list(&String::from_utf8(uris).unwrap()), paths);
        let (_, gnome) = fetch(&mut src, a.GNOME_FILES).unwrap();
        assert!(gnome.starts_with(b"copy\nfile:///tmp/a%20b/c.txt"));
        let (_, text) = fetch(&mut src, a.UTF8_STRING).unwrap();
        assert_eq!(text, "/tmp/a b/c.txt\n/tmp/ü".as_bytes());
        let (ty, targets) = fetch(&mut src, a.TARGETS).unwrap();
        assert_eq!(ty, u32::from(AtomEnum::ATOM));
        assert_eq!(targets.len(), 4 * 7);
        assert!(fetch(&mut src, a.XdndAware).is_none(), "unknown targets are refused");
        assert!(!src.delete_requested);
        let (ty, _) = fetch(&mut src, a.DELETE).unwrap();
        assert_eq!(ty, a.NULL);
        assert!(src.delete_requested);
    }
}
