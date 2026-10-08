//! The MCP server core: request routing, the client trust model, approvals, audit and
//! the newline-delimited JSON-RPC connection loop shared by stdio and the named pipe.
//!
//! The server is synchronous: each `tools/call` / `resources/read` of a stream
//! connection runs on its own thread so `notifications/cancelled` can stop it.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use soyuz_ai::audit::{AuditRecord, summarize_args};
use soyuz_ai::tools::{
    self, CallContext, Plan, Risk, Scope, ToolError, ToolHost, ToolInput, ToolResponse, frame_error, frame_untrusted,
};
use soyuz_core::OpControl;
use soyuz_core::config::{Ai, AiPrivacy};

use crate::jsonrpc::{self, Incoming, RpcError};
use crate::protocol::{self, META_CLIENT_CAPABILITIES, META_CLIENT_INFO, META_PROTOCOL_VERSION};
use crate::trust::{self, ClientPolicy, Transport};

/// Application error: the client is not allowed to use Soyuz.
pub const CLIENT_NOT_ALLOWED: i64 = -31001;

/// `(progress, total, message)`.
pub type ProgressFn<'a> = dyn Fn(f64, Option<f64>, Option<String>) + Send + Sync + 'a;

/// Outbound messages of a connection (responses and notifications).
pub type Sink = Arc<dyn Fn(Value) + Send + Sync>;

/// Is the app window there to show approval cards and drive the UI?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Attached to the running Soyuz window.
    Gui,
    /// `soyuz --mcp` without a window.
    Headless,
}

/// A plan an MCP client wants to run.
#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    pub client: String,
    pub transport: Transport,
    pub tool: String,
    pub args: Value,
    pub plan: Plan,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalDecision {
    Approved,
    Rejected(String),
    /// Nobody can approve (headless) — the message explains what to do.
    Unavailable(String),
    TimedOut,
    /// The call was cancelled while waiting.
    Cancelled,
    /// The user edited the arguments: the server plans them again (dry run, same client
    /// policy) and asks for approval of the new plan.
    Edited(Value),
}

/// What the server needs from its host process (the app window or the headless runner).
pub trait Backend: Send + Sync {
    fn mode(&self) -> Mode;
    /// The tool host; its scope is the global `[ai.scope]` (clients are narrowed on top).
    fn host(&self) -> Arc<dyn ToolHost>;
    /// Current `[ai]` configuration.
    fn ai(&self) -> Ai;
    /// Ask the user to approve a plan; blocks until answered, timed out or `ctl` cancelled.
    fn request_approval(&self, req: ApprovalRequest, ctl: &OpControl) -> ApprovalDecision;
    /// Run an approved plan (submit its job and wait, with progress).
    fn execute(&self, plan: &Plan, ctx: &CallContext, progress: &ProgressFn<'_>) -> Result<Value, ToolError>;
    /// Remember an unknown client as pending (read-only until approved in Settings).
    fn register_pending(&self, name: &str, transport: Transport);
    /// Append an audit record.
    fn audit(&self, rec: AuditRecord) {
        soyuz_ai::audit::record(&rec);
    }
}

/// A tool host with a narrower scope (one client's folders).
struct ScopedHost<'a> {
    inner: &'a dyn ToolHost,
    scope: &'a Scope,
}

impl ToolHost for ScopedHost<'_> {
    fn scope(&self) -> &Scope {
        self.scope
    }
    fn read(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        self.inner.read(input, ctx)
    }
    fn plan(&self, input: &ToolInput, ctx: &CallContext) -> Result<Plan, ToolError> {
        self.inner.plan(input, ctx)
    }
    fn ui(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        self.inner.ui(input, ctx)
    }
    fn meta(&self, input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        self.inner.meta(input, ctx)
    }
    fn execute(&self, plan: &Plan, ctx: &CallContext) -> Result<Value, ToolError> {
        self.inner.execute(plan, ctx)
    }
}

/// Per-connection state (stdio / pipe connection, or one legacy HTTP session).
pub struct Session {
    pub transport: Transport,
    /// Name fixed by the transport (`--mcp-client`, pipe hello, HTTP token).
    fixed_name: Option<String>,
    /// Legacy protocol version negotiated by `initialize`.
    legacy: Mutex<Option<String>>,
    /// Name the client reported (`clientInfo.name`).
    reported_name: Mutex<Option<String>>,
    registered: Mutex<bool>,
    inflight: Mutex<HashMap<String, Arc<OpControl>>>,
}

