//! A tiny WebDAV server over a local folder, bound to `127.0.0.1` on a random port.
//!
//! Only for tests and the `SOYUZ_TEST_DAV` debug hook (try remote browsing and
//! transfers without a real server): PROPFIND (Depth 0/1), GET (with `Range`), PUT
//! (Content-Length or chunked), MKCOL, MOVE, DELETE, optional Basic auth. One thread per
//! connection, keep-alive, no TLS, no locking.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;

use crate::uri::{normalize_user_path as normalize_path, pct_decode, url_encode_path};

pub struct DavServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl DavServer {
    /// Serve `root` on a random port; `auth = Some((user, password))` requires Basic auth.
    pub fn start(root: &Path, auth: Option<(String, String)>) -> std::io::Result<DavServer> {
        Self::start_on(root, auth, 0)
    }

    /// [`DavServer::start`] on a fixed port (`0` = random).
    pub fn start_on(root: &Path, auth: Option<(String, String)>, port: u16) -> std::io::Result<DavServer> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let root = root.to_path_buf();
        let stop2 = stop.clone();
        let auth = auth.map(|(u, p)| base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}")));
        std::thread::Builder::new().name("soyuz-dav-server".into()).spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let root = root.clone();
                let auth = auth.clone();
                let _ = std::thread::Builder::new().name("soyuz-dav-conn".into()).spawn(move || {
                    let _ = serve(conn, &root, auth.as_deref());
                });
            }
        })?;
        Ok(DavServer { port, stop })
    }

    /// `webdav://127.0.0.1:<port>/`.
    pub fn uri(&self) -> String {
        format!("webdav://127.0.0.1:{}/", self.port)
    }
}

impl Drop for DavServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn read_req(r: &mut BufReader<TcpStream>) -> std::io::Result<Option<Req>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        r.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut req = Req { method, path: target, headers, body: Vec::new() };
    if req.header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut sz = String::new();
            r.read_line(&mut sz)?;
            let n = usize::from_str_radix(sz.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
            if n == 0 {
                let mut end = String::new();
                r.read_line(&mut end)?;
                break;
            }
            let mut chunk = vec![0u8; n];
            r.read_exact(&mut chunk)?;
            req.body.extend_from_slice(&chunk);
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf)?;
        }
    } else if let Some(n) = req.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        req.body = vec![0u8; n];
        r.read_exact(&mut req.body)?;
    }
    Ok(Some(req))
}

fn respond(w: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &[u8]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()
}

fn fs_path(root: &Path, url_path: &str) -> PathBuf {
    let p = normalize_path(&pct_decode(url_path.split('?').next().unwrap_or("/")));
    let mut out = root.to_path_buf();
    for seg in p.split('/').filter(|s| !s.is_empty()) {
        out.push(seg);
    }
    out
}

fn http_date(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn prop_xml(href: &str, md: &std::fs::Metadata) -> String {
    let rt = if md.is_dir() { "<D:collection/>" } else { "" };
    let len =
        if md.is_dir() { String::new() } else { format!("<D:getcontentlength>{}</D:getcontentlength>", md.len()) };
    let lm = md.modified().map(http_date).unwrap_or_default();
    let href = href.replace('&', "&amp;");
    format!(
        "<D:response><D:href>{href}</D:href><D:propstat><D:prop><D:resourcetype>{rt}</D:resourcetype>{len}<D:getlastmodified>{lm}</D:getlastmodified></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"
    )
}

fn serve(conn: TcpStream, root: &Path, auth: Option<&str>) -> std::io::Result<()> {
    let mut w = conn.try_clone()?;
    let mut r = BufReader::new(conn);
    while let Some(req) = read_req(&mut r)? {
        if let Some(expected) = auth {
            let ok = req
                .header("authorization")
                .and_then(|v| v.strip_prefix("Basic "))
                .is_some_and(|v| v.trim() == expected);
            if !ok {
                respond(&mut w, "401 Unauthorized", &[("WWW-Authenticate", "Basic realm=\"soyuz\"".into())], b"")?;
                continue;
            }
        }
        let path = fs_path(root, &req.path);
        let url_path = normalize_path(&pct_decode(req.path.split('?').next().unwrap_or("/")));
        match req.method.as_str() {
            "PROPFIND" => {
                let Ok(md) = std::fs::metadata(&path) else {
                    respond(&mut w, "404 Not Found", &[], b"")?;
                    continue;
                };
                let mut body = String::from(r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">"#);
                body.push_str(&prop_xml(&url_encode_path(&url_path), &md));
                if md.is_dir() && req.header("depth") != Some("0") {
                    for e in std::fs::read_dir(&path)?.flatten() {
                        if let Ok(cmd) = e.metadata() {
                            let child = crate::uri::join_path(&url_path, &e.file_name().to_string_lossy());
                            body.push_str(&prop_xml(&url_encode_path(&child), &cmd));
                        }
                    }
                }
                body.push_str("</D:multistatus>");
                respond(&mut w, "207 Multi-Status", &[("Content-Type", "application/xml".into())], body.as_bytes())?;
            }
            "GET" => match std::fs::read(&path) {
                Ok(data) => {
                    let range = req
                        .header("range")
                        .and_then(|v| v.strip_prefix("bytes="))
                        .and_then(|v| v.strip_suffix('-'))
                        .and_then(|v| v.parse::<usize>().ok());
                    match range {
                        Some(from) if from <= data.len() => respond(
                            &mut w,
                            "206 Partial Content",
                            &[(
                                "Content-Range",
                                format!("bytes {from}-{}/{}", data.len().saturating_sub(1), data.len()),
                            )],
                            &data[from..],
                        )?,
                        _ => respond(&mut w, "200 OK", &[], &data)?,
                    }
                }
                Err(_) => respond(&mut w, "404 Not Found", &[], b"")?,
            },
            "PUT" => {
                if !path.parent().is_some_and(Path::is_dir) {
                    respond(&mut w, "409 Conflict", &[], b"")?;
                } else {
                    std::fs::write(&path, &req.body)?;
                    respond(&mut w, "201 Created", &[], b"")?;
                }
            }
            "MKCOL" => {
                if path.exists() {
                    respond(&mut w, "405 Method Not Allowed", &[], b"")?;
                } else if std::fs::create_dir(&path).is_ok() {
                    respond(&mut w, "201 Created", &[], b"")?;
                } else {
                    respond(&mut w, "409 Conflict", &[], b"")?;
                }
            }
            "MOVE" => {
                let dest = req.header("destination").unwrap_or("").to_string();
                let dest_path = match dest.find("://") {
                    Some(i) => dest[i + 3..].find('/').map(|j| dest[i + 3 + j..].to_string()).unwrap_or_default(),
                    None => dest,
                };
                let to = fs_path(root, &dest_path);
                if to.exists() && req.header("overwrite") == Some("F") {
                    respond(&mut w, "412 Precondition Failed", &[], b"")?;
                } else if std::fs::rename(&path, &to).is_ok() {
                    respond(&mut w, "201 Created", &[], b"")?;
                } else {
                    respond(&mut w, "409 Conflict", &[], b"")?;
                }
            }
            "DELETE" => {
                let r = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                respond(&mut w, if r.is_ok() { "204 No Content" } else { "404 Not Found" }, &[], b"")?;
            }
            "OPTIONS" => respond(&mut w, "200 OK", &[("DAV", "1".into())], b"")?,
            _ => respond(&mut w, "405 Method Not Allowed", &[], b"")?,
        }
    }
    Ok(())
}
