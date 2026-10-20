//! WebDAV (RFC 4918) over reqwest: PROPFIND (Depth 0/1) listings, GET with `Range`,
//! streaming PUT, MKCOL, MOVE and DELETE. Basic or Digest authentication (RFC 7616,
//! `crate::digest`; a probe on the first request finds out which the server wants).

use std::io::Read;
use std::time::{Duration, SystemTime};

use quick_xml::Reader;
use quick_xml::events::Event;
use reqwest::Method;
use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::http::{BodyReader, BodyWriter, client, send, send_text, status_error};
use crate::provider::{Capabilities, Provider, RemoteEntry, WriteOpts, WriteStream, err};
use crate::registry::{Credentials, net_settings};
use crate::uri::{RemoteUri, Scheme, normalize_path, pct_decode, remote_name_of, url_encode_path};

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getcontentlength/><D:getlastmodified/></D:prop></D:propfind>"#;

pub struct WebDavProvider {
    uri: RemoteUri,
    /// `http(s)://host:port` (no trailing slash).
    base: String,
    client: reqwest::Client,
    auth: Option<(String, Option<String>)>,
    /// Digest authentication, once a probe found the server asks for it.
    digest: std::sync::Arc<parking_lot::Mutex<DigestState>>,
    timeout: Duration,
}

/// What the probe learned: Basic works, or the Digest challenge to answer.
#[derive(Default)]
struct DigestState {
    probed: bool,
    challenge: Option<crate::digest::Challenge>,
    /// Nonce count of the last request answered with `challenge`.
    nc: u32,
}

impl WebDavProvider {
    pub fn connect(uri: RemoteUri, creds: Credentials) -> FsResult<Self> {
        let net = net_settings();
        let scheme = if uri.scheme == Scheme::WebDavs { "https" } else { "http" };
        let host = if uri.host.contains(':') { format!("[{}]", uri.host) } else { uri.host.clone() };
        let base = match uri.port {
            Some(p) => format!("{scheme}://{host}:{p}"),
            None => format!("{scheme}://{host}"),
        };
        let user = creds.user.clone().or_else(|| uri.user.clone());
        let auth = user.map(|u| (u, creds.secret.clone()));
        Ok(WebDavProvider {
            uri,
            base,
            client: client(net.connect_timeout)?,
            auth,
            digest: Default::default(),
            timeout: net.io_timeout,
        })
    }

    /// Once per connection (and again after a failed sign-in): ask the server with Basic
    /// credentials; a `401` with a Digest challenge switches the requests to Digest.
    fn probe(&self) {
        let Some((u, p)) = &self.auth else { return };
        if self.digest.lock().probed {
            return;
        }
        let path = normalize_path(&self.uri.path);
        let rb = self.client.request(Method::OPTIONS, self.url(&path)).basic_auth(u, p.as_deref());
        let challenge = match send(rb, self.timeout, "connect", &path) {
            Ok(resp) if resp.status().as_u16() == 401 => {
                let values: Vec<String> = resp
                    .headers()
                    .get_all(reqwest::header::WWW_AUTHENTICATE)
                    .iter()
                    .filter_map(|v| v.to_str().ok().map(str::to_string))
                    .collect();
                crate::digest::parse_challenges(values.iter().map(String::as_str))
            }
            _ => None,
        };
        if challenge.is_some() {
            log::info!("webdav {}: Digest authentication", self.base);
        }
        *self.digest.lock() = DigestState { probed: true, challenge, nc: 0 };
    }

    /// Forget what the probe learned (a sign-in failed: the nonce may be stale).
    fn reprobe(&self) -> bool {
        let mut d = self.digest.lock();
        let had = d.probed;
        *d = DigestState::default();
        had && self.auth.is_some()
    }

