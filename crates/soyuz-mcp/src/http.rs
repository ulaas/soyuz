//! Streamable HTTP transport on 127.0.0.1 (off by default).
//!
//! * One endpoint, `POST /mcp`. GET / DELETE get `405` (no standalone SSE stream, no
//!   protocol sessions in 2026-07-28; legacy clients fall back fine).
//! * Every request needs `Authorization: Bearer <token>`; the token identifies the client
//!   (`[[ai.mcp_clients]]`, tokens live in the OS credential store).
//! * DNS-rebinding defence: the `Host` must be a loopback name with our port, and an
//!   `Origin`, when present, must be loopback or explicitly allowed — else `403`.
//! * Modern requests must mirror `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name`
//!   into headers (mismatch → `400` with `HeaderMismatch`). Legacy (`initialize`) clients
//!   get an `Mcp-Session-Id`.
//! * Requests with a `progressToken` and `Accept: text/event-stream` are answered as an
//!   SSE stream (progress notifications, then the response); closing the connection
//!   cancels the request.
//!
//! Plain std networking, one thread per connection, `Connection: close`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine;
use parking_lot::{Mutex, RwLock};
use serde_json::{Value, json};
use soyuz_ai::Secret;
use soyuz_core::OpControl;

use crate::jsonrpc::{self, Incoming, RpcError};
use crate::protocol::{self, META_PROTOCOL_VERSION};
use crate::server::{Server, Session, Sink};
use crate::trust::{self, Transport};

const MAX_HEADER: usize = 64 * 1024;
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 32;
const MAX_SESSIONS: usize = 256;

/// `(client name, token)` pairs accepted by the endpoint.
pub type Tokens = Arc<RwLock<Vec<(String, Secret)>>>;

/// Live settings of a running HTTP server.
#[derive(Clone, Default)]
pub struct HttpSettings {
    pub tokens: Tokens,
    pub allowed_origins: Arc<RwLock<Vec<String>>>,
}

pub struct HttpServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl HttpServer {
    /// Bind `127.0.0.1:port` (0 = any free port) and serve in the background.
    pub fn start(server: Arc<Server>, port: u16, settings: HttpSettings) -> std::io::Result<HttpServer> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let state = Arc::new(State {
            server,
            port: addr.port(),
            settings,
            sessions: Mutex::new(HashMap::new()),
            active: AtomicUsize::new(0),
        });
        std::thread::Builder::new().name("soyuz-mcp-http".into()).spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = conn else { continue };
                if state.active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    let mut s = stream;
                    let _ = respond(&mut s, 503, "Service Unavailable", &[], b"");
                    continue;
                }
                state.active.fetch_add(1, Ordering::SeqCst);
                let st = state.clone();
                let spawned = std::thread::Builder::new().name("soyuz-mcp-http-conn".into()).spawn(move || {
                    st.handle(stream);
                    st.active.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    state.active.fetch_sub(1, Ordering::SeqCst);
                }
            }
        })?;
        Ok(HttpServer { addr, stop })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
    }
}

struct State {
    server: Arc<Server>,
    port: u16,
    settings: HttpSettings,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    active: AtomicUsize,
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn respond(s: &mut TcpStream, code: u16, reason: &str, headers: &[(&str, String)], body: &[u8]) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {code} {reason}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.flush()
}

fn json_body(v: &Value) -> Vec<u8> {
    v.to_string().into_bytes()
}

fn rpc_error_body(id: Option<&Value>, e: &RpcError) -> Vec<u8> {
    let mut v = json!({"jsonrpc": "2.0", "error": e.to_value()});
    if let Some(id) = id {
        v["id"] = id.clone();
    }
    json_body(&v)
}

