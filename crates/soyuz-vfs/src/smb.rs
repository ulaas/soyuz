//! SMB 2 / 3 over the pure-Rust `smb2` client (Linux, macOS and other unix; Windows opens
//! `\\server\share` through its kernel redirector instead).
//!
//! * Provider paths are `/share/folder/file`; `/` lists the server's disk shares (srvsvc
//!   `NetShareEnumAll` over the `IPC$` named pipe) as folders.
//! * Sign-in: NTLMv2 with `user` + password, `DOMAIN\user` or `DOMAIN;user` (the
//!   `smb://` URI form) for a domain account; no user (or `guest`) asks for a guest /
//!   anonymous session. Signing and encryption follow what the server negotiates (SMB 3
//!   AES-CMAC / GMAC signing, AES-CCM / GCM encryption for shares that require it).
//! * Resume: reads are positioned (`range_read`); writes reopen the partial file without
//!   truncating and continue at its size.
//! * A dropped session is revived by the client itself; a dead one reconnects on the
//!   next call.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use smb2::{ClientConfig, ErrorKind, FileReader, FileTimes, FileWriter, SmbClient, Tree};
use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream};
use crate::registry::{Credentials, net_settings};
use crate::rt::runtime;
use crate::uri::{RemoteUri, normalize_path, remote_name_of};

/// Bytes fetched per positioned read (split into `MaxReadSize` READs by the client).
const READ_CHUNK: u64 = 4 * 1024 * 1024;

/// `(domain, user)` from `DOMAIN\user`, `DOMAIN;user` (the `smb://` URI form) or a plain
/// `user` / `user@realm` (empty domain).
pub fn split_domain_user(s: &str) -> (String, String) {
    match s.find(['\\', ';']) {
        Some(i) => (s[..i].to_string(), s[i + 1..].to_string()),
        None => (String::new(), s.to_string()),
    }
}

/// `(share, path inside the share)` of a provider path: `/Media/a/b` → `("Media", "a/b")`.
/// `None` at the server root.
pub fn split_share(path: &str) -> Option<(String, String)> {
    let p = normalize_path(path);
    let t = p.trim_start_matches('/');
    if t.is_empty() {
        return None;
    }
    Some(match t.find('/') {
        Some(i) => (t[..i].to_string(), t[i + 1..].to_string()),
        None => (t.to_string(), String::new()),
    })
}

/// Map an `smb2` error to an [`FsError`] for `path`.
pub fn smb_err(op: &'static str, path: &str, e: &smb2::Error) -> FsError {
    let p = Some(std::path::Path::new(path));
    if let Some(status) = e.status() {
        let special = match status.0 {
            0xC000_0101 => Some((FsErrorKind::NotEmpty, "the folder is not empty")),
            0xC000_0121 => Some((FsErrorKind::PermissionDenied, "the server refused to delete it")),
            0xC000_00A2 => Some((FsErrorKind::ReadOnlyFs, "the share is read-only")),
            0xC000_0106 => Some((FsErrorKind::NameTooLong, "the name is too long")),
            0xC000_0056 => Some((FsErrorKind::InUse, "the file is being deleted")),
            0xC000_00D4 => Some((FsErrorKind::CrossDevice, "cannot move between volumes on the server")),
            0xC000_0044 | 0xC000_007F => Some((FsErrorKind::NoSpace, "not enough space on the share (or quota)")),
            0xC000_00CC => Some((FsErrorKind::NotFound, "the server has no share by that name")),
            _ => None,
        };
        if let Some((kind, msg)) = special {
            return FsError::with_message(kind, op, p, msg);
        }
    }
    let (kind, msg) = match e.kind() {
        ErrorKind::AuthRequired => {
            (FsErrorKind::PermissionDenied, "sign-in failed (check the user name, domain and password)".to_string())
        }
        ErrorKind::SigningRequired => {
            (FsErrorKind::PermissionDenied, "the server requires signed sessions: sign in with an account".to_string())
        }
        ErrorKind::AccessDenied => (FsErrorKind::PermissionDenied, "access denied".to_string()),
        ErrorKind::NotFound => (FsErrorKind::NotFound, "not found".to_string()),
        ErrorKind::AlreadyExists => (FsErrorKind::AlreadyExists, "already exists".to_string()),
        ErrorKind::SharingViolation => (FsErrorKind::InUse, "the file is open on the server".to_string()),
        ErrorKind::IsADirectory => (FsErrorKind::IsDirectory, "is a folder".to_string()),
        ErrorKind::NotADirectory => (FsErrorKind::NotDirectory, "not a folder".to_string()),
        ErrorKind::DiskFull => (FsErrorKind::NoSpace, "the share is full".to_string()),
        ErrorKind::InvalidName => (FsErrorKind::InvalidName, "the server does not accept this name".to_string()),
        ErrorKind::Unsupported => (FsErrorKind::Unsupported, e.to_string()),
        ErrorKind::Cancelled => (FsErrorKind::Interrupted, "cancelled".to_string()),
        ErrorKind::ConnectionLost => (FsErrorKind::Io, format!("lost the connection to the server ({e})")),
        ErrorKind::TimedOut => (FsErrorKind::Io, "the server did not answer in time".to_string()),
        _ => (FsErrorKind::Io, e.to_string()),
    };
    FsError::with_message(kind, op, p, msg)
}

