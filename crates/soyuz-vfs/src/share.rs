//! Quick share: a temporary, read-only HTTP server for a few local files and folders, so
//! a phone or another computer on the same network can download them (the app shows the
//! address as a QR code).
//!
//! Everything lives under a random token (`/<token>/…`, 128 bits): a request without it
//! gets 404, as does anything outside the shared items (`..`, symlinks leading out).
//! Routes: `/<token>/` lists the items; `/<token>/<n>/<name>` is item `n` (a file, or a
//! folder listed as a page, browsable inside); `?dl=1` downloads instead of showing.
//! GET / HEAD only, single byte ranges (video seeking), one thread per connection (at
//! most [`MAX_CONNECTIONS`]), a 30 s idle timeout, no TLS (the token is the secret, like
//! a one-time link). Files are served inline under `Content-Security-Policy: sandbox`, so
//! a shared HTML page runs without access to the share. The server stops when dropped,
//! by [`ShareServer::stop`], or at its expiry.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;

use crate::uri::{pct_decode, url_encode_path};

/// Connections served at once; more wait in the listen queue.
pub const MAX_CONNECTIONS: usize = 32;
/// A connection that sends nothing for this long is closed.
const IDLE: Duration = Duration::from_secs(30);
/// Longest request head accepted (request line plus headers).
const MAX_HEAD: usize = 16 * 1024;

/// What the share has done so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareStats {
    /// Requests answered (pages, files, errors).
    pub requests: u64,
    /// Files fetched from their start (a resumed or seeking range does not count).
    pub downloads: u64,
    /// File bytes sent.
    pub bytes_sent: u64,
    /// The addresses that fetched something (at most 16, first seen first).
    pub clients: Vec<IpAddr>,
}

/// A running share (stopped on drop).
pub struct ShareServer {
    addr: SocketAddr,
    inner: Arc<Inner>,
}

struct Inner {
    token: String,
    items: Vec<Item>,
    stop: AtomicBool,
    expires: Option<Instant>,
    stats: Mutex<ShareStats>,
    conns: AtomicUsize,
}

struct Item {
    /// The file or folder name shown and used in its URL.
    name: String,
    /// Canonical path (what requests are checked against).
    path: PathBuf,
    is_dir: bool,
}

impl Inner {
    fn running(&self) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.expires.is_none_or(|t| Instant::now() < t)
    }
}

