//! Servers on the local network through multicast DNS service discovery (RFC 6762 /
//! RFC 6763): SFTP (`_sftp-ssh._tcp`, `_ssh._tcp`), WebDAV (`_webdav._tcp`,
//! `_webdavs._tcp`), FTP (`_ftp._tcp`) and SMB (`_smb._tcp`).
//!
//! Queries are one-shot "legacy unicast" queries (RFC 6762 §6.7): they go to
//! `224.0.0.251:5353` from an ephemeral port, and responders answer straight to that
//! port. Nothing binds 5353 or joins the multicast group, so the system's own mDNS
//! responder (Bonjour, Avahi, Windows' DNS client) is undisturbed. The query goes out on
//! every IPv4 interface. Instances whose answer lacks the SRV / TXT / address records get
//! a follow-up query.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::Scheme;

/// The mDNS group and port.
pub const MDNS: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 251)), 5353);

/// Service types asked for, with the scheme each one opens.
pub const SERVICES: &[(&str, Scheme)] = &[
    ("_sftp-ssh._tcp.local", Scheme::Sftp),
    ("_ssh._tcp.local", Scheme::Sftp),
    ("_webdavs._tcp.local", Scheme::WebDavs),
    ("_webdav._tcp.local", Scheme::WebDav),
    ("_ftp._tcp.local", Scheme::Ftp),
    ("_smb._tcp.local", Scheme::Smb),
];

const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_AAAA: u16 = 28;
const TYPE_SRV: u16 = 33;
const CLASS_IN: u16 = 1;

/// A server found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub scheme: Scheme,
    /// The service's own name ("Living room NAS").
    pub name: String,
    /// Its host name without the trailing dot (`nas.local`).
    pub host: String,
    /// Addresses the responder gave for the host (IPv4 first).
    pub addrs: Vec<IpAddr>,
    pub port: u16,
    /// TXT `path=`: the folder to open (WebDAV, FTP).
    pub path: Option<String>,
    /// TXT `u=`: a user name.
    pub user: Option<String>,
}

impl Found {
    /// What to connect to: the host name when the system resolves it (to one of the
    /// advertised addresses, when there are any), else the first address.
    pub fn connect_host(&self) -> String {
        use std::net::ToSocketAddrs;
        let resolved: Vec<IpAddr> =
            (self.host.as_str(), self.port).to_socket_addrs().map(|a| a.map(|s| s.ip()).collect()).unwrap_or_default();
        let name_works =
            !resolved.is_empty() && (self.addrs.is_empty() || resolved.iter().any(|a| self.addrs.contains(a)));
        match self.addrs.first() {
            Some(a) if !name_works => a.to_string(),
            _ => self.host.clone(),
        }
    }
}

/// A DNS name as labels (labels may contain dots); compared without regard to case.
type Name = Vec<String>;

fn lower(n: &[String]) -> Name {
    n.iter().map(|l| l.to_ascii_lowercase()).collect()
}

fn name_of(s: &str) -> Name {
    s.trim_end_matches('.').split('.').map(|l| l.to_ascii_lowercase()).collect()
}

