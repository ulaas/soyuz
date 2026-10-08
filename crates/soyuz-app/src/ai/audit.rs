//! Append-only audit log of the assistant (`ai-audit.jsonl` in the config dir, roadmap
//! A7): every prompt, tool call, plan, approval / rejection and resulting job id.
//!
//! This is a small local writer; the shared `soyuz_ai::audit` module (MCP work) will
//! replace it with the same line format. Writes happen on worker threads only.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Longest `detail` kept per line.
const MAX_DETAIL: usize = 4000;

/// One audit line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AuditEvent {
    /// RFC 3339 timestamp.
    pub ts: String,
    /// Assistant task (`1728383-3`) or MCP session.
    pub session: String,
    /// `assistant`, `recipe` or an MCP client name.
    pub client: String,
    pub model: String,
    /// `prompt` | `tool_call` | `tool_result` | `plan` | `approval` | `rejection` | `job` |
    /// `job_result` | `undo` | `error` | `finish`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<u64>,
}

/// `ai-audit.jsonl` next to `config.toml`.
pub fn default_path() -> PathBuf {
    soyuz_core::config::config_dir().join("ai-audit.jsonl")
}

/// A writer bound to one session.
#[derive(Clone)]
pub struct Audit {
    path: PathBuf,
    session: String,
    client: String,
    model: Arc<Mutex<String>>,
    lock: Arc<Mutex<()>>,
}

impl std::fmt::Debug for Audit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audit").field("path", &self.path).field("session", &self.session).finish()
    }
}

impl Audit {
    pub fn new(path: PathBuf, session: impl Into<String>, client: impl Into<String>) -> Self {
        Audit {
            path,
            session: session.into(),
            client: client.into(),
            model: Arc::new(Mutex::new(String::new())),
            lock: Arc::new(Mutex::new(())),
        }
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

    /// Append one line (best effort: failures are logged, never fatal).
    pub fn log(&self, kind: &str, tool: Option<&str>, detail: &str, job: Option<u64>) {
        let full = detail;
        let mut detail: String = full.chars().take(MAX_DETAIL).collect();
        if detail.len() < full.len() {
            detail.push('\u{2026}');
        }
        let ev = AuditEvent {
            ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
            session: self.session.clone(),
            client: self.client.clone(),
            model: self.model.lock().clone(),
            kind: kind.to_string(),
            tool: tool.map(str::to_string),
            detail,
            job,
        };
        let Ok(mut line) = serde_json::to_string(&ev) else { return };
        line.push('\n');
        let _g = self.lock.lock();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let r = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = r {
            log::warn!("ai audit: cannot write {}: {e}", self.path.display());
        }
    }
}

/// The newest `max` events (oldest first). Malformed lines are skipped.
pub fn read_tail(path: &Path, max: usize) -> Result<Vec<AuditEvent>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(max);
    Ok(lines[start..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ai-audit.jsonl");
        let a = Audit::new(p.clone(), "s1", "assistant");
        a.set_model("fake");
        a.log("prompt", None, "organize this", None);
        a.log("job", Some("move"), "Move 3 items", Some(7));
        std::fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"garbage\n").unwrap();
        let ev = read_tail(&p, 10).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[1].job, Some(7));
        assert_eq!(ev[0].model, "fake");
        assert_eq!(read_tail(&p, 1).unwrap().len(), 0, "tail of 1 is the garbage line");
        assert!(read_tail(&dir.path().join("none"), 5).unwrap().is_empty());
    }
}
