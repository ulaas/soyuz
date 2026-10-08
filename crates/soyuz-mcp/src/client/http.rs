//! Streamable HTTP client transport (reqwest + rustls on a small private runtime).

use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use parking_lot::Mutex;
use serde_json::Value;
use soyuz_ai::Secret;
use soyuz_core::OpControl;

use super::{ClientError, Transport};
use crate::http::encode_header_value;
use crate::protocol::META_PROTOCOL_VERSION;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("soyuz-mcp-http-client")
            .enable_all()
            .build()
            .expect("cannot start the MCP client runtime")
    })
}

fn client() -> Result<&'static reqwest::Client, ClientError> {
    static C: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    C.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .user_agent(concat!("soyuz/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| ClientError::Transport(format!("cannot build HTTP client: {e}")))
}

pub struct HttpTransport {
    url: String,
    token: Option<Secret>,
    session: Mutex<Option<String>>,
    legacy_version: Mutex<Option<String>>,
}

impl HttpTransport {
    pub fn new(url: &str, token: Option<Secret>) -> Self {
        HttpTransport { url: url.to_string(), token, session: Mutex::new(None), legacy_version: Mutex::new(None) }
    }

    fn build(&self, msg: &Value) -> Result<reqwest::RequestBuilder, ClientError> {
        let mut rb = client()?
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(msg.to_string());
        if let Some(t) = &self.token {
            rb = rb.bearer_auth(t.expose());
        }
        let params = msg.get("params");
        let modern =
            params.and_then(|p| p.get("_meta")).and_then(|m| m.get(META_PROTOCOL_VERSION)).and_then(Value::as_str);
        if let Some(v) = modern {
            rb = rb.header("MCP-Protocol-Version", v);
            if let Some(m) = msg.get("method").and_then(Value::as_str) {
                rb = rb.header("Mcp-Method", m);
                let name = match m {
                    "tools/call" | "prompts/get" => params.and_then(|p| p.get("name")),
                    "resources/read" => params.and_then(|p| p.get("uri")),
                    _ => None,
                };
                if let Some(n) = name.and_then(Value::as_str) {
                    rb = rb.header("Mcp-Name", encode_header_value(n));
                }
            }
        } else if let Some(v) = self.legacy_version.lock().clone() {
            rb = rb.header("MCP-Protocol-Version", v);
        }
        if let Some(s) = self.session.lock().clone() {
            rb = rb.header("Mcp-Session-Id", s);
        }
        Ok(rb)
    }

    async fn exchange(&self, msg: &Value) -> Result<Option<Value>, ClientError> {
        let resp = self.build(msg)?.send().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        if let Some(sid) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session.lock() = Some(sid.to_string());
        }
        let status = resp.status();
        let is_sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("text/event-stream"));
        let id = msg.get("id").cloned();
        if status.as_u16() == 202 || id.is_none() {
            return Ok(None);
        }
        if is_sse && status.is_success() {
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| ClientError::Transport(e.to_string()))?;
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(end) = buf.find("\n\n") {
                    let event: String = buf.drain(..end + 2).collect();
                    let data: Vec<&str> = event
                        .lines()
                        .filter_map(|l| l.strip_prefix("data:").map(|d| d.strip_prefix(' ').unwrap_or(d)))
                        .collect();
                    if data.is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(&data.join("\n"))
                        && v.get("id") == id.as_ref()
                        && (v.get("result").is_some() || v.get("error").is_some())
                    {
                        return Ok(Some(v));
                    }
                }
            }
            return Err(ClientError::Transport("the event stream ended without a response".into()));
        }
        let body = resp.bytes().await.map_err(|e| ClientError::Transport(e.to_string()))?;
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) if v.get("result").is_some() || v.get("error").is_some() => Ok(Some(v)),
            _ if status.is_success() => Err(ClientError::Protocol("response is not JSON-RPC".into())),
            _ => Err(ClientError::Transport(format!("HTTP {status}"))),
        }
    }
}

impl Transport for HttpTransport {
    fn request(&self, msg: Value, timeout: Duration, ctl: &OpControl) -> Result<Value, ClientError> {
        runtime().block_on(async {
            let work = self.exchange(&msg);
            tokio::pin!(work);
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                tokio::select! {
                    r = &mut work => return r?.ok_or_else(|| ClientError::Protocol("no response".into())),
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        // Dropping the request closes the stream, which cancels it server side.
                        if ctl.is_cancelled() {
                            return Err(ClientError::Cancelled);
                        }
                        if tokio::time::Instant::now() > deadline {
                            return Err(ClientError::Timeout);
                        }
                    }
                }
            }
        })
    }

    fn notify(&self, msg: Value) -> Result<(), ClientError> {
        runtime().block_on(self.exchange(&msg)).map(|_| ())
    }

    fn set_protocol_version(&self, v: &str) {
        *self.legacy_version.lock() = Some(v.to_string());
    }
}
