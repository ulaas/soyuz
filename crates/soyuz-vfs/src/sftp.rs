//! SFTP over pure-Rust SSH (`russh` + `russh-sftp`).
//!
//! * Host keys are checked against Soyuz's `known_hosts`; an unknown or changed key is
//!   shown to the user (SHA-256 fingerprint) through [`crate::registry::Hooks`].
//! * Authentication, in order: key file (with passphrase), SSH agent (Windows OpenSSH
//!   agent pipe, then Pageant), password, keyboard-interactive with the password.
//! * Resume: reads seek; writes reopen the partial file and continue at its size.
//! * A dropped session reconnects transparently on the next call.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use russh::client::{self, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use soyuz_core::{FsError, FsErrorKind, FsResult};
use tokio::io::AsyncSeekExt;

use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream};
use crate::registry::{Credentials, net_settings, verify_host_key};
use crate::rt::{BlockingRead, BlockingWrite, block_on, runtime};
use crate::uri::{RemoteUri, name_of, normalize_path};

struct Client {
    host: String,
    port: u16,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let pk = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => russh::keys::PublicKey::from(c.public_key().clone()),
        };
        let algo = pk.algorithm().to_string();
        let fp = pk.fingerprint(HashAlg::Sha256).to_string();
        let (host, port) = (self.host.clone(), self.port);
        // The user may take a while to answer: never block a runtime worker.
        let ok = tokio::task::spawn_blocking(move || verify_host_key(&host, port, &algo, &fp)).await.unwrap_or(false);
        Ok(ok)
    }
}

struct Session {
    handle: Handle<Client>,
    sftp: Arc<SftpSession>,
}

pub struct SftpProvider {
    uri: RemoteUri,
    creds: Credentials,
    session: Mutex<Option<Session>>,
}

fn sftp_err(op: &'static str, path: &str, e: SftpError) -> FsError {
    let (kind, msg) = match &e {
        SftpError::Status(s) => {
            let kind = match s.status_code {
                StatusCode::NoSuchFile => FsErrorKind::NotFound,
                StatusCode::PermissionDenied => FsErrorKind::PermissionDenied,
                StatusCode::OpUnsupported => FsErrorKind::Unsupported,
                StatusCode::NoConnection | StatusCode::ConnectionLost => FsErrorKind::Io,
                _ => FsErrorKind::Io,
            };
            let msg = if s.error_message.is_empty() { s.status_code.to_string() } else { s.error_message.clone() };
            (kind, msg)
        }
        other => (FsErrorKind::Io, other.to_string()),
    };
    FsError::with_message(kind, op, Some(std::path::Path::new(path)), msg)
}

fn ssh_err(op: &'static str, e: impl std::fmt::Display) -> FsError {
    FsError::with_message(FsErrorKind::Io, op, None, e.to_string())
}

fn auth_failed(msg: &str) -> FsError {
    FsError::with_message(FsErrorKind::PermissionDenied, "login", None, msg.to_string())
}

impl SftpProvider {
    pub fn connect(uri: RemoteUri, creds: Credentials) -> FsResult<Self> {
        let p = SftpProvider { uri, creds, session: Mutex::new(None) };
        let s = p.open_session()?;
        *p.session.lock() = Some(s);
        Ok(p)
    }

    fn user(&self) -> FsResult<String> {
        self.creds
            .user
            .clone()
            .or_else(|| self.uri.user.clone())
            .filter(|u| !u.is_empty())
            .ok_or_else(|| auth_failed("a user name is required for SFTP"))
    }

    fn open_session(&self) -> FsResult<Session> {
        let net = net_settings();
        let user = self.user()?;
        let host = self.uri.host.clone();
        let port = self.uri.effective_port();
        let creds = self.creds.clone();
        block_on(net.connect_timeout * 3, async move {
            let config = Arc::new(client::Config {
                inactivity_timeout: Some(Duration::from_secs(600)),
                keepalive_interval: Some(Duration::from_secs(30)),
                ..Default::default()
            });
            let handler = Client { host: host.clone(), port };
            let mut handle =
                tokio::time::timeout(net.connect_timeout, client::connect(config, (host.as_str(), port), handler))
                    .await
                    .map_err(|_| ssh_err("connect", "the server did not answer in time"))?
                    .map_err(|e| match e {
                        russh::Error::UnknownKey => FsError::with_message(
                            FsErrorKind::PermissionDenied,
                            "connect",
                            None,
                            "the server's host key was not trusted",
                        ),
                        other => ssh_err("connect", other),
                    })?;
            authenticate(&mut handle, &user, &creds).await?;
            let channel = handle.channel_open_session().await.map_err(|e| ssh_err("connect", e))?;
            channel.request_subsystem(true, "sftp").await.map_err(|e| ssh_err("connect", e))?;
            let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| sftp_err("connect", "/", e))?;
            sftp.set_timeout(net.io_timeout.as_secs().max(5));
            Ok(Session { handle, sftp: Arc::new(sftp) })
        })
    }

    /// The live SFTP session, reconnecting when the SSH connection dropped.
    fn sftp(&self) -> FsResult<Arc<SftpSession>> {
        let mut g = self.session.lock();
        if let Some(s) = g.as_ref()
            && !s.handle.is_closed()
        {
            return Ok(s.sftp.clone());
        }
        let s = self.open_session()?;
        let sftp = s.sftp.clone();
        *g = Some(s);
        Ok(sftp)
    }

    fn run<T: Send + 'static>(
        &self,
        op: &'static str,
        path: &str,
        f: impl AsyncFnOnce(Arc<SftpSession>, String) -> Result<T, SftpError>,
    ) -> FsResult<T> {
        let sftp = self.sftp()?;
        let p = path.to_string();
        let timeout = net_settings().io_timeout;
        block_on(timeout, async move { f(sftp, p.clone()).await.map_err(|e| sftp_err(op, &p, e)) })
    }
}

