//! Remote locations on the app side: saved connections (sidebar "Network"),
//! credentials (OS credential store + per-session secrets), the `soyuz-vfs` hooks
//! (credentials, host-key confirmation through a UI prompt queue), cached remote
//! listings for panes / dropdowns, `\\server` share enumeration, and a size-capped temp
//! cache of downloaded remote files for open / preview.
//!
//! All network I/O runs on lister / worker threads; the UI thread only reads caches and
//! queues.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{Sender, bounded};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use soyuz_core::config::RemoteSection;
use soyuz_core::{EntryInfo, FsError, FsErrorKind, FsResult, Location};
use soyuz_vfs::registry::{self, ConnState, Credentials, Hooks, HostKeyPrompt, NetSettings};
use soyuz_vfs::{RemoteUri, Scheme};

use super::mailbox::{Postman, UiMsg, Wake};
use crate::ui::toasts::Toast;

/// How a saved connection signs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    #[default]
    Password,
    /// SFTP private key file (the stored secret is its passphrase, if any).
    KeyFile,
    /// SFTP via the SSH agent.
    Agent,
    /// No credentials (anonymous FTP, public WebDAV / S3).
    Anonymous,
}

/// A connection saved in the sidebar's Network section (`remote_connections.json`).
/// The password / passphrase / secret key is never in this file: it lives in the OS
/// credential store under `soyuz:ai:remote-<id>` (the shared Soyuz credential helper).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedConn {
    pub id: String,
    pub name: String,
    /// Normalized URI (no password).
    pub uri: String,
    #[serde(default)]
    pub auth: AuthKind,
    #[serde(default)]
    pub key_file: Option<PathBuf>,
    /// A secret is stored in the OS credential store.
    #[serde(default)]
    pub has_secret: bool,
}

impl SavedConn {
    pub fn parsed(&self) -> Option<RemoteUri> {
        RemoteUri::parse(&self.uri).ok()
    }

    fn credential_name(&self) -> String {
        format!("remote-{}", self.id)
    }
}

const STORE_FILE: &str = "remote_connections.json";
const CACHE_DIR: &str = "soyuz-remote-cache";

struct State {
    conns: RwLock<Vec<SavedConn>>,
    /// Session-only connections (debug test servers), shown in the sidebar.
    temp_conns: RwLock<Vec<SavedConn>>,
    /// conn_key → secret typed this session (not saved, or saved and already loaded).
    secrets: Mutex<HashMap<String, String>>,
    /// conn_key → auth chosen in the dialog for an unsaved connection.
    auth: Mutex<HashMap<String, (AuthKind, Option<PathBuf>)>>,
    /// URI → (fetched at, entries).
    cache: Mutex<HashMap<String, (Instant, Vec<EntryInfo>)>>,
    host_prompts: Mutex<VecDeque<(HostKeyPrompt, Sender<bool>)>>,
    /// URIs whose listing failed to sign in (the UI offers the connect dialog).
    sign_in: Mutex<Vec<String>>,
    /// Downloads for open / preview in flight (URI → started).
    fetching: Mutex<HashMap<String, Instant>>,
    cfg: RwLock<RemoteSection>,
    wake: Mutex<Option<Wake>>,
    /// Warnings from background credential-store work, shown as toasts by the UI pump.
    notices: Mutex<Vec<(String, String)>>,
}

fn st() -> &'static State {
    static S: OnceLock<State> = OnceLock::new();
    S.get_or_init(|| State {
        conns: RwLock::new(load_store(&store_path())),
        temp_conns: RwLock::new(Vec::new()),
        secrets: Mutex::new(HashMap::new()),
        auth: Mutex::new(HashMap::new()),
        cache: Mutex::new(HashMap::new()),
        host_prompts: Mutex::new(VecDeque::new()),
        sign_in: Mutex::new(Vec::new()),
        fetching: Mutex::new(HashMap::new()),
        cfg: RwLock::new(RemoteSection::default()),
        wake: Mutex::new(None),
        notices: Mutex::new(Vec::new()),
    })
}