/// Errors after which the connection is dropped and the next call dials again.
fn is_dead(e: &smb2::Error) -> bool {
    matches!(e.kind(), ErrorKind::ConnectionLost | ErrorKind::SessionExpired | ErrorKind::TimedOut)
}

fn time(t: smb2::pack::FileTime) -> Option<SystemTime> {
    if t.0 == 0 { None } else { t.to_system_time() }
}

#[derive(Default)]
struct Conn {
    client: Option<SmbClient>,
    /// Lower-cased share name → connected tree.
    trees: HashMap<String, Tree>,
}

struct Shared {
    uri: RemoteUri,
    config: ClientConfig,
    conn: tokio::sync::Mutex<Conn>,
}

/// An SMB server (all of its shares) as one provider.
pub struct SmbProvider {
    shared: Arc<Shared>,
}

/// What [`Shared::run`] hands the operation: the client and, inside a share, its tree.
struct Ctx<'a> {
    client: &'a mut SmbClient,
    tree: Option<&'a mut Tree>,
    /// Path inside the share (`""` = share root).
    rel: String,
}

impl Shared {
    fn timeout(&self) -> Duration {
        net_settings().io_timeout
    }

    async fn dial(&self) -> Result<SmbClient, smb2::Error> {
        SmbClient::connect(self.config.clone()).await
    }

    /// Run `f` with a live client (and the tree of `path`'s share), dialing as needed.
    fn run<T>(
        &self,
        op: &'static str,
        path: &str,
        f: impl AsyncFnOnce(Ctx<'_>) -> Result<T, smb2::Error>,
    ) -> FsResult<T> {
        let path = normalize_path(path);
        // `\` is a separator on the wire: a provider path segment holding one would
        // address something else than it names (a hostile `..\..\Other\x`).
        if path.contains('\\') {
            return Err(FsError::with_message(
                FsErrorKind::InvalidName,
                op,
                Some(std::path::Path::new(&path)),
                "SMB names cannot contain a backslash",
            ));
        }
        let share = split_share(&path);
        let timeout = self.timeout();
        let connect_timeout = net_settings().connect_timeout * 2;
        runtime().block_on(async {
            let mut g = self.conn.lock().await;
            let conn = &mut *g;
            if conn.client.is_none() {
                conn.trees.clear();
                let c = tokio::time::timeout(connect_timeout, self.dial())
                    .await
                    .map_err(|_| timed_out("connect", &path))?
                    .map_err(|e| smb_err("connect", &path, &e))?;
                conn.client = Some(c);
            }
            let client = conn.client.as_mut().expect("client");
            let fut = async {
                let (tree, rel) = match &share {
                    None => (None, String::new()),
                    Some((name, rel)) => {
                        let key = name.to_lowercase();
                        if !conn.trees.contains_key(&key) {
                            let t = client.connect_share(name).await?;
                            conn.trees.insert(key.clone(), t);
                        }
                        (conn.trees.get_mut(&key), rel.clone())
                    }
                };
                f(Ctx { client, tree, rel }).await
            };
            match tokio::time::timeout(timeout, fut).await {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(e)) => {
                    if is_dead(&e) {
                        conn.client = None;
                        conn.trees.clear();
                    }
                    Err(smb_err(op, &path, &e))
                }
                Err(_) => {
                    conn.client = None;
                    conn.trees.clear();
                    Err(timed_out(op, &path))
                }
            }
        })
    }
}

fn timed_out(op: &'static str, path: &str) -> FsError {
    FsError::with_message(FsErrorKind::Io, op, Some(std::path::Path::new(path)), "the server did not answer in time")
}

