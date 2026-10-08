//! "Test connection" and the tool-calling capability probe.
//!
//! 1. List models (free; proves the URL and key work and whether the model exists).
//! 2. Send one tiny request offering a `probe_echo` tool and asking the model to call it.
//!    A native tool call means tool calling works; a 4xx that mentions tools, or a plain
//!    text answer, means the model needs the JSON tool protocol ([`crate::json_tools`]).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::Provider;
use crate::error::AiError;
use crate::runtime::Wake;
use crate::types::{ChatRequest, Message, ToolSpec, Usage};

pub const PROBE_TOOL: &str = "probe_echo";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeReport {
    /// The chat request succeeded.
    pub ok: bool,
    pub latency: Duration,
    pub models: Vec<String>,
    /// Model listing failed (some servers do not implement it); not fatal.
    pub models_error: Option<String>,
    /// `Some(false)` when the listing worked but does not contain the configured model.
    pub model_listed: Option<bool>,
    /// Native tool calling works (`None` = undetermined).
    pub native_tools: Option<bool>,
    pub usage: Usage,
    pub error: Option<String>,
    /// The error is an authentication failure.
    pub auth_failed: bool,
}

impl ProbeReport {
    /// One-line human summary for the settings page.
    pub fn summary(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        let tools = match self.native_tools {
            Some(true) => "native tool calling",
            Some(false) => "no tool calling (JSON fallback)",
            None => "tool support unknown",
        };
        format!("Connected in {} ms · {tools}", self.latency.as_millis())
    }
}

pub(crate) fn spawn(p: Arc<Provider>, wake: Wake) -> crossbeam_channel::Receiver<ProbeReport> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("soyuz-ai-probe".into()).spawn(move || {
        let _ = tx.send(run(&p));
        wake();
    });
    if let Err(e) = spawned {
        log::warn!("cannot spawn the probe thread: {e}");
    }
    rx
}

/// Blocking probe (call from a worker thread).
pub fn run(p: &Provider) -> ProbeReport {
    let mut r = ProbeReport::default();
    match p.list_models(crate::runtime::no_wake()).recv() {
        Ok(Ok(models)) => {
            r.model_listed = (!models.is_empty() && !p.model.is_empty())
                .then(|| models.iter().any(|m| m == &p.model || m.strip_prefix("models/") == Some(&p.model)));
            r.models = models;
        }
        Ok(Err(e)) => {
            r.auth_failed = matches!(e, AiError::Auth { .. });
            r.models_error = Some(e.to_string());
            if r.auth_failed {
                r.error = Some(e.to_string());
                return r;
            }
        }
        Err(_) => r.models_error = Some("model listing did not answer".into()),
    }
    if p.model.is_empty() {
        r.error = Some("no model configured".into());
        return r;
    }
    let mut req = ChatRequest::new(
        p.model.clone(),
        vec![Message::user(format!(
            "Connection test. Call the {PROBE_TOOL} tool with value \"soyuz\". Do not answer with text."
        ))],
    );
    req.max_tokens = 1024;
    req.tools = vec![ToolSpec {
        name: PROBE_TOOL.into(),
        description: "Echoes a value back. Used to test tool calling.".into(),
        schema: json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false
        }),
    }];
    let started = Instant::now();
    let c = p.stream(req, crate::runtime::no_wake()).collect();
    r.latency = started.elapsed();
    r.usage = c.usage;
    match c.error {
        None => {
            r.ok = true;
            r.native_tools = Some(c.tool_calls.iter().any(|(_, name, _, _)| name == PROBE_TOOL));
        }
        Some(AiError::Http { status, message }) if (400..500).contains(&status) && mentions_tools(&message) => {
            // The server rejected the tools field: reachable, but no native tool calling.
            r.ok = true;
            r.native_tools = Some(false);
        }
        Some(e) => {
            r.auth_failed = matches!(e, AiError::Auth { .. });
            r.error = Some(e.to_string());
        }
    }
    r
}

fn mentions_tools(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("tool") || m.contains("function")
}
