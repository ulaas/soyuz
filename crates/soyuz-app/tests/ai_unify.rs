//! The unified AI seams: MCP write calls become the assistant's plan cards and run as
//! labelled jobs after approval (with edits, live highlights and per-client undo), the
//! assistant and MCP clients share one audit log, external MCP servers' tools join the
//! assistant's catalog under the same risk / approval gates, and "Suggest names" sends
//! contents only where allowed (pictures only to vision models).

mod ai_common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ai_common::{Harness, Policy, fixture, tree};
use crossbeam_channel::Receiver;
use serde_json::{Value, json};
use soyuz_app::ai::agent::{Attachments, Decision, Finish};
use soyuz_app::ai::fake::{FakeModel, FakeTurn};
use soyuz_app::ai::mcp::{self, McpApprovals};
use soyuz_app::ai::state::{CardOrigin, CardState};
use soyuz_app::ai::touch::{self, TouchKind};
use soyuz_app::ai::vision::AttachKind;
use soyuz_app::ai::{CancelFlag, audit};
use soyuz_app::services::mcp_bridge::{BridgeMsg, GuiBackend, UiCall};
use soyuz_core::OpControl;
use soyuz_core::config::{Ai, AiAccess, AiMcpClient, AiMcpServer, AiPrivacy, AiRisk};
use soyuz_jobs::{JobEngine, JobId};
use soyuz_mcp::{Server, Session, Sink, Transport};

/// A temp folder (canonical, so scope checks match) with a few files.
fn root_with(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("w");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    fixture(&root, files);
    let root = soyuz_fs::strip_verbatim(&std::fs::canonicalize(&root).unwrap()).to_path_buf();
    (dir, root)
}

fn ai_for(root: &Path) -> Ai {
    let mut ai = Ai { enabled: true, privacy: AiPrivacy::Contents, ..Default::default() };
    ai.scope.allowed_roots = vec![root.to_path_buf()];
    ai.mcp_clients.push(AiMcpClient {
        name: "tester".into(),
        max_risk: AiRisk::Write,
        require_approval: true,
        ..Default::default()
    });
    ai
}

/// The window side of the MCP bridge, driven by hand: a `GuiBackend` whose UI requests
/// arrive on `rx`; approvals become cards in `cards`.
struct Window {
    server: Arc<Server>,
    rx: Receiver<BridgeMsg>,
    engine: Arc<JobEngine>,
    cards: McpApprovals,
    audit: PathBuf,
}

impl Window {
    fn new(ai: &Ai, audit: PathBuf) -> Window {
        let engine = ai_common::engine();
        let (tx, rx) = crossbeam_channel::unbounded();
        let backend = GuiBackend::new(ai, engine.clone(), tx, Arc::new(|| {}), audit.clone());
        Window { server: Server::new(backend), rx, engine, cards: McpApprovals::default(), audit }
    }

    /// Start `tools/call` on a server thread (the client waits there).
    fn call(&self, tool: &str, args: Value) -> std::thread::JoinHandle<Option<Value>> {
        let server = self.server.clone();
        let params = json!({"name": tool, "arguments": args});
        std::thread::spawn(move || {
            let session = Session::new(Transport::Memory, Some("tester".into()));
            let sink: Sink = Arc::new(|_| {});
            server.handle_request(&session, &json!(1), "tools/call", &params, &sink, &OpControl::new())
        })
    }

    /// Next approval request as a card id.
    fn next_card(&mut self) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "no approval request arrived");
            if let Ok(BridgeMsg::Approval(p)) = self.rx.recv_timeout(Duration::from_millis(100)) {
                return self.cards.push(p);
            }
        }
    }

    /// Answer the bridge's "execute the approved plan" the way the window does.
    fn run_execute(&mut self) -> JobId {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "no execute request arrived");
            let Ok(BridgeMsg::Ui(req)) = self.rx.recv_timeout(Duration::from_millis(100)) else { continue };
            let UiCall::Execute(plan, caller, key) = req.call else { panic!("unexpected UI call") };
            let spec = soyuz_app::services::mcp_bridge::job_spec(plan.job().unwrap().clone(), &plan, &caller);
            let job = self.engine.submit(spec);
            self.cards.on_submitted(key, &plan, Some(job));
            let _ = req.reply.send(Ok(json!({"job_id": job.0})));
            return job;
        }
    }

    fn tick(&mut self) {
        let engine = self.engine.clone();
        self.cards.tick(|id| engine.snapshot(id).map(|s| s.state));
    }
}