fn read_request(s: &mut TcpStream) -> Result<Request, (u16, &'static str)> {
    let mut reader = BufReader::new(s.try_clone().map_err(|_| (500, "Internal Server Error"))?);
    let mut line = String::new();
    let mut total = 0usize;
    reader.read_line(&mut line).map_err(|_| (400, "Bad Request"))?;
    total += line.len();
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or((400, "Bad Request"))?.to_string();
    let target = parts.next().ok_or((400, "Bad Request"))?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).map_err(|_| (400, "Bad Request"))?;
        total += n;
        if total > MAX_HEADER {
            return Err((431, "Request Header Fields Too Large"));
        }
        let h = h.trim_end_matches(['\r', '\n']);
        if n == 0 || h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let req = Request { method, path: target.split('?').next().unwrap_or("").to_string(), headers, body: Vec::new() };
    if req.header("transfer-encoding").is_some() {
        return Err((411, "Length Required"));
    }
    let len: usize = req.header("content-length").map(|v| v.parse().unwrap_or(usize::MAX)).unwrap_or(0);
    if len > MAX_BODY {
        return Err((413, "Payload Too Large"));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(|_| (400, "Bad Request"))?;
    Ok(Request { body, ..req })
}

/// Decode an `Mcp-Name` / `Mcp-Param-*` value (`=?base64?…?=` sentinel).
pub fn decode_header_value(v: &str) -> Option<String> {
    match v.strip_prefix("=?base64?").and_then(|r| r.strip_suffix("?=")) {
        Some(b64) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
            String::from_utf8(bytes).ok()
        }
        None => Some(v.to_string()),
    }
}

/// Encode a header value, using the base64 sentinel when it is not plain ASCII.
pub fn encode_header_value(v: &str) -> String {
    let plain = v.bytes().all(|b| (0x20..=0x7e).contains(&b))
        && !v.starts_with(' ')
        && !v.ends_with(' ')
        && !(v.starts_with("=?base64?") && v.ends_with("?="));
    if plain {
        v.to_string()
    } else {
        format!("=?base64?{}?=", base64::engine::general_purpose::STANDARD.encode(v.as_bytes()))
    }
}

impl State {
    fn loopback_ok(&self, value: &str, with_scheme: bool) -> bool {
        let port = self.port;
        ["127.0.0.1", "localhost", "[::1]"].iter().any(|h| {
            let hp = format!("{h}:{port}");
            if with_scheme {
                value.eq_ignore_ascii_case(&format!("http://{hp}"))
            } else {
                value.eq_ignore_ascii_case(&hp)
            }
        })
    }