    /// `send_text` for a request `make` builds, sent again once after a refused sign-in
    /// (a Digest nonce goes stale; the probe gets a new one).
    fn send_text_retry(
        &self,
        make: impl Fn() -> reqwest::RequestBuilder,
        op: &'static str,
        path: &str,
    ) -> FsResult<String> {
        match send_text(make(), self.timeout, op, path) {
            Err(e) if e.kind == FsErrorKind::PermissionDenied && self.reprobe() => {
                send_text(make(), self.timeout, op, path)
            }
            r => r,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, url_encode_path(&normalize_path(path)))
    }

    fn req(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.probe();
        let target = url_encode_path(&normalize_path(path));
        let rb = self.client.request(method.clone(), format!("{}{target}", self.base));
        let Some((u, p)) = &self.auth else { return rb };
        let mut d = self.digest.lock();
        let digest = d.challenge.clone();
        match digest {
            Some(c) => {
                d.nc += 1;
                let nc = d.nc;
                drop(d);
                let pw = p.as_deref().unwrap_or_default();
                let value =
                    crate::digest::authorization(&c, u, pw, method.as_str(), &target, nc, &crate::digest::cnonce(nc));
                rb.header(reqwest::header::AUTHORIZATION, value)
            }
            None => rb.basic_auth(u, p.as_deref()),
        }
    }

    fn propfind(&self, path: &str, depth: &str) -> FsResult<Vec<DavEntry>> {
        let method = Method::from_bytes(b"PROPFIND").expect("valid method");
        let make = || {
            self.req(method.clone(), path)
                .header("Depth", depth)
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(PROPFIND_BODY)
        };
        let text = self.send_text_retry(make, "list", path)?;
        parse_multistatus(&text).map_err(|m| err(FsErrorKind::Io, "list", path, format!("bad server reply: {m}")))
    }
}

