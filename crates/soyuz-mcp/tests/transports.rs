//! Streamable HTTP (token / Host / Origin / header validation, legacy sessions, the
//! HTTP client) and the named-pipe attach round trip (Windows).

mod common;

#[cfg(windows)]
use std::io::{BufRead, BufReader};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use soyuz_ai::Secret;
use soyuz_mcp::http::{HttpServer, HttpSettings};

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, n: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.as_str())
    }
}

fn raw(addr: SocketAddr, method: &str, headers: &[(&str, &str)], body: &str) -> Resp {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut req = format!("{method} /mcp HTTP/1.1\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut text = String::new();
    s.read_to_string(&mut text).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().into(), v.trim().into())).collect();
    Resp { status, headers, body: body.to_string() }
}

fn start(fx: &Fixture) -> (HttpServer, String) {
    let (server, _) = fx.headless(fx.ai());
    let settings = HttpSettings::default();
    settings.tokens.write().push(("tester".into(), Secret::new("tok-tester-123")));
    let http = HttpServer::start(server, 0, settings).unwrap();
    let host = format!("127.0.0.1:{}", http.addr().port());
    (http, host)
}

#[test]
fn http_rejects_bad_tokens_hosts_and_origins() {
    let fx = Fixture::new();
    let (http, host) = start(&fx);
    let addr = http.addr();
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    let ct = ("Content-Type", "application/json");
    let r = raw(addr, "POST", &[("Host", &host), ct], &body);
    assert_eq!(r.status, 401);
    assert!(r.header("www-authenticate").unwrap().starts_with("Bearer"));
    let r = raw(addr, "POST", &[("Host", &host), ct, ("Authorization", "Bearer wrong")], &body);
    assert_eq!(r.status, 401);
    let auth = ("Authorization", "Bearer tok-tester-123");
    let r = raw(addr, "POST", &[("Host", &host), ct, auth, ("Origin", "https://evil.example")], &body);
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["message"], "Origin not allowed");
    let r = raw(addr, "POST", &[("Host", "evil.example:80"), ct, auth], &body);
    assert_eq!(r.status, 403, "DNS rebinding: foreign Host");
    let r = raw(addr, "GET", &[("Host", &host), auth], "");
    assert_eq!(r.status, 405);
    let origin = format!("http://{host}");
    let r = raw(addr, "POST", &[("Host", &host), ct, auth, ("Origin", &origin)], &body);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["result"], json!({}));
}

#[test]
fn http_legacy_session_and_modern_headers() {
    let fx = Fixture::new();
    let (http, host) = start(&fx);
    let addr = http.addr();
    let auth = ("Authorization", "Bearer tok-tester-123");
    let ct = ("Content-Type", "application/json");
    // Legacy: initialize mints a session.
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}});
    let r = raw(addr, "POST", &[("Host", &host), ct, auth], &init.to_string());
    assert_eq!(r.status, 200);
    let sid = r.header("mcp-session-id").unwrap().to_string();
    assert_eq!(r.json()["result"]["protocolVersion"], "2025-11-25");
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string();
    let r = raw(
        addr,
        "POST",
        &[("Host", &host), ct, auth, ("Mcp-Session-Id", &sid), ("MCP-Protocol-Version", "2025-11-25")],
        &list,
    );
    assert_eq!(r.status, 200);
    assert!(r.json()["result"]["tools"].as_array().unwrap().len() > 30);
    let r = raw(addr, "POST", &[("Host", &host), ct, auth, ("Mcp-Session-Id", "nope")], &list);
    assert_eq!(r.status, 404);
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
    assert_eq!(raw(addr, "POST", &[("Host", &host), ct, auth, ("Mcp-Session-Id", &sid)], &note).status, 202);

    // Modern: headers must mirror the body.
    let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_dir", "arguments": {"path": fx.root}, "_meta": modern_meta()}}).to_string();
    let good = [
        ("Host", host.as_str()),
        ct,
        auth,
        ("MCP-Protocol-Version", MODERN),
        ("Mcp-Method", "tools/call"),
        ("Mcp-Name", "list_dir"),
    ];
    let r = raw(addr, "POST", &good, &call);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["result"]["resultType"], "complete");
    assert!(r.json()["result"]["content"][0]["text"].as_str().unwrap().contains("hello.txt"));
    let mut bad = good;
    bad[5] = ("Mcp-Name", "stat");
    let r = raw(addr, "POST", &bad, &call);
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], -32020);
    let r = raw(addr, "POST", &good[..5], &call);
    assert_eq!(r.status, 400, "missing Mcp-Name");
    let mut meta = modern_meta();
    meta["io.modelcontextprotocol/protocolVersion"] = json!("1900-01-01");
    let old = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {"_meta": meta}}).to_string();
    let r = raw(
        addr,
        "POST",
        &[("Host", &host), ct, auth, ("MCP-Protocol-Version", "1900-01-01"), ("Mcp-Method", "tools/list")],
        &old,
    );
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], -32022);
    let unknown = json!({"jsonrpc": "2.0", "id": 5, "method": "x/y", "params": {"_meta": modern_meta()}}).to_string();
    let r = raw(
        addr,
        "POST",
        &[("Host", &host), ct, auth, ("MCP-Protocol-Version", MODERN), ("Mcp-Method", "x/y")],
        &unknown,
    );
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["error"]["code"], -32601);
}

