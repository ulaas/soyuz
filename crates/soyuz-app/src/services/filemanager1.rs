//! "Show in folder" requests from other applications, and extra command-line paths.
//!
//! * Linux: while `[linux] own_file_manager1` is on (or Soyuz was started with
//!   `--dbus-file-manager` by the opt-in D-Bus activation file), the window owns
//!   `org.freedesktop.FileManager1` on the session bus and serves `ShowFolders`,
//!   `ShowItems` and `ShowItemProperties` (browsers' "show in folder", terminals,
//!   portals…). The name is requested with `DoNotQueue`, so an existing owner (Dolphin,
//!   Nautilus) is never replaced; the reason is reported by [`status`]. Switching the
//!   setting off releases the name. The bus connection runs on zbus' own executor thread;
//!   connecting, releasing and stat-ing the requested paths all happen off the UI thread.
//! * Every OS: paths after the first on the command line (`soyuz a b c`, desktop files'
//!   `%U`) open as extra tabs through the same queue ([`open_paths_later`]).
//!
//! Requests land in a queue that [`pump`] drains on the UI thread each frame.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use soyuz_core::{EntryInfo, Location};

use crate::core::AppCore;
use crate::host::Host;
use crate::services::mailbox::Wake;

/// A request for the UI thread.
#[derive(Debug)]
pub enum Request {
    /// Open each folder in a new tab, with the named entry under the cursor.
    Open { targets: Vec<(Location, Option<String>)>, focus: bool },
    /// Show the properties dialog for these items (their folder opens in a new tab).
    Properties { entries: Vec<EntryInfo> },
}

static QUEUE: Mutex<Vec<Request>> = Mutex::new(Vec::new());
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);
static FORCED: AtomicBool = AtomicBool::new(false);

/// `--dbus-file-manager`: own the bus name even when `[linux] own_file_manager1` is off.
pub fn set_forced(on: bool) {
    FORCED.store(on, Ordering::Relaxed);
}

/// Whether `--dbus-file-manager` was given.
pub fn forced() -> bool {
    FORCED.load(Ordering::Relaxed)
}

fn push(req: Request) {
    QUEUE.lock().push(req);
    if let Some(w) = WAKE.lock().clone() {
        w();
    }
}

/// Requests queued so far (drained).
fn take_requests() -> Vec<Request> {
    std::mem::take(&mut *QUEUE.lock())
}

/// Open `paths` as extra tabs once the window runs (resolved on a helper thread; missing
/// paths are logged and skipped).
pub fn open_paths_later(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new().name("soyuz-open-args".into()).spawn(move || {
        let targets: Vec<_> = paths.iter().filter_map(|p| crate::core::startup_target(p)).collect();
        if !targets.is_empty() {
            push(Request::Open { targets, focus: false });
        }
    });
}

