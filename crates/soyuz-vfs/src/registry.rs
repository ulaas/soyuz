//! Provider registry: one live provider per connection (scheme + user + host + port +
//! parameters), connected lazily on first use, plus the app hooks that supply
//! credentials and confirm unknown SSH host keys.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::known_hosts::{HostKeyStatus, KnownHosts};
use crate::local::LocalProvider;
use crate::provider::Provider;
use crate::uri::{RemoteUri, Scheme};

/// How to authenticate a connection.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    /// Overrides the URI's user (S3: the access key id).
    pub user: Option<String>,
    /// Password (SFTP/FTP/WebDAV), key passphrase (SFTP key file) or S3 secret key.
    pub secret: Option<String>,
    /// SFTP private key file.
    pub key_file: Option<PathBuf>,
    /// SFTP: try the SSH agent (Windows OpenSSH agent pipe / Pageant).
    pub agent: bool,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .field("secret", &self.secret.as_ref().map(|_| "***"))
            .field("key_file", &self.key_file)
            .field("agent", &self.agent)
            .finish()
    }
}

/// A host key the user must confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    /// `SHA256:…` (OpenSSH style).
    pub fingerprint: String,
    pub status: HostKeyStatus,
}

/// Implemented by the app.
pub trait Hooks: Send + Sync {
    /// Credentials for a connection (saved connection + OS credential store, or what the
    /// user typed this session).
    fn credentials(&self, uri: &RemoteUri) -> Credentials;
    /// Ask the user whether to trust an unknown or changed host key. Blocks the calling
    /// worker thread until answered; `true` trusts (and the registry saves it).
    fn confirm_host_key(&self, prompt: &HostKeyPrompt) -> bool;
}

/// Connection state shown in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnState {
    Idle,
    Connecting,
    Connected,
    Failed(String),
}

struct NoHooks;

impl Hooks for NoHooks {
    fn credentials(&self, _: &RemoteUri) -> Credentials {
        Credentials::default()
    }
    fn confirm_host_key(&self, _: &HostKeyPrompt) -> bool {
        false
    }
}

/// Connection timeouts and limits the providers read.
#[derive(Clone, Copy, Debug)]
pub struct NetSettings {
    pub connect_timeout: std::time::Duration,
    pub io_timeout: std::time::Duration,
}

impl Default for NetSettings {
    fn default() -> Self {
        NetSettings {
            connect_timeout: std::time::Duration::from_secs(20),
            io_timeout: std::time::Duration::from_secs(60),
        }
    }
}

type Slot = Arc<Mutex<Option<Arc<dyn Provider>>>>;

struct Registry {
    hooks: RwLock<Arc<dyn Hooks>>,
    known_hosts: RwLock<Arc<KnownHosts>>,
    net: RwLock<NetSettings>,
    slots: Mutex<HashMap<String, Slot>>,
    states: Mutex<HashMap<String, (ConnState, Instant)>>,
    local: Arc<LocalProvider>,
}

fn reg() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Registry {
        hooks: RwLock::new(Arc::new(NoHooks)),
        known_hosts: RwLock::new(Arc::new(KnownHosts::in_memory())),
        net: RwLock::new(NetSettings::default()),
        slots: Mutex::new(HashMap::new()),
        states: Mutex::new(HashMap::new()),
        local: Arc::new(LocalProvider),
    })
}

pub fn set_hooks(h: Arc<dyn Hooks>) {
    *reg().hooks.write() = h;
}

pub(crate) fn hooks() -> Arc<dyn Hooks> {
    reg().hooks.read().clone()
}

/// Use (and persist to) this known_hosts file.
pub fn set_known_hosts(path: &Path) {
    *reg().known_hosts.write() = Arc::new(KnownHosts::open(path));
}

pub fn known_hosts() -> Arc<KnownHosts> {
    reg().known_hosts.read().clone()
}

pub fn set_net_settings(n: NetSettings) {
    *reg().net.write() = n;
}

pub fn net_settings() -> NetSettings {
    *reg().net.read()
}

