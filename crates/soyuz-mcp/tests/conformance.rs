//! MCP lifecycle / conformance against an in-process client: legacy handshake, modern
//! per-request metadata, tools/list vs the catalog, read and write tools in headless
//! mode, the trust model, resources, prompts, cancellation and audit.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use soyuz_ai::tools::{self, CallContext, Plan, Scope, ToolError, ToolHost, ToolInput};
use soyuz_core::OpControl;
use soyuz_core::config::Ai;
use soyuz_mcp::{ApprovalDecision, ApprovalRequest, Backend, Mode, ProgressFn, Server};

#[test]
fn legacy_lifecycle() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("tester"));
    let r = c.call(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "x", "version": "1"}}),
    );
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "soyuz");
    for cap in ["tools", "resources", "prompts"] {
        assert!(r["result"]["capabilities"][cap].is_object(), "{cap}");
    }
    assert!(r["result"].get("resultType").is_none(), "legacy results carry no resultType");
    // Unknown version → we answer with our latest legacy version.
    let r = c.call("initialize", json!({"protocolVersion": "1999-01-01", "capabilities": {}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    assert!(c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
    assert!(c.rx.recv_timeout(Duration::from_millis(100)).is_err(), "notifications get no response");
    assert_eq!(c.call("ping", json!({}))["result"], json!({}));
    let r = c.call("bogus/method", json!({}));
    assert_eq!(r["error"]["code"], -32601);
}

#[test]
fn modern_requests_are_stateless_and_validated() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("tester"));
    let r = c.call("server/discover", json!({"_meta": modern_meta()}));
    assert_eq!(r["result"]["resultType"], "complete");
    assert!(r["result"]["supportedVersions"].as_array().unwrap().contains(&json!(MODERN)));
    assert_eq!(r["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "soyuz");

    let r = c.call("tools/list", json!({"_meta": modern_meta()}));
    assert_eq!(r["result"]["resultType"], "complete");

    let mut meta = modern_meta();
    meta["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let r = c.call("tools/list", json!({"_meta": meta}));
    assert_eq!(r["error"]["code"], -32022);
    assert_eq!(r["error"]["data"]["requested"], "1900-01-01");
    assert!(r["error"]["data"]["supported"].as_array().unwrap().contains(&json!(MODERN)));

    let mut meta = modern_meta();
    meta.as_object_mut().unwrap().remove("io.modelcontextprotocol/clientCapabilities");
    let r = c.call("tools/list", json!({"_meta": meta}));
    assert_eq!(r["error"]["code"], -32602);

    // Malformed messages.
    c.send(json!({"jsonrpc": "2.0", "id": 99, "method": "ping", "params": [1]}));
    let v = c.rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(v["error"]["code"], -32602);
}

#[test]
fn tools_list_matches_the_catalog() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("tester"));
    let r = c.call("tools/list", json!({}));
    let listed = r["result"]["tools"].as_array().unwrap();
    let catalog = tools::catalog();
    assert_eq!(listed.len(), catalog.len());
    for (t, d) in listed.iter().zip(&catalog) {
        assert_eq!(t["name"], d.name);
        assert_eq!(t["inputSchema"], d.schema, "{}", d.name);
        assert_eq!(t["annotations"]["readOnlyHint"], d.risk == tools::Risk::Read);
        assert_eq!(t["annotations"]["destructiveHint"], d.risk == tools::Risk::Destructive);
        assert!(t["description"].as_str().unwrap().starts_with(d.description));
    }

    // Metadata-only privacy hides content tools; a write-limited client sees no destructive tools.
    let mut ai = fx.ai();
    ai.privacy = soyuz_core::config::AiPrivacy::MetadataOnly;
    let (server, _) = fx.headless(ai);
    let mut c = Conn::new(server, Some("trusted"));
    let names: Vec<String> = c.call("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(!names.contains(&"read_text".to_string()));
    assert!(names.contains(&"move".to_string()));
    assert!(!names.contains(&"trash".to_string()));
}

#[test]
fn unknown_clients_are_registered_pending_and_read_only() {
    let fx = Fixture::new();
    let (server, backend) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("Some New Agent"));
    let tools = c.call("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone();
    assert!(!tools.is_empty());
    assert!(tools.iter().all(|t| t["annotations"]["readOnlyHint"] == true), "pending clients see read tools only");
    let pending = backend.ai().mcp_clients.iter().find(|c| c.name == "some-new-agent").cloned().unwrap();
    assert!(pending.pending);
    let text = std::fs::read_to_string(&fx.config).unwrap();
    assert!(text.contains("some-new-agent") && text.contains("pending = true"), "{text}");
    // Registered once per session.
    c.call("tools/list", json!({}));
    assert_eq!(backend.ai().mcp_clients.iter().filter(|c| c.name == "some-new-agent").count(), 1);
    // A write tool is refused with a hint.
    let r = c.tool("mkdir", json!({"path": fx.root.join("x")}));
    assert!(is_error(&r));
    assert!(text_of(&r).contains("waiting for approval"), "{}", text_of(&r));
    assert!(!fx.root.join("x").exists());

    // register_unknown = false refuses unknown clients.
    let mut ai = fx.ai();
    ai.mcp.register_unknown = false;
    let (server, _) = fx.headless(ai);
    let mut c = Conn::new(server, Some("stranger"));
    let r = c.call("tools/list", json!({}));
    assert_eq!(r["error"]["code"], soyuz_mcp::server::CLIENT_NOT_ALLOWED);
}

#[test]
fn read_tool_returns_framed_data() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("tester"));
    let r = c.tool("list_dir", json!({"path": fx.root}));
    assert!(!is_error(&r), "{r}");
    let t = text_of(&r);
    assert!(t.contains("trust=\"untrusted\"") && t.contains("hello.txt") && t.contains("sub"), "{t}");
    let r = c.tool("read_text", json!({"path": fx.root.join("hello.txt")}));
    assert!(text_of(&r).contains("hello from a file"));
    // Outside the scope.
    let outside = fx.dir.path().join("config.toml");
    std::fs::write(&outside, "").unwrap();
    let r = c.tool("stat", json!({"path": outside}));
    assert!(is_error(&r) && text_of(&r).contains("outside"), "{}", text_of(&r));
    // Bad arguments are tool errors; unknown tools are protocol errors.
    assert!(is_error(&c.tool("stat", json!({"nope": 1}))));
    assert_eq!(c.tool("no_such_tool", json!({}))["error"]["code"], -32602);
}

#[test]
fn write_tools_in_headless_mode() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server.clone(), Some("tester"));
    let target = fx.root.join("new-folder");
    // Dry run: a plan, nothing happens.
    let r = c.tool("mkdir", json!({"path": target, "dry_run": true}));
    assert!(!is_error(&r));
    assert!(
        text_of(&r).contains("\\\"dry_run\\\":true") || text_of(&r).contains("\"dry_run\":true"),
        "{}",
        text_of(&r)
    );
    // Without dry run: approval required, nobody can approve headless.
    let r = c.tool("mkdir", json!({"path": target}));
    assert!(is_error(&r));
    assert!(text_of(&r).contains("approval_unavailable"), "{}", text_of(&r));
    assert!(!target.exists());

    // A client configured without approval runs the plan as a job.
    let mut c = Conn::new(server, Some("trusted"));
    let r = c.tool("mkdir", json!({"path": target}));
    assert!(!is_error(&r), "{}", text_of(&r));
    assert!(target.is_dir());
    assert!(text_of(&r).contains("\"state\":\"done\""), "{}", text_of(&r));
    // But never above its risk ceiling.
    let r = c.tool("trash", json!({"paths": [target]}));
    assert!(is_error(&r));
    assert!(target.is_dir());

    let audit = audit_lines(&fx.audit);
    let calls: Vec<_> = audit.iter().filter(|r| r.event == "tool_call").collect();
    assert!(calls.iter().any(|r| r.tool == "mkdir" && r.outcome == "plan"));
    assert!(calls.iter().any(|r| r.tool == "mkdir" && r.outcome == "refused"));
    let done = calls.iter().find(|r| r.tool == "mkdir" && r.outcome == "ok").unwrap();
    assert_eq!(done.client, "trusted");
    assert!(done.job_id.is_some());
    assert!(done.args.contains("new-folder"));
}