fn store_path() -> PathBuf {
    soyuz_core::config::config_dir().join(STORE_FILE)
}

fn load_store(p: &Path) -> Vec<SavedConn> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_store(conns: &[SavedConn]) {
    let p = store_path();
    let text = serde_json::to_vec_pretty(conns).unwrap_or_default();
    if let Err(e) = soyuz_fs::write_atomic(&p, &text) {
        log::warn!("cannot save {}: {e}", p.display());
    }
}

fn wake() {
    if let Some(w) = st().wake.lock().as_ref() {
        w();
    }
}

// ---- setup ----------------------------------------------------------------------------

struct AppHooks;

impl Hooks for AppHooks {
    fn credentials(&self, uri: &RemoteUri) -> Credentials {
        credentials_for(uri)
    }

    fn confirm_host_key(&self, prompt: &HostKeyPrompt) -> bool {
        let (tx, rx) = bounded(1);
        st().host_prompts.lock().push_back((prompt.clone(), tx));
        wake();
        // The user decides; give up (reject) after a while so a worker never hangs forever.
        rx.recv_timeout(Duration::from_secs(600)).unwrap_or(false)
    }
}

/// Install the vfs hooks and apply `[remote]` (idempotent; call every frame cheaply).
pub fn init(cfg: &RemoteSection, wake_fn: Wake) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        *st().wake.lock() = Some(wake_fn);
        registry::set_hooks(Arc::new(AppHooks));
        registry::set_known_hosts(&soyuz_core::config::config_dir().join("known_hosts"));
        debug_servers();
    });
    if *st().cfg.read() != *cfg {
        *st().cfg.write() = cfg.clone();
        registry::set_net_settings(NetSettings {
            connect_timeout: Duration::from_secs(cfg.connect_timeout_secs.max(1)),
            io_timeout: Duration::from_secs(cfg.io_timeout_secs.max(5)),
        });
    }
}

/// `SOYUZ_FAKE_REMOTE=1`: an in-memory test server `fake://demo/`;
/// `SOYUZ_TEST_DAV=<folder>`: a local WebDAV server over that folder (user `soyuz`,
/// password `soyuz`; port `SOYUZ_TEST_DAV_PORT`, random by default). Both appear in the
/// sidebar for this session only.
fn debug_servers() {
    if std::env::var_os("SOYUZ_FAKE_REMOTE").is_some() {
        let fake = soyuz_vfs::fake::FakeProvider::new("demo");
        fake.put("/Documents/Quarterly report.txt", b"Revenue grew 12% quarter over quarter.\n");
        fake.put("/Documents/Notes/todo.md", b"# Todo\n\n- Ship remote locations\n- Write docs\n");
        fake.put("/Photos/README.txt", b"Holiday photos go here.\n");
        fake.put("/backup.tar", &vec![7u8; 3 * 1024 * 1024]);
        let uri = fake.uri();
        fake.install();
        st().temp_conns.write().push(SavedConn {
            id: "debug-fake".into(),
            name: "Test server (memory)".into(),
            uri: uri.display(),
            auth: AuthKind::Anonymous,
            key_file: None,
            has_secret: false,
        });
    }
    if let Some(dir) = std::env::var_os("SOYUZ_TEST_DAV") {
        static SERVER: OnceLock<soyuz_vfs::dav_server::DavServer> = OnceLock::new();
        let port = std::env::var("SOYUZ_TEST_DAV_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
        match soyuz_vfs::dav_server::DavServer::start_on(Path::new(&dir), Some(("soyuz".into(), "soyuz".into())), port)
        {
            Ok(s) => {
                let uri = format!("webdav://soyuz@127.0.0.1:{}/", s.port);
                if let Ok(u) = RemoteUri::parse(&uri) {
                    st().secrets.lock().insert(u.conn_key(), "soyuz".into());
                }
                st().temp_conns.write().push(SavedConn {
                    id: "debug-dav".into(),
                    name: "Test WebDAV (local)".into(),
                    uri,
                    auth: AuthKind::Password,
                    key_file: None,
                    has_secret: false,
                });
                let _ = SERVER.set(s);
            }
            Err(e) => log::warn!("SOYUZ_TEST_DAV: cannot start the WebDAV test server: {e}"),
        }
    }
}

// ---- saved connections & credentials ----------------------------------------------------

/// Saved connections followed by this session's debug servers.
pub fn connections() -> Vec<SavedConn> {
    let mut v = st().conns.read().clone();
    v.extend(st().temp_conns.read().iter().cloned());
    v
}

pub fn is_saved(id: &str) -> bool {
    st().conns.read().iter().any(|c| c.id == id)
}

fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    format!("{:012x}", h.finish() & 0xFFFF_FFFF_FFFF)
}