fn share_level(op: &'static str, path: &str) -> FsError {
    FsError::with_message(
        FsErrorKind::Unsupported,
        op,
        Some(std::path::Path::new(path)),
        "shares are managed on the server, not from here",
    )
}

/// The tree of a [`Ctx`] (operations inside a share only).
fn tree<'a>(t: &'a mut Option<&mut Tree>) -> Result<&'a mut Tree, smb2::Error> {
    t.as_deref_mut().ok_or_else(|| smb2::Error::invalid_data("not inside a share"))
}

impl SmbProvider {
    /// Connect (and sign in) to the server behind `uri`.
    pub fn connect(uri: RemoteUri, creds: Credentials) -> FsResult<Self> {
        let user = creds.user.clone().or_else(|| uri.user.clone()).unwrap_or_default();
        let (domain, user) = split_domain_user(user.trim());
        let net = net_settings();
        let config = ClientConfig {
            addr: if uri.host.contains(':') {
                format!("[{}]:{}", uri.host, uri.effective_port())
            } else {
                format!("{}:{}", uri.host, uri.effective_port())
            },
            timeout: net.connect_timeout,
            username: user,
            password: creds.secret.clone().unwrap_or_default(),
            domain,
            auto_reconnect: true,
            ..Default::default()
        };
        let p =
            SmbProvider { shared: Arc::new(Shared { uri, config, conn: tokio::sync::Mutex::new(Conn::default()) }) };
        // Dial now so a wrong password surfaces as a sign-in error right away.
        p.shared.run("connect", "/", async |_| Ok(()))?;
        Ok(p)
    }

    /// Disk shares of the server (admin shares such as `C$` / `IPC$` are left out).
    pub fn shares(&self) -> FsResult<Vec<smb2::ShareInfo>> {
        self.shared.run("list_shares", "/", async |cx| cx.client.list_shares().await)
    }
}