#[test]
fn resources_and_prompts() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("tester"));
    let list = c.call("resources/list", json!({}));
    let uris: Vec<&str> =
        list["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"soyuz://history"));
    let root_uri = soyuz_mcp::protocol::path_to_file_uri(&fx.root);
    assert!(uris.contains(&root_uri.as_str()), "{uris:?}");
    let r = c.call("resources/read", json!({"uri": root_uri}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("hello.txt"));
    let file_uri = soyuz_mcp::protocol::path_to_file_uri(&fx.root.join("hello.txt"));
    let r = c.call("resources/read", json!({"uri": file_uri}));
    assert_eq!(r["result"]["contents"][0]["mimeType"], "text/plain");
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("line two"));
    let r = c.call("resources/read", json!({"uri": "soyuz://history"}));
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("entries"));
    // Headless: no window, no current location.
    let r = c.call("resources/read", json!({"uri": "soyuz://location/current"}));
    assert_eq!(r["error"]["code"], -32002, "legacy resource-not-found code");
    let r = c.call("resources/read", json!({"uri": "file:///C:/Windows/win.ini", "_meta": modern_meta()}));
    assert_eq!(r["error"]["code"], -32602);

    let p = c.call("prompts/list", json!({}));
    let names: Vec<&str> =
        p["result"]["prompts"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["organize_folder", "find_large_old_files", "clean_duplicates", "rename_photos_by_date"]);
    let g = c.call("prompts/get", json!({"name": "clean_duplicates", "arguments": {"folder": fx.root}}));
    assert!(g["result"]["messages"][0]["content"]["text"].as_str().unwrap().contains("find_duplicates"));
    assert_eq!(c.call("prompts/get", json!({"name": "clean_duplicates"}))["error"]["code"], -32602);
    assert!(audit_lines(&fx.audit).iter().any(|r| r.event == "resource_read"));
}