impl ShareServer {
    /// Share `items` on `bind:port` (`port` 0 = a free one) until stopped, dropped or
    /// `lifetime` has passed. Items that do not exist are left out; none left is an error.
    pub fn start(items: &[PathBuf], bind: IpAddr, port: u16, lifetime: Option<Duration>) -> io::Result<ShareServer> {
        let mut shared = Vec::new();
        for p in items {
            let Ok(path) = std::fs::canonicalize(p) else { continue };
            let Ok(md) = std::fs::metadata(&path) else { continue };
            let name =
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
            shared.push(Item { name, path, is_dir: md.is_dir() });
        }
        if shared.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "nothing to share"));
        }
        let listener = TcpListener::bind((bind, port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let inner = Arc::new(Inner {
            token: new_token()?,
            items: shared,
            stop: AtomicBool::new(false),
            expires: lifetime.map(|d| Instant::now() + d),
            stats: Mutex::new(ShareStats::default()),
            conns: AtomicUsize::new(0),
        });
        let accept = inner.clone();
        std::thread::Builder::new().name("soyuz-share".into()).spawn(move || accept_loop(listener, accept))?;
        Ok(ShareServer { addr, inner })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The URL path of the share (`/<token>/`).
    pub fn path(&self) -> String {
        format!("/{}/", self.inner.token)
    }

    /// The full URL for clients reaching this machine at `host`.
    pub fn url(&self, host: IpAddr) -> String {
        let host = match host {
            IpAddr::V4(a) => a.to_string(),
            IpAddr::V6(a) => format!("[{a}]"),
        };
        format!("http://{host}:{}{}", self.port(), self.path())
    }

    /// Still serving (not stopped, not expired).
    pub fn running(&self) -> bool {
        self.inner.running()
    }

    /// When it stops by itself.
    pub fn expires(&self) -> Option<Instant> {
        self.inner.expires
    }

    pub fn stats(&self) -> ShareStats {
        self.inner.stats.lock().clone()
    }

    /// Stop accepting; running downloads end at their next write.
    pub fn stop(&self) {
        self.inner.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for ShareServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 128 random bits, URL-safe base64 (22 characters).
fn new_token() -> io::Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(format!("no system randomness: {e}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b))
}

fn accept_loop(listener: TcpListener, inner: Arc<Inner>) {
    while inner.running() {
        match listener.accept() {
            Ok((conn, peer)) => {
                if inner.conns.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    drop(conn);
                    continue;
                }
                inner.conns.fetch_add(1, Ordering::Relaxed);
                let spawned = std::thread::Builder::new().name("soyuz-share-conn".into()).spawn({
                    let inner = inner.clone();
                    move || {
                        if let Err(e) = serve(conn, peer.ip(), &inner) {
                            log::debug!("share: {peer}: {e}");
                        }
                        inner.conns.fetch_sub(1, Ordering::Relaxed);
                    }
                });
                if spawned.is_err() {
                    inner.conns.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => {
                log::debug!("share: accept: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

struct Request {
    head_only: bool,
    method_ok: bool,
    target: String,
    range: Option<String>,
    close: bool,
}

/// The next request head on `r`; `None` at a clean end of the connection.
fn read_request(r: &mut impl BufRead) -> io::Result<Option<Request>> {
    let mut line = String::new();
    let mut total = 0;
    // Skip blank lines between requests.
    loop {
        line.clear();
        let n = r.by_ref().take((MAX_HEAD - total) as u64).read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        total += n;
        if !line.trim().is_empty() {
            break;
        }
        if total >= MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request too long"));
        }
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("");
    let mut req = Request {
        head_only: method == "HEAD",
        method_ok: method == "GET" || method == "HEAD",
        target,
        range: None,
        close: version != "HTTP/1.1",
    };
    let mut body = 0u64;
    loop {
        line.clear();
        let n = r.by_ref().take((MAX_HEAD.saturating_sub(total)) as u64).read_line(&mut line)?;
        total += n;
        if n == 0 || total >= MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request head too long or cut"));
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        let Some((k, v)) = l.split_once(':') else { continue };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "range" => req.range = Some(v.to_string()),
            "connection" if v.eq_ignore_ascii_case("close") => req.close = true,
            "connection" if v.eq_ignore_ascii_case("keep-alive") => req.close = false,
            "content-length" => body = v.parse().unwrap_or(u64::MAX),
            "transfer-encoding" => body = u64::MAX,
            _ => {}
        }
    }
    if body > 0 {
        // GET / HEAD carry no body; don't try to skip one.
        req.close = true;
        req.method_ok = false;
    }
    Ok(Some(req))
}

fn serve(conn: TcpStream, peer: IpAddr, inner: &Inner) -> io::Result<()> {
    conn.set_nonblocking(false)?;
    conn.set_read_timeout(Some(IDLE))?;
    conn.set_write_timeout(Some(IDLE))?;
    let mut r = BufReader::new(conn.try_clone()?);
    let mut w = conn;
    while inner.running() {
        let Some(req) = read_request(&mut r)? else { break };
        inner.stats.lock().requests += 1;
        respond(&mut w, &req, peer, inner)?;
        if req.close {
            break;
        }
    }
    Ok(())
}

/// What a request resolves to.
enum Target {
    Index,
    /// A folder: its path, its URL and the URL one level up.
    Dir {
        path: PathBuf,
        url: String,
        back: String,
    },
    File {
        path: PathBuf,
        name: String,
    },
}

fn respond(w: &mut TcpStream, req: &Request, peer: IpAddr, inner: &Inner) -> io::Result<()> {
    if !req.method_ok {
        return simple(w, req, "405 Method Not Allowed", "Only GET and HEAD.");
    }
    let (path, query) = req.target.split_once('?').unwrap_or((&req.target, ""));
    let download = query.split('&').any(|q| q == "dl=1");
    let target = match resolve(inner, path) {
        Some(t) => t,
        None => return simple(w, req, "404 Not Found", "Not found."),
    };
    match target {
        Target::Index => page(w, req, &index_page(inner)),
        Target::Dir { path, url, back } => match dir_page(&path, &url, &back) {
            Ok(html) => page(w, req, &html),
            Err(_) => simple(w, req, "404 Not Found", "Not found."),
        },
        Target::File { path, name } => file(w, req, inner, peer, &path, &name, download),
    }
}

/// Map a URL path to what it names, or `None` (404) for anything outside the share.
fn resolve(inner: &Inner, path: &str) -> Option<Target> {
    let rest = path.strip_prefix('/')?.strip_prefix(inner.token.as_str())?;
    if rest.is_empty() || rest == "/" {
        return Some(Target::Index);
    }
    let rest = rest.strip_prefix('/')?;
    let mut segs = rest.split('/');
    let n: usize = segs.next()?.parse().ok()?;
    let item = inner.items.get(n)?;
    let mut parts = Vec::new();
    for s in segs {
        if s.is_empty() {
            continue;
        }
        let s = pct_decode(s);
        if !safe_segment(&s) {
            return None;
        }
        parts.push(s);
    }
    if !item.is_dir {
        // `/<n>/` or `/<n>/<name>`.
        return match parts.as_slice() {
            [] => Some(Target::File { path: item.path.clone(), name: item.name.clone() }),
            [one] if *one == item.name => Some(Target::File { path: item.path.clone(), name: item.name.clone() }),
            _ => None,
        };
    }
    // `/<n>/<name>/<inside…>` (the name segment is for people; `/<n>/` works too).
    if parts.first().is_some_and(|p| *p == item.name) {
        parts.remove(0);
    }
    let mut p = item.path.clone();
    for s in &parts {
        p.push(s);
    }
    // Symlinks inside the folder may lead out of it: check where they land.
    let canon = std::fs::canonicalize(&p).ok()?;
    if !canon.starts_with(&item.path) {
        return None;
    }
    let md = std::fs::metadata(&canon).ok()?;
    if md.is_dir() {
        let mut url = format!("/{}/{n}/{}/", inner.token, url_encode_path(&item.name));
        let mut back = format!("/{}/", inner.token);
        for s in &parts {
            back = url.clone();
            url.push_str(&url_encode_path(s));
            url.push('/');
        }
        Some(Target::Dir { path: canon, url, back })
    } else {
        let name = canon.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Some(Target::File { path: canon, name })
    }
}

/// A single path component that stays where it is.
fn safe_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', '\0']) && !(cfg!(windows) && s.contains(':'))
}

fn head(w: &mut impl Write, status: &str, headers: &[(&str, String)], len: u64) -> io::Result<()> {
    let mut h = format!("HTTP/1.1 {status}\r\nContent-Length: {len}\r\nX-Content-Type-Options: nosniff\r\n");
    h.push_str("Referrer-Policy: no-referrer\r\nCache-Control: no-store\r\n");
    for (k, v) in headers {
        let _ = write!(h, "{k}: {v}\r\n");
    }
    h.push_str("\r\n");
    w.write_all(h.as_bytes())
}

fn simple(w: &mut impl Write, req: &Request, status: &str, text: &str) -> io::Result<()> {
    head(w, status, &[("Content-Type", "text/plain; charset=utf-8".into())], text.len() as u64)?;
    if !req.head_only {
        w.write_all(text.as_bytes())?;
    }
    Ok(())
}

fn page(w: &mut impl Write, req: &Request, html: &str) -> io::Result<()> {
    let headers = [
        ("Content-Type", "text/html; charset=utf-8".to_string()),
        ("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'".to_string()),
    ];
    head(w, "200 OK", &headers, html.len() as u64)?;
    if !req.head_only {
        w.write_all(html.as_bytes())?;
    }
    Ok(())
}

/// `bytes=a-b` / `bytes=a-` / `bytes=-n` against `len`: the inclusive range, `Err` when
/// unsatisfiable, `Ok(None)` for no (or a multi-) range.
fn parse_range(h: &str, len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return Ok(None) };
    if spec.contains(',') {
        return Ok(None);
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        (len.saturating_sub(n), len.checked_sub(1).ok_or(())?)
    } else {
        let s: u64 = a.parse().map_err(|_| ())?;
        let e = if b.is_empty() {
            len.saturating_sub(1)
        } else {
            b.parse::<u64>().map_err(|_| ())?.min(len.saturating_sub(1))
        };
        (s, e)
    };
    if start >= len || start > end {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn file(
    w: &mut TcpStream,
    req: &Request,
    inner: &Inner,
    peer: IpAddr,
    path: &Path,
    name: &str,
    download: bool,
) -> io::Result<()> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return simple(w, req, "403 Forbidden", "This file can't be read.");
    };
    let len = f.metadata()?.len();
    let range = match req.range.as_deref().map(|h| parse_range(h, len)) {
        None | Some(Ok(None)) => None,
        Some(Ok(Some(r))) => Some(r),
        Some(Err(())) => {
            head(w, "416 Range Not Satisfiable", &[("Content-Range", format!("bytes */{len}"))], 0)?;
            return Ok(());
        }
    };
    let (start, count) = match range {
        Some((s, e)) => (s, e - s + 1),
        None => (0, len),
    };
    let disposition = if download { "attachment" } else { "inline" };
    let mut headers = vec![
        ("Content-Type", mime(name).to_string()),
        ("Content-Disposition", format!("{disposition}; filename*=UTF-8''{}", url_encode_path(name))),
        ("Content-Security-Policy", "sandbox".to_string()),
        ("Accept-Ranges", "bytes".to_string()),
    ];
    let status = if let Some((s, e)) = range {
        headers.push(("Content-Range", format!("bytes {s}-{e}/{len}")));
        "206 Partial Content"
    } else {
        "200 OK"
    };
    head(w, status, &headers, count)?;
    if req.head_only {
        return Ok(());
    }
    {
        let mut st = inner.stats.lock();
        if start == 0 {
            st.downloads += 1;
        }
        if !st.clients.contains(&peer) && st.clients.len() < 16 {
            st.clients.push(peer);
        }
    }
    f.seek(SeekFrom::Start(start))?;
    let mut left = count;
    let mut buf = vec![0u8; 256 * 1024];
    while left > 0 {
        if !inner.running() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "share stopped"));
        }
        let want = buf.len().min(left as usize);
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            // The file shrank while sending: the client sees a short body.
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "file shrank"));
        }
        w.write_all(&buf[..n])?;
        left -= n as u64;
        inner.stats.lock().bytes_sent += n as u64;
    }
    Ok(())
}

/// The content type for a file name (what phones can show inline; else a download).
fn mime(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" | "log" | "md" | "csv" | "ini" | "toml" | "json" | "xml" | "yaml" | "yml" | "rs" | "py" | "c" | "h"
        | "cpp" | "js" | "ts" | "css" | "sh" | "html" | "htm" | "svg" => "text/plain; charset=utf-8",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if c.is_control() => o.push('\u{fffd}'),
            c => o.push(c),
        }
    }
    o
}