/// `ShowFolders`: a folder opens itself; a file (some callers pass files) opens its
/// folder with the file under the cursor.
pub fn folder_targets(paths: &[PathBuf]) -> Vec<(Location, Option<String>)> {
    let mut out: Vec<(Location, Option<String>)> = Vec::new();
    for p in paths {
        let t = match std::fs::metadata(p) {
            Ok(m) if m.is_dir() => (Location::Dir(p.clone()), None),
            Ok(_) => match item_target(p) {
                Some(t) => t,
                None => continue,
            },
            Err(e) => {
                log::info!("FileManager1: {}: {e}", p.display());
                continue;
            }
        };
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn item_target(p: &Path) -> Option<(Location, Option<String>)> {
    let parent = p.parent().filter(|d| !d.as_os_str().is_empty())?;
    let name = p.file_name()?.to_string_lossy().into_owned();
    Some((Location::Dir(parent.to_path_buf()), Some(name)))
}

/// `ShowItems`: each item selected in its folder (folders too); one tab per folder, the
/// first item of that folder under the cursor.
pub fn item_targets(paths: &[PathBuf]) -> Vec<(Location, Option<String>)> {
    let mut out: Vec<(Location, Option<String>)> = Vec::new();
    for p in paths {
        if std::fs::symlink_metadata(p).is_err() {
            log::info!("FileManager1: {} does not exist", p.display());
            continue;
        }
        let Some((loc, name)) = item_target(p) else {
            // `/` has no parent: open it.
            out.push((Location::Dir(p.clone()), None));
            continue;
        };
        if !out.iter().any(|(l, _)| *l == loc) {
            out.push((loc, name));
        }
    }
    out
}

/// Decode the `file://` URIs of a FileManager1 call (other schemes are skipped).
pub fn uris_to_paths(uris: &[String]) -> Vec<PathBuf> {
    uris.iter()
        .filter_map(|u| {
            let p = soyuz_fs::launch::path_from_file_uri(u);
            if p.is_none() {
                log::info!("FileManager1: ignoring non-local URI {u}");
            }
            p
        })
        .collect()
}

/// Per-frame pump (UI thread): applies `[linux]` settings that live in background
/// services (FileManager1 ownership, clipboard fallback tools) and handles the queue.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    {
        let mut w = WAKE.lock();
        if w.is_none() {
            *w = Some(host.wake());
        }
    }
    crate::services::os_clipboard::set_fallback_tools(app.config.linux.clipboard_fallback_tools);
    #[cfg(target_os = "linux")]
    service::sync(app.config.linux.own_file_manager1 || forced());
    for req in take_requests() {
        handle(app, host, req);
    }
}

fn handle(app: &mut AppCore, host: &dyn Host, req: Request) {
    match req {
        Request::Open { targets, focus } => {
            // Command-line extras keep the first path's tab in front.
            let front = app.ws.tabs.get(app.ws.active).map(|t| t.id);
            for (loc, select) in targets {
                crate::ops::open_in_new_tab(app, loc, true);
                app.ws.pane_mut().pending_cursor = select;
            }
            if focus {
                raise(host);
            } else if let Some(i) = front.and_then(|id| app.ws.tab_index(id)) {
                app.ws.active = i;
            }
        }
        Request::Properties { entries } => {
            let Some(first) = entries.first() else { return };
            if let Some((loc, select)) = item_target(&first.path) {
                crate::ops::open_in_new_tab(app, loc, true);
                app.ws.pane_mut().pending_cursor = select;
            }
            let pane = app.ws.pane().id;
            app.frontend.push(crate::models::frontend::FrontendCmd::Properties {
                pane,
                entries,
                tab: crate::models::properties::Tab::General,
                start: None,
            });
            raise(host);
        }
    }
}

/// Bring the window to the front (un-minimize + focus). Wayland compositors may only
/// flash it: activation needs a token the window itself cannot take from the caller.
fn raise(host: &dyn Host) {
    host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Minimized(false));
    host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Focus);
}