fn same(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// A query with one question per `(name, type)`.
pub fn query(id: u16, questions: &[(&[String], u16)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + questions.len() * 32);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // flags: standard query
    out.extend_from_slice(&(questions.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    for (name, ty) in questions {
        for label in name.iter() {
            let b = label.as_bytes();
            out.push(b.len().min(63) as u8);
            out.extend_from_slice(&b[..b.len().min(63)]);
        }
        out.push(0);
        out.extend_from_slice(&ty.to_be_bytes());
        out.extend_from_slice(&CLASS_IN.to_be_bytes());
    }
    out
}

/// One resource record of a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Ptr { name: Name, target: Name },
    Srv { name: Name, port: u16, target: Name },
    Txt { name: Name, entries: Vec<(String, String)> },
    Addr { name: Name, addr: IpAddr },
}

/// Read a (possibly compressed) name at `at`; returns it and the offset after it.
fn read_name(msg: &[u8], mut at: usize) -> Option<(Name, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    let mut jumps = 0;
    let mut total = 0usize;
    loop {
        let len = *msg.get(at)? as usize;
        match len & 0xC0 {
            0x00 if len == 0 => {
                return Some((labels, end.unwrap_or(at + 1)));
            }
            0x00 => {
                let label = msg.get(at + 1..at + 1 + len)?;
                total += len + 1;
                if total > 255 {
                    return None;
                }
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + len;
            }
            0xC0 => {
                let ptr = (usize::from(*msg.get(at)? & 0x3F) << 8) | usize::from(*msg.get(at + 1)?);
                end.get_or_insert(at + 2);
                jumps += 1;
                if jumps > 32 || ptr >= msg.len() {
                    return None;
                }
                at = ptr;
            }
            _ => return None,
        }
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

/// The records of a response (answers, authority and additional sections). `None` for
/// something that isn't a well-formed DNS response.
pub fn parse(msg: &[u8]) -> Option<Vec<Record>> {
    let flags = u16_at(msg, 2)?;
    if flags & 0x8000 == 0 {
        return None; // a query, not a response
    }
    let qd = u16_at(msg, 4)?;
    let rr = u32::from(u16_at(msg, 6)?) + u32::from(u16_at(msg, 8)?) + u32::from(u16_at(msg, 10)?);
    let mut at = 12;
    for _ in 0..qd {
        at = read_name(msg, at)?.1 + 4;
    }
    let mut out = Vec::new();
    for _ in 0..rr {
        let (name, next) = read_name(msg, at)?;
        let ty = u16_at(msg, next)?;
        let len = usize::from(u16_at(msg, next + 8)?);
        let data = next + 10;
        let rdata = msg.get(data..data + len)?;
        at = data + len;
        match ty {
            TYPE_PTR => {
                if let Some((target, _)) = read_name(msg, data) {
                    out.push(Record::Ptr { name, target });
                }
            }
            TYPE_SRV if len >= 7 => {
                let port = u16_at(rdata, 4)?;
                if let Some((target, _)) = read_name(msg, data + 6) {
                    out.push(Record::Srv { name, port, target });
                }
            }
            TYPE_TXT => {
                let mut entries = Vec::new();
                let mut i = 0;
                while i < rdata.len() {
                    let n = usize::from(rdata[i]);
                    let Some(s) = rdata.get(i + 1..i + 1 + n) else { break };
                    let s = String::from_utf8_lossy(s);
                    let (k, v) = s.split_once('=').unwrap_or((&s, ""));
                    if !k.is_empty() {
                        entries.push((k.to_ascii_lowercase(), v.to_string()));
                    }
                    i += 1 + n;
                }
                out.push(Record::Txt { name, entries });
            }
            TYPE_A if len == 4 => {
                out.push(Record::Addr { name, addr: IpAddr::from([rdata[0], rdata[1], rdata[2], rdata[3]]) });
            }
            TYPE_AAAA if len == 16 => {
                let b: [u8; 16] = rdata.try_into().ok()?;
                out.push(Record::Addr { name, addr: IpAddr::from(b) });
            }
            _ => {}
        }
    }
    Some(out)
}

/// What is known about one service instance.
#[derive(Default)]
struct Instance {
    scheme: Option<Scheme>,
    srv: Option<(u16, Name)>,
    txt: Option<Vec<(String, String)>>,
}

/// Everything heard so far.
#[derive(Default)]
struct Heard {
    /// Instance name → what is known, in the order first heard.
    instances: Vec<(Name, Instance)>,
    addrs: HashMap<Name, Vec<IpAddr>>,
}

impl Heard {
    fn instance(&mut self, name: &[String]) -> &mut Instance {
        let i = match self.instances.iter().position(|(n, _)| same(n, name)) {
            Some(i) => i,
            None => {
                self.instances.push((name.to_vec(), Instance::default()));
                self.instances.len() - 1
            }
        };
        &mut self.instances[i].1
    }

    fn take(&mut self, records: Vec<Record>, services: &[(Name, Scheme)]) {
        for r in records {
            match r {
                Record::Ptr { name, target } => {
                    if let Some((_, scheme)) = services.iter().find(|(s, _)| same(s, &name)) {
                        self.instance(&target).scheme = Some(*scheme);
                    }
                }
                Record::Srv { name, port, target } => self.instance(&name).srv = Some((port, target)),
                Record::Txt { name, entries } => self.instance(&name).txt = Some(entries),
                Record::Addr { name, addr } => {
                    let v = self.addrs.entry(lower(&name)).or_default();
                    if !v.contains(&addr) {
                        v.push(addr);
                    }
                }
            }
        }
    }

    /// Follow-up questions for what's still missing.
    fn missing(&self) -> Vec<(Name, u16)> {
        let mut q = Vec::new();
        for (name, i) in &self.instances {
            if i.scheme.is_none() {
                continue;
            }
            match &i.srv {
                None => q.push((name.clone(), TYPE_SRV)),
                Some((_, host)) if !self.addrs.contains_key(&lower(host)) => q.push((host.clone(), TYPE_A)),
                Some(_) => {}
            }
            if i.txt.is_none() {
                q.push((name.clone(), TYPE_TXT));
            }
        }
        q.dedup();
        q
    }

    fn found(&self) -> Vec<Found> {
        let mut out: Vec<Found> = Vec::new();
        for (name, i) in &self.instances {
            let (Some(scheme), Some((port, host))) = (i.scheme, &i.srv) else { continue };
            let mut addrs = self.addrs.get(&lower(host)).cloned().unwrap_or_default();
            addrs.sort_by_key(|a| a.is_ipv6());
            let txt = |k: &str| {
                i.txt.as_ref()?.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).filter(|v| !v.is_empty())
            };
            let f = Found {
                scheme,
                name: name.first().cloned().unwrap_or_default(),
                host: host.join("."),
                addrs,
                port: *port,
                path: txt("path"),
                user: txt("u"),
            };
            // `_ssh` and `_sftp-ssh` of one machine are the same server.
            if !out.iter().any(|o| o.scheme == f.scheme && o.host.eq_ignore_ascii_case(&f.host) && o.port == f.port) {
                out.push(f);
            }
        }
        out
    }
}

/// Ask the network for `services` (see [`SERVICES`]) for `wait`, or until `stop` is set.
pub fn browse(services: &[(&str, Scheme)], wait: Duration, stop: &AtomicBool) -> std::io::Result<Vec<Found>> {
    browse_at(MDNS, services, wait, stop)
}

/// [`browse`] asking `target` (tests use a fake responder on the loopback address).
pub fn browse_at(
    target: SocketAddr,
    services: &[(&str, Scheme)],
    wait: Duration,
    stop: &AtomicBool,
) -> std::io::Result<Vec<Found>> {
    let socks = if target.ip().is_multicast() { interface_sockets() } else { Vec::new() };
    let socks = if socks.is_empty() {
        let bind: SocketAddr = if target.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
        vec![UdpSocket::bind(bind)?]
    } else {
        socks
    };
    for s in &socks {
        s.set_nonblocking(true)?;
    }
    let types: Vec<(Name, Scheme)> = services.iter().map(|(s, k)| (name_of(s), *k)).collect();
    let id = (std::process::id() as u16) ^ 0x5a17;
    let ptrs: Vec<(&[String], u16)> = types.iter().map(|(n, _)| (n.as_slice(), TYPE_PTR)).collect();
    let first = query(id, &ptrs);

    let start = Instant::now();
    let deadline = start + wait;
    // Send the browse query now and again after a short while (lost packets), then ask
    // for what is missing.
    let mut sends = [Duration::ZERO, wait / 4, wait / 2, wait * 3 / 4].into_iter().peekable();
    let mut heard = Heard::default();
    let mut buf = vec![0u8; 9000];
    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        if sends.peek().is_some_and(|t| start.elapsed() >= *t) {
            let round = sends.next().unwrap_or_default();
            let missing = heard.missing();
            let mut packets = Vec::new();
            if round == Duration::ZERO || round == wait / 4 {
                packets.push(first.clone());
            }
            let qs: Vec<(&[String], u16)> = missing.iter().map(|(n, t)| (n.as_slice(), *t)).collect();
            packets.extend(qs.chunks(16).map(|c| query(id, c)));
            for s in &socks {
                for p in &packets {
                    let _ = s.send_to(p, target);
                }
            }
        }
        let mut got = false;
        for s in &socks {
            match s.recv_from(&mut buf) {
                Ok((n, _)) => {
                    got = true;
                    if let Some(records) = parse(&buf[..n]) {
                        heard.take(records, &types);
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                // Windows reports an ICMP "port unreachable" for an earlier send as a
                // receive error on UDP sockets; keep listening.
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                Err(e) => log::debug!("mdns: {e}"),
            }
        }
        if !got {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    Ok(heard.found())
}

/// One socket per IPv4 interface (loopback and link-local 169.254/16 left out), each
/// sending its multicast through that interface: a PC with VPN / VM / WSL adapters would
/// otherwise ask only whichever one the system picks.
fn interface_sockets() -> Vec<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let addrs = match if_addrs::get_if_addrs() {
        Ok(a) => a,
        Err(e) => {
            log::debug!("mdns: no interface list: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for ip in addrs.iter().filter_map(|i| match i.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(v4),
        _ => None,
    }) {
        let open = || -> std::io::Result<UdpSocket> {
            let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
            s.bind(&SocketAddr::new(IpAddr::V4(ip), 0).into())?;
            s.set_multicast_if_v4(&ip)?;
            s.set_multicast_ttl_v4(255)?;
            Ok(s.into())
        };
        match open() {
            Ok(s) => out.push(s),
            Err(e) => log::debug!("mdns: {ip}: {e}"),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encode a name without compression.
    fn name_bytes(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for l in s.trim_end_matches('.').split('.') {
            out.push(l.len() as u8);
            out.extend_from_slice(l.as_bytes());
        }
        out.push(0);
        out
    }

    fn rr(out: &mut Vec<u8>, name: &[u8], ty: u16, rdata: &[u8]) {
        out.extend_from_slice(name);
        out.extend_from_slice(&ty.to_be_bytes());
        out.extend_from_slice(&0x8001u16.to_be_bytes()); // cache-flush + IN
        out.extend_from_slice(&120u32.to_be_bytes());
        out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        out.extend_from_slice(rdata);
    }

    /// A response like Avahi's to a PTR browse: the answer plus SRV / TXT / A in the
    /// additional section, the instance name compressed against the answer.
    pub(crate) fn response(
        id: u16,
        service: &str,
        instance: &str,
        host: &str,
        port: u16,
        txt: &[&str],
        with_extra: bool,
    ) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&id.to_be_bytes());
        m.extend_from_slice(&0x8400u16.to_be_bytes());
        m.extend_from_slice(&[0, 0, 0, 1, 0, 0]);
        m.extend_from_slice(&(if with_extra { 3u16 } else { 0 }).to_be_bytes());
        // Answer: PTR service -> instance.service (the target written in full once).
        let svc = name_bytes(service);
        let mut target = vec![instance.len() as u8];
        target.extend_from_slice(instance.as_bytes());
        let svc_at = 12;
        target.extend_from_slice(&[0xC0, svc_at as u8]);
        rr(&mut m, &svc, TYPE_PTR, &target);
        if with_extra {
            let inst_at = 12 + svc.len() + 10;
            let inst_ptr = [0xC0 | (inst_at >> 8) as u8, inst_at as u8];
            let mut srv = vec![0, 0, 0, 0];
            srv.extend_from_slice(&port.to_be_bytes());
            srv.extend_from_slice(&name_bytes(host));
            rr(&mut m, &inst_ptr, TYPE_SRV, &srv);
            let mut t = Vec::new();
            for e in txt {
                t.push(e.len() as u8);
                t.extend_from_slice(e.as_bytes());
            }
            rr(&mut m, &inst_ptr, TYPE_TXT, &t);
            rr(&mut m, &name_bytes(host), TYPE_A, &[192, 168, 1, 20]);
        }
        m
    }

    #[test]
    fn query_and_response_round_trip() {
        let q = query(7, &[(name_of("_webdav._tcp.local").as_slice(), TYPE_PTR)]);
        assert_eq!(&q[..6], &[0, 7, 0, 0, 0, 1]);
        assert!(parse(&q).is_none(), "a query is not a response");
        let r =
            response(7, "_webdav._tcp.local", "Shared Files", "nas.local", 8080, &["path=/dav", "u=ann", "x"], true);
        let recs = parse(&r).unwrap();
        assert_eq!(recs.len(), 4);
        assert_eq!(
            recs[0],
            Record::Ptr {
                name: name_of("_webdav._tcp.local"),
                target: vec!["Shared Files".into(), "_webdav".into(), "_tcp".into(), "local".into()],
            }
        );
        assert!(matches!(&recs[1], Record::Srv { port: 8080, target, .. } if same(target, &name_of("nas.local"))));
        assert!(parse(&r[..r.len() - 3]).is_none(), "truncated");
        // A pointer loop is refused, not followed forever.
        let mut bad = r.clone();
        bad[12] = 0xC0;
        bad[13] = 12;
        assert!(parse(&bad).is_none());
    }

    /// A responder on the loopback address that answers the browse without extras, then
    /// the follow-up SRV / TXT / A questions.
    #[test]
    fn browse_follows_up_and_reports_servers() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        server.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let done = std::sync::Arc::new(AtomicBool::new(false));
        let done2 = done.clone();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let mut answered = 0;
            while !done2.load(Ordering::Relaxed) {
                let Ok((n, from)) = server.recv_from(&mut buf) else { continue };
                let id = u16::from_be_bytes([buf[0], buf[1]]);
                let asks_ptr = buf[..n].windows(9).any(|w| w == b"\x07_webdav");
                let full = !(asks_ptr && answered == 0);
                let r = response(id, "_webdav._tcp.local", "Shared Files", "nas.local", 8080, &["path=/dav"], full);
                server.send_to(&r, from).unwrap();
                // Also an SSH server, twice (`_ssh` and `_sftp-ssh`): listed once.
                for svc in ["_ssh._tcp.local", "_sftp-ssh._tcp.local"] {
                    server.send_to(&response(id, svc, "box", "box.local", 22, &[], true), from).unwrap();
                }
                answered += 1;
            }
        });
        let stop = AtomicBool::new(false);
        let found = browse_at(addr, SERVICES, Duration::from_millis(1200), &stop).unwrap();
        done.store(true, Ordering::Relaxed);
        t.join().unwrap();
        let dav = found.iter().find(|f| f.scheme == Scheme::WebDav).expect("the WebDAV server");
        assert_eq!(dav.name, "Shared Files");
        assert_eq!((dav.host.as_str(), dav.port, dav.path.as_deref()), ("nas.local", 8080, Some("/dav")));
        assert_eq!(dav.addrs, [IpAddr::from([192, 168, 1, 20])]);
        assert_eq!(found.iter().filter(|f| f.scheme == Scheme::Sftp).count(), 1);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore = "asks the real local network"]
    fn live_browse() {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let mut services = super::SERVICES.to_vec();
        // Printers are common on home networks: a real responder's answer to parse.
        services.push(("_ipp._tcp.local", crate::Scheme::WebDav));
        for f in super::browse(&services, std::time::Duration::from_secs(3), &stop).unwrap() {
            println!("{f:?} -> {}", f.connect_host());
        }
    }
}
