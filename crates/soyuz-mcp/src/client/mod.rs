//! MCP client: Soyuz consumes external MCP servers (`[[ai.mcp_servers]]`)
//! and offers their tools to the in-app assistant through [`ExternalTools`].
//!
//! * Transports: stdio (the server is started with `std::process::Command` — no shell)
//!   and Streamable HTTP (reqwest + rustls).
//! * Dual-era: the client probes with `server/discover` (2026-07-28) and falls back to
//!   the `initialize` handshake for legacy servers, as the spec prescribes.
//! * Trust: every external tool gets the server's configured risk class (optionally
//!   `read` for tools the server marks `readOnlyHint` when `trust_annotations` is on).
//!   Tools above read risk return a [`Plan`] that needs approval (`require_approval`),
//!   exactly like Soyuz's own write tools. Results are framed as untrusted data.

mod http;
mod registry;
mod stdio;

use std::time::Duration;

use serde_json::{Value, json};
use soyuz_core::OpControl;

use crate::jsonrpc;
use crate::protocol::{self, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION};

pub use registry::{
    ExternalTool, ExternalTools, McpRegistry, ServerStatus, open_transport, probe, probe_summary, tool_name,
};

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ClientError {
    #[error("{0}")]
    Transport(String),
    #[error("timed out")]
    Timeout,
    #[error("cancelled")]
    Cancelled,
    #[error("server error {code}: {message}")]
    Rpc { code: i64, message: String, data: Option<Value> },
    #[error("{0}")]
    Protocol(String),
}

impl ClientError {
    fn from_rpc(e: &Value) -> ClientError {
        ClientError::Rpc {
            code: e.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
            data: e.get("data").cloned(),
        }
    }

    /// An error defined by the 2026-07-28 spec (identifies a modern server).
    fn is_modern(&self) -> bool {
        matches!(self, ClientError::Rpc { code, .. } if (-32099..=-32020).contains(code))
    }
}

/// A message transport: send a request and wait for its response.
pub trait Transport: Send + Sync {
    /// Send `msg` (a request with `id`) and return the response object. Cancelling `ctl`
    /// abandons the request (the transport sends `notifications/cancelled` / closes).
    fn request(&self, msg: Value, timeout: Duration, ctl: &OpControl) -> Result<Value, ClientError>;
    fn notify(&self, msg: Value) -> Result<(), ClientError>;
    /// Legacy HTTP: remember / send the protocol version header.
    fn set_protocol_version(&self, _v: &str) {}
}

/// A remote tool.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only_hint: bool,
    pub destructive_hint: bool,
}

/// Result of `tools/call`.
#[derive(Clone, Debug, PartialEq)]
pub struct CallResult {
    /// Text of all text content blocks (non-text blocks are summarized).
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

/// A connected MCP client.
pub struct McpClient {
    transport: Box<dyn Transport>,
    next_id: std::sync::atomic::AtomicU64,
    /// `Some(version)` for modern servers; `None` = legacy session.
    modern: Option<String>,
    pub server_name: String,
    pub instructions: Option<String>,
    timeout: Duration,
}

fn client_meta(version: &str) -> Value {
    json!({
        META_PROTOCOL_VERSION: version,
        META_CLIENT_INFO: {"name": "soyuz", "version": env!("CARGO_PKG_VERSION")},
        META_CLIENT_CAPABILITIES: {},
    })
}

impl McpClient {
    /// Connect over `transport` and negotiate the protocol era.
    pub fn connect(transport: Box<dyn Transport>, timeout: Duration) -> Result<McpClient, ClientError> {
        let mut c = McpClient {
            transport,
            next_id: 1.into(),
            modern: None,
            server_name: String::new(),
            instructions: None,
            timeout,
        };
        let ctl = OpControl::new();
        // Modern probe.
        let probe = c.raw_request(
            "server/discover",
            json!({"_meta": client_meta(protocol::LATEST)}),
            &ctl,
            Some(timeout.min(Duration::from_secs(10))),
        );
        match probe {
            Ok(r) => {
                let versions: Vec<String> = r
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let v = protocol::MODERN_VERSIONS
                    .iter()
                    .find(|m| versions.iter().any(|s| s == *m))
                    .map(|s| s.to_string())
                    .ok_or_else(|| {
                        ClientError::Protocol(format!("no common protocol version (server: {versions:?})"))
                    })?;
                c.server_name = r
                    .pointer(&format!("/_meta/{}/name", protocol::META_SERVER_INFO.replace('/', "~1")))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                c.instructions = r.get("instructions").and_then(Value::as_str).map(str::to_string);
                c.modern = Some(v);
                return Ok(c);
            }
            Err(e) if e.is_modern() => {
                // UnsupportedProtocolVersion: pick from its list.
                if let ClientError::Rpc { data: Some(d), .. } = &e {
                    let supported: Vec<&str> = d
                        .get("supported")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    if let Some(v) = protocol::MODERN_VERSIONS.iter().find(|m| supported.contains(m)) {
                        c.modern = Some(v.to_string());
                        return Ok(c);
                    }
                }
                return Err(e);
            }
            Err(ClientError::Cancelled) => return Err(ClientError::Cancelled),
            Err(ClientError::Transport(t)) => return Err(ClientError::Transport(t)),
            Err(_) => {} // legacy server
        }
        let init = c.raw_request(
            "initialize",
            json!({
                "protocolVersion": protocol::LATEST_LEGACY,
                "capabilities": {},
                "clientInfo": {"name": "soyuz", "version": env!("CARGO_PKG_VERSION")},
            }),
            &ctl,
            None,
        )?;
        let v = init.get("protocolVersion").and_then(Value::as_str).unwrap_or(protocol::LATEST_LEGACY).to_string();
        if !protocol::LEGACY_VERSIONS.contains(&v.as_str()) {
            return Err(ClientError::Protocol(format!("unsupported protocol version {v}")));
        }
        c.server_name =
            init.get("serverInfo").and_then(|s| s.get("name")).and_then(Value::as_str).unwrap_or("").to_string();
        c.instructions = init.get("instructions").and_then(Value::as_str).map(str::to_string);
        c.transport.set_protocol_version(&v);
        c.transport.notify(jsonrpc::notification("notifications/initialized", json!({})))?;
        Ok(c)
    }