#[test]
fn http_client_round_trip() {
    let fx = Fixture::new();
    let (http, host) = start(&fx);
    let url = format!("http://{host}/mcp");
    let t = soyuz_mcp::client::HttpTransport::new(&url, Some(Secret::new("tok-tester-123")));
    let c = soyuz_mcp::client::McpClient::connect(Box::new(t), Duration::from_secs(20)).unwrap();
    assert!(c.is_modern());
    let ctl = soyuz_core::OpControl::new();
    assert!(c.list_tools(&ctl).unwrap().iter().any(|t| t.name == "list_dir"));
    let r = c.call_tool("read_text", json!({"path": fx.root.join("hello.txt")}), &ctl).unwrap();
    assert!(r.text.contains("hello from a file"), "{}", r.text);
    let t = soyuz_mcp::client::HttpTransport::new(&url, Some(Secret::new("bad")));
    assert!(soyuz_mcp::client::McpClient::connect(Box::new(t), Duration::from_secs(5)).is_err());
    drop(http);
}

#[test]
fn external_tools_registry_over_http() {
    use soyuz_ai::tools::{CallContext, Risk, ToolError, ToolResponse};
    use soyuz_mcp::client::{ExternalTools, McpRegistry, ServerStatus};
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    // The registry resolves `env:PATH` as the bearer token (no `set_var` needed).
    let path_token = std::env::var("PATH").unwrap();
    let settings = HttpSettings::default();
    settings.tokens.write().push(("tester".into(), Secret::new(path_token)));
    let http = HttpServer::start(server, 0, settings).unwrap();
    let mut cfg = soyuz_core::config::AiMcpServer {
        name: "self".into(),
        url: format!("http://127.0.0.1:{}/mcp", http.addr().port()),
        token: "env:SOYUZ_TEST_NO_SUCH_VAR".into(),
        trust_annotations: true,
        ..Default::default()
    };
    let reg = McpRegistry::new();
    reg.configure(std::slice::from_ref(&cfg));
    reg.connect_all();
    assert!(matches!(reg.status()[0].1, ServerStatus::Failed(_)), "no token → refused");
    cfg.token = "env:PATH".into();
    reg.configure(std::slice::from_ref(&cfg));
    reg.connect_all();
    assert!(matches!(reg.status()[0].1, ServerStatus::Connected { modern: true, .. }), "{:?}", reg.status());
    let ctx = CallContext::new("assistant", Risk::Write, soyuz_core::config::AiPrivacy::Contents);
    assert!(reg.owns("mcp__self__list_dir"));
    assert!(reg.specs(Risk::Read).iter().all(|s| s.name.starts_with("mcp__self__")));
    assert!(reg.specs(Risk::Read).len() < reg.specs(Risk::Write).len());
    // Read-only (trusted annotation) → runs at once, framed by the caller.
    match reg.dispatch("mcp__self__list_dir", json!({"path": fx.root}), &ctx).unwrap() {
        ToolResponse::Data(v) => assert!(v["text"].as_str().unwrap().contains("hello.txt"), "{v}"),
        other => panic!("{other:?}"),
    }
    // Write tool → approval plan; executing it calls the server.
    let plan = match reg.dispatch("mcp__self__mkdir", json!({"path": fx.root.join("n")}), &ctx).unwrap() {
        ToolResponse::NeedsApproval(p) => p,
        other => panic!("{other:?}"),
    };
    assert!(plan.summary.contains("mkdir"));
    let v = reg.execute(&plan, &ctx).unwrap();
    assert_eq!(v["is_error"], true, "the headless Soyuz server refuses unapproved writes: {v}");
    let ro = CallContext::new("assistant", Risk::Read, soyuz_core::config::AiPrivacy::Contents);
    assert!(matches!(
        reg.dispatch("mcp__self__mkdir", json!({"path": fx.root.join("n")}), &ro),
        Err(ToolError::RiskTooHigh { .. })
    ));
    assert!(matches!(reg.dispatch("mcp__self__stat", json!({"bogus": 1}), &ctx), Err(ToolError::InvalidArgs(_))));
    assert!(matches!(reg.dispatch("mcp__nope__x", json!({}), &ctx), Err(ToolError::UnknownTool(_))));
    drop(http);
}