fn result_text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[test]
fn mcp_write_call_becomes_a_plan_card_and_runs_as_a_job_after_approval() {
    let (dir, root) = root_with(&[("a.txt", "alpha"), ("b.txt", "beta")]);
    let mut w = Window::new(&ai_for(&root), dir.path().join("ai-audit.jsonl"));
    let call = w.call("move", json!({"sources": [root.join("a.txt")], "dest_dir": root.join("sub")}));
    let id = w.next_card();
    {
        let c = w.cards.card(id).unwrap();
        assert_eq!(c.card.tool, "move");
        assert_eq!(c.card.state, CardState::Pending);
        assert_eq!(c.card.origin, CardOrigin::Mcp { client: "tester".into(), via: Transport::Memory.label().into() });
        assert_eq!(c.card.plan.items.len(), 1);
        assert!(c.card.expires.is_some(), "the card shows the approval countdown");
    }
    let t = touch::get(&root.join("a.txt")).expect("planned files are highlighted");
    assert!(t.mcp && t.kind == TouchKind::Planned);
    // Nothing happened before the approval.
    assert!(root.join("a.txt").exists());
    w.cards.decide(id, Decision::Approve);
    let job = w.run_execute();
    let resp = call.join().unwrap().expect("a response");
    assert!(result_text(&resp).contains("\"status\":\"done\""), "{resp}");
    w.tick();
    let c = w.cards.card(id).unwrap();
    assert!(matches!(&c.card.state, CardState::Done { ok: true, .. }), "{:?}", c.card.state);
    assert_eq!(c.job, Some(job));
    assert!(touch::get(&root.join("sub").join("a.txt")).is_some_and(|t| t.kind == TouchKind::Done && t.mcp));
    assert_eq!(tree(&root), ["b.txt", "sub/", "sub/a.txt"]);
    // The job is labelled for the client (activity tag, history, undo).
    let hist = w.engine.history();
    let labels: Vec<_> = hist.iter().filter_map(|h| h.options.as_ref()?.label.clone()).collect();
    assert!(labels.iter().all(|l| mcp::label_client(l) == Some("tester")), "{labels:?}");
    assert_eq!(mcp::clients_with_history(&hist), vec![("tester".to_string(), 1)]);
    // "Undo everything this client did in this session".
    let (n, failed) = mcp::undo_client(&w.engine, "tester", &CancelFlag::new(), |_| {});
    assert_eq!((n, failed.len()), (1, 0), "{failed:?}");
    assert_eq!(tree(&root), ["a.txt", "b.txt", "sub/"]);
    // Audit: the call, the approval and the job.
    let recs = audit::read_tail(&w.audit, 100).unwrap();
    assert!(recs.iter().any(|r| r.source == "mcp" && r.event == "approval" && r.client == "tester"));
    assert!(recs.iter().any(|r| r.source == "mcp" && r.tool == "move" && r.outcome == "ok" && r.job_id.is_some()));
}

#[test]
fn mcp_edit_rejection_and_approve_kind() {
    let (dir, root) = root_with(&[("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")]);
    let mut w = Window::new(&ai_for(&root), dir.path().join("ai-audit.jsonl"));
    // Edit: the server plans the edited arguments again and asks with a new card.
    let call = w.call("mkdir", json!({"path": root.join("Wrong")}));
    let first = w.next_card();
    w.cards.decide(first, Decision::Edit(json!({"path": root.join("Right")})));
    assert_eq!(w.cards.card(first).unwrap().card.state, CardState::Edited);
    let second = w.next_card();
    assert!(w.cards.card(second).unwrap().card.plan.summary.contains("Right"));
    w.cards.decide(second, Decision::Approve);
    w.run_execute();
    let resp = call.join().unwrap().unwrap();
    assert!(result_text(&resp).contains("edited_by_user"), "{resp}");
    assert!(root.join("Right").is_dir() && !root.join("Wrong").exists());
    // Reject: the client hears why, nothing changes.
    let call = w.call("move", json!({"sources": [root.join("a.txt")], "dest_dir": root.join("sub")}));
    let id = w.next_card();
    w.cards.decide(id, Decision::Reject(String::new()));
    let resp = call.join().unwrap().unwrap();
    assert!(result_text(&resp).contains("rejected_by_user"), "{resp}");
    assert!(touch::get(&root.join("a.txt")).is_none(), "rejected plans lose their highlight");
    assert!(root.join("a.txt").exists());
    // "Approve all of this kind for this client": the next move runs without asking.
    let call = w.call("move", json!({"sources": [root.join("a.txt")], "dest_dir": root.join("sub")}));
    let id = w.next_card();
    w.cards.approve_kind(id);
    w.run_execute();
    call.join().unwrap().unwrap();
    let call = w.call("move", json!({"sources": [root.join("b.txt")], "dest_dir": root.join("sub")}));
    let id = w.next_card();
    assert_eq!(w.cards.card(id).unwrap().card.state, CardState::Approved { auto: true });
    w.run_execute();
    call.join().unwrap().unwrap();
    assert!(root.join("sub").join("b.txt").exists());
    // Irreversible tools are never approved in bulk.
    assert!(soyuz_app::ui::ai::cards::irreversible("delete_permanent"));
}

