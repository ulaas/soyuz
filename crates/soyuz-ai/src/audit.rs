//! The AI audit log: `ai-audit.jsonl` in the config dir. Every tool call,
//! plan, approval and resulting job from the MCP server and the in-app assistant is
//! appended as one JSON object per line.
//!
//! Writers are several threads and possibly several processes (the GUI and a headless
//! `soyuz --mcp`), so [`append`] opens the file in append mode, takes an exclusive file
//! lock, writes the whole line with one `write_all` and releases the lock. The file is
//! never rewritten.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest argument summary kept in a record.
pub const MAX_ARGS_CHARS: usize = 400;

/// `ai-audit.jsonl` next to `config.toml`.
pub fn default_path() -> PathBuf {
    soyuz_core::config::config_dir().join("ai-audit.jsonl")
}

/// One audit line. Optional fields are omitted when empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// RFC 3339 UTC timestamp (filled by [`append`] when empty).
    #[serde(default)]
    pub ts: String,
    /// `mcp` (external client), `assistant` (in-app harness), …
    pub source: String,
    /// MCP client name, or `assistant` / `recipe` for the in-app harness.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client: String,
    /// Assistant task (`20261008-101500-3`) or MCP connection; empty when unknown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session: String,
    /// Transport or model (`stdio`, `pipe`, `http`, a model id …).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub via: String,
    /// What happened: `tool_call`, `resource_read`, `approval`, `prompt`, …
    pub event: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    /// Compact, truncated argument JSON.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    /// `ok`, `error`, `plan`, `approved`, `rejected`, `refused`, `cancelled`, …
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl AuditRecord {
    pub fn new(source: &str, event: &str, outcome: &str) -> Self {
        AuditRecord { source: source.into(), event: event.into(), outcome: outcome.into(), ..Default::default() }
    }
}

/// Compact JSON of `args`, cut to [`MAX_ARGS_CHARS`] characters.
pub fn summarize_args(args: &Value) -> String {
    let s = args.to_string();
    if s.chars().count() <= MAX_ARGS_CHARS {
        return s;
    }
    let mut out: String = s.chars().take(MAX_ARGS_CHARS).collect();
    out.push('…');
    out
}

/// Append one record to `path` (created with its folder when missing).
pub fn append(path: &Path, record: &AuditRecord) -> std::io::Result<()> {
    let mut rec = record.clone();
    if rec.ts.is_empty() {
        rec.ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    }
    let mut line = serde_json::to_string(&rec).map_err(std::io::Error::other)?;
    line.push('\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // `read` as well: Windows `LockFileEx` needs GENERIC_READ or GENERIC_WRITE, and an
    // append-only handle has neither.
    let mut f = std::fs::OpenOptions::new().read(true).create(true).append(true).open(path)?;
    f.lock()?;
    let res = f.write_all(line.as_bytes()).and_then(|_| f.flush());
    let _ = f.unlock();
    res
}

/// Append to the default log; failures are logged, never fatal.
pub fn record(rec: &AuditRecord) {
    if let Err(e) = append(&default_path(), rec) {
        log::warn!("ai-audit.jsonl: {e}");
    }
}

/// The newest `max` parsable records (oldest first; the in-app viewer).
pub fn read_tail(path: &Path, max: usize) -> std::io::Result<Vec<AuditRecord>> {
    let mut all = read_all(path)?;
    let cut = all.len().saturating_sub(max);
    Ok(all.split_off(cut))
}

/// Read every parsable record (for the in-app viewer and tests).
pub fn read_all(path: &Path) -> std::io::Result<Vec<AuditRecord>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_is_line_atomic_across_threads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("ai-audit.jsonl");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        let mut r = AuditRecord::new("mcp", "tool_call", "ok");
                        r.client = format!("client-{t}");
                        r.tool = "list_dir".into();
                        r.args = "x".repeat(i * 37);
                        r.job_id = Some(i as u64);
                        append(&path, &r).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 200);
        let all = read_all(&path).unwrap();
        assert_eq!(all.len(), 200, "every line parses");
        assert!(all.iter().all(|r| !r.ts.is_empty() && r.source == "mcp"));
    }

    #[test]
    fn args_are_truncated() {
        let v = serde_json::json!({"paths": ["a".repeat(1000)]});
        let s = summarize_args(&v);
        assert_eq!(s.chars().count(), MAX_ARGS_CHARS + 1);
        assert!(s.ends_with('…'));
        assert_eq!(summarize_args(&serde_json::json!({"a": 1})), r#"{"a":1}"#);
    }
}