impl Provider for SmbProvider {
    fn describe(&self) -> String {
        format!("SMB {}", self.shared.uri.server_label())
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
        if split_share(path).is_none() {
            let shares = self.shares()?.into_iter().map(|s| RemoteEntry::dir(s.name, None)).collect();
            return Ok(crate::provider::sanitize_listing("SMB", path, shares));
        }
        self.shared
            .run("list", path, async |mut cx| {
                let t = tree(&mut cx.tree)?;
                let list = cx.client.list_directory(t, &cx.rel).await?;
                Ok(list
                    .into_iter()
                    .filter(|e| e.name != "." && e.name != "..")
                    .map(|e| RemoteEntry {
                        is_dir: e.is_directory,
                        is_link: false,
                        size: if e.is_directory { 0 } else { e.size },
                        modified: time(e.modified),
                        mode: None,
                        name: e.name,
                    })
                    .collect())
            })
            // smb2 has already decoded U+F026 back to `\` here, so a name planted as
            // `..\..\Other\x` by a Windows / Mac client is caught.
            .map(|l| crate::provider::sanitize_listing("SMB", path, l))
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let p = normalize_path(path);
        let name = remote_name_of(&p).to_string();
        match split_share(&p) {
            None => Ok(RemoteEntry::dir(self.shared.uri.host.clone(), None)),
            // The share root: connecting its tree proves it exists.
            Some((_, rel)) if rel.is_empty() => self.shared.run("stat", &p, async |_| Ok(RemoteEntry::dir(name, None))),
            Some(_) => self.shared.run("stat", &p, async |mut cx| {
                let t = tree(&mut cx.tree)?;
                let i = cx.client.stat(t, &cx.rel).await?;
                Ok(RemoteEntry {
                    name,
                    is_dir: i.is_directory,
                    is_link: false,
                    size: if i.is_directory { 0 } else { i.size },
                    modified: time(i.modified),
                    mode: None,
                })
            }),
        }
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        if split_share(path).is_none_or(|(_, rel)| rel.is_empty()) {
            return Err(FsError::with_message(
                FsErrorKind::IsDirectory,
                "open",
                Some(std::path::Path::new(path)),
                "is a folder",
            ));
        }
        let reader = self.shared.run("open", path, async |mut cx| {
            let t = tree(&mut cx.tree)?;
            cx.client.open_file_reader(t, &cx.rel).await
        })?;
        Ok(Box::new(SmbRead {
            size: reader.size(),
            reader: Some(reader),
            pos: offset,
            buf: Vec::new(),
            at: 0,
            path: normalize_path(path),
            timeout: self.shared.timeout(),
        }))
    }

    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        if split_share(path).is_none_or(|(_, rel)| rel.is_empty()) {
            return Err(share_level("create", path));
        }
        let resume = opts.resume;
        let (writer, resumed) = self.shared.run("create", path, async |mut cx| {
            let t = tree(&mut cx.tree)?;
            let existing = if resume { cx.client.stat(t, &cx.rel).await.ok().map(|i| i.size) } else { None };
            match existing {
                Some(n) if n > 0 => Ok((cx.client.create_file_writer_at(t, &cx.rel, n).await?, n)),
                _ => Ok((cx.client.create_file_writer(t, &cx.rel).await?, 0)),
            }
        })?;
        Ok(Box::new(SmbWrite {
            writer: Some(writer),
            path: normalize_path(path),
            resumed,
            shared: self.shared.clone(),
        }))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        if split_share(path).is_none_or(|(_, rel)| rel.is_empty()) {
            return Err(share_level("mkdir", path));
        }
        self.shared.run("mkdir", path, async |mut cx| {
            let t = tree(&mut cx.tree)?;
            cx.client.create_directory(t, &cx.rel).await
        })
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let (Some((fs, frel)), Some((ts, trel))) = (split_share(from), split_share(to)) else {
            return Err(share_level("rename", from));
        };
        if frel.is_empty() || trel.is_empty() {
            return Err(share_level("rename", from));
        }
        if !fs.eq_ignore_ascii_case(&ts) {
            return Err(FsError::with_message(
                FsErrorKind::CrossDevice,
                "rename",
                Some(std::path::Path::new(from)),
                "cannot move between shares",
            ));
        }
        self.shared.run("rename", from, async |mut cx| {
            let t = tree(&mut cx.tree)?;
            cx.client.rename(t, &cx.rel, &trel).await
        })
    }

    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
        if split_share(path).is_none_or(|(_, rel)| rel.is_empty()) {
            return Err(share_level("delete", path));
        }
        self.shared.run("delete", path, async |mut cx| {
            let t = tree(&mut cx.tree)?;
            if is_dir { cx.client.delete_directory(t, &cx.rel).await } else { cx.client.delete_file(t, &cx.rel).await }
        })
    }

    fn set_times(&self, path: &str, modified: SystemTime) -> FsResult<()> {
        if split_share(path).is_none_or(|(_, rel)| rel.is_empty()) {
            return Err(share_level("set_times", path));
        }
        self.shared.run("set_times", path, async |mut cx| {
            let t = tree(&mut cx.tree)?;
            cx.client.set_times(t, &cx.rel, FileTimes::new().set_modified(modified)).await
        })
    }

    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        Some(self.shared.uri.with_path(path))
    }
}

/// A positioned, chunked reader over one open handle.
struct SmbRead {
    reader: Option<FileReader>,
    size: u64,
    /// File offset of the next chunk.
    pos: u64,
    buf: Vec<u8>,
    at: usize,
    path: String,
    timeout: Duration,
}

impl Read for SmbRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.buf.len() {
            if self.pos >= self.size {
                return Ok(0);
            }
            let Some(r) = self.reader.as_ref() else { return Ok(0) };
            let (pos, t, path) = (self.pos, self.timeout, &self.path);
            let data = runtime().block_on(async move {
                match tokio::time::timeout(t, r.read_at(pos, READ_CHUNK)).await {
                    Ok(r) => r.map_err(|e| io::Error::other(smb_err("read", path, &e))),
                    Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "the server stopped sending data")),
                }
            })?;
            if data.is_empty() {
                self.size = self.pos; // the file shrank
                return Ok(0);
            }
            self.pos += data.len() as u64;
            self.buf = data;
            self.at = 0;
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

impl Drop for SmbRead {
    fn drop(&mut self) {
        if let Some(r) = self.reader.take() {
            runtime().spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(10), r.close()).await;
            });
        }
    }
}

struct SmbWrite {
    writer: Option<FileWriter>,
    path: String,
    resumed: u64,
    shared: Arc<Shared>,
}

impl SmbWrite {
    fn io_err(&self, e: &smb2::Error) -> io::Error {
        io::Error::other(smb_err("write", &self.path, e))
    }
}