#[test]
fn audit_log_has_assistant_and_mcp_records() {
    let (dir, root) = root_with(&[("a.jpg", "x")]);
    let log = dir.path().join("ai-audit.jsonl");
    // An MCP client reads a folder.
    let w = Window::new(&ai_for(&root), log.clone());
    let resp = w.call("list_dir", json!({"path": root})).join().unwrap().unwrap();
    assert!(result_text(&resp).contains("a.jpg"), "{resp}");
    // The assistant works in the same folder.
    let mut h = Harness::new(&root);
    h.audit = Some(audit::Audit::new(log.clone(), "s-1", "assistant"));
    let model = Arc::new(FakeModel::new(vec![
        FakeTurn::text("Looking.").call("list_dir", json!({"path": root})),
        FakeTurn::text("Done."),
    ]));
    let (_, out) = h.chat(model, "what is here?", Policy::ApproveAll);
    assert_eq!(out.finish, Some(Finish::Done));
    let recs = audit::read_tail(&log, 100).unwrap();
    let labels: std::collections::BTreeSet<String> = recs.iter().map(audit::source_label).collect();
    assert!(labels.contains("Assistant") && labels.contains("MCP \u{00b7} tester"), "{labels:?}");
    assert!(recs.iter().any(|r| r.source == "assistant" && r.session == "s-1" && r.tool == "list_dir"));
    assert!(recs.iter().any(|r| r.source == "mcp" && r.via == Transport::Memory.label() && r.tool == "list_dir"));
}

/// A Soyuz MCP server (headless, on its own temp folder) connected in-process as an
/// external server of the assistant.
fn external_registry(root: &Path, dir: &Path) -> Arc<soyuz_mcp::client::McpRegistry> {
    let mut ai = ai_for(root);
    ai.mcp_clients.push(AiMcpClient {
        name: "soyuz-self".into(),
        max_risk: AiRisk::Write,
        require_approval: false,
        ..Default::default()
    });
    let backend = soyuz_mcp::headless::HeadlessBackend::new(ai, None, None, dir.join("ext-audit.jsonl"));
    let server = Server::new(backend);
    let (to_server_r, to_server_w) = std::io::pipe().unwrap();
    let (from_server_r, from_server_w) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let session = Session::new(Transport::Stdio, Some("soyuz-self".into()));
        server.serve_lines(std::io::BufReader::new(to_server_r), Box::new(from_server_w), session);
    });
    let t = soyuz_mcp::client::StreamTransport::new(from_server_r, to_server_w);
    let client = soyuz_mcp::client::McpClient::connect(Box::new(t), Duration::from_secs(20)).unwrap();
    let reg = soyuz_mcp::client::McpRegistry::new();
    let cfg = AiMcpServer {
        name: "self".into(),
        risk: AiRisk::Write,
        trust_annotations: true,
        require_approval: true,
        ..Default::default()
    };
    assert!(reg.add_connected(cfg, client).unwrap() > 5);
    reg
}

#[test]
fn external_mcp_tools_join_the_catalog_behind_the_same_gates() {
    let (dir, root) = root_with(&[("a.txt", "alpha")]);
    let reg = external_registry(&root, dir.path());
    let turns = || {
        vec![
            FakeTurn::text("Asking the other server.").call("mcp__self__list_dir", json!({"path": root})),
            FakeTurn::text("Making a folder there.").call("mcp__self__mkdir", json!({"path": root.join("New")})),
            FakeTurn::text("Done."),
        ]
    };
    // Rejected: nothing changes.
    let mut h = Harness::new(&root);
    h.external = Some(reg.clone());
    let model = Arc::new(FakeModel::new(turns()));
    let requests = model.requests();
    let (_, out) = h.chat(model, "use the other server", Policy::RejectAll);
    assert_eq!(out.finish, Some(Finish::Done), "{out:?}");
    assert!(requests.lock()[0].tools.iter().any(|t| t.name == "mcp__self__mkdir"), "registered in the catalog");
    assert_eq!(out.proposed.len(), 1, "the read tool ran at once; the write became a plan card");
    assert_eq!(out.proposed[0].tool, "mcp__self__mkdir");
    assert!(out.proposed[0].plan.summary.contains("external MCP server"));
    assert!(!root.join("New").exists());
    // The read result was framed as untrusted, so the write plan is tainted.
    assert!(out.proposed[0].tainted);
    // Approved: the external server runs it.
    let model = Arc::new(FakeModel::new(turns()));
    let (_, out) = h.chat(model, "use the other server", Policy::ApproveAll);
    assert_eq!(out.finish, Some(Finish::Done));
    assert!(out.executed.iter().all(|(_, ok, _)| *ok), "{:?}", out.executed);
    assert!(root.join("New").is_dir());
    // Above the risk ceiling: not offered and refused.
    let mut h = Harness::new(&root);
    h.external = Some(reg);
    h.limits.max_risk = soyuz_ai::tools::Risk::Read;
    let model = Arc::new(FakeModel::new(turns()));
    let requests = model.requests();
    let (_, out) = h.chat(model, "use the other server", Policy::ApproveAll);
    assert_eq!(out.finish, Some(Finish::Done));
    let tools: Vec<String> = requests.lock()[0].tools.iter().map(|t| t.name.clone()).collect();
    assert!(tools.contains(&"mcp__self__list_dir".to_string()) && !tools.contains(&"mcp__self__mkdir".to_string()));
    assert!(out.proposed.is_empty());
}