fn size_text(n: u64) -> String {
    soyuz_core::format_size(n, soyuz_core::SizeStyle::Binary)
}

const STYLE: &str = "body{font:16px/1.4 system-ui,sans-serif;margin:0 auto;max-width:720px;padding:16px;\
background:#fafafa;color:#1d1d1f}@media(prefers-color-scheme:dark){body{background:#17181a;color:#e8e8ea}\
a{color:#8ab4ff}.r{border-color:#2c2e33}.m{color:#9a9ca3}}h1{font-size:20px;margin:8px 0 16px}\
.r{display:flex;gap:12px;align-items:center;padding:12px 4px;border-bottom:1px solid #e3e3e6}\
.n{flex:1;min-width:0;overflow-wrap:anywhere}.m{color:#6e6e73;font-size:14px;white-space:nowrap}\
a{color:#0a5cd6;text-decoration:none}.d{font-size:14px}";

fn html_page(title: &str, rows: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\">\
         <title>{t}</title><style>{STYLE}</style></head><body><h1>{t}</h1>{rows}</body></html>",
        t = esc(title)
    )
}

/// A row: the name (a link), its size and a download link (files).
fn row(out: &mut String, href: &str, name: &str, size: Option<u64>) {
    let n = esc(name);
    match size {
        Some(s) => {
            let _ = write!(
                out,
                "<div class=\"r\"><a class=\"n\" href=\"{href}\">{n}</a><span class=\"m\">{}</span>\
                 <a class=\"d\" href=\"{href}?dl=1\">Download</a></div>",
                size_text(s)
            );
        }
        None => {
            let _ = write!(out, "<div class=\"r\"><a class=\"n\" href=\"{href}\">{n}/</a></div>");
        }
    }
}