#[cfg(windows)]
#[test]
fn named_pipe_attach_round_trip() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let name = format!(r"\\.\pipe\soyuz-mcp-test-{}-{}", std::process::id(), chrono::Utc::now().timestamp_micros());
    let pipe = soyuz_mcp::pipe::PipeServer::start(&name, server).unwrap();
    // A second server on the same name is refused (first-instance flag).
    let (server2, _) = fx.headless(fx.ai());
    assert!(soyuz_mcp::pipe::PipeServer::start(&name, server2).is_err());

    let stream = soyuz_mcp::pipe::connect(&name).unwrap().expect("pipe is served");
    let mut w = stream.clone();
    w.write_all(soyuz_mcp::pipe::hello(Some("tester")).as_bytes()).unwrap();
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}}});
    w.write_all(format!("{req}\n").as_bytes()).unwrap();
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "list_dir", "arguments": {"path": fx.root}}});
    w.write_all(format!("{call}\n").as_bytes()).unwrap();
    let mut r = BufReader::new(stream.clone());
    let mut got = std::collections::HashMap::new();
    while got.len() < 2 {
        let mut line = String::new();
        assert!(r.read_line(&mut line).unwrap() > 0, "pipe closed early");
        let v: Value = serde_json::from_str(&line).unwrap();
        got.insert(v["id"].as_u64().unwrap(), v);
    }
    assert_eq!(got[&1]["result"]["serverInfo"]["name"], "soyuz");
    assert!(got[&2]["result"]["content"][0]["text"].as_str().unwrap().contains("hello.txt"));
    let audit = audit_lines(&fx.audit);
    assert!(audit.iter().any(|a| a.via == "pipe" && a.client == "tester" && a.tool == "list_dir"));

    // The relay used by `soyuz --mcp`: stdin/stdout ↔ pipe.
    let (in_r, mut in_w) = std::io::pipe().unwrap();
    let (out_r, out_w) = std::io::pipe().unwrap();
    let stream2 = soyuz_mcp::pipe::connect(&name).unwrap().unwrap();
    let relay = std::thread::spawn(move || soyuz_mcp::pipe::relay(stream2, Some("tester"), in_r, out_w));
    in_w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n").unwrap();
    let mut line = String::new();
    BufReader::new(out_r).read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 7);
    drop(in_w);
    relay.join().unwrap().unwrap();
    drop(pipe);
}