impl Write for SmbWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let t = self.shared.timeout();
        let Some(w) = self.writer.as_mut() else { return Err(io::Error::other("closed")) };
        let r = runtime().block_on(async {
            match tokio::time::timeout(t, w.write_chunk(buf)).await {
                Ok(r) => Ok(r),
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "the server stopped accepting data")),
            }
        })?;
        r.map_err(|e| self.io_err(&e))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(()) // `commit` sends what is buffered, flushes and closes
    }
}

impl WriteStream for SmbWrite {
    fn resumed_at(&self) -> u64 {
        self.resumed
    }

    fn commit(mut self: Box<Self>) -> FsResult<()> {
        let Some(w) = self.writer.take() else { return Ok(()) };
        let t = self.shared.timeout();
        let r = runtime().block_on(async move { tokio::time::timeout(t, w.finish()).await });
        match r {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(smb_err("write", &self.path, &e)),
            Err(_) => Err(timed_out("write", &self.path)),
        }
    }

    fn abort(mut self: Box<Self>, keep_partial: bool) {
        if let Some(w) = self.writer.take() {
            let _ = runtime().block_on(async move { tokio::time::timeout(Duration::from_secs(10), w.abort()).await });
        }
        if !keep_partial {
            let _ = self.shared.run("delete", &self.path, async |mut cx| {
                let t = tree(&mut cx.tree)?;
                cx.client.delete_file(t, &cx.rel).await
            });
        }
    }
}

