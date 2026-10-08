//! The assistant's side of the shared audit log (roadmap A7): a session-bound writer of
//! [`soyuz_ai::audit::AuditRecord`]s into the same `ai-audit.jsonl` the MCP server writes
//! (one file, one line format, appended under a file lock). Every prompt, tool call,
//! plan, approval / rejection and resulting job id of a task is recorded with
//! `source = "assistant"`, the task as `session` and the model as `via`.
//!
//! Writes happen on worker threads only.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
pub use soyuz_ai::audit::{AuditRecord, default_path, read_tail};

/// Longest `detail` kept per line.
const MAX_DETAIL: usize = 4000;

/// `source` of records written by the in-app harness.
pub const SOURCE_ASSISTANT: &str = "assistant";
/// `source` of records written by the MCP server.
pub const SOURCE_MCP: &str = "mcp";

/// What the viewer's source filter shows for a record: `Assistant` or `MCP · <client>`.
pub fn source_label(r: &AuditRecord) -> String {
    if r.source == SOURCE_MCP {
        if r.client.is_empty() { "MCP".into() } else { format!("MCP \u{00b7} {}", r.client) }
    } else if r.client == "recipe" {
        "Assistant (recipe)".into()
    } else {
        "Assistant".into()
    }
}

/// Outcome column for an assistant event kind.
fn outcome_of(kind: &str) -> &'static str {
    match kind {
        "tool_call" => "call",
        "plan" => "plan",
        "approval" => "approved",
        "rejection" => "rejected",
        "edit" => "edited",
        "job" => "submitted",
        "error" => "error",
        "taint" => "tainted",
        "undo" => "undo",
        _ => "ok",
    }
}

/// A writer bound to one assistant session (task).
#[derive(Clone)]
pub struct Audit {
    path: PathBuf,
    session: String,
    client: String,
    model: Arc<Mutex<String>>,
}

impl std::fmt::Debug for Audit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audit").field("path", &self.path).field("session", &self.session).finish()
    }
}

impl Audit {
    /// `client`: `assistant` or `recipe`.
    pub fn new(path: PathBuf, session: impl Into<String>, client: impl Into<String>) -> Self {
        Audit { path, session: session.into(), client: client.into(), model: Arc::new(Mutex::new(String::new())) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn set_model(&self, model: &str) {
        *self.model.lock() = model.to_string();
    }

    /// Append one record (best effort: failures are logged, never fatal).
    pub fn log(&self, kind: &str, tool: Option<&str>, detail: &str, job: Option<u64>) {
        let mut rec = AuditRecord::new(SOURCE_ASSISTANT, kind, outcome_of(kind));
        rec.client = self.client.clone();
        rec.session = self.session.clone();
        rec.via = self.model.lock().clone();
        rec.tool = tool.unwrap_or_default().to_string();
        rec.job_id = job;
        if kind == "tool_call"
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(detail)
        {
            rec.args = soyuz_ai::audit::summarize_args(&v);
        } else {
            let mut d: String = detail.chars().take(MAX_DETAIL).collect();
            if d.len() < detail.len() {
                d.push('\u{2026}');
            }
            rec.detail = d;
        }
        if let Err(e) = soyuz_ai::audit::append(&self.path, &rec) {
            log::warn!("ai audit: cannot write {}: {e}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn append_and_read_back_in_the_shared_format() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ai-audit.jsonl");
        let a = Audit::new(p.clone(), "s1", "assistant");
        a.set_model("fake");
        a.log("prompt", None, "organize this", None);
        a.log("tool_call", Some("list_dir"), r#"{"path":"C:\\x"}"#, None);
        a.log("job", Some("move"), "Move 3 items", Some(7));
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"garbage\n").unwrap();
        let ev = read_tail(&p, 10).unwrap();
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[2].job_id, Some(7));
        assert_eq!(ev[2].outcome, "submitted");
        assert_eq!(ev[1].args, r#"{"path":"C:\\x"}"#);
        assert!(ev.iter().all(|r| r.source == "assistant" && r.via == "fake" && r.session == "s1"));
        assert_eq!(source_label(&ev[0]), "Assistant");
        assert_eq!(read_tail(&p, 1).unwrap().len(), 1);
        assert!(read_tail(&dir.path().join("none"), 5).unwrap().is_empty());
    }
}
