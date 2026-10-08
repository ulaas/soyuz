//! FTP and explicit FTPS (`AUTH TLS`, rustls + the OS certificate verifier) over the
//! pure-Rust `suppaftp` client. Listings use MLSD (falling back to LIST in Unix or DOS
//! format) on a shared control connection; every transfer opens its own connection so
//! a long download never blocks browsing. Resume: `REST` for downloads, `APPE` for
//! uploads.

use std::io::{Read, Write};
use std::net::ToSocketAddrs;
use std::sync::Arc;

use parking_lot::Mutex;
use soyuz_core::{FsError, FsErrorKind, FsResult};
use suppaftp::list::{File as FtpFile, ListParser};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode, RustlsConnector, RustlsFtpStream, TlsStream, TransferStream};

use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream, err};
use crate::registry::{Credentials, net_settings};
use crate::uri::{RemoteUri, Scheme, name_of, normalize_path, parent_path};

type Conn = RustlsFtpStream;

pub struct FtpProvider {
    uri: RemoteUri,
    user: String,
    password: String,
    secure: bool,
    control: Mutex<Option<Conn>>,
}

fn ftp_err(op: &'static str, path: &str, e: FtpError) -> FsError {
    let (kind, msg) = match &e {
        FtpError::UnexpectedResponse(r) => {
            let body = String::from_utf8_lossy(&r.body).trim().to_string();
            let kind = match r.status.code() {
                530 | 532 => FsErrorKind::PermissionDenied,
                550 => {
                    let l = body.to_ascii_lowercase();
                    if l.contains("permission") || l.contains("denied") {
                        FsErrorKind::PermissionDenied
                    } else if l.contains("exist") && !l.contains("not") {
                        FsErrorKind::AlreadyExists
                    } else {
                        FsErrorKind::NotFound
                    }
                }
                552 => FsErrorKind::NoSpace,
                553 => FsErrorKind::InvalidName,
                _ => FsErrorKind::Io,
            };
            (kind, if body.is_empty() { format!("server replied {}", r.status.code()) } else { body })
        }
        FtpError::ConnectionError(io) => (FsErrorKind::Io, format!("connection: {io}")),
        FtpError::SecureError(s) => (FsErrorKind::Io, format!("TLS: {s}")),
        other => (FsErrorKind::Io, other.to_string()),
    };
    FsError::with_message(kind, op, Some(std::path::Path::new(path)), msg)
}

impl FtpProvider {
    pub fn connect(uri: RemoteUri, creds: Credentials) -> FsResult<Self> {
        let user = creds.user.clone().or_else(|| uri.user.clone()).unwrap_or_else(|| "anonymous".into());
        let password =
            creds.secret.clone().unwrap_or_else(|| if user == "anonymous" { "soyuz@".into() } else { String::new() });
        let p = FtpProvider { secure: uri.scheme == Scheme::Ftps, uri, user, password, control: Mutex::new(None) };
        // Connect once now so bad credentials surface on the first listing.
        let c = p.open_conn()?;
        *p.control.lock() = Some(c);
        Ok(p)
    }

    fn open_conn(&self) -> FsResult<Conn> {
        let net = net_settings();
        let addr = (self.uri.host.as_str(), self.uri.effective_port())
            .to_socket_addrs()
            .map_err(|e| err(FsErrorKind::NotFound, "connect", &self.uri.host, format!("cannot resolve: {e}")))?
            .next()
            .ok_or_else(|| err(FsErrorKind::NotFound, "connect", &self.uri.host, "cannot resolve the server"))?;
        let mut c = Conn::connect_timeout(addr, net.connect_timeout).map_err(|e| ftp_err("connect", "/", e))?;
        if self.secure {
            crate::rt::ensure_crypto_provider();
            use rustls_platform_verifier::ConfigVerifierExt;
            let cfg = rustls::ClientConfig::with_platform_verifier()
                .map_err(|e| FsError::with_message(FsErrorKind::Io, "connect", None, format!("TLS setup: {e}")))?;
            c = c
                .into_secure(RustlsConnector::from(Arc::new(cfg)), &self.uri.host)
                .map_err(|e| ftp_err("connect", "/", e))?;
        }
        c.login(self.user.as_str(), self.password.as_str()).map_err(|e| ftp_err("login", "/", e))?;
        c.transfer_type(FileType::Binary).map_err(|e| ftp_err("connect", "/", e))?;
        c.set_mode(if self.uri.host.contains(':') { Mode::ExtendedPassive } else { Mode::Passive });
        let _ = c.get_ref().set_read_timeout(Some(net.io_timeout));
        Ok(c)
    }

