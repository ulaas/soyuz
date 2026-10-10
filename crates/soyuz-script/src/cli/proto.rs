//! Wire protocol of the CLI pipe (newline-delimited JSON), and the window side.
//!
//! ```text
//! client → {"soyuz_cli":1,"pid":…}
//! client → {"op":"submit","spec":{JobSpec},"wait":true} | {"op":"open","path":…}
//!          | {"op":"reveal","path":…} | {"op":"jobs"}
//! window → {"event":"submitted","job":{…}} {"event":"progress","job":{…}}… {"event":"finished","job":{…}}
//!          | {"event":"ok"} | {"event":"jobs","jobs":[…],"history":{…}} | {"event":"error","message":…}
//! ```
//! Jobs are submitted on the UI thread (they get a CLI badge in the activity panel);
//! the connection thread then follows the job on the shared engine.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use soyuz_jobs::{JobEngine, JobId, JobSpec};

/// `\\.\pipe\soyuz-cli-<user>` (unix: `$XDG_RUNTIME_DIR/soyuz/soyuz-cli.sock`).
pub fn pipe_name() -> String {
    soyuz_mcp::pipe::user_pipe_name("soyuz-cli")
}

pub fn hello() -> String {
    format!("{}\n", json!({"soyuz_cli": 1, "pid": std::process::id()}))
}

fn is_hello(line: &str) -> bool {
    serde_json::from_str::<Value>(line.trim()).ok().and_then(|v| v.get("soyuz_cli").and_then(Value::as_u64)) == Some(1)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Submit { spec: Box<JobSpec>, wait: bool },
    Open { path: PathBuf },
    Reveal { path: PathBuf },
    Jobs,
}

/// What the window provides to CLI connections.
pub trait Host: Send + Sync {
    fn engine(&self) -> Arc<JobEngine>;
    /// Submit a job (from the UI thread, with the window's copy settings).
    fn submit(&self, spec: JobSpec) -> Result<JobId, String>;
    /// Open a folder / launch a file, or reveal an item.
    fn open(&self, path: &Path, reveal: bool) -> Result<(), String>;
}

fn send(w: &mut impl Write, v: &Value) -> bool {
    let mut s = v.to_string();
    s.push('\n');
    w.write_all(s.as_bytes()).and_then(|_| w.flush()).is_ok()
}

/// Serve one connection (window side).
pub fn serve_connection<S: Read + Write + Clone>(host: &dyn Host, stream: S) {
    let mut reader = BufReader::new(stream.clone());
    let mut out = stream;
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 || !is_hello(&line) {
        log::warn!("cli pipe: connection without a hello line; closing");
        return;
    }
    line.clear();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let req: Request = match serde_json::from_str(line.trim()) {
        Ok(r) => r,
        Err(e) => {
            send(&mut out, &json!({"event": "error", "message": format!("bad request: {e}")}));
            return;
        }
    };
    match req {
        Request::Submit { spec, wait } => match host.submit(*spec) {
            Ok(id) => {
                let engine = host.engine();
                let snap = engine.snapshot(id).map(|s| soyuz_mcp::jobs::snapshot_json(&s));
                if !send(&mut out, &json!({"event": "submitted", "job": snap.unwrap_or(json!({"job_id": id.0}))})) {
                    return;
                }
                if wait {
                    follow(&engine, id, &mut out, false);
                }
            }
            Err(e) => {
                send(&mut out, &json!({"event": "error", "message": e}));
            }
        },
        Request::Open { path } => reply_unit(&mut out, host.open(&path, false)),
        Request::Reveal { path } => reply_unit(&mut out, host.open(&path, true)),
        Request::Jobs => {
            let engine = host.engine();
            send(&mut out, &jobs_json(&engine));
        }
    }
}

fn reply_unit(out: &mut impl Write, r: Result<(), String>) {
    let v = match r {
        Ok(()) => json!({"event": "ok"}),
        Err(e) => json!({"event": "error", "message": e}),
    };
    send(out, &v);
}

/// `{"event":"jobs"}` reply: unfinished jobs and recent history.
pub fn jobs_json(engine: &JobEngine) -> Value {
    let jobs: Vec<Value> =
        engine.snapshots().iter().filter(|s| !s.state.is_terminal()).map(soyuz_mcp::jobs::snapshot_json).collect();
    json!({"event": "jobs", "jobs": jobs, "history": soyuz_mcp::jobs::history_json(&engine.history(), 10)})
}

/// Stream `progress` events for `id` until it finishes, then `finished`. With
/// `answer_prompts` (headless), prompts get the non-destructive default answers.
/// Returns the final snapshot JSON (`None` if the client went away / job vanished).
pub fn follow(engine: &JobEngine, id: JobId, out: &mut impl Write, answer_prompts: bool) -> Option<Value> {
    let mut last: Option<(u64, u64, &'static str)> = None;
    let mut last_sent = Instant::now() - Duration::from_secs(1);
    loop {
        if answer_prompts {
            while engine.events().try_recv().is_ok() {}
        }
        let snap = engine.snapshot(id)?;
        let v = soyuz_mcp::jobs::snapshot_json(&snap);
        if snap.state.is_terminal() {
            let alive = send(out, &json!({"event": "finished", "job": v.clone()}));
            return alive.then_some(v);
        }
        if answer_prompts && let Some(p) = &snap.pending_prompt {
            engine.respond(id, soyuz_mcp::jobs::headless_answer(p));
        }
        let key = (snap.progress.bytes_done, snap.progress.items_done, soyuz_mcp::jobs::state_name(&snap.state));
        if last != Some(key) && last_sent.elapsed() >= Duration::from_millis(200) {
            last = Some(key);
            last_sent = Instant::now();
            if !send(out, &json!({"event": "progress", "job": v})) {
                return None;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