/// Save (or update, by id) a connection; `secret` goes to the OS credential store.
/// Returns a warning when the secret could not be stored.
///
/// Blocking (the credential store may wait on an unlock prompt, e.g. the Secret Service
/// on Linux): call it from a worker thread, never the UI thread.
pub fn save_connection(mut conn: SavedConn, secret: Option<&str>) -> Option<String> {
    if conn.id.is_empty() {
        conn.id = new_id();
    }
    let mut warning = None;
    match secret {
        Some(s) if !s.is_empty() => {
            match soyuz_ai::credentials::store(&conn.credential_name(), &soyuz_ai::credentials::Secret::new(s)) {
                Ok(()) => conn.has_secret = true,
                Err(e) => {
                    warning = Some(e.to_string());
                    conn.has_secret = false;
                }
            }
        }
        Some(_) => {
            let _ = soyuz_ai::credentials::delete(&conn.credential_name());
            conn.has_secret = false;
        }
        None => {}
    }
    {
        let mut conns = st().conns.write();
        match conns.iter_mut().find(|c| c.id == conn.id) {
            Some(c) => *c = conn,
            None => conns.push(conn),
        }
        save_store(&conns);
    }
    if let Some(w) = &warning {
        st().notices.lock().push((tr!("remote-secret-not-saved").to_string(), w.clone()));
        wake();
    }
    warning
}

/// Forget a saved connection (and its stored secret). Safe on the UI thread: the
/// sidebar entry goes away at once; the credential-store delete and the JSON write run
/// on a worker thread.
pub fn remove_connection(id: &str) {
    let removed = {
        let mut conns = st().conns.write();
        let removed = conns.iter().find(|c| c.id == id).cloned();
        conns.retain(|c| c.id != id);
        removed.map(|r| (r, conns.clone()))
    };
    let Some((conn, remaining)) = removed else { return };
    let _ = std::thread::Builder::new().name("soyuz-remote-forget".into()).spawn(move || {
        save_store(&remaining);
        // Unconditional (`has_secret` may be stale); a missing secret is not an error.
        let _ = soyuz_ai::credentials::delete(&conn.credential_name());
    });
}

/// Next warning (title, details) from background credential-store work (shown as a toast).
pub fn take_notice() -> Option<(String, String)> {
    let mut q = st().notices.lock();
    if q.is_empty() { None } else { Some(q.remove(0)) }
}

/// Remember a secret / auth choice for this session (connect dialog without saving, or
/// a password typed in the address bar).
pub fn set_session_auth(uri: &RemoteUri, auth: AuthKind, key_file: Option<PathBuf>, secret: Option<String>) {
    let key = uri.conn_key();
    st().auth.lock().insert(key.clone(), (auth, key_file));
    match secret {
        Some(s) => {
            st().secrets.lock().insert(key, s);
        }
        None => {
            st().secrets.lock().remove(&key);
        }
    }
    invalidate_server(uri);
    registry::disconnect(uri);
}

/// The saved connection whose server matches `uri` (same connection key).
pub fn saved_for(uri: &RemoteUri) -> Option<SavedConn> {
    let key = uri.conn_key();
    connections().into_iter().find(|c| c.parsed().is_some_and(|u| u.conn_key() == key))
}