async fn authenticate(handle: &mut Handle<Client>, user: &str, creds: &Credentials) -> FsResult<()> {
    let rsa_hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
    // 1. Key file.
    if let Some(path) = &creds.key_file {
        let key = russh::keys::load_secret_key(path, creds.secret.as_deref()).map_err(|e| {
            let msg = match e {
                russh::keys::Error::KeyIsEncrypted => "the key file needs a passphrase".to_string(),
                other => format!("cannot read the key file: {other}"),
            };
            FsError::with_message(FsErrorKind::PermissionDenied, "login", Some(path), msg)
        })?;
        let r = handle
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash))
            .await
            .map_err(|e| ssh_err("login", e))?;
        if r.success() {
            return Ok(());
        }
    }
    // 2. Agent.
    if creds.agent && try_agent(handle, user, rsa_hash).await {
        return Ok(());
    }
    // 3. Password, then keyboard-interactive with the same password.
    if let Some(pw) = creds.secret.as_deref().filter(|_| creds.key_file.is_none()) {
        let r = handle.authenticate_password(user, pw).await.map_err(|e| ssh_err("login", e))?;
        if r.success() {
            return Ok(());
        }
        let mut resp = handle
            .authenticate_keyboard_interactive_start(user, None::<String>)
            .await
            .map_err(|e| ssh_err("login", e))?;
        for _ in 0..4 {
            match resp {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => break,
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let answers = prompts.iter().map(|_| pw.to_string()).collect();
                    resp = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(|e| ssh_err("login", e))?;
                }
            }
        }
    }
    Err(auth_failed("sign-in failed (check the user name, password or key)"))
}