/// A host whose `list_dir` blocks until cancelled.
struct BlockingHost {
    scope: Scope,
    saw_cancel: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
}

impl ToolHost for BlockingHost {
    fn scope(&self) -> &Scope {
        &self.scope
    }
    fn read(&self, _input: &ToolInput, ctx: &CallContext) -> Result<Value, ToolError> {
        self.started.store(true, Ordering::SeqCst);
        for _ in 0..500 {
            if ctx.ctl.is_cancelled() {
                self.saw_cancel.store(true, Ordering::SeqCst);
                return Err(ToolError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(json!({"finished": true}))
    }
    fn plan(&self, input: &ToolInput, _ctx: &CallContext) -> Result<Plan, ToolError> {
        Err(ToolError::Unsupported(input.name().into()))
    }
}

struct TestBackend {
    host: Arc<BlockingHost>,
    ai: Ai,
}

impl Backend for TestBackend {
    fn mode(&self) -> Mode {
        Mode::Gui
    }
    fn host(&self) -> Arc<dyn ToolHost> {
        self.host.clone()
    }
    fn ai(&self) -> Ai {
        self.ai.clone()
    }
    fn request_approval(&self, _req: ApprovalRequest, _ctl: &OpControl) -> ApprovalDecision {
        ApprovalDecision::Rejected("test".into())
    }
    fn execute(&self, _plan: &Plan, _ctx: &CallContext, _p: &ProgressFn<'_>) -> Result<Value, ToolError> {
        Err(ToolError::Unsupported("test".into()))
    }
    fn register_pending(&self, _name: &str, _t: soyuz_mcp::Transport) {}
    fn audit(&self, _rec: soyuz_ai::audit::AuditRecord) {}
}

#[test]
fn cancellation_stops_the_call_and_suppresses_the_response() {
    let fx = Fixture::new();
    let ai = fx.ai();
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let backend = Arc::new(TestBackend {
        host: Arc::new(BlockingHost {
            scope: Scope::from_config(&ai.scope),
            saw_cancel: saw_cancel.clone(),
            started: started.clone(),
        }),
        ai,
    });
    let server = Server::new(backend);
    let mut c = Conn::new(server, Some("tester"));
    let h = c
        .send(json!({"jsonrpc": "2.0", "id": "slow-1", "method": "tools/call", "params": {"name": "list_dir", "arguments": {"path": fx.root}}}))
        .expect("tools/call runs on a worker");
    while !started.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(5));
    }
    // Requests on the same connection keep working meanwhile.
    assert_eq!(c.call("ping", json!({}))["result"], json!({}));
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "slow-1", "reason": "test"}}));
    h.join().unwrap();
    assert!(saw_cancel.load(Ordering::SeqCst), "the host saw the cancellation");
    assert!(c.rx.recv_timeout(Duration::from_millis(200)).is_err(), "no response for a cancelled request");
    // Unknown ids are ignored.
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 12345}}));
    // Approval rejected by the user → tool error.
    let r = c.tool("mkdir", json!({"path": fx.root.join("z")}));
    assert!(is_error(&r));
}

#[test]
fn stream_round_trip_with_the_mcp_client() {
    // Server on one end of two OS pipes, the MCP client (modern probe) on the other.
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let (c2s_r, c2s_w) = std::io::pipe().unwrap();
    let (s2c_r, s2c_w) = std::io::pipe().unwrap();
    let session = soyuz_mcp::Session::new(soyuz_mcp::Transport::Memory, Some("tester".into()));
    let t = std::thread::spawn(move || server.serve_lines(std::io::BufReader::new(c2s_r), Box::new(s2c_w), session));
    let transport = soyuz_mcp::client::StreamTransport::new(s2c_r, c2s_w);
    let client = soyuz_mcp::client::McpClient::connect(Box::new(transport), Duration::from_secs(20)).unwrap();
    assert!(client.is_modern());
    assert_eq!(client.server_name, "soyuz");
    let ctl = OpControl::new();
    let tools = client.list_tools(&ctl).unwrap();
    assert_eq!(tools.len(), tools::catalog().len());
    let r = client.call_tool("list_dir", json!({"path": fx.root}), &ctl).unwrap();
    assert!(!r.is_error && r.text.contains("hello.txt"));
    drop(client);
    t.join().unwrap();
}