fn index_page(inner: &Inner) -> String {
    let mut rows = String::new();
    for (i, it) in inner.items.iter().enumerate() {
        let href = format!("/{}/{i}/{}", inner.token, url_encode_path(&it.name));
        if it.is_dir {
            row(&mut rows, &format!("{href}/"), &it.name, None);
        } else {
            let size = std::fs::metadata(&it.path).map(|m| m.len()).unwrap_or(0);
            row(&mut rows, &href, &it.name, Some(size));
        }
    }
    html_page("Shared from Soyuz", &rows)
}

fn dir_page(dir: &Path, url: &str, back: &str) -> io::Result<String> {
    let mut entries: Vec<(bool, String, u64)> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| {
            let md = std::fs::metadata(e.path()).ok();
            let is_dir = md.as_ref().is_some_and(|m| m.is_dir());
            (is_dir, e.file_name().to_string_lossy().into_owned(), md.map_or(0, |m| m.len()))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let mut rows = String::new();
    let _ = write!(rows, "<div class=\"r\"><a class=\"n\" href=\"{}\">..</a></div>", esc(back));
    for (is_dir, name, size) in entries {
        let href = format!("{url}{}", url_encode_path(&name));
        if is_dir {
            row(&mut rows, &format!("{href}/"), &name, None);
        } else {
            row(&mut rows, &href, &name, Some(size));
        }
    }
    let title = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(html_page(&title, &rows))
}

/// This machine's addresses other devices on the network can reach: IPv4, not loopback
/// or link-local, home / office ranges first and virtual adapters (WSL, Hyper-V,
/// Docker, VirtualBox, VMware) last.
pub fn lan_addresses() -> Vec<IpAddr> {
    let Ok(ifs) = if_addrs::get_if_addrs() else { return Vec::new() };
    let mut out: Vec<(u8, IpAddr)> = Vec::new();
    for i in ifs {
        let IpAddr::V4(a) = i.ip() else { continue };
        if a.is_loopback() || a.is_link_local() || a.is_unspecified() || a.is_multicast() {
            continue;
        }
        let name = i.name.to_ascii_lowercase();
        let virt = ["vethernet", "wsl", "docker", "virtualbox", "vmware", "vbox", "virbr", "br-", "veth", "hyper-v"]
            .iter()
            .any(|v| name.contains(v));
        let o = a.octets();
        let rank = match (virt, o) {
            (false, [192, 168, ..]) => 0,
            (false, [10, ..]) => 1,
            (false, [172, b, ..]) if (16..32).contains(&b) => 2,
            (false, _) => 3,
            (true, _) => 4,
        };
        if !out.iter().any(|(_, x)| *x == IpAddr::V4(a)) {
            out.push((rank, IpAddr::V4(a)));
        }
    }
    out.sort_by_key(|(r, _)| *r);
    out.into_iter().map(|(_, a)| a).collect()
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn get(addr: SocketAddr, path: &str, extra: &str) -> (String, Vec<u8>) {
        let mut s = TcpStream::connect(addr).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{extra}\r\n").unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        let split = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&all[..split]).into_owned(), all[split + 4..].to_vec())
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("photo one.jpg");
        std::fs::write(&file, b"0123456789").unwrap();
        let dir = d.path().join("docs");
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("inner").join("b.txt"), b"deep").unwrap();
        std::fs::write(d.path().join("secret.txt"), b"not shared").unwrap();
        (d, file, dir)
    }

    fn server(items: &[PathBuf]) -> (ShareServer, SocketAddr, String) {
        let s = ShareServer::start(items, IpAddr::V4(Ipv4Addr::LOCALHOST), 0, None).unwrap();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), s.port());
        let p = s.path();
        (s, addr, p)
    }

    #[test]
    fn serves_the_items_under_the_token_only() {
        let (_d, file, dir) = fixture();
        let (s, addr, p) = server(&[file, dir]);
        let (h, body) = get(addr, &p, "");
        assert!(h.starts_with("HTTP/1.1 200"), "{h}");
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("photo one.jpg") && html.contains("docs/"), "{html}");
        assert!(!html.contains("secret"));
        let (h, body) = get(addr, &format!("{p}0/photo%20one.jpg"), "");
        assert!(h.contains("Content-Type: image/jpeg") && h.contains("inline"), "{h}");
        assert_eq!(body, b"0123456789");
        let (h, _) = get(addr, &format!("{p}0/photo%20one.jpg?dl=1"), "");
        assert!(h.contains("attachment; filename*=UTF-8''photo%20one.jpg"), "{h}");
        // A folder lists and browses inside.
        let (_, body) = get(addr, &format!("{p}1/docs/"), "");
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("inner/") && html.contains("a.txt"), "{html}");
        let (_, body) = get(addr, &format!("{p}1/docs/inner/b.txt"), "");
        assert_eq!(body, b"deep");
        // No token, a wrong one, `..` (raw or encoded), an unknown item: 404.
        for bad in [
            "/".to_string(),
            "/AAAAAAAAAAAAAAAAAAAAAA/".to_string(),
            format!("{p}1/docs/../secret.txt"),
            format!("{p}1/docs/%2E%2E/secret.txt"),
            format!("{p}1/docs/..%2Fsecret.txt"),
            format!("{p}0/other.jpg"),
            format!("{p}7/"),
        ] {
            let (h, _) = get(addr, &bad, "");
            assert!(h.starts_with("HTTP/1.1 404"), "{bad}: {h}");
        }
        let st = s.stats();
        // The photo twice (shown, downloaded) and b.txt.
        assert_eq!(st.downloads, 3);
        assert_eq!(st.bytes_sent, 24);
        assert_eq!(st.clients, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    }

    #[test]
    fn ranges_head_and_methods() {
        let (_d, file, _) = fixture();
        let (s, addr, p) = server(&[file]);
        let url = format!("{p}0/photo%20one.jpg");
        let (h, body) = get(addr, &url, "Range: bytes=2-4\r\n");
        assert!(h.starts_with("HTTP/1.1 206") && h.contains("Content-Range: bytes 2-4/10"), "{h}");
        assert_eq!(body, b"234");
        let (_, body) = get(addr, &url, "Range: bytes=-3\r\n");
        assert_eq!(body, b"789");
        let (_, body) = get(addr, &url, "Range: bytes=8-\r\n");
        assert_eq!(body, b"89");
        let (h, _) = get(addr, &url, "Range: bytes=20-\r\n");
        assert!(h.starts_with("HTTP/1.1 416"), "{h}");
        assert_eq!(s.stats().downloads, 0, "ranges not from the start are not downloads");
        let mut c = TcpStream::connect(addr).unwrap();
        write!(c, "HEAD {url} HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut all = String::new();
        c.read_to_string(&mut all).unwrap();
        assert!(all.contains("Content-Length: 10") && all.ends_with("\r\n\r\n"), "{all}");
        let mut c = TcpStream::connect(addr).unwrap();
        write!(c, "PUT {url} HTTP/1.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        let mut all = String::new();
        c.read_to_string(&mut all).unwrap();
        assert!(all.starts_with("HTTP/1.1 405"), "{all}");
    }

    #[test]
    fn keep_alive_then_stop_and_expiry() {
        let (_d, file, _) = fixture();
        let (s, addr, p) = server(std::slice::from_ref(&file));
        let mut c = TcpStream::connect(addr).unwrap();
        write!(c, "GET {p} HTTP/1.1\r\n\r\nGET {p} HTTP/1.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut all = String::new();
        c.read_to_string(&mut all).unwrap();
        assert_eq!(all.matches("HTTP/1.1 200").count(), 2);
        assert!(s.running());
        s.stop();
        assert!(!s.running());
        let e = ShareServer::start(&[file], IpAddr::V4(Ipv4Addr::LOCALHOST), 0, Some(Duration::ZERO)).unwrap();
        assert!(!e.running(), "expired at once");
        assert!(
            ShareServer::start(&[PathBuf::from("/no/such/thing")], IpAddr::V4(Ipv4Addr::LOCALHOST), 0, None).is_err()
        );
    }

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("bytes=0-0", 5), Ok(Some((0, 0))));
        assert_eq!(parse_range("bytes=1-99", 5), Ok(Some((1, 4))));
        assert_eq!(parse_range("bytes=-10", 5), Ok(Some((0, 4))));
        assert_eq!(parse_range("bytes=0-1,3-4", 5), Ok(None));
        assert_eq!(parse_range("items=1-2", 5), Ok(None));
        assert_eq!(parse_range("bytes=4-2", 5), Err(()));
        assert_eq!(parse_range("bytes=0-", 0), Err(()));
        assert!(safe_segment("a b.txt") && !safe_segment("..") && !safe_segment("a\\b"));
    }

    /// Lists this machine's addresses (`cargo test -p soyuz-vfs share::tests::lan -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn lan() {
        println!("{:?}", lan_addresses());
    }
}
