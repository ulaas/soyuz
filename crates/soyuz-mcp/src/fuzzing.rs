//! Entry points for the `fuzz/` targets (cargo feature `fuzzing`): untrusted
//! JSON-RPC / MCP input in both directions. Each function must never panic for any input.
//!
//! * [`server_messages`]: newline-delimited JSON-RPC from an MCP client, routed through the
//!   real [`Server`] (era negotiation, trust model, tool dispatch, prompts, resources) with
//!   a backend that never touches the disk, never approves and never writes an audit log.
//! * [`client_responses`]: responses from an external MCP server, fed to the
//!   [`McpClient`] (version probe, legacy handshake, `tools/list` pagination, `tools/call`
//!   content decoding).
//! * [`strings`]: the small string parsers (`file://` URIs, RFC 2047-style header values,
//!   client names, prompt arguments).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Map, Value};
use soyuz_ai::tools::{CallContext, Plan, Scope, ToolError, ToolHost, ToolInput};
use soyuz_core::OpControl;
use soyuz_core::config::Ai;

use crate::client::{ClientError, McpClient, Transport as ClientTransport};
use crate::jsonrpc::{self, Incoming};
use crate::server::{ApprovalDecision, ApprovalRequest, Backend, Mode, ProgressFn, Server, Session, Sink};
use crate::trust::Transport;

/// A tool host with no roots: every read / plan fails without I/O.
struct NullHost {
    scope: Scope,
}

impl ToolHost for NullHost {
    fn scope(&self) -> &Scope {
        &self.scope
    }
    fn read(&self, input: &ToolInput, _ctx: &CallContext) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::Unsupported(input.name().into()))
    }
    fn plan(&self, input: &ToolInput, _ctx: &CallContext) -> Result<Plan, ToolError> {
        Err(ToolError::Unsupported(input.name().into()))
    }
}

struct NullBackend {
    host: Arc<NullHost>,
    ai: Ai,
    mode: Mode,
}

impl Backend for NullBackend {
    fn mode(&self) -> Mode {
        self.mode
    }
    fn host(&self) -> Arc<dyn ToolHost> {
        self.host.clone()
    }
    fn ai(&self) -> Ai {
        self.ai.clone()
    }
    fn request_approval(&self, _req: ApprovalRequest, _ctl: &OpControl) -> ApprovalDecision {
        ApprovalDecision::Rejected("fuzzing".into())
    }
    fn execute(&self, _plan: &Plan, _ctx: &CallContext, _p: &ProgressFn<'_>) -> Result<Value, ToolError> {
        Err(ToolError::Unsupported("fuzzing".into()))
    }
    fn register_pending(&self, _name: &str, _t: Transport) {}
    fn audit(&self, _rec: soyuz_ai::audit::AuditRecord) {}
}

fn server(mode: Mode) -> Arc<Server> {
    let mut ai = Ai::default();
    // A root that cannot exist, so no scope check ever resolves into the real home folder.
    ai.scope.allowed_roots = vec![PathBuf::from("Z:\\soyuz-fuzz\\nonexistent")];
    let scope = Scope::new(Vec::new(), &[], false);
    Server::new(Arc::new(NullBackend { host: Arc::new(NullHost { scope }), ai, mode }))
}

/// Feed newline-delimited JSON-RPC messages from an untrusted MCP client to the server.
/// The first byte picks GUI / headless mode and whether the transport names the client.
pub fn server_messages(data: &[u8]) {
    let Some((&flags, rest)) = data.split_first() else { return };
    let mode = if flags & 1 == 0 { Mode::Gui } else { Mode::Headless };
    let fixed = (flags & 2 != 0).then(|| "fuzz-client".to_string());
    let server = server(mode);
    let session = Session::new(Transport::Memory, fixed);
    let sink: Sink = Arc::new(|v: Value| {
        let _ = v.to_string();
    });
    for line in rest.split(|&b| b == b'\n').take(64) {
        let Ok(msg) = serde_json::from_slice::<Value>(line) else { continue };
        if let Incoming::Request { id, method, params } = jsonrpc::classify(msg.clone()) {
            // Handled inline (the stream loop runs long calls on worker threads; the
            // routing is the same).
            let ctl = OpControl::new();
            if let Some(resp) = server.handle_request(&session, &id, &method, &params, &sink, &ctl) {
                sink(resp);
            }
        } else if let Some(h) = server.dispatch_message(&session, msg, &sink) {
            // Notifications, responses and invalid messages never spawn a worker.
            let _ = h.join();
        }
    }
    session.cancel_all();
}