    fn handle(&self, mut s: TcpStream) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = s.set_nodelay(true);
        let req = match read_request(&mut s) {
            Ok(r) => r,
            Err((code, reason)) => {
                let _ = respond(&mut s, code, reason, &[], b"");
                return;
            }
        };
        let json_ct = ("Content-Type", "application/json".to_string());
        // DNS rebinding: Host must be us.
        if !req.header("host").is_some_and(|h| self.loopback_ok(h, false)) {
            let _ = respond(&mut s, 403, "Forbidden", &[], b"invalid Host");
            return;
        }
        if let Some(origin) = req.header("origin") {
            let extra = self.settings.allowed_origins.read();
            if !self.loopback_ok(origin, true) && !extra.iter().any(|o| o.eq_ignore_ascii_case(origin)) {
                let body = rpc_error_body(None, &RpcError::new(jsonrpc::INVALID_REQUEST, "Origin not allowed"));
                let _ = respond(&mut s, 403, "Forbidden", &[json_ct], &body);
                return;
            }
        }
        if req.path != "/mcp" {
            let _ = respond(&mut s, 404, "Not Found", &[], b"");
            return;
        }
        if req.method != "POST" {
            let _ = respond(&mut s, 405, "Method Not Allowed", &[("Allow", "POST".into())], b"");
            return;
        }
        let client = req
            .header("authorization")
            .and_then(|a| a.strip_prefix("Bearer ").or_else(|| a.strip_prefix("bearer ")))
            .map(str::trim)
            .and_then(|tok| {
                self.settings
                    .tokens
                    .read()
                    .iter()
                    .find(|(_, t)| trust::token_eq(t.expose(), tok))
                    .map(|(c, _)| c.clone())
            });
        let Some(client) = client else {
            let body = rpc_error_body(
                None,
                &RpcError::new(crate::server::CLIENT_NOT_ALLOWED, "missing or invalid bearer token"),
            );
            let _ = respond(
                &mut s,
                401,
                "Unauthorized",
                &[json_ct, ("WWW-Authenticate", "Bearer realm=\"soyuz\"".into())],
                &body,
            );
            return;
        };
        let msg: Value = match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(e) => {
                let body = rpc_error_body(None, &RpcError::new(jsonrpc::PARSE_ERROR, format!("parse error: {e}")));
                let _ = respond(&mut s, 400, "Bad Request", &[json_ct], &body);
                return;
            }
        };
        match jsonrpc::classify(msg) {
            Incoming::Notification { method, params } => {
                if method == "notifications/cancelled"
                    && let Some(sid) = req.header("mcp-session-id")
                    && let Some(sess) = self.sessions.lock().get(sid).cloned()
                {
                    let sink: Sink = Arc::new(|_| {});
                    self.server.dispatch_message(
                        &sess,
                        json!({"jsonrpc": "2.0", "method": method, "params": params}),
                        &sink,
                    );
                }
                let _ = respond(&mut s, 202, "Accepted", &[], b"");
            }
            Incoming::Response => {
                let _ = respond(&mut s, 202, "Accepted", &[], b"");
            }
            Incoming::Invalid(e, id) => {
                let _ = respond(&mut s, 400, "Bad Request", &[json_ct], &rpc_error_body(Some(&id), &e));
            }
            Incoming::Request { id, method, params } => self.request(s, &req, client, id, method, params),
        }
    }

    fn request(&self, mut s: TcpStream, req: &Request, client: String, id: Value, method: String, params: Value) {
        let json_ct = ("Content-Type", "application/json".to_string());
        let modern_version = params.get("_meta").and_then(|m| m.get(META_PROTOCOL_VERSION)).and_then(Value::as_str);
        let mut extra_headers: Vec<(&str, String)> = vec![json_ct.clone()];
        let session = if let Some(v) = modern_version {
            if let Err(e) = validate_headers(req, v, &method, &params) {
                let _ = respond(&mut s, 400, "Bad Request", &[json_ct], &rpc_error_body(Some(&id), &e));
                return;
            }
            Session::new(Transport::Http, Some(client))
        } else if method == "initialize" {
            let sid = trust::generate_token().trim_start_matches("soyuz_").to_string();
            let sess = Session::new(Transport::Http, Some(client));
            let mut map = self.sessions.lock();
            if map.len() >= MAX_SESSIONS
                && let Some(k) = map.keys().next().cloned()
            {
                map.remove(&k);
            }
            map.insert(sid.clone(), sess.clone());
            extra_headers.push(("Mcp-Session-Id", sid));
            sess
        } else if let Some(sid) = req.header("mcp-session-id") {
            match self.sessions.lock().get(sid).cloned() {
                Some(sess) => sess,
                None => {
                    let e = RpcError::new(jsonrpc::INVALID_REQUEST, "unknown or expired session");
                    let _ = respond(&mut s, 404, "Not Found", &[json_ct], &rpc_error_body(Some(&id), &e));
                    return;
                }
            }
        } else {
            Session::new(Transport::Http, Some(client))
        };
        let sse = params.get("_meta").and_then(|m| m.get("progressToken")).is_some()
            && req.header("accept").is_some_and(|a| a.contains("text/event-stream"));

        let ctl = OpControl::new();
        // Closing the connection cancels the request.
        let _ = s.set_read_timeout(None);
        if let Ok(mut watch) = s.try_clone() {
            let c = ctl.clone();
            let _ = std::thread::Builder::new().name("soyuz-mcp-http-watch".into()).spawn(move || {
                let mut b = [0u8; 1];
                if matches!(watch.read(&mut b), Ok(0) | Err(_)) {
                    c.cancel();
                }
            });
        }
        if sse {
            let head = "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nX-Accel-Buffering: no\r\n";
            let mut head = head.to_string();
            for (k, v) in extra_headers.iter().skip(1) {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            if s.write_all(head.as_bytes()).is_err() {
                return;
            }
            let out = Arc::new(Mutex::new(s.try_clone().ok()));
            let o2 = out.clone();
            let sink: Sink = Arc::new(move |v: Value| {
                if let Some(w) = o2.lock().as_mut() {
                    let _ = w.write_all(format!("event: message\ndata: {v}\n\n").as_bytes());
                    let _ = w.flush();
                }
            });
            if let Some(resp) = self.server.handle_request(&session, &id, &method, &params, &sink, &ctl) {
                sink(resp);
            }
            let _ = s.shutdown(Shutdown::Both);
            return;
        }
        let sink: Sink = Arc::new(|_| {});
        match self.server.handle_request(&session, &id, &method, &params, &sink, &ctl) {
            Some(resp) => {
                let code = match resp.get("error").and_then(|e| e.get("code")).and_then(Value::as_i64) {
                    Some(c) if modern_version.is_some() => match c {
                        jsonrpc::METHOD_NOT_FOUND => 404,
                        jsonrpc::UNSUPPORTED_PROTOCOL_VERSION
                        | jsonrpc::HEADER_MISMATCH
                        | jsonrpc::MISSING_CLIENT_CAPABILITY
                        | jsonrpc::INVALID_REQUEST => 400,
                        jsonrpc::INVALID_PARAMS if is_meta_error(&resp) => 400,
                        crate::server::CLIENT_NOT_ALLOWED => 403,
                        _ => 200,
                    },
                    _ => 200,
                };
                let reason = match code {
                    200 => "OK",
                    400 => "Bad Request",
                    403 => "Forbidden",
                    _ => "Not Found",
                };
                let _ = respond(&mut s, code, reason, &extra_headers, &json_body(&resp));
            }
            None => {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
        let _ = s.shutdown(Shutdown::Both);
    }
}

fn is_meta_error(resp: &Value) -> bool {
    resp.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).is_some_and(|m| m.contains("_meta"))
}

/// 2026-07-28 header ↔ body validation.
fn validate_headers(req: &Request, version: &str, method: &str, params: &Value) -> Result<(), RpcError> {
    let mismatch = |m: String| RpcError::new(jsonrpc::HEADER_MISMATCH, m);
    match req.header("mcp-protocol-version") {
        None => return Err(mismatch("missing MCP-Protocol-Version header".into())),
        Some(h) if h != version => {
            return Err(mismatch(format!(
                "Header mismatch: MCP-Protocol-Version header value '{h}' does not match body value '{version}'"
            )));
        }
        _ => {}
    }
    match req.header("mcp-method") {
        None => return Err(mismatch("missing Mcp-Method header".into())),
        Some(h) if h != method => {
            return Err(mismatch(format!(
                "Header mismatch: Mcp-Method header value '{h}' does not match body value '{method}'"
            )));
        }
        _ => {}
    }
    let name_field = match method {
        "tools/call" | "prompts/get" => params.get("name"),
        "resources/read" => params.get("uri"),
        _ => None,
    };
    if let Some(body) = name_field.and_then(Value::as_str) {
        let Some(raw) = req.header("mcp-name") else {
            return Err(mismatch("missing Mcp-Name header".into()));
        };
        let decoded = decode_header_value(raw).ok_or_else(|| mismatch("malformed Mcp-Name header".into()))?;
        if decoded != body {
            return Err(mismatch(format!(
                "Header mismatch: Mcp-Name header value '{decoded}' does not match body value '{body}'"
            )));
        }
    }
    let _ = protocol::LATEST;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_value_encoding() {
        assert_eq!(encode_header_value("us-west1"), "us-west1");
        assert_eq!(encode_header_value("Hello, 世界"), "=?base64?SGVsbG8sIOS4lueVjA==?=");
        assert_eq!(encode_header_value(" padded "), "=?base64?IHBhZGRlZCA=?=");
        assert_eq!(encode_header_value("=?base64?literal?="), "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=");
        for v in ["us-west1", "Hello, 世界", " padded ", "line1\nline2", "=?base64?literal?="] {
            assert_eq!(decode_header_value(&encode_header_value(v)).as_deref(), Some(v));
        }
    }
}
