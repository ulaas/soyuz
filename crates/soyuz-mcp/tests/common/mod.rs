#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use serde_json::{Value, json};
use soyuz_core::config::{Ai, AiMcpClient, AiPrivacy, AiRisk};
use soyuz_mcp::headless::HeadlessBackend;
use soyuz_mcp::{Server, Session, Sink, Transport};

pub const MODERN: &str = "2026-07-28";

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
    pub audit: PathBuf,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("files");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("hello.txt"), "hello from a file\nline two\n").unwrap();
        std::fs::write(root.join("sub").join("b.bin"), [0u8, 1, 2]).unwrap();
        let root = std::fs::canonicalize(&root).map(|p| soyuz_fs::strip_verbatim(&p).to_path_buf()).unwrap();
        Fixture { config: dir.path().join("config.toml"), audit: dir.path().join("ai-audit.jsonl"), dir, root }
    }

    /// `[ai]` with the fixture as the only allowed folder, contents allowed and a client
    /// "tester" (approved, everything, approval required).
    pub fn ai(&self) -> Ai {
        let mut ai = Ai { privacy: AiPrivacy::Contents, ..Default::default() };
        ai.scope.allowed_roots = vec![self.root.clone()];
        ai.mcp_clients.push(AiMcpClient {
            name: "tester".into(),
            max_risk: AiRisk::Destructive,
            require_approval: true,
            ..Default::default()
        });
        ai.mcp_clients.push(AiMcpClient {
            name: "trusted".into(),
            max_risk: AiRisk::Write,
            require_approval: false,
            ..Default::default()
        });
        ai
    }

    pub fn headless(&self, ai: Ai) -> (Arc<Server>, Arc<HeadlessBackend>) {
        let backend = HeadlessBackend::new(ai, Some(self.config.clone()), None, self.audit.clone());
        (Server::new(backend.clone()), backend)
    }
}

/// An in-process client on top of `Server::dispatch_message`.
pub struct Conn {
    pub server: Arc<Server>,
    pub session: Arc<Session>,
    pub sink: Sink,
    pub rx: Receiver<Value>,
    next: u64,
}

impl Conn {
    pub fn new(server: Arc<Server>, client: Option<&str>) -> Conn {
        let (tx, rx) = unbounded();
        let sink: Sink = Arc::new(move |v| {
            let _ = tx.send(v);
        });
        Conn { server, session: Session::new(Transport::Memory, client.map(str::to_string)), sink, rx, next: 1 }
    }

    pub fn send(&mut self, msg: Value) -> Option<std::thread::JoinHandle<()>> {
        self.server.dispatch_message(&self.session, msg, &self.sink)
    }

    /// Send a request and wait for its response (notifications are skipped).
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        if let Some(h) = self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})) {
            h.join().unwrap();
        }
        loop {
            let v = self.rx.recv_timeout(Duration::from_secs(30)).expect("response");
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    pub fn tool(&mut self, name: &str, args: Value) -> Value {
        self.call("tools/call", json!({"name": name, "arguments": args}))
    }
}

pub fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientInfo": {"name": "test-client", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

pub fn text_of(resp: &Value) -> String {
    resp["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

pub fn is_error(resp: &Value) -> bool {
    resp["result"]["isError"].as_bool().unwrap_or(false)
}

pub fn audit_lines(path: &Path) -> Vec<soyuz_ai::audit::AuditRecord> {
    soyuz_ai::audit::read_all(path).unwrap()
}