/// State of the `org.freedesktop.FileManager1` service: `None` = off (or not Linux),
/// `Some(Ok(()))` = serving, `Some(Err(why))` = could not take the name (localized).
pub fn status() -> Option<Result<(), String>> {
    #[cfg(target_os = "linux")]
    {
        service::status()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
mod service {
    use std::time::Duration;

    use parking_lot::Mutex;
    use zbus::blocking::Connection;
    use zbus::fdo::{RequestNameFlags, RequestNameReply};

    use super::{Request, folder_targets, item_targets, push, uris_to_paths};

    pub(super) const NAME: &str = "org.freedesktop.FileManager1";
    const PATH: &str = "/org/freedesktop/FileManager1";

    /// The served interface.
    pub(super) struct Server;

    #[zbus::interface(name = "org.freedesktop.FileManager1")]
    impl Server {
        fn show_folders(&self, uris: Vec<String>, startup_id: String) {
            dispatch(Kind::Folders, uris, startup_id);
        }

        fn show_items(&self, uris: Vec<String>, startup_id: String) {
            dispatch(Kind::Items, uris, startup_id);
        }

        fn show_item_properties(&self, uris: Vec<String>, startup_id: String) {
            dispatch(Kind::Properties, uris, startup_id);
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Folders,
        Items,
        Properties,
    }

    /// Resolve the request on a helper thread (stat calls must not stall the bus
    /// executor) and queue it for the window.
    fn dispatch(kind: Kind, uris: Vec<String>, startup_id: String) {
        log::info!("FileManager1.{kind:?}: {} item(s)", uris.len());
        let _ = startup_id; // xdg-activation token of the caller: winit cannot activate with it.
        let _ = std::thread::Builder::new().name("soyuz-fm1".into()).spawn(move || {
            let paths = uris_to_paths(&uris);
            let req = match kind {
                Kind::Folders => Request::Open { targets: folder_targets(&paths), focus: true },
                Kind::Items => Request::Open { targets: item_targets(&paths), focus: true },
                Kind::Properties => {
                    let entries: Vec<_> = paths.iter().filter_map(|p| soyuz_fs::stat(p, false).ok()).collect();
                    if entries.is_empty() {
                        return;
                    }
                    Request::Properties { entries }
                }
            };
            if let Request::Open { targets, .. } = &req
                && targets.is_empty()
            {
                return;
            }
            push(req);
        });
    }

    /// Connect to the session bus, serve the interface and take `name` (never replacing
    /// or queueing behind another owner).
    pub(super) fn start(name: &str) -> Result<Connection, String> {
        let conn = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.method_timeout(Duration::from_secs(5)).serve_at(PATH, Server))
            .and_then(|b| b.build())
            .map_err(|e| tr!("services-fm1-failed", error = e.to_string()).to_string())?;
        let reply = conn
            .request_name_with_flags(name, RequestNameFlags::DoNotQueue.into())
            .map_err(|e| tr!("services-fm1-failed", error = e.to_string()).to_string())?;
        match reply {
            RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(conn),
            RequestNameReply::Exists | RequestNameReply::InQueue => {
                let owner = owner_name(&conn, name).unwrap_or_else(|| "?".into());
                Err(tr!("services-fm1-taken", owner = owner).to_string())
            }
        }
    }

    /// Process name of the current owner of `name` (`/proc/<pid>/comm`).
    fn owner_name(conn: &Connection, name: &str) -> Option<String> {
        let dbus = zbus::blocking::fdo::DBusProxy::new(conn).ok()?;
        let bus_name = zbus::names::BusName::try_from(name).ok()?;
        let pid = dbus.get_connection_unix_process_id(bus_name).ok()?;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        Some(comm.trim().to_string()).filter(|s| !s.is_empty())
    }

    #[derive(Default)]
    struct State {
        /// Ownership currently wanted (as last passed to [`sync`]).
        want: bool,
        conn: Option<Connection>,
        /// A connect attempt is running.
        starting: bool,
        /// The last attempt failed: no retry until the setting is toggled.
        failed: bool,
        status: Option<Result<(), String>>,
    }

    static STATE: Mutex<State> =
        Mutex::new(State { want: false, conn: None, starting: false, failed: false, status: None });

    pub(super) fn status() -> Option<Result<(), String>> {
        STATE.lock().status.clone()
    }

    /// Start or stop serving so ownership matches `want` (cheap when nothing changes).
    pub(super) fn sync(want: bool) {
        let mut st = STATE.lock();
        if want != st.want {
            st.want = want;
            st.failed = false;
            if !want {
                st.status = None;
                if let Some(conn) = st.conn.take() {
                    release(conn);
                }
            }
        }
        if !want || st.conn.is_some() || st.starting || st.failed {
            return;
        }
        st.starting = true;
        let spawned = std::thread::Builder::new().name("soyuz-fm1-start".into()).spawn(|| {
            let res = start(NAME);
            let mut st = STATE.lock();
            st.starting = false;
            match res {
                Ok(conn) if st.want => {
                    log::info!("FileManager1: serving {NAME}");
                    st.conn = Some(conn);
                    st.status = Some(Ok(()));
                }
                Ok(conn) => release(conn),
                Err(e) => {
                    log::warn!("FileManager1: {e}");
                    st.failed = true;
                    st.status = Some(Err(e));
                }
            }
        });
        if spawned.is_err() {
            st.starting = false;
            st.failed = true;
        }
    }

    /// Give the name back and close the connection (helper thread).
    fn release(conn: Connection) {
        let _ = std::thread::Builder::new().name("soyuz-fm1-stop".into()).spawn(move || {
            if let Err(e) = conn.release_name(NAME) {
                log::debug!("FileManager1: release: {e}");
            }
            log::info!("FileManager1: released {NAME}");
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Serve the interface under a private name and call it like a browser would.
        #[test]
        fn serves_show_items_and_folders() {
            if std::env::var_os("SOYUZ_DBUS_TESTS").is_none() {
                return;
            }
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("a b.txt");
            std::fs::write(&file, b"x").unwrap();
            let name = format!("dev.soyuz.Test.FileManager1.p{}", std::process::id());
            let server = start(&name).expect("serve on the session bus");
            // A second owner is refused, not queued.
            assert!(start(&name).is_err());
            let client = zbus::blocking::Connection::session().unwrap();
            let call = |method: &str, path: &std::path::Path| {
                client
                    .call_method(
                        Some(name.as_str()),
                        PATH,
                        Some(NAME),
                        method,
                        &(vec![soyuz_fs::launch::file_uri(path)], ""),
                    )
                    .unwrap();
            };
            call("ShowItems", &file);
            call("ShowFolders", dir.path());
            call("ShowItemProperties", &file);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut got = Vec::new();
            while got.len() < 3 && std::time::Instant::now() < deadline {
                got.extend(super::super::take_requests());
                std::thread::sleep(Duration::from_millis(20));
            }
            let dirloc = soyuz_core::Location::Dir(dir.path().to_path_buf());
            assert!(got.iter().any(|r| matches!(r, Request::Open { targets, focus: true }
                if targets == &vec![(dirloc.clone(), Some("a b.txt".to_string()))])));
            assert!(got.iter().any(|r| matches!(r, Request::Open { targets, .. }
                if targets == &vec![(dirloc.clone(), None)])));
            assert!(got.iter().any(|r| matches!(r, Request::Properties { entries } if entries[0].path == file)));
            server.release_name(name.as_str()).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_from_paths() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let f1 = sub.join("one");
        let f2 = sub.join("two");
        std::fs::write(&f1, b"1").unwrap();
        std::fs::write(&f2, b"2").unwrap();
        let missing = dir.path().join("missing");
        // Folders open themselves, files open their folder; duplicates collapse.
        let t = folder_targets(&[sub.clone(), f1.clone(), sub.clone(), missing.clone()]);
        assert_eq!(t, vec![(Location::Dir(sub.clone()), None), (Location::Dir(sub.clone()), Some("one".into()))]);
        // Items: one tab per folder, the first item of each under the cursor.
        let t = item_targets(&[f1.clone(), f2.clone(), sub.clone(), missing]);
        assert_eq!(
            t,
            vec![
                (Location::Dir(sub.clone()), Some("one".into())),
                (Location::Dir(dir.path().to_path_buf()), Some("sub".into())),
            ]
        );
    }

    #[test]
    fn uris_decode() {
        let uris = vec!["https://example.com/x".to_string(), soyuz_fs::launch::file_uri(Path::new("/tmp/a b"))];
        let paths = uris_to_paths(&uris);
        assert_eq!(paths.len(), 1);
        if cfg!(unix) {
            assert_eq!(paths[0], PathBuf::from("/tmp/a b"));
        }
    }
}
