//! "On this network" in the Connect to server dialog: SFTP / WebDAV / FTP (and SMB off
//! Windows) servers that announce themselves over mDNS / DNS-SD (`soyuz_vfs::discover`),
//! looked for on a worker while the dialog is open (`[remote] discover`). Picking one
//! fills the form.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::Receiver;
use soyuz_vfs::Scheme;
use soyuz_vfs::discover;
/// What the network announced for a server (named by the frontends, which do not depend
/// on `soyuz-vfs` themselves).
pub use soyuz_vfs::discover::Found;

use crate::host::Wake;
use crate::models::remote::{ConnectForm, PROTOCOLS};

/// How long one look around the network takes.
const BROWSE: Duration = Duration::from_millis(2500);

/// A server found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanServer {
    pub found: Found,
    /// What the form connects to: the host name when the system resolves it, else the
    /// address it announced.
    pub host: String,
}

impl LanServer {
    /// The form's protocol for this server.
    pub fn proto(&self) -> Scheme {
        base_scheme(self.found.scheme)
    }

    /// "Shared Files" + "WebDAV · nas.local:8080".
    pub fn detail(&self) -> String {
        let proto = PROTOCOLS
            .iter()
            .find(|(s, _)| *s == base_scheme(self.found.scheme))
            .map_or_else(|| self.found.scheme.to_string(), |(_, l)| l.to_string());
        let tls = matches!(self.found.scheme, Scheme::WebDavs | Scheme::Ftps);
        let port = if self.found.port == self.found.scheme.default_port() {
            String::new()
        } else {
            format!(":{}", self.found.port)
        };
        format!("{proto}{} · {}{port}", if tls { " (TLS)" } else { "" }, self.found.host)
    }
}

/// The form's protocol for a scheme (TLS is a checkbox there).
fn base_scheme(s: Scheme) -> Scheme {
    match s {
        Scheme::WebDavs => Scheme::WebDav,
        Scheme::Ftps => Scheme::Ftp,
        other => other,
    }
}

/// The services asked for: those the form offers on this platform.
fn services() -> Vec<(&'static str, Scheme)> {
    discover::SERVICES.iter().copied().filter(|(_, s)| PROTOCOLS.iter().any(|(p, _)| *p == base_scheme(*s))).collect()
}

/// One look around the network, run on a worker.
#[derive(Debug)]
pub struct Discovery {
    rx: Option<Receiver<Vec<LanServer>>>,
    stop: Arc<AtomicBool>,
    pub servers: Vec<LanServer>,
}

impl Discovery {
    /// Start looking (`None` when `[remote] discover` is off).
    pub fn start(enabled: bool, wake: &Wake) -> Option<Discovery> {
        if !enabled {
            return None;
        }
        let mut d = Discovery { rx: None, stop: Arc::new(AtomicBool::new(false)), servers: Vec::new() };
        d.again(wake);
        Some(d)
    }

    /// Look again (the list is kept until the new answers arrive).
    pub fn again(&mut self, wake: &Wake) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let stop = self.stop.clone();
        let wake = wake.clone();
        let spawned = std::thread::Builder::new().name("soyuz-mdns".into()).spawn(move || {
            let found = discover::browse(&services(), BROWSE, &stop).unwrap_or_else(|e| {
                log::debug!("network discovery failed: {e}");
                Vec::new()
            });
            // Resolving the host names is a blocking lookup: done here, not on the UI.
            let servers = found.into_iter().map(|f| LanServer { host: f.connect_host(), found: f }).collect();
            let _ = tx.send(servers);
            wake();
        });
        if spawned.is_ok() {
            self.rx = Some(rx);
        }
    }

    /// Take finished results; `true` when a look just finished.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(servers) => {
                self.rx = None;
                self.servers = servers;
                true
            }
            Err(crossbeam_channel::TryRecvError::Empty) => false,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.rx = None;
                true
            }
        }
    }

    /// A finished look with these servers and no worker (tests, previews).
    pub fn fixed(servers: Vec<LanServer>) -> Discovery {
        Discovery { rx: None, stop: Arc::new(AtomicBool::new(false)), servers }
    }

    pub fn searching(&self) -> bool {
        self.rx.is_some()
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl ConnectForm {
    /// Fill the form for a server found on the network (sign-in fields are left alone).
    pub fn fill_lan(&mut self, s: &LanServer) {
        let f = &s.found;
        self.proto = base_scheme(f.scheme);
        self.tls = matches!(f.scheme, Scheme::WebDavs | Scheme::Ftps);
        self.host = s.host.clone();
        self.port = if f.port == f.scheme.default_port() { String::new() } else { f.port.to_string() };
        self.path = f.path.clone().unwrap_or_default();
        if let Some(u) = &f.user {
            self.user = u.clone();
        }
        if self.edit.is_none() {
            self.name = f.name.clone();
        }
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(scheme: Scheme, port: u16) -> LanServer {
        LanServer {
            found: Found {
                scheme,
                name: "Shared Files".into(),
                host: "nas.local".into(),
                addrs: vec![],
                port,
                path: Some("/dav".into()),
                user: Some("ann".into()),
            },
            host: "192.168.1.20".into(),
        }
    }

    #[test]
    fn picking_a_server_fills_the_form() {
        let mut f = ConnectForm::new();
        f.secret = "kept".into();
        f.fill_lan(&server(Scheme::WebDavs, 8443));
        assert_eq!((f.proto, f.tls), (Scheme::WebDav, true));
        assert_eq!((f.host.as_str(), f.port.as_str(), f.path.as_str()), ("192.168.1.20", "8443", "/dav"));
        assert_eq!((f.user.as_str(), f.name.as_str(), f.secret.as_str()), ("ann", "Shared Files", "kept"));
        assert_eq!(f.build().unwrap().display(), "webdavs://ann@192.168.1.20:8443/dav");
        f.fill_lan(&server(Scheme::Sftp, 22));
        assert_eq!((f.proto, f.port.as_str()), (Scheme::Sftp, ""), "the default port stays empty");
        assert_eq!(server(Scheme::WebDav, 80).detail(), "WebDAV · nas.local");
        assert_eq!(server(Scheme::WebDavs, 8443).detail(), "WebDAV (TLS) · nas.local:8443");
    }

    #[test]
    fn asks_only_for_offered_protocols() {
        let s = services();
        assert!(s.iter().any(|(n, _)| *n == "_sftp-ssh._tcp.local"));
        assert_eq!(s.iter().any(|(_, k)| *k == Scheme::Smb), !cfg!(windows), "Windows opens \\\\server itself");
        assert!(Discovery::start(false, &(Arc::new(|| {}) as Wake)).is_none());
    }
}