/// One `<D:response>` of a multistatus reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DavEntry {
    /// Decoded absolute path (scheme / host stripped, no trailing `/`).
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Parse a PROPFIND multistatus body (namespace prefixes are ignored; only `200`
/// propstats are used).
pub fn parse_multistatus(xml: &str) -> Result<Vec<DavEntry>, String> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(true);
    let mut out = Vec::new();
    let mut cur: Option<DavEntry> = None;
    // Props of the current propstat (applied when its status is 200).
    let mut ps: Option<(bool, u64, Option<SystemTime>, bool)> = None; // (is_dir, size, modified, ok)
    let mut text_of: Option<String> = None;
    let mut buf_text = String::new();
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = local(e.name().as_ref());
                match name.as_str() {
                    "response" => cur = Some(DavEntry { path: String::new(), is_dir: false, size: 0, modified: None }),
                    "propstat" => ps = Some((false, 0, None, true)),
                    "collection" => {
                        if let Some(p) = ps.as_mut() {
                            p.0 = true;
                        }
                    }
                    "href" | "getcontentlength" | "getlastmodified" | "status" => {
                        text_of = Some(name);
                        buf_text.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                if local(e.name().as_ref()) == "collection"
                    && let Some(p) = ps.as_mut()
                {
                    p.0 = true;
                }
            }
            Ok(Event::Text(t)) => {
                if text_of.is_some() {
                    buf_text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(g)) => {
                if text_of.is_some() {
                    buf_text.push_str(match &*g {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    });
                }
            }
            Ok(Event::CData(t)) => {
                if text_of.is_some() {
                    buf_text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(e)) => {
                let name = local(e.name().as_ref());
                if text_of.as_deref() == Some(name.as_str()) {
                    let v = buf_text.trim().to_string();
                    match name.as_str() {
                        "href" => {
                            if let Some(c) = cur.as_mut() {
                                c.path = href_path(&v);
                            }
                        }
                        "getcontentlength" => {
                            if let Some(p) = ps.as_mut() {
                                p.1 = v.parse().unwrap_or(0);
                            }
                        }
                        "getlastmodified" => {
                            if let Some(p) = ps.as_mut() {
                                p.2 = parse_http_date(&v);
                            }
                        }
                        "status" => {
                            if let Some(p) = ps.as_mut() {
                                p.3 = v.split_whitespace().nth(1) == Some("200");
                            }
                        }
                        _ => {}
                    }
                    text_of = None;
                }
                match name.as_str() {
                    "propstat" => {
                        if let (Some(c), Some((is_dir, size, modified, ok))) = (cur.as_mut(), ps.take())
                            && ok
                        {
                            c.is_dir |= is_dir;
                            if size > 0 {
                                c.size = size;
                            }
                            if modified.is_some() {
                                c.modified = modified;
                            }
                        }
                    }
                    "response" => {
                        if let Some(c) = cur.take()
                            && !c.path.is_empty()
                        {
                            out.push(c);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(out)
}

fn local(qname: &str) -> String {
    qname.rsplit(':').next().unwrap_or("").to_ascii_lowercase()
}

/// `https://h/a%20b/` → `/a b`.
fn href_path(href: &str) -> String {
    let path = match href.find("://") {
        Some(i) => match href[i + 3..].find('/') {
            Some(j) => &href[i + 3 + j..],
            None => "/",
        },
        None => href,
    };
    normalize_path(&pct_decode(path))
}

/// RFC 1123 (`Sun, 06 Nov 1994 08:49:37 GMT`) or RFC 3339.
pub fn parse_http_date(s: &str) -> Option<SystemTime> {
    let dt = chrono::DateTime::parse_from_rfc2822(s).or_else(|_| chrono::DateTime::parse_from_rfc3339(s)).ok()?;
    let secs = dt.timestamp();
    if secs >= 0 {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64))
    } else {
        Some(SystemTime::UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs()))
    }
}

impl Provider for WebDavProvider {
    fn describe(&self) -> String {
        format!("WebDAV {}", self.uri.server_label())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            range_read: true,
            resume_write: false,
            atomic_write: false,
            rename: true,
            rename_replaces: false,
            set_times: false,
            watch: false,
        }
    }

    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
        let me = normalize_path(path);
        let all = self.propfind(&me, "1")?;
        let list = all
            .into_iter()
            .filter(|e| e.path != me)
            .map(|e| RemoteEntry {
                name: remote_name_of(&e.path).to_string(),
                is_dir: e.is_dir,
                is_link: false,
                size: e.size,
                modified: e.modified,
                mode: None,
            })
            .filter(|e| !e.name.is_empty())
            .collect();
        Ok(crate::provider::sanitize_listing("WebDAV", &me, list))
    }

    fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
        let me = normalize_path(path);
        let e = self
            .propfind(&me, "0")?
            .into_iter()
            .next()
            .ok_or_else(|| err(FsErrorKind::NotFound, "stat", &me, "not found"))?;
        Ok(RemoteEntry {
            name: remote_name_of(&me).to_string(),
            is_dir: e.is_dir,
            is_link: false,
            size: e.size,
            modified: e.modified,
            mode: None,
        })
    }

    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
        let mut rb = self.req(Method::GET, path);
        if offset > 0 {
            rb = rb.header("Range", format!("bytes={offset}-"));
        }
        let resp = send(rb, self.timeout, "open", path)?;
        if let Some(e) = status_error(resp.status(), "open", path, "") {
            return Err(e);
        }
        // 206 = the range was honoured; 200 = whole file, skip what we already have.
        let skip = if offset > 0 && resp.status().as_u16() != 206 { offset } else { 0 };
        Ok(Box::new(BodyReader::new(resp, skip, self.timeout)))
    }

    fn create_write(&self, path: &str, _opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
        let rb = self.req(Method::PUT, path);
        Ok(Box::new(DavWrite {
            w: Some(BodyWriter::start(rb, self.timeout * 10)),
            path: path.to_string(),
            dav: self.clone_light(),
        }))
    }

    fn mkdir(&self, path: &str) -> FsResult<()> {
        let method = Method::from_bytes(b"MKCOL").expect("valid method");
        self.send_text_retry(|| self.req(method.clone(), path), "mkdir", path).map(|_| ())
    }

    fn rename(&self, from: &str, to: &str) -> FsResult<()> {
        let method = Method::from_bytes(b"MOVE").expect("valid method");
        let make = || self.req(method.clone(), from).header("Destination", self.url(to)).header("Overwrite", "F");
        self.send_text_retry(make, "rename", from).map(|_| ())
    }

    fn delete(&self, path: &str, _is_dir: bool) -> FsResult<()> {
        self.send_text_retry(|| self.req(Method::DELETE, path), "delete", path).map(|_| ())
    }

    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        Some(self.uri.with_path(path))
    }
}