fn set_state(key: &str, s: ConnState) {
    reg().states.lock().insert(key.to_string(), (s, Instant::now()));
}

/// State of the connection behind `uri`.
pub fn state(uri: &RemoteUri) -> ConnState {
    reg().states.lock().get(&uri.conn_key()).map(|(s, _)| s.clone()).unwrap_or(ConnState::Idle)
}

/// Install a ready provider for `uri`'s connection (fake providers, tests).
pub fn register(uri: &RemoteUri, p: Arc<dyn Provider>) {
    let key = uri.conn_key();
    reg().slots.lock().insert(key.clone(), Arc::new(Mutex::new(Some(p))));
    set_state(&key, ConnState::Connected);
}

/// Drop the connection (the next use reconnects, e.g. with new credentials).
pub fn disconnect(uri: &RemoteUri) {
    let key = uri.conn_key();
    if uri.scheme == Scheme::Fake {
        return; // in-memory: nothing to reconnect to
    }
    reg().slots.lock().remove(&key);
    set_state(&key, ConnState::Idle);
}

/// The provider for `uri`'s connection, connecting (blocking) on first use.
pub fn provider_for(uri: &RemoteUri) -> FsResult<Arc<dyn Provider>> {
    let key = uri.conn_key();
    let slot = reg().slots.lock().entry(key.clone()).or_insert_with(|| Arc::new(Mutex::new(None))).clone();
    let mut guard = slot.lock();
    if let Some(p) = guard.as_ref() {
        return Ok(p.clone());
    }
    set_state(&key, ConnState::Connecting);
    match connect(uri) {
        Ok(p) => {
            *guard = Some(p.clone());
            set_state(&key, ConnState::Connected);
            Ok(p)
        }
        Err(e) => {
            set_state(&key, ConnState::Failed(e.message.clone()));
            Err(e)
        }
    }
}

fn connect(uri: &RemoteUri) -> FsResult<Arc<dyn Provider>> {
    let creds = hooks().credentials(uri);
    let root = uri.root();
    Ok(match uri.scheme {
        Scheme::Sftp => Arc::new(crate::sftp::SftpProvider::connect(root, creds)?),
        Scheme::WebDav | Scheme::WebDavs => Arc::new(crate::webdav::WebDavProvider::connect(root, creds)?),
        Scheme::Ftp | Scheme::Ftps => Arc::new(crate::ftp::FtpProvider::connect(root, creds)?),
        Scheme::S3 => Arc::new(crate::s3::S3Provider::connect(root, creds)?),
        Scheme::Fake => {
            return Err(FsError::with_message(
                FsErrorKind::NotFound,
                "connect",
                None,
                format!("no test server named “{}”", uri.host),
            ));
        }
    })
}

/// The local filesystem provider.
pub fn local() -> Arc<dyn Provider> {
    reg().local.clone()
}

/// Resolve a job / pane path: a remote URI (its provider + provider path) or a native
/// path (the local provider + the path as a string).
pub fn resolve(path: &Path) -> FsResult<(Arc<dyn Provider>, String)> {
    match RemoteUri::from_path(path) {
        Some(u) => Ok((provider_for(&u)?, u.path.clone())),
        None => Ok((local(), path.to_string_lossy().into_owned())),
    }
}

/// Check a server key against known_hosts, asking the user when unknown / changed.
/// Returns whether to proceed (a confirmed key is saved).
pub(crate) fn verify_host_key(host: &str, port: u16, algorithm: &str, fingerprint: &str) -> bool {
    let kh = known_hosts();
    let status = kh.check(host, port, fingerprint);
    if status == HostKeyStatus::Trusted {
        return true;
    }
    let prompt = HostKeyPrompt {
        host: host.to_string(),
        port,
        algorithm: algorithm.to_string(),
        fingerprint: fingerprint.to_string(),
        status,
    };
    if hooks().confirm_host_key(&prompt) {
        if let Err(e) = kh.trust(host, port, algorithm, fingerprint) {
            log::warn!("soyuz-vfs: cannot save known_hosts: {e}");
        }
        true
    } else {
        false
    }
}