    pub fn is_modern(&self) -> bool {
        self.modern.is_some()
    }

    fn raw_request(
        &self,
        method: &str,
        params: Value,
        ctl: &OpControl,
        timeout: Option<Duration>,
    ) -> Result<Value, ClientError> {
        let id = json!(self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        let resp =
            self.transport.request(jsonrpc::request(&id, method, params), timeout.unwrap_or(self.timeout), ctl)?;
        if let Some(e) = resp.get("error") {
            return Err(ClientError::from_rpc(e));
        }
        let result =
            resp.get("result").cloned().ok_or_else(|| ClientError::Protocol("response without result".into()))?;
        match result.get("resultType").and_then(Value::as_str) {
            None | Some("complete") => Ok(result),
            Some(other) => Err(ClientError::Protocol(format!("unsupported resultType “{other}”"))),
        }
    }

    /// A request in the negotiated era.
    pub fn request(&self, method: &str, mut params: Value, ctl: &OpControl) -> Result<Value, ClientError> {
        if let Some(v) = &self.modern {
            if !params.is_object() {
                params = json!({});
            }
            params["_meta"] = client_meta(v);
        }
        self.raw_request(method, params, ctl, None)
    }

    /// All tools (follows pagination).
    pub fn list_tools(&self, ctl: &OpControl) -> Result<Vec<RemoteTool>, ClientError> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let r = self.request("tools/list", params, ctl)?;
            for t in r.get("tools").and_then(Value::as_array).into_iter().flatten() {
                let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
                let ann = t.get("annotations");
                out.push(RemoteTool {
                    name: name.to_string(),
                    description: t.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                    input_schema: t.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"})),
                    read_only_hint: ann.and_then(|a| a.get("readOnlyHint")).and_then(Value::as_bool).unwrap_or(false),
                    destructive_hint: ann
                        .and_then(|a| a.get("destructiveHint"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                });
            }
            cursor = r.get("nextCursor").and_then(Value::as_str).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    pub fn call_tool(&self, name: &str, args: Value, ctl: &OpControl) -> Result<CallResult, ClientError> {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}), ctl)?;
        let mut text = Vec::new();
        for c in r.get("content").and_then(Value::as_array).into_iter().flatten() {
            match c.get("type").and_then(Value::as_str) {
                Some("text") => text.push(c.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
                Some("resource_link") => {
                    text.push(format!("[resource link {}]", c.get("uri").and_then(Value::as_str).unwrap_or("?")))
                }
                Some("resource") => text.push(
                    c.get("resource")
                        .and_then(|r| r.get("text"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| "[embedded resource]".into()),
                ),
                Some(other) => text.push(format!("[{other} content omitted]")),
                None => {}
            }
        }
        Ok(CallResult {
            text: text.join("\n"),
            structured: r.get("structuredContent").cloned(),
            is_error: r.get("isError").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    pub fn ping(&self, ctl: &OpControl) -> Result<(), ClientError> {
        self.request("ping", json!({}), ctl).map(|_| ())
    }
}

pub use http::HttpTransport;
pub use stdio::{StdioTransport, StreamTransport};