#[cfg(windows)]
async fn try_agent(handle: &mut Handle<Client>, user: &str, rsa_hash: Option<HashAlg>) -> bool {
    use russh::keys::agent::AgentIdentity;
    use russh::keys::agent::client::AgentClient;
    if let Ok(mut agent) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        let ids = agent.request_identities().await.unwrap_or_default();
        for id in ids {
            if let AgentIdentity::PublicKey { key, .. } = id {
                let hash = if key.algorithm().is_rsa() { rsa_hash } else { None };
                if let Ok(r) = handle.authenticate_publickey_with(user, key, hash, &mut agent).await
                    && r.success()
                {
                    return true;
                }
            }
        }
    }
    if let Ok(agent) = AgentClient::connect_pageant().await {
        let mut agent = agent.dynamic();
        let ids = agent.request_identities().await.unwrap_or_default();
        for id in ids {
            if let AgentIdentity::PublicKey { key, .. } = id {
                let hash = if key.algorithm().is_rsa() { rsa_hash } else { None };
                if let Ok(r) = handle.authenticate_publickey_with(user, key, hash, &mut agent).await
                    && r.success()
                {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(not(windows))]
async fn try_agent(handle: &mut Handle<Client>, user: &str, rsa_hash: Option<HashAlg>) -> bool {
    use russh::keys::agent::AgentIdentity;
    use russh::keys::agent::client::AgentClient;
    let Ok(mut agent) = AgentClient::connect_env().await else { return false };
    let ids = agent.request_identities().await.unwrap_or_default();
    for id in ids {
        if let AgentIdentity::PublicKey { key, .. } = id {
            let hash = if key.algorithm().is_rsa() { rsa_hash } else { None };
            if let Ok(r) = handle.authenticate_publickey_with(user, key, hash, &mut agent).await
                && r.success()
            {
                return true;
            }
        }
    }
    false
}

fn mtime(a: &FileAttributes) -> Option<SystemTime> {
    a.mtime.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s as u64))
}

fn entry(name: String, a: &FileAttributes) -> RemoteEntry {
    RemoteEntry {
        name,
        is_dir: a.is_dir(),
        is_link: a.is_symlink(),
        size: a.size.unwrap_or(0),
        modified: mtime(a),
        mode: a.permissions.map(|p| p & 0o7777),
    }
}

impl Provider for SftpProvider {
    fn describe(&self) -> String {
        format!("SFTP {}", self.uri.server_label())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            range_read: true,
            resume_write: true,
            atomic_write: false,
            rename: true,
            rename_replaces: false,
            set_times: true,
            watch: false,
        }
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        let p = normalize_path(path);
        self.run("list", &p, async move |sftp, p| {
            let mut out = Vec::new();
            for e in sftp.read_dir(p.clone()).await? {
                let name = e.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                let mut ent = entry(name.clone(), &e.metadata());
                if ent.is_link {
                    // Follow the link to tell folders from files.
                    if let Ok(t) = sftp.metadata(crate::uri::join_path(&p, &name)).await {
                        ent.is_dir = t.is_dir();
                        ent.size = t.size.unwrap_or(0);
                    }
                }
                out.push(ent);
            }
            Ok(out)
        })
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let p = normalize_path(path);
        let name = name_of(&p).to_string();
        self.run("stat", &p, async move |sftp, p| Ok(entry(name, &sftp.metadata(p).await?)))
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        let p = normalize_path(path);
        let file = self.run("open", &p, async move |sftp, p| {
            let mut f = sftp.open(p).await?;
            if offset > 0 {
                f.seek(std::io::SeekFrom::Start(offset)).await.map_err(|e| SftpError::IO(e.to_string()))?;
            }
            Ok(f)
        })?;
        Ok(Box::new(BlockingRead { inner: Box::pin(file), timeout: net_settings().io_timeout }))
    }

    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        let p = normalize_path(path);
        let resume = opts.resume;
        let (file, resumed) = self.run("create", &p, async move |sftp, p| {
            let existing = if resume { sftp.metadata(p.clone()).await.ok().and_then(|m| m.size) } else { None };
            match existing {
                Some(n) if n > 0 => {
                    let mut f = sftp.open_with_flags(p, OpenFlags::WRITE).await?;
                    f.seek(std::io::SeekFrom::Start(n)).await.map_err(|e| SftpError::IO(e.to_string()))?;
                    Ok((f, n))
                }
                _ => {
                    let f = sftp.open_with_flags(p, OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE).await?;
                    Ok((f, 0))
                }
            }
        })?;
        Ok(Box::new(SftpWrite {
            w: Some(BlockingWrite { inner: Box::pin(file), timeout: net_settings().io_timeout }),
            path: p,
            resumed,
            sftp: self.sftp()?,
        }))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        self.run("mkdir", &normalize_path(path), async move |sftp, p| sftp.create_dir(p).await)
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let t = normalize_path(to);
        self.run("rename", &normalize_path(from), async move |sftp, f| sftp.rename(f, t).await)
    }

    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
        self.run("delete", &normalize_path(path), async move |sftp, p| {
            if is_dir { sftp.remove_dir(p).await } else { sftp.remove_file(p).await }
        })
    }

    fn set_times(&self, path: &str, modified: SystemTime) -> FsResult<()> {
        let secs = modified.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
        self.run("set_times", &normalize_path(path), async move |sftp, p| {
            let attrs = FileAttributes { atime: Some(secs), mtime: Some(secs), ..FileAttributes::empty() };
            sftp.set_metadata(p, attrs).await
        })
    }

    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        Some(self.uri.with_path(path))
    }
}

struct SftpWrite {
    w: Option<BlockingWrite<russh_sftp::client::fs::File>>,
    path: String,
    resumed: u64,
    sftp: Arc<SftpSession>,
}

impl Write for SftpWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.w.as_mut() {
            Some(w) => w.write(buf),
            None => Err(std::io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.w.as_mut() {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }
}

impl WriteStream for SftpWrite {
    fn resumed_at(&self) -> u64 {
        self.resumed
    }

    fn commit(mut self: Box<Self>) -> FsResult<()> {
        if let Some(mut w) = self.w.take() {
            w.flush().map_err(|e| FsError::from_io("write", Some(self.path.as_ref()), &e))?;
            w.shutdown().map_err(|e| FsError::from_io("write", Some(self.path.as_ref()), &e))?;
        }
        Ok(())
    }

    fn abort(mut self: Box<Self>, keep_partial: bool) {
        if let Some(mut w) = self.w.take() {
            let _ = w.shutdown();
        }
        if !keep_partial {
            let sftp = self.sftp.clone();
            let p = self.path.clone();
            let _ = runtime()
                .block_on(async move { tokio::time::timeout(Duration::from_secs(10), sftp.remove_file(p)).await });
        }
    }
}
