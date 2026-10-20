//! Quick share (`Action::QuickShare`): the selection served read-only over HTTP on the
//! local network (`soyuz_vfs::share`) while the window / dialog is open, its address as
//! text and as a QR code to scan with a phone. `[remote] share_minutes` ends it by
//! itself, `[remote] share_port` fixes the port (0 = a free one).
//!
//! Shared by the GUI window (`ui/tools/share.rs`) and the terminal UI dialog
//! (`input/dialogs/share.rs`); closing either stops the server ([`QuickShare`]'s drop).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use soyuz_core::config::RemoteSection;
use soyuz_core::{SizeStyle, format_eta, format_size};
use soyuz_vfs::share::{ShareServer, lan_addresses};

/// A QR code: `width` × `width` modules, without the quiet zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Qr {
    pub width: usize,
    dark: Vec<bool>,
}

impl Qr {
    /// Encode `text` (low error correction: a screen is clean, and the code stays small
    /// enough for a terminal); `None` when it does not fit.
    pub fn encode(text: &str) -> Option<Qr> {
        let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::L).ok()?;
        let width = code.width();
        let dark = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
        Some(Qr { width, dark })
    }

    /// Whether module (`x`, `y`) is dark; outside the code (the quiet zone) it is light.
    pub fn dark(&self, x: isize, y: isize) -> bool {
        let w = self.width as isize;
        (0..w).contains(&x) && (0..w).contains(&y) && self.dark[(y * w + x) as usize]
    }

    /// The code as text for a terminal, two module rows per line (`▀`, `▄`, `█`) with a
    /// quiet zone of `quiet` modules. Filled cells are the **light** modules, so the code
    /// reads right on a dark terminal background (scanners also read it inverted).
    pub fn half_blocks(&self, quiet: usize) -> Vec<String> {
        let q = quiet as isize;
        let w = self.width as isize;
        let light = |x, y| !self.dark(x, y);
        let mut out = Vec::new();
        let mut y = -q;
        while y < w + q {
            let line = (-q..w + q)
                .map(|x| match (light(x, y), y + 1 < w + q && light(x, y + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                })
                .collect();
            out.push(line);
            y += 2;
        }
        out
    }
}

/// A running (or ended) quick share.
pub struct QuickShare {
    pub items: Vec<PathBuf>,
    server: Option<ShareServer>,
    /// The addresses other devices may reach this machine at (likeliest first).
    pub addresses: Vec<IpAddr>,
    /// The address shown (and encoded in the QR code).
    pub picked: usize,
    qr: Option<Qr>,
    /// Starting failed (the port is taken, nothing left to share, …).
    pub error: Option<String>,
    /// Stopped by the user.
    stopped: bool,
    minutes: u64,
}

impl QuickShare {
    /// Share `items` on every IPv4 interface per `[remote] share_port` / `share_minutes`.
    /// Debug hook: `SOYUZ_SHARE_BIND=127.0.0.1` listens (and offers) only that address,
    /// so scripted runs stay off the network (and raise no firewall prompt).
    pub fn start(items: Vec<PathBuf>, cfg: &RemoteSection) -> QuickShare {
        match std::env::var("SOYUZ_SHARE_BIND").ok().and_then(|v| v.parse::<IpAddr>().ok()) {
            Some(ip) => Self::start_on(items, cfg, ip, vec![ip]),
            None => Self::start_on(items, cfg, IpAddr::V4(Ipv4Addr::UNSPECIFIED), lan_addresses()),
        }
    }

    /// [`QuickShare::start`] bound to `bind`, offering `addresses` (tests: loopback).
    pub fn start_on(items: Vec<PathBuf>, cfg: &RemoteSection, bind: IpAddr, addresses: Vec<IpAddr>) -> QuickShare {
        let lifetime = (cfg.share_minutes > 0).then(|| Duration::from_secs(cfg.share_minutes * 60));
        let (server, error) = match ShareServer::start(&items, bind, cfg.share_port, lifetime) {
            Ok(s) => (Some(s), None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, Some(tr!("tools-share-nothing").to_string())),
            Err(e) => (None, Some(tr!("tools-share-failed", error = e.to_string()))),
        };
        let mut s = QuickShare {
            items,
            server,
            addresses,
            picked: 0,
            qr: None,
            error,
            stopped: false,
            minutes: cfg.share_minutes,
        };
        s.pick(0);
        s
    }

    /// The address shown, when serving and the machine has a network address.
    pub fn url(&self) -> Option<String> {
        let server = self.server.as_ref()?;
        Some(server.url(*self.addresses.get(self.picked)?))
    }

    /// Show address `i` (and its QR code).
    pub fn pick(&mut self, i: usize) {
        self.picked = i.min(self.addresses.len().saturating_sub(1));
        self.qr = self.url().and_then(|u| Qr::encode(&u));
    }