/// A transport that answers each request with the next line of fuzz input.
struct Scripted {
    replies: Mutex<VecDeque<Value>>,
}

impl ClientTransport for Scripted {
    fn request(&self, _msg: Value, _timeout: Duration, _ctl: &OpControl) -> Result<Value, ClientError> {
        self.replies.lock().pop_front().ok_or_else(|| ClientError::Transport("eof".into()))
    }
    fn notify(&self, _msg: Value) -> Result<(), ClientError> {
        Ok(())
    }
}

/// Answer the MCP client's requests (probe / initialize, `tools/list` pages, `tools/call`,
/// `ping`) with untrusted server responses, one JSON value per line.
pub fn client_responses(data: &[u8]) {
    let replies: VecDeque<Value> =
        data.split(|&b| b == b'\n').take(64).filter_map(|l| serde_json::from_slice(l).ok()).collect();
    let transport = Box::new(Scripted { replies: Mutex::new(replies) });
    let Ok(client) = McpClient::connect(transport, Duration::from_millis(10)) else { return };
    let ctl = OpControl::new();
    let _ = client.is_modern();
    let _ = client.list_tools(&ctl);
    let _ = client.call_tool("fuzz", serde_json::json!({}), &ctl);
    let _ = client.ping(&ctl);
}

/// String parsers reached by untrusted input.
pub fn strings(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    let _ = crate::protocol::file_uri_to_path(&s);
    let _ = crate::protocol::file_uri_to_path(&format!("file://{s}"));
    let _ = crate::http::decode_header_value(&s);
    let _ = crate::http::decode_header_value(&format!("=?base64?{s}?="));
    let _ = crate::trust::sanitize_name(&s);
    let _ = jsonrpc::id_key(&Value::String(s.to_string()));
    // NUL-separated values for each prompt's arguments, in declaration order.
    for p in crate::protocol::PROMPTS {
        let mut args = Map::new();
        for ((k, _, _), part) in p.args.iter().zip(s.split('\u{0}')) {
            args.insert((*k).into(), Value::String(part.into()));
        }
        let _ = crate::protocol::render_prompt(p.name, &args);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_do_not_panic() {
        let session = concat!(
            "\u{1}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"clientInfo\":{\"name\":\"x\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"mkdir\",\"arguments\":{\"path\":\"Z:\\a\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"resources/read\",\"params\":{\"uri\":\"file:///Z:/a\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"prompts/get\",\"params\":{\"name\":\"clean_duplicates\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":[],\"method\":\"x\"}\n[1,2]\n{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{}}\nnot json\n",
        );
        for flags in 0u8..4 {
            let mut d = vec![flags];
            d.extend_from_slice(&session.as_bytes()[1..]);
            server_messages(&d);
        }
        server_messages(b"");
        client_responses(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"supportedVersions\":[\"2026-07-28\"]}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"a\"},{}],\"nextCursor\":\"c\"}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"tools\":[]}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":4,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"},{\"type\":\"image\"}]}}\n",
            )
            .as_bytes(),
        );
        client_responses(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32601,\"message\":\"x\"}}\n{\"result\":{\"protocolVersion\":\"2024-11-05\"}}");
        strings("file:///C:/a%20b\u{0}date".as_bytes());
        strings(&[0xff, b'%', b'%', 0]);
    }
}
