//! Shared D-Bus connections (session and system bus) for the Linux desktop integration:
//! `zbus::blocking` with a method timeout, connected on a helper thread so a stuck bus
//! can never hang the caller, cached, and retried at most every 30 s after a failure
//! (no bus in a container / CI). Meant for background threads.

use std::time::{Duration, Instant};

use zbus::blocking::Connection;

/// Default timeout for method calls on the cached connections.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(3);

/// How long a failed connection attempt is remembered.
const RETRY: Duration = Duration::from_secs(30);

enum Slot {
    Empty,
    Up(Connection),
    Failed(Instant),
}

static SESSION: parking_lot::Mutex<Slot> = parking_lot::Mutex::new(Slot::Empty);
static SYSTEM: parking_lot::Mutex<Slot> = parking_lot::Mutex::new(Slot::Empty);

/// Open a new connection with `timeout` for both the handshake and later calls.
pub(crate) fn connect(system: bool, timeout: Duration) -> Option<Connection> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("soyuz-dbus-connect".into())
        .spawn(move || {
            let b = if system {
                zbus::blocking::connection::Builder::system()
            } else {
                zbus::blocking::connection::Builder::session()
            };
            let c = b.and_then(|b| b.method_timeout(timeout).build());
            if let Err(e) = &c {
                log::debug!("dbus: {} bus unavailable: {e}", if system { "system" } else { "session" });
            }
            let _ = tx.send(c.ok());
        })
        .ok()?;
    rx.recv_timeout(timeout).ok().flatten()
}

fn cached(slot: &parking_lot::Mutex<Slot>, system: bool) -> Option<Connection> {
    let mut g = slot.lock();
    match &*g {
        Slot::Up(c) => return Some(c.clone()),
        Slot::Failed(at) if at.elapsed() < RETRY => return None,
        _ => {}
    }
    match connect(system, TIMEOUT) {
        Some(c) => {
            *g = Slot::Up(c.clone());
            Some(c)
        }
        None => {
            *g = Slot::Failed(Instant::now());
            None
        }
    }
}

/// The cached session-bus connection.
pub(crate) fn session() -> Option<Connection> {
    cached(&SESSION, false)
}

/// The cached system-bus connection.
pub(crate) fn system() -> Option<Connection> {
    cached(&SYSTEM, true)
}

const DBUS: &str = "org.freedesktop.DBus";

/// Call a method on the bus daemon itself.
fn daemon<B, R>(c: &Connection, method: &str, body: &B) -> Option<R>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    let m = c.call_method(Some(DBUS), "/org/freedesktop/DBus", Some(DBUS), method, body).ok()?;
    m.body().deserialize::<R>().ok()
}

/// `name` currently has an owner.
pub(crate) fn has_owner(c: &Connection, name: &str) -> bool {
    daemon::<_, bool>(c, "NameHasOwner", &(name,)).unwrap_or(false)
}

/// `name` can be started on demand (a `.service` file exists).
pub(crate) fn activatable(c: &Connection, name: &str) -> bool {
    daemon::<_, Vec<String>>(c, "ListActivatableNames", &()).is_some_and(|v| v.iter().any(|n| n == name))
}

/// Process id of `name`'s current owner.
pub(crate) fn owner_pid(c: &Connection, name: &str) -> Option<u32> {
    daemon::<_, u32>(c, "GetConnectionUnixProcessID", &(name,))
}