    pub fn qr(&self) -> Option<&Qr> {
        self.qr.as_ref().filter(|_| self.running())
    }

    /// Serving: started, not stopped, not expired.
    pub fn running(&self) -> bool {
        self.server.as_ref().is_some_and(|s| s.running())
    }

    pub fn stop(&mut self) {
        if let Some(s) = &self.server {
            s.stop();
        }
        self.stopped = true;
    }

    /// The subtitle: the one item's name or "3 items".
    pub fn what(&self) -> String {
        match self.items.as_slice() {
            [one] => soyuz_core::entry::display_name(one),
            many => tr!("tools-share-items", count = many.len()),
        }
    }

    /// Why there is nothing to scan (failed to start, no network, stopped, expired).
    pub fn problem(&self) -> Option<String> {
        if let Some(e) = &self.error {
            return Some(e.clone());
        }
        if self.stopped {
            return Some(tr!("tools-share-stopped").to_string());
        }
        if !self.running() {
            return Some(tr!("tools-share-expired", minutes = self.minutes));
        }
        if self.addresses.is_empty() {
            return Some(tr!("tools-share-no-network").to_string());
        }
        None
    }

    /// What happened so far and when it ends ("2 downloads · 12 MiB sent · stops in 28 min").
    pub fn status(&self) -> String {
        let Some(server) = &self.server else { return String::new() };
        let st = server.stats();
        let mut parts = Vec::new();
        if st.requests == 0 {
            parts.push(tr!("tools-share-waiting").to_string());
        } else {
            parts.push(tr!("tools-share-devices", count = st.clients.len().max(1)));
            parts.push(tr!("tools-share-downloads", count = st.downloads));
            parts.push(tr!("tools-share-sent", size = format_size(st.bytes_sent, SizeStyle::Binary)));
        }
        if self.running() {
            parts.push(match server.expires() {
                Some(t) => {
                    let left = t.saturating_duration_since(Instant::now()).as_secs_f64();
                    tr!("tools-share-ends", time = format_eta(left.max(1.0)))
                }
                None => tr!("tools-share-until-closed").to_string(),
            });
        }
        parts.join(" \u{b7} ")
    }
}

impl std::fmt::Debug for QuickShare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuickShare")
            .field("items", &self.items)
            .field("url", &self.url())
            .field("running", &self.running())
            .finish()
    }
}

impl Drop for QuickShare {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use super::*;

    #[test]
    fn shares_until_stopped_and_draws_a_code() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("note.txt");
        std::fs::write(&f, b"hi").unwrap();
        let cfg = RemoteSection { share_minutes: 5, ..RemoteSection::default() };
        let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let mut s = QuickShare::start_on(vec![f], &cfg, lo, vec![lo]);
        assert!(s.running() && s.problem().is_none(), "{:?}", s.problem());
        assert_eq!(s.what(), "note.txt");
        let url = s.url().unwrap();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        let qr = s.qr().unwrap();
        assert!(qr.width >= 21);
        let lines = qr.half_blocks(2);
        assert_eq!(lines.len(), (qr.width + 4).div_ceil(2));
        assert!(lines.iter().all(|l| l.chars().count() == qr.width + 4));
        assert!(lines[0].chars().all(|c| c == '█'), "the quiet zone is light");
        assert!(s.status().contains(&tr!("tools-share-waiting").to_string()));
        // Fetch the file over the advertised address.
        let rest = url.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_once('/').unwrap();
        let mut c = TcpStream::connect(host).unwrap();
        write!(c, "GET /{path}0/note.txt HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut body = String::new();
        c.read_to_string(&mut body).unwrap();
        assert!(body.ends_with("\r\n\r\nhi"), "{body}");
        let status = s.status();
        assert!(status.contains(&tr!("tools-share-downloads", count = 1)), "{status}");
        s.stop();
        assert!(!s.running() && s.qr().is_none());
        assert_eq!(s.problem().unwrap(), tr!("tools-share-stopped"));
    }

    #[test]
    fn no_network_and_nothing_to_share() {
        let cfg = RemoteSection::default();
        let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let d = tempfile::tempdir().unwrap();
        let s = QuickShare::start_on(vec![d.path().to_path_buf(), d.path().join("x")], &cfg, lo, vec![]);
        assert!(s.running() && s.url().is_none());
        assert_eq!(s.problem().unwrap(), tr!("tools-share-no-network"));
        assert_eq!(s.what(), tr!("tools-share-items", count = 2));
        let s = QuickShare::start_on(vec![d.path().join("missing")], &cfg, lo, vec![lo]);
        assert_eq!(s.problem().unwrap(), tr!("tools-share-nothing"));
        assert!(!s.running());
    }
}