fn has_image(req: &soyuz_ai::ChatRequest) -> bool {
    req.messages[0].parts.iter().any(|p| matches!(p, soyuz_ai::Part::Image { .. }))
}

#[test]
fn suggest_names_uses_contents_and_pictures_only_where_allowed() {
    let (_dir, root) = root_with(&[("notes.txt", "Quarterly budget for the Lisbon office")]);
    image::RgbImage::from_pixel(900, 600, image::Rgb([20, 120, 220])).save(root.join("IMG_0001.png")).unwrap();
    let attachments = Attachments {
        items: vec![(root.join("notes.txt"), AttachKind::Text), (root.join("IMG_0001.png"), AttachKind::Image)],
        excerpt_chars: 200,
        image_px: 128,
    };
    let rename = || {
        vec![
            FakeTurn::text("Here are better names.").call(
                "batch_rename",
                json!({"pairs": [
                    {"path": root.join("notes.txt"), "new_name": "lisbon-budget.txt"},
                    {"path": root.join("IMG_0001.png"), "new_name": "blue-sky.png"},
                ]}),
            ),
            FakeTurn::text("Renamed."),
        ]
    };
    // Contents allowed + vision profile: excerpt and picture are sent; one rename plan.
    let mut h = Harness::new(&root);
    h.default_access = AiAccess::Contents;
    h.attachments = attachments.clone();
    let model = Arc::new(FakeModel::new(rename()).vision());
    let requests = model.requests();
    let (_, out) = h.chat(model, "suggest names", Policy::ApproveAll);
    assert_eq!(out.finish, Some(Finish::Done), "{out:?}");
    {
        let reqs = requests.lock();
        let first = format!("{:?}", reqs[0].messages[0].parts);
        assert!(first.contains("Quarterly budget"), "text excerpt attached");
        assert!(first.contains("trust=\\\"untrusted\\\""), "framed as untrusted data");
        assert!(has_image(&reqs[0]), "picture attached for a vision model");
        assert!(!has_image(&reqs[1]) || reqs[1].messages.len() > 1);
    }
    assert_eq!(out.proposed.len(), 1);
    assert_eq!(out.proposed[0].tool, "batch_rename");
    assert!(out.proposed[0].tainted, "plans after reading contents are never auto-approved");
    assert_eq!(tree(&root), ["blue-sky.png", "lisbon-budget.txt", "sub/"]);

    // No vision: text only.
    let (_dir2, root2) = root_with(&[("notes.txt", "Quarterly budget")]);
    image::RgbImage::from_pixel(64, 64, image::Rgb([1, 2, 3])).save(root2.join("p.png")).unwrap();
    let mut h = Harness::new(&root2);
    h.default_access = AiAccess::Contents;
    h.attachments = Attachments {
        items: vec![(root2.join("notes.txt"), AttachKind::Text), (root2.join("p.png"), AttachKind::Image)],
        excerpt_chars: 200,
        image_px: 128,
    };
    let model = Arc::new(FakeModel::new(vec![FakeTurn::text("ok")]));
    let requests = model.requests();
    h.chat(model, "suggest names", Policy::RejectAll);
    assert!(!has_image(&requests.lock()[0]));
    assert!(format!("{:?}", requests.lock()[0].messages[0].parts).contains("Quarterly budget"));

    // Metadata only: nothing from the files is sent.
    let mut h = Harness::new(&root2);
    h.default_access = AiAccess::Metadata;
    h.attachments = Attachments {
        items: vec![(root2.join("notes.txt"), AttachKind::Text), (root2.join("p.png"), AttachKind::Image)],
        excerpt_chars: 200,
        image_px: 128,
    };
    let model = Arc::new(FakeModel::new(vec![FakeTurn::text("ok")]).vision());
    let requests = model.requests();
    h.chat(model, "suggest names", Policy::RejectAll);
    let first = format!("{:?}", requests.lock()[0].messages[0].parts);
    assert!(!first.contains("Quarterly budget") && !has_image(&requests.lock()[0]));
}