/// Credentials for a connection. Only ever called from `registry::connect` (lister,
/// download and job worker threads), never the UI thread: loading a stored secret may
/// block on the OS credential store (Secret Service unlock prompt on Linux).
fn credentials_for(uri: &RemoteUri) -> Credentials {
    let key = uri.conn_key();
    let session_secret = st().secrets.lock().get(&key).cloned();
    let session_auth = st().auth.lock().get(&key).cloned();
    let saved = saved_for(uri);
    let (auth, key_file) = match (&session_auth, &saved) {
        (Some((a, k)), _) => (*a, k.clone()),
        (None, Some(c)) => (c.auth, c.key_file.clone()),
        (None, None) => (AuthKind::Password, None),
    };
    let secret = session_secret.or_else(|| {
        let c = saved.as_ref().filter(|c| c.has_secret)?;
        let s = soyuz_ai::credentials::load(&c.credential_name()).ok().flatten()?;
        let s = s.expose().to_string();
        st().secrets.lock().insert(key.clone(), s.clone());
        Some(s)
    });
    match auth {
        AuthKind::Anonymous => Credentials::default(),
        AuthKind::Agent => Credentials { agent: true, ..Default::default() },
        AuthKind::KeyFile => Credentials { key_file, secret, agent: false, user: None },
        AuthKind::Password => Credentials { secret, agent: uri.scheme == Scheme::Sftp, ..Default::default() },
    }
}

/// Connection state for the sidebar dot.
pub fn conn_state(uri: &RemoteUri) -> ConnState {
    registry::state(uri)
}

/// Disconnect (the next use reconnects).
pub fn disconnect(uri: &RemoteUri) {
    registry::disconnect(uri);
    invalidate_server(uri);
}

// ---- locations ---------------------------------------------------------------------------