impl Drop for SmbWrite {
    fn drop(&mut self) {
        // Dropped without commit / abort: close the handle, keep the partial file.
        if let Some(w) = self.writer.take() {
            runtime().spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(10), w.abort()).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smb2::types::status::NtStatus;

    #[test]
    fn domain_user_forms() {
        assert_eq!(split_domain_user(r"CORP\alice"), ("CORP".into(), "alice".into()));
        assert_eq!(split_domain_user("CORP;alice"), ("CORP".into(), "alice".into()));
        assert_eq!(split_domain_user("alice"), (String::new(), "alice".into()));
        assert_eq!(split_domain_user("alice@corp.example"), (String::new(), "alice@corp.example".into()));
        assert_eq!(split_domain_user(""), (String::new(), String::new()));
    }

    #[test]
    fn share_paths() {
        assert_eq!(split_share("/"), None);
        assert_eq!(split_share(""), None);
        assert_eq!(split_share("/Media"), Some(("Media".into(), String::new())));
        assert_eq!(split_share("/Media/"), Some(("Media".into(), String::new())));
        assert_eq!(split_share("/Media/Films/a b.mkv"), Some(("Media".into(), "Films/a b.mkv".into())));
        assert_eq!(split_share("/Media/../x/./y"), Some(("x".into(), "y".into())));
    }

    fn proto(status: u32) -> smb2::Error {
        smb2::Error::Protocol { status: NtStatus(status), command: smb2::types::Command::Create }
    }

    #[test]
    fn error_mapping() {
        let k = |e: smb2::Error| smb_err("op", "/s/x", &e).kind;
        assert_eq!(k(proto(0xC000_0034)), FsErrorKind::NotFound); // OBJECT_NAME_NOT_FOUND
        assert_eq!(k(proto(0xC000_003A)), FsErrorKind::NotFound); // OBJECT_PATH_NOT_FOUND
        assert_eq!(k(proto(0xC000_0022)), FsErrorKind::PermissionDenied); // ACCESS_DENIED
        assert_eq!(k(proto(0xC000_006D)), FsErrorKind::PermissionDenied); // LOGON_FAILURE
        assert_eq!(k(proto(0xC000_0035)), FsErrorKind::AlreadyExists); // OBJECT_NAME_COLLISION
        assert_eq!(k(proto(0xC000_0043)), FsErrorKind::InUse); // SHARING_VIOLATION
        assert_eq!(k(proto(0xC000_0101)), FsErrorKind::NotEmpty); // DIRECTORY_NOT_EMPTY
        assert_eq!(k(proto(0xC000_00A2)), FsErrorKind::ReadOnlyFs); // MEDIA_WRITE_PROTECTED
        assert_eq!(k(proto(0xC000_007F)), FsErrorKind::NoSpace); // DISK_FULL
        assert_eq!(k(proto(0xC000_00CC)), FsErrorKind::NotFound); // BAD_NETWORK_NAME
        assert_eq!(k(proto(0xC000_0033)), FsErrorKind::InvalidName); // OBJECT_NAME_INVALID
        assert_eq!(k(proto(0xC000_00BA)), FsErrorKind::IsDirectory); // FILE_IS_A_DIRECTORY
        assert_eq!(k(smb2::Error::Auth { message: "x".into() }), FsErrorKind::PermissionDenied);
        assert_eq!(k(smb2::Error::Disconnected), FsErrorKind::Io);
        assert_eq!(k(smb2::Error::Cancelled), FsErrorKind::Interrupted);
        assert!(is_dead(&smb2::Error::Disconnected));
        assert!(!is_dead(&proto(0xC000_0034)));
        let e = smb_err("connect", "/", &smb2::Error::Auth { message: "x".into() });
        assert!(e.message.contains("sign-in"), "{}", e.message);
    }

    /// A NetShareEnumAll RESPONSE PDU (level 1) as Samba sends it: `Media` (disk),
    /// `IPC$` (IPC, special) and `Print` (printer queue).
    fn share_enum_fixture() -> Vec<u8> {
        fn ndr_str(out: &mut Vec<u8>, s: &str) {
            let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
            let n = units.len() as u32;
            out.extend_from_slice(&n.to_le_bytes()); // max count
            out.extend_from_slice(&0u32.to_le_bytes()); // offset
            out.extend_from_slice(&n.to_le_bytes()); // actual count
            for u in units {
                out.extend_from_slice(&u.to_le_bytes());
            }
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        let shares = [("Media", 0u32, "Films and music"), ("IPC$", 0x8000_0003, "IPC Service"), ("Print", 1, "")];
        let mut stub = Vec::new();
        stub.extend_from_slice(&1u32.to_le_bytes()); // level
        stub.extend_from_slice(&1u32.to_le_bytes()); // union discriminant
        stub.extend_from_slice(&0x0002_0000u32.to_le_bytes()); // container pointer
        stub.extend_from_slice(&(shares.len() as u32).to_le_bytes()); // count
        stub.extend_from_slice(&0x0002_0004u32.to_le_bytes()); // array pointer
        stub.extend_from_slice(&(shares.len() as u32).to_le_bytes()); // max count
        for (i, (_, ty, _)) in shares.iter().enumerate() {
            stub.extend_from_slice(&(0x0002_0008u32 + 8 * i as u32).to_le_bytes()); // name ptr
            stub.extend_from_slice(&ty.to_le_bytes());
            stub.extend_from_slice(&(0x0002_000Cu32 + 8 * i as u32).to_le_bytes()); // comment ptr
        }
        for (name, _, comment) in shares {
            ndr_str(&mut stub, name);
            ndr_str(&mut stub, comment);
        }
        stub.extend_from_slice(&(shares.len() as u32).to_le_bytes()); // total entries
        stub.extend_from_slice(&0u32.to_le_bytes()); // resume handle pointer (null)
        stub.extend_from_slice(&0u32.to_le_bytes()); // WERROR success
        let mut pdu = vec![5, 0, 2, 0x03, 0x10, 0, 0, 0]; // v5.0 RESPONSE, first+last frag, LE
        let frag_len = (24 + stub.len()) as u16;
        pdu.extend_from_slice(&frag_len.to_le_bytes());
        pdu.extend_from_slice(&0u16.to_le_bytes()); // auth length
        pdu.extend_from_slice(&2u32.to_le_bytes()); // call id
        pdu.extend_from_slice(&(stub.len() as u32).to_le_bytes()); // alloc hint
        pdu.extend_from_slice(&0u16.to_le_bytes()); // context id
        pdu.extend_from_slice(&[0, 0]); // cancel count, reserved
        pdu.extend_from_slice(&stub);
        pdu
    }

    #[test]
    fn share_enum_ndr_parsing() {
        let all = smb2::rpc::srvsvc::parse_net_share_enum_all_response(&share_enum_fixture()).unwrap();
        let names: Vec<_> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Media", "IPC$", "Print"]);
        assert_eq!(all[0].comment, "Films and music");
        assert_eq!(all[1].share_type, 0x8000_0003);
        let disk = smb2::rpc::srvsvc::filter_disk_shares(all);
        assert_eq!(disk.len(), 1);
        assert_eq!(disk[0].name, "Media");
        // Truncated PDUs are errors, not panics.
        let fx = share_enum_fixture();
        for cut in [0, 10, 30, fx.len() / 2] {
            assert!(smb2::rpc::srvsvc::parse_net_share_enum_all_response(&fx[..cut]).is_err(), "cut {cut}");
        }
    }
}