impl Session {
    pub fn new(transport: Transport, fixed_name: Option<String>) -> Arc<Session> {
        Arc::new(Session {
            transport,
            fixed_name: fixed_name.map(|n| trust::sanitize_name(&n)).filter(|n| !n.is_empty()),
            legacy: Mutex::new(None),
            reported_name: Mutex::new(None),
            registered: Mutex::new(false),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    /// The client name used for policy and audit.
    pub fn client_name(&self, meta_name: Option<&str>) -> String {
        if let Some(n) = &self.fixed_name {
            return n.clone();
        }
        let reported = meta_name.map(str::to_string).or_else(|| self.reported_name.lock().clone());
        reported.map(|n| trust::sanitize_name(&n)).filter(|n| !n.is_empty()).unwrap_or_else(|| "unknown-client".into())
    }

    pub fn legacy_version(&self) -> Option<String> {
        self.legacy.lock().clone()
    }

    /// Cancel every in-flight request (connection closed).
    pub fn cancel_all(&self) {
        for c in self.inflight.lock().values() {
            c.cancel();
        }
    }
}

/// How a request is framed.
#[derive(Clone, Debug)]
struct Era {
    modern: bool,
    meta_client: Option<String>,
}

pub struct Server {
    backend: Arc<dyn Backend>,
}

fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

impl Server {
    pub fn new(backend: Arc<dyn Backend>) -> Arc<Server> {
        Arc::new(Server { backend })
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// Validate the protocol era of a request.
    fn era(&self, session: &Session, method: &str, params: &Value) -> Result<Era, RpcError> {
        let meta = params.get("_meta");
        let version = meta.and_then(|m| m.get(META_PROTOCOL_VERSION)).and_then(Value::as_str);
        let meta_client = meta
            .and_then(|m| m.get(META_CLIENT_INFO))
            .and_then(|c| c.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string);
        match version {
            Some(v) if method != "initialize" => {
                if !protocol::is_modern(v) {
                    return Err(RpcError::new(jsonrpc::UNSUPPORTED_PROTOCOL_VERSION, "Unsupported protocol version")
                        .with_data(json!({"supported": protocol::all_versions(), "requested": v})));
                }
                if !meta.and_then(|m| m.get(META_CLIENT_CAPABILITIES)).is_some_and(Value::is_object) {
                    return Err(RpcError::invalid_params(format!("_meta is missing “{META_CLIENT_CAPABILITIES}”")));
                }
                Ok(Era { modern: true, meta_client })
            }
            _ => {
                // Legacy, or a lenient client that skipped `initialize`.
                let _ = session;
                Ok(Era { modern: false, meta_client })
            }
        }
    }

    /// Compute the client's policy (registering unknown stdio clients as pending).
    fn policy(&self, session: &Session, era: &Era, ai: &Ai) -> Result<ClientPolicy, RpcError> {
        if !ai.enabled {
            return Err(RpcError::new(
                CLIENT_NOT_ALLOWED,
                "AI features are turned off in Soyuz (Settings → AI assistant)",
            ));
        }
        let name = session.client_name(era.meta_client.as_deref());
        let entry = trust::find(ai, &name);
        if entry.is_none() {
            let can_register = matches!(session.transport, Transport::Stdio | Transport::Pipe | Transport::Memory);
            if !can_register || !ai.mcp.register_unknown {
                return Err(RpcError::new(
                    CLIENT_NOT_ALLOWED,
                    format!("MCP client “{name}” is not registered in Soyuz (Settings → AI assistant → Connections)"),
                ));
            }
            let mut reg = session.registered.lock();
            if !*reg {
                *reg = true;
                self.backend.register_pending(&name, session.transport);
                self.audit(session, &name, "client_registered", "", &Value::Null, "pending", None, "");
            }
        }
        Ok(ClientPolicy::new(ai, entry, &name, session.transport))
    }

    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        session: &Session,
        client: &str,
        event: &str,
        tool: &str,
        args: &Value,
        outcome: &str,
        job_id: Option<u64>,
        detail: &str,
    ) {
        let mut r = AuditRecord::new("mcp", event, outcome);
        r.client = client.to_string();
        r.via = session.transport.label().to_string();
        r.tool = tool.to_string();
        if !args.is_null() {
            r.args = summarize_args(args);
        }
        r.job_id = job_id;
        r.detail = detail.chars().take(500).collect();
        self.backend.audit(r);
    }

    /// Handle one request; `None` = no response (cancelled).
    pub fn handle_request(
        &self,
        session: &Session,
        id: &Value,
        method: &str,
        params: &Value,
        sink: &Sink,
        ctl: &Arc<OpControl>,
    ) -> Option<Value> {
        let era = match self.era(session, method, params) {
            Ok(e) => e,
            Err(e) => return Some(jsonrpc::error(id, &e)),
        };
        match self.route(session, &era, id, method, params, sink, ctl) {
            Ok(Some(mut result)) => {
                if era.modern
                    && let Some(o) = result.as_object_mut()
                {
                    o.insert("resultType".into(), json!("complete"));
                    let meta = o.entry("_meta").or_insert_with(|| json!({}));
                    meta[protocol::META_SERVER_INFO] = protocol::server_info();
                }
                Some(jsonrpc::result(id, result))
            }
            Ok(None) => None,
            Err(e) => Some(jsonrpc::error(id, &e)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn route(
        &self,
        session: &Session,
        era: &Era,
        id: &Value,
        method: &str,
        params: &Value,
        sink: &Sink,
        ctl: &Arc<OpControl>,
    ) -> Result<Option<Value>, RpcError> {
        let _ = id;
        match method {
            "initialize" => {
                let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = if protocol::LEGACY_VERSIONS.contains(&requested) {
                    requested.to_string()
                } else {
                    protocol::LATEST_LEGACY.to_string()
                };
                *session.legacy.lock() = Some(version.clone());
                if let Some(n) = params.get("clientInfo").and_then(|c| c.get("name")).and_then(Value::as_str) {
                    *session.reported_name.lock() = Some(n.to_string());
                }
                Ok(Some(json!({
                    "protocolVersion": version,
                    "capabilities": protocol::capabilities(),
                    "serverInfo": protocol::server_info(),
                    "instructions": protocol::INSTRUCTIONS,
                })))
            }
            "ping" => Ok(Some(json!({}))),
            "server/discover" => Ok(Some(json!({
                "supportedVersions": protocol::MODERN_VERSIONS,
                "capabilities": protocol::capabilities(),
                "instructions": protocol::INSTRUCTIONS,
            }))),
            "tools/list" => {
                let ai = self.backend.ai();
                let policy = self.policy(session, era, &ai)?;
                Ok(Some(json!({"tools": list_tools(policy.max_risk, ai.privacy)})))
            }
            "tools/call" => self.call_tool(session, era, params, sink, ctl),
            "resources/list" => {
                let ai = self.backend.ai();
                let policy = self.policy(session, era, &ai)?;
                Ok(Some(json!({"resources": list_resources(&policy, self.backend.mode())})))
            }
            "resources/templates/list" => Ok(Some(json!({"resourceTemplates": [{
                "uriTemplate": "file:///{path}",
                "name": "file",
                "title": "Files and folders in the allowed folders",
                "description": "A folder reads as a listing; a text file reads as its text (needs content access).",
            }]}))),
            "resources/read" => self.read_resource(session, era, params, ctl),
            "prompts/list" => {
                Ok(Some(json!({"prompts": protocol::PROMPTS.iter().map(protocol::prompt_json).collect::<Vec<_>>()})))
            }
            "prompts/get" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::invalid_params("missing name"))?;
                let args = params.get("arguments").map(obj).unwrap_or_default();
                protocol::render_prompt(name, &args).map(Some).map_err(RpcError::invalid_params)
            }
            "logging/setLevel" | "notifications/roots/list_changed" => Ok(Some(json!({}))),
            _ => Err(RpcError::new(jsonrpc::METHOD_NOT_FOUND, format!("method not found: {method}"))),
        }
    }

    fn call_tool(
        &self,
        session: &Session,
        era: &Era,
        params: &Value,
        sink: &Sink,
        ctl: &Arc<OpControl>,
    ) -> Result<Option<Value>, RpcError> {
        let name =
            params.get("name").and_then(Value::as_str).ok_or_else(|| RpcError::invalid_params("missing tool name"))?;
        let Some(def) = tools::def(name) else {
            return Err(RpcError::invalid_params(format!("Unknown tool: {name}")));
        };
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let ai = self.backend.ai();
        let policy = self.policy(session, era, &ai)?;
        let ctx = CallContext {
            max_risk: policy.max_risk,
            privacy: ai.privacy,
            caller: format!("mcp:{}", policy.name),
            ctl: ctl.clone(),
        };
        let inner = self.backend.host();
        let host = ScopedHost { inner: &*inner, scope: &policy.scope };
        let progress_token = params.get("_meta").and_then(|m| m.get("progressToken")).cloned();
        let progress = |p: f64, total: Option<f64>, message: Option<String>| {
            if let Some(tok) = &progress_token
                && !ctl.is_cancelled()
            {
                let mut n = json!({"progressToken": tok, "progress": p});
                if let Some(t) = total {
                    n["total"] = json!(t);
                }
                if let Some(m) = message {
                    n["message"] = json!(m);
                }
                sink(jsonrpc::notification("notifications/progress", n));
            }
        };
        let audit = |outcome: &str, job: Option<u64>, detail: &str| {
            self.audit(session, &policy.name, "tool_call", name, &args, outcome, job, detail);
        };
        let response = tools::dispatch(&host, name, args.clone(), &ctx);
        let result = match response {
            Ok(ToolResponse::Data(v)) => {
                audit("ok", None, "");
                text_result(frame_untrusted(name, &v), false)
            }
            Ok(r @ ToolResponse::Plan(_)) => {
                audit("plan", None, "dry run");
                text_result(r.to_model_text(name), false)
            }
            Ok(ToolResponse::NeedsApproval(first)) => {
                let mut plan = first;
                let mut edited: Option<Value> = None;
                if !plan.executable {
                    audit("refused", None, "plan not executable");
                    let v = json!({"status": "not_executable", "reason": "the plan has blocking conflicts or nothing to do", "plan": plan.to_model_value()});
                    return Ok(Some(text_result(frame_untrusted(name, &v), true)));
                }
                let must_ask = policy.require_approval || name == "run_custom_command";
                let mut rounds = 0;
                if must_ask {
                    loop {
                        rounds += 1;
                        let req = ApprovalRequest {
                            client: policy.name.clone(),
                            transport: session.transport,
                            tool: name.to_string(),
                            args: edited.clone().unwrap_or_else(|| args.clone()),
                            plan: plan.clone(),
                        };
                        match self.backend.request_approval(req, ctl) {
                            ApprovalDecision::Edited(new_args) if rounds < 5 => {
                                self.audit(
                                    session,
                                    &policy.name,
                                    "edit",
                                    name,
                                    &new_args,
                                    "edited",
                                    None,
                                    "the user edited the arguments",
                                );
                                match tools::dispatch(&host, name, new_args.clone(), &ctx) {
                                    Ok(ToolResponse::NeedsApproval(p)) if p.executable => {
                                        plan = p;
                                        edited = Some(new_args);
                                        continue;
                                    }
                                    Ok(_) => {
                                        audit("refused", None, "the edited plan is not executable");
                                        let v = json!({"status": "not_executable", "reason": "the user's edit produced a plan with blocking conflicts or nothing to do"});
                                        return Ok(Some(text_result(frame_untrusted(name, &v), true)));
                                    }
                                    Err(e) => {
                                        audit("error", None, &e.to_string());
                                        return Ok(Some(text_result(frame_error(name, &e), true)));
                                    }
                                }
                            }
                            ApprovalDecision::Edited(_) => {
                                audit("rejected", None, "too many edits");
                                let v = json!({"status": "rejected_by_user", "reason": "edited too many times"});
                                return Ok(Some(text_result(frame_untrusted(name, &v), true)));
                            }
                            ApprovalDecision::Approved => {
                                self.audit(
                                    session,
                                    &policy.name,
                                    "approval",
                                    name,
                                    &Value::Null,
                                    "approved",
                                    None,
                                    &plan.summary,
                                );
                                break;
                            }
                            ApprovalDecision::Rejected(why) => {
                                audit("rejected", None, &why);
                                let v = json!({"status": "rejected_by_user", "reason": why, "plan": plan.summary});
                                return Ok(Some(text_result(frame_untrusted(name, &v), true)));
                            }
                            ApprovalDecision::Unavailable(msg) => {
                                audit("refused", None, &msg);
                                let v = json!({"status": "approval_unavailable", "reason": msg, "plan": plan.to_model_value()});
                                return Ok(Some(text_result(frame_untrusted(name, &v), true)));
                            }
                            ApprovalDecision::TimedOut => {
                                audit("timed_out", None, "");
                                let v = json!({"status": "approval_timed_out", "reason": "nobody answered the approval card in Soyuz"});
                                return Ok(Some(text_result(frame_untrusted(name, &v), true)));
                            }
                            ApprovalDecision::Cancelled => {
                                audit("cancelled", None, "");
                                return Ok(None);
                            }
                        }
                    }
                }
                match self.backend.execute(&plan, &ctx, &progress) {
                    Ok(v) => {
                        let job = v.get("job_id").and_then(Value::as_u64);
                        audit("ok", job, &plan.summary);
                        let mut v = json!({"status": "done", "summary": plan.summary, "result": v});
                        if let Some(a) = edited {
                            v["edited_by_user"] = json!(true);
                            v["arguments_used"] = a;
                        }
                        text_result(frame_untrusted(name, &v), false)
                    }
                    Err(ToolError::Cancelled) => {
                        audit("cancelled", None, "");
                        return Ok(None);
                    }
                    Err(e) => {
                        audit("error", None, &e.to_string());
                        text_result(frame_error(name, &e), true)
                    }
                }
            }
            Err(ToolError::Cancelled) => {
                audit("cancelled", None, "");
                return Ok(None);
            }
            Err(ToolError::UnknownTool(t)) => return Err(RpcError::invalid_params(format!("Unknown tool: {t}"))),
            Err(e) => {
                audit("error", None, &e.to_string());
                let mut msg = frame_error(name, &e);
                if let ToolError::RiskTooHigh { .. } = e {
                    msg.push_str(if policy.pending {
                        "\nThis MCP client is waiting for approval in Soyuz (Settings → AI assistant → Connections) and is read-only until then."
                    } else {
                        "\nThe user limited what this MCP client may do (Settings → AI assistant → Connections)."
                    });
                }
                text_result(msg, true)
            }
        };
        if ctl.is_cancelled() {
            return Ok(None);
        }
        let _ = def;
        Ok(Some(result))
    }

    fn read_resource(
        &self,
        session: &Session,
        era: &Era,
        params: &Value,
        ctl: &Arc<OpControl>,
    ) -> Result<Option<Value>, RpcError> {
        let uri = params.get("uri").and_then(Value::as_str).ok_or_else(|| RpcError::invalid_params("missing uri"))?;
        let ai = self.backend.ai();
        let policy = self.policy(session, era, &ai)?;
        let not_found = |why: String| {
            let code = if era.modern { jsonrpc::INVALID_PARAMS } else { jsonrpc::LEGACY_RESOURCE_NOT_FOUND };
            RpcError::new(code, format!("Resource not found: {why}")).with_data(json!({"uri": uri}))
        };
        let (tool, args) = match uri {
            "soyuz://location/current" => ("get_current_location", json!({})),
            "soyuz://selection" => ("get_selection", json!({})),
            "soyuz://history" => ("history_list", json!({"limit": 50})),
            u if u.starts_with("file://") => {
                let path = protocol::file_uri_to_path(u).ok_or_else(|| not_found("bad file URI".into()))?;
                let checked = policy.scope.check_read(&path).map_err(|e| not_found(e.to_string()))?;
                match soyuz_fs::stat(&checked, true) {
                    Ok(e) if e.is_dir_like() => ("list_dir", json!({"path": checked})),
                    Ok(_) => ("read_text", json!({"path": checked})),
                    Err(e) => return Err(not_found(e.to_string())),
                }
            }
            _ => return Err(not_found("unknown URI".into())),
        };
        let ctx = CallContext {
            max_risk: Risk::Read,
            privacy: ai.privacy,
            caller: format!("mcp:{}", policy.name),
            ctl: ctl.clone(),
        };
        let inner = self.backend.host();
        let host = ScopedHost { inner: &*inner, scope: &policy.scope };
        let res = tools::dispatch(&host, tool, args.clone(), &ctx);
        let outcome = if res.is_ok() { "ok" } else { "error" };
        let detail = res.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
        self.audit(session, &policy.name, "resource_read", tool, &json!({"uri": uri}), outcome, None, &detail);
        match res {
            Ok(r) => {
                let mime = if tool == "read_text" { "text/plain" } else { "application/json" };
                Ok(Some(json!({"contents": [{"uri": uri, "mimeType": mime, "text": r.to_model_text(tool)}]})))
            }
            Err(ToolError::Cancelled) => Ok(None),
            Err(ToolError::ContentNotAllowed) if ai.privacy != AiPrivacy::Contents => Err(RpcError::invalid_params(
                "reading file contents is not allowed by the Soyuz privacy settings (metadata only)",
            )),
            Err(e) => Err(not_found(e.to_string())),
        }
    }

    /// Serve newline-delimited JSON-RPC on a byte stream until EOF.
    pub fn serve_lines(self: &Arc<Self>, reader: impl BufRead, writer: Box<dyn Write + Send>, session: Arc<Session>) {
        let writer = Arc::new(Mutex::new(writer));
        let sink: Sink = {
            let w = writer.clone();
            Arc::new(move |v: Value| {
                let mut line = v.to_string();
                line.push('\n');
                let mut w = w.lock();
                if w.write_all(line.as_bytes()).and_then(|_| w.flush()).is_err() {
                    log::debug!("mcp: write failed (peer gone)");
                }
            })
        };
        let mut workers = Vec::new();
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(e) => {
                    sink(jsonrpc::error(
                        &Value::Null,
                        &RpcError::new(jsonrpc::PARSE_ERROR, format!("parse error: {e}")),
                    ));
                    continue;
                }
            };
            if let Some(h) = self.dispatch_message(&session, msg, &sink) {
                workers.push(h);
            }
            workers.retain(|h: &std::thread::JoinHandle<()>| !h.is_finished());
        }
        session.cancel_all();
        for h in workers {
            let _ = h.join();
        }
    }

    /// Handle one decoded message of a stream connection. Long requests run on a worker
    /// thread (returned) so they can be cancelled.
    pub fn dispatch_message(
        self: &Arc<Self>,
        session: &Arc<Session>,
        msg: Value,
        sink: &Sink,
    ) -> Option<std::thread::JoinHandle<()>> {
        match jsonrpc::classify(msg) {
            Incoming::Request { id, method, params } => {
                let ctl = OpControl::new();
                let key = jsonrpc::id_key(&id);
                let long = matches!(method.as_str(), "tools/call" | "resources/read");
                if !long {
                    if let Some(resp) = self.handle_request(session, &id, &method, &params, sink, &ctl) {
                        sink(resp);
                    }
                    return None;
                }
                session.inflight.lock().insert(key.clone(), ctl.clone());
                let (server, session, sink) = (self.clone(), session.clone(), sink.clone());
                std::thread::Builder::new()
                    .name("soyuz-mcp-call".into())
                    .spawn(move || {
                        let resp = server.handle_request(&session, &id, &method, &params, &sink, &ctl);
                        session.inflight.lock().remove(&key);
                        if let Some(r) = resp
                            && !ctl.is_cancelled()
                        {
                            sink(r);
                        }
                    })
                    .ok()
            }
            Incoming::Notification { method, params } => {
                if method == "notifications/cancelled"
                    && let Some(rid) = params.get("requestId")
                    && let Some(c) = session.inflight.lock().get(&jsonrpc::id_key(rid))
                {
                    log::info!(
                        "mcp: request {rid} cancelled ({})",
                        params.get("reason").and_then(Value::as_str).unwrap_or("no reason")
                    );
                    c.cancel();
                }
                None
            }
            Incoming::Response => None,
            Incoming::Invalid(err, id) => {
                sink(jsonrpc::error(&id, &err));
                None
            }
        }
    }
}

/// `tools/list` for a risk ceiling and privacy level (catalog order).
pub fn list_tools(max_risk: Risk, privacy: AiPrivacy) -> Vec<Value> {
    tools::catalog()
        .iter()
        .filter(|d| d.risk <= max_risk && (!d.content || privacy == AiPrivacy::Contents))
        .map(protocol::tool_json)
        .collect()
}

fn list_resources(policy: &ClientPolicy, mode: Mode) -> Vec<Value> {
    let mut out = Vec::new();
    if mode == Mode::Gui {
        out.push(json!({"uri": "soyuz://location/current", "name": "current-location", "title": "Current folder", "description": "The folder shown in the focused Soyuz pane.", "mimeType": "application/json"}));
        out.push(json!({"uri": "soyuz://selection", "name": "selection", "title": "Selection", "description": "The files selected in the focused Soyuz pane.", "mimeType": "application/json"}));
    }
    out.push(json!({"uri": "soyuz://history", "name": "history", "title": "Operation history", "description": "Recent file operations that can be undone.", "mimeType": "application/json"}));
    for r in policy.scope.roots() {
        out.push(json!({
            "uri": protocol::path_to_file_uri(r),
            "name": soyuz_fs::strip_verbatim(r).to_string_lossy(),
            "title": "Allowed folder",
            "mimeType": "inode/directory",
        }));
    }
    out
}