/// Normalize a remote location typed by the user: strips a `user:password@` password
/// into the session secrets. `Err` = not a valid remote address.
pub fn normalize(loc: Location) -> Result<Location, String> {
    if let Some(msg) = smb_unsupported(&loc) {
        return Err(msg);
    }
    // Generic code builds `Dir(entry.path)` for folders; remote entry paths are URIs.
    let loc = match loc {
        Location::Dir(p) if is_remote_path(&p) => Location::Remote(p.to_string_lossy().into_owned()),
        other => other,
    };
    let Location::Remote(s) = &loc else { return Ok(loc) };
    match RemoteUri::parse_with_password(s) {
        Ok((u, pw)) => {
            if let Some(pw) = pw {
                st().secrets.lock().insert(u.conn_key(), pw);
                registry::disconnect(&u);
            }
            Ok(Location::Remote(u.display()))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Off Windows there is no SMB provider (and no share enumeration): `smb://server`,
/// `cifs://…` and `\\server\share` get a friendly hint instead of a raw error.
#[cfg(not(windows))]
fn smb_unsupported(loc: &Location) -> Option<String> {
    let s = match loc {
        Location::Remote(s) => s.as_str(),
        Location::Dir(p) => p.to_str()?,
        _ => return None,
    };
    let lower = s.trim_start().to_ascii_lowercase();
    let smb = lower.starts_with("smb://") || lower.starts_with("cifs://") || lower.starts_with(r"\\");
    smb.then(smb_hint)
}

#[cfg(windows)]
fn smb_unsupported(_: &Location) -> Option<String> {
    None
}

/// "SMB shares can't be opened directly yet …" (non-Windows).
#[cfg(not(windows))]
fn smb_hint() -> String {
    tr!("remote-smb-unsupported").to_string()
}

/// The remote URI behind a pane / entry path.
pub fn uri_of(path: &Path) -> Option<RemoteUri> {
    RemoteUri::from_path(path)
}

pub fn is_remote_path(path: &Path) -> bool {
    path.to_str().is_some_and(soyuz_core::location::looks_remote)
}

// ---- listings ------------------------------------------------------------------------------

fn ttl() -> Duration {
    Duration::from_secs(st().cfg.read().cache_ttl_secs)
}

/// Forget cached listings of a folder.
pub fn invalidate(uri: &RemoteUri) {
    st().cache.lock().remove(&uri.display());
}

/// Forget every cached listing of a server.
pub fn invalidate_server(uri: &RemoteUri) {
    let key = uri.conn_key();
    st().cache.lock().retain(|k, _| RemoteUri::parse(k).map(|u| u.conn_key() != key).unwrap_or(true));
}

fn to_entries(dir: &RemoteUri, list: Vec<soyuz_vfs::RemoteEntry>) -> Vec<EntryInfo> {
    list.into_iter()
        .map(|e| {
            let mut info = e.to_entry_info(dir.join(&e.name).to_path_buf());
            if e.name.starts_with('.') {
                info.attrs |= soyuz_core::Attributes::HIDDEN;
            }
            info
        })
        .collect()
}

/// Blocking listing for the lister pools: `Some` for remote URIs and bare `\\server`
/// paths (shares), `None` for everything else. `force` bypasses the cache.
pub fn list_blocking(path: &Path, force: bool) -> Option<FsResult<Vec<EntryInfo>>> {
    let s = path.to_str()?;
    if soyuz_fs::shares::is_bare_server(s) {
        if !st().cfg.read().share_enumeration {
            return None;
        }
        #[cfg(not(windows))]
        return Some(Err(FsError::with_message(FsErrorKind::Unsupported, "list_shares", Some(path), smb_hint())));
        #[cfg(windows)]
        return Some(list_shares(s));
    }
    if !soyuz_core::location::looks_remote(s) {
        return None;
    }
    let uri = match RemoteUri::parse(s) {
        Ok(u) => u,
        Err(e) => return Some(Err(FsError::with_message(FsErrorKind::InvalidName, "list", Some(path), e.to_string()))),
    };
    let key = uri.display();
    if !force
        && let Some((at, entries)) = st().cache.lock().get(&key)
        && at.elapsed() < ttl()
    {
        return Some(Ok(entries.clone()));
    }
    let r = registry::provider_for(&uri).and_then(|p| p.list(&uri.path)).map(|l| to_entries(&uri, l));
    match &r {
        Ok(entries) => {
            st().cache.lock().insert(key, (Instant::now(), entries.clone()));
        }
        Err(e) if e.kind == FsErrorKind::PermissionDenied => {
            let mut q = st().sign_in.lock();
            if !q.contains(&key) {
                q.push(key);
            }
            // Wrong credentials: drop the connection so the next attempt reconnects.
            registry::disconnect(&uri);
            wake();
        }
        Err(_) => {}
    }
    Some(r)
}

#[cfg(windows)]
fn list_shares(server: &str) -> FsResult<Vec<EntryInfo>> {
    let base = format!(r"\\{}", server.trim_start_matches(['\\', '/']).trim_end_matches(['\\', '/']));
    let shares = soyuz_fs::shares::list_shares(&base)?;
    Ok(shares
        .into_iter()
        .filter(|s| s.disk)
        .map(|s| {
            let mut e = EntryInfo::minimal(PathBuf::from(format!(r"{base}\{}", s.name)), soyuz_core::EntryKind::Dir);
            e.name = s.name;
            if s.special {
                e.attrs |= soyuz_core::Attributes::HIDDEN;
            }
            e
        })
        .collect())
}

/// Next listing that failed to sign in (the UI opens the connect dialog for it).
pub fn take_sign_in_request() -> Option<String> {
    let mut q = st().sign_in.lock();
    if q.is_empty() { None } else { Some(q.remove(0)) }
}

/// Next host key the user must confirm, with the channel that unblocks the worker.
pub fn take_host_prompt() -> Option<(HostKeyPrompt, Sender<bool>)> {
    st().host_prompts.lock().pop_front()
}

// ---- downloads for open / preview -------------------------------------------------------

fn cache_root() -> PathBuf {
    std::env::temp_dir().join(CACHE_DIR)
}

fn cache_key(uri: &str, size: u64, modified: Option<SystemTime>) -> String {
    let mut h = blake3::Hasher::new();
    h.update(uri.as_bytes());
    h.update(&size.to_le_bytes());
    let m = modified.and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    h.update(&m.to_le_bytes());
    h.finalize().to_hex()[..20].to_string()
}

/// Where the cached copy of a remote file lives (it may not exist yet).
pub fn cached_path(e: &EntryInfo) -> PathBuf {
    let key = cache_key(&e.path.to_string_lossy(), e.size, e.modified);
    cache_root().join(key).join(sanitize(&e.name))
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    if s.trim().is_empty() { "file".into() } else { s }
}

/// The cached local copy of a remote file, if it is complete.
pub fn cached_copy(e: &EntryInfo) -> Option<PathBuf> {
    let p = cached_path(e);
    std::fs::metadata(&p).ok().filter(|m| m.len() == e.size).map(|_| p)
}

/// What to do once a remote file is downloaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchThen {
    /// Open it with its default application.
    Launch,
    /// Just repaint (the preview picks the cached copy up).
    Preview,
}

/// Download a remote file into the temp cache on a worker thread (no-op when already
/// cached or in flight).
pub fn fetch(e: &EntryInfo, then: FetchThen, postman: Postman) {
    if let Some(p) = cached_copy(e) {
        if then == FetchThen::Launch {
            postman.send(UiMsg::Launch(p));
        }
        return;
    }
    let key = e.path.to_string_lossy().into_owned();
    {
        let mut f = st().fetching.lock();
        if f.contains_key(&key) {
            return;
        }
        f.insert(key.clone(), Instant::now());
    }
    let e = e.clone();
    let cap = st().cfg.read().preview_cache_mb.saturating_mul(1024 * 1024);
    let _ = std::thread::Builder::new().name("soyuz-remote-fetch".into()).spawn(move || {
        let r = download(&e);
        st().fetching.lock().remove(&key);
        match r {
            Ok(p) => {
                evict(cap, &p);
                match then {
                    FetchThen::Launch => {
                        postman.send(UiMsg::Launch(p));
                    }
                    FetchThen::Preview => postman.wake()(),
                }
            }
            Err(err) => {
                postman.send(UiMsg::Notice(
                    Toast::error(tr!("remote-download-failed", name = &e.name)).body(err.to_string()),
                ));
            }
        }
    });
}

/// A download for `e` is running.
pub fn is_fetching(e: &EntryInfo) -> bool {
    st().fetching.lock().contains_key(e.path.to_string_lossy().as_ref())
}

fn download(e: &EntryInfo) -> FsResult<PathBuf> {
    let uri = RemoteUri::from_path(&e.path).ok_or_else(|| {
        FsError::with_message(FsErrorKind::InvalidName, "download", Some(&e.path), "not a remote file")
    })?;
    let prov = registry::provider_for(&uri)?;
    let dst = cached_path(e);
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir).map_err(|err| FsError::from_io("download", Some(dir), &err))?;
    }
    let local = soyuz_vfs::local::LocalProvider;
    let d = dst.to_string_lossy().into_owned();
    soyuz_vfs::transfer::copy_file(
        &soyuz_vfs::transfer::FileCopy {
            src: &*prov,
            src_path: &uri.path,
            dst: &local,
            dst_path: &d,
            size: e.size,
            modified: e.modified,
            replace: true,
        },
        &soyuz_vfs::transfer::NoHooks,
    )?;
    Ok(dst)
}