    /// Run `f` on the shared control connection, reconnecting once if it dropped.
    fn with_control<T>(
        &self,
        op: &'static str,
        path: &str,
        f: impl Fn(&mut Conn) -> Result<T, FtpError>,
    ) -> FsResult<T> {
        let mut guard = self.control.lock();
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(self.open_conn()?);
            }
            let c = guard.as_mut().expect("connected");
            match f(c) {
                Ok(v) => return Ok(v),
                Err(FtpError::ConnectionError(_) | FtpError::BadResponse) if attempt == 0 => {
                    *guard = None; // dropped: reconnect and retry once
                }
                Err(e) => return Err(ftp_err(op, path, e)),
            }
        }
        Err(err(FsErrorKind::Io, op, path, "the connection keeps dropping"))
    }

    fn list_raw(&self, path: &str) -> FsResult<Vec<FtpFile>> {
        let p = normalize_path(path);
        let lines = self.with_control("list", &p, |c| c.mlsd(Some(&p)));
        let (lines, mlsd) = match lines {
            Ok(l) => (l, true),
            Err(e) if e.kind == FsErrorKind::Io || e.kind == FsErrorKind::NotFound && path != "/" => {
                (self.with_control("list", &p, |c| c.list(Some(&p)))?, false)
            }
            Err(e) => return Err(e),
        };
        Ok(lines
            .iter()
            .filter_map(|l| {
                if mlsd {
                    ListParser::parse_mlsd(l).ok()
                } else {
                    ListParser::parse_posix(l).or_else(|_| ListParser::parse_dos(l)).ok()
                }
            })
            .filter(|f| !matches!(f.name(), "." | ".." | ""))
            .collect())
    }
}

fn entry_of(f: &FtpFile) -> RemoteEntry {
    RemoteEntry {
        name: name_of(f.name()).to_string(),
        is_dir: f.is_directory(),
        is_link: f.is_symlink(),
        size: f.size() as u64,
        modified: Some(f.modified()).filter(|m| *m > std::time::UNIX_EPOCH),
        mode: None,
    }
}

impl Provider for FtpProvider {
    fn describe(&self) -> String {
        format!("{} {}", self.uri.scheme.label(), self.uri.server_label())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            range_read: true,
            resume_write: true,
            atomic_write: false,
            rename: true,
            rename_replaces: false,
            set_times: false,
            watch: false,
        }
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        Ok(self.list_raw(path)?.iter().map(entry_of).collect())
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let p = normalize_path(path);
        let Some(parent) = parent_path(&p) else { return Ok(RemoteEntry::dir("", None)) };
        let name = name_of(&p);
        self.list_raw(&parent)?
            .iter()
            .find(|f| name_of(f.name()) == name)
            .map(entry_of)
            .ok_or_else(|| err(FsErrorKind::NotFound, "stat", &p, "not found"))
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        let mut c = self.open_conn()?;
        if offset > 0 {
            c.resume_transfer(offset as usize).map_err(|e| ftp_err("open", path, e))?;
        }
        let s = c.retr_as_stream(normalize_path(path)).map_err(|e| ftp_err("open", path, e))?;
        Ok(Box::new(FtpRead { s: Some(s), conn: Some(c) }))
    }

    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        let p = normalize_path(path);
        let mut c = self.open_conn()?;
        let existing = if opts.resume { c.size(&p).ok().map(|n| n as u64) } else { None };
        let s = match existing {
            Some(n) if n > 0 => c.append_with_stream(&p),
            _ => c.put_with_stream(&p),
        }
        .map_err(|e| ftp_err("create", &p, e))?;
        Ok(Box::new(FtpWrite { s: Some(s), conn: Some(c), path: p, resumed: existing.unwrap_or(0) }))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        let p = normalize_path(path);
        self.with_control("mkdir", &p, |c| c.mkdir(&p))
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let (f, t) = (normalize_path(from), normalize_path(to));
        self.with_control("rename", &f, |c| c.rename(&f, &t))
    }

    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
        let p = normalize_path(path);
        if is_dir {
            self.with_control("delete", &p, |c| c.rmdir(&p))
        } else {
            self.with_control("delete", &p, |c| c.rm(&p))
        }
    }

    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        Some(self.uri.with_path(path))
    }
}

struct FtpRead<S: TlsStream> {
    s: Option<TransferStream<S>>,
    conn: Option<Conn>,
}

impl<S: TlsStream> Read for FtpRead<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.s.as_mut() {
            Some(s) => s.read(buf),
            None => Ok(0),
        }
    }
}

impl<S: TlsStream> Drop for FtpRead<S> {
    fn drop(&mut self) {
        drop(self.s.take());
        if let Some(mut c) = self.conn.take() {
            let _ = c.quit();
        }
    }
}

struct FtpWrite<S: TlsStream> {
    s: Option<TransferStream<S>>,
    conn: Option<Conn>,
    path: String,
    resumed: u64,
}

impl<S: TlsStream> Write for FtpWrite<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.s.as_mut() {
            Some(s) => s.write(buf),
            None => Err(std::io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.s.as_mut() {
            Some(s) => s.flush(),
            None => Ok(()),
        }
    }
}

impl<S: TlsStream + Send + 'static> WriteStream for FtpWrite<S> {
    fn resumed_at(&self) -> u64 {
        self.resumed
    }

    fn commit(mut self: Box<Self>) -> FsResult<()> {
        let r = match self.s.take() {
            Some(s) => s.finish().map_err(|e| ftp_err("write", &self.path, e)),
            None => Ok(()),
        };
        if let Some(mut c) = self.conn.take() {
            let _ = c.quit();
        }
        r
    }

    fn abort(mut self: Box<Self>, keep_partial: bool) {
        drop(self.s.take());
        if let Some(mut c) = self.conn.take() {
            if !keep_partial {
                let _ = c.rm(&self.path);
            }
            let _ = c.quit();
        }
    }
}

impl<S: TlsStream> Drop for FtpWrite<S> {
    fn drop(&mut self) {
        drop(self.s.take());
        if let Some(mut c) = self.conn.take() {
            let _ = c.quit();
        }
    }
}