impl WebDavProvider {
    /// A cheap handle for a writer's cleanup (shares the connection pool).
    fn clone_light(&self) -> WebDavProvider {
        WebDavProvider {
            uri: self.uri.clone(),
            base: self.base.clone(),
            client: self.client.clone(),
            auth: self.auth.clone(),
            digest: self.digest.clone(),
            timeout: self.timeout,
        }
    }
}

struct DavWrite {
    w: Option<BodyWriter>,
    path: String,
    dav: WebDavProvider,
}

impl std::io::Write for DavWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.w.as_mut() {
            Some(w) => w.write(buf),
            None => Err(std::io::Error::other("closed")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl WriteStream for DavWrite {
    fn commit(mut self: Box<Self>) -> FsResult<()> {
        let Some(w) = self.w.take() else { return Ok(()) };
        let resp = w.finish().map_err(|e| FsError::from_io("write", Some(self.path.as_ref()), &e))?;
        match status_error(resp.status(), "write", &self.path, "") {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn abort(mut self: Box<Self>, _keep_partial: bool) {
        if let Some(w) = self.w.take() {
            w.abort();
        }
        // A server may have stored a truncated file: remove it (no resume on WebDAV).
        let _ = self.dav.delete(&self.path, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEXTCLOUD: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
 <d:response>
  <d:href>/remote.php/dav/files/alice/Docs/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>
   <d:getlastmodified>Tue, 01 Oct 2024 10:00:00 GMT</d:getlastmodified></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/alice/Docs/Report%202024.pdf</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>48213</d:getcontentlength>
   <d:getlastmodified>Wed, 02 Oct 2024 08:30:15 GMT</d:getlastmodified></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/alice/Docs/Photos/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"#;

    #[test]
    fn parses_nextcloud_style_multistatus() {
        let v = parse_multistatus(NEXTCLOUD).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].path, "/remote.php/dav/files/alice/Docs");
        assert!(v[0].is_dir);
        assert_eq!(v[1].path, "/remote.php/dav/files/alice/Docs/Report 2024.pdf");
        assert!(!v[1].is_dir);
        assert_eq!(v[1].size, 48213);
        let m = v[1].modified.unwrap().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(m, 1_727_857_815);
        assert!(v[2].is_dir);
        assert_eq!(v[2].modified, None);
    }

    #[test]
    fn parses_apache_style_with_absolute_hrefs_and_default_ns() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<multistatus xmlns="DAV:">
<response><href>http://example.com/dav/</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response>
<response><href>http://example.com/dav/a&amp;b.txt</href><propstat><prop><resourcetype/><getcontentlength>5</getcontentlength></prop><status>HTTP/1.1 200 OK</status></propstat></response>
</multistatus>"#;
        let v = parse_multistatus(xml).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].path, "/dav");
        assert_eq!(v[1].path, "/dav/a&b.txt");
        assert_eq!(v[1].size, 5);
    }

    #[test]
    fn http_dates() {
        assert!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT").is_some());
        assert!(parse_http_date("2024-10-02T08:30:15Z").is_some());
        assert_eq!(parse_http_date("garbage"), None);
    }
}