/// Keep the cache under `cap` bytes (oldest folders first; never the one just written).
fn evict(cap: u64, keep: &Path) {
    let root = cache_root();
    let Ok(rd) = std::fs::read_dir(&root) else { return };
    let mut dirs: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|d| {
            let p = d.path();
            let mut size = 0;
            let mut newest = SystemTime::UNIX_EPOCH;
            for f in std::fs::read_dir(&p).ok()?.flatten() {
                if let Ok(m) = f.metadata() {
                    size += m.len();
                    newest = newest.max(m.modified().unwrap_or(SystemTime::UNIX_EPOCH));
                }
            }
            Some((newest, size, p))
        })
        .collect();
    let mut total: u64 = dirs.iter().map(|d| d.1).sum();
    dirs.sort_by_key(|d| d.0);
    for (_, size, p) in dirs {
        if total <= cap {
            break;
        }
        if keep.starts_with(&p) {
            continue;
        }
        if std::fs::remove_dir_all(&p).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

/// Largest file downloaded just for a preview.
pub fn preview_max_bytes() -> u64 {
    st().cfg.read().preview_max_mb.saturating_mul(1024 * 1024)
}

/// Poll interval for visible remote panes (`None` = off).
pub fn poll_interval() -> Option<Duration> {
    let s = st().cfg.read().poll_secs;
    (s > 0).then(|| Duration::from_secs(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::EntryKind;

    #[test]
    fn normalize_strips_password() {
        let l = normalize(Location::Remote("ftp://bob:hunter2@Example.org:21/pub/".into())).unwrap();
        assert_eq!(l, Location::Remote("ftp://bob@example.org/pub".into()));
        let u = RemoteUri::parse("ftp://bob@example.org/").unwrap();
        assert_eq!(credentials_for(&u).secret.as_deref(), Some("hunter2"));
        assert!(normalize(Location::Remote("gopher://x".into())).is_err());
        #[cfg(not(windows))]
        {
            let e = normalize(Location::Remote("smb://nas/share".into())).unwrap_err();
            assert!(e.contains("SMB"), "{e}");
            assert!(normalize(Location::Dir(PathBuf::from(r"\\nas\share"))).is_err());
            assert!(normalize(Location::Dir(PathBuf::from("/home"))).is_ok());
        }
        assert_eq!(normalize(Location::Trash).unwrap(), Location::Trash);
        assert_eq!(
            normalize(Location::Dir(PathBuf::from("sftp://u@h/x"))).unwrap(),
            Location::Remote("sftp://u@h/x".into())
        );
    }

    #[test]
    fn fake_server_listing_and_cache() {
        let fake = soyuz_vfs::fake::FakeProvider::new("app-list-test");
        fake.put("/a/x.txt", b"12345");
        fake.put("/a/.hidden", b"");
        let fake = fake.install();
        let p = PathBuf::from("fake://app-list-test/a");
        let v = list_blocking(&p, false).unwrap().unwrap();
        assert_eq!(v.len(), 2);
        let x = v.iter().find(|e| e.name == "x.txt").unwrap();
        assert_eq!(x.size, 5);
        assert_eq!(x.path, PathBuf::from("fake://app-list-test/a/x.txt"));
        assert!(v.iter().find(|e| e.name == ".hidden").unwrap().attrs.contains(soyuz_core::Attributes::HIDDEN));
        // Cached (TTL 30 s by default) until forced.
        fake.put("/a/y.txt", b"");
        assert_eq!(list_blocking(&p, false).unwrap().unwrap().len(), 2);
        assert_eq!(list_blocking(&p, true).unwrap().unwrap().len(), 3);
        assert!(list_blocking(Path::new("C:/x"), false).is_none());
    }

    #[test]
    fn download_to_cache() {
        let fake = soyuz_vfs::fake::FakeProvider::new("app-fetch-test");
        fake.put("/doc.txt", b"remote text");
        fake.install();
        let mut e = EntryInfo::minimal(PathBuf::from("fake://app-fetch-test/doc.txt"), EntryKind::File);
        e.size = 11;
        let p = download(&e).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"remote text");
        assert_eq!(cached_copy(&e), Some(p.clone()));
        assert!(p.starts_with(cache_root()));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("a:b?.txt"), "a_b_.txt");
        assert_eq!(sanitize("  "), "file");
    }
}
