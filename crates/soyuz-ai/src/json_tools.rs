//! Constrained JSON tool protocol for models without native tool calling.
//!
//! The tools are described in the system prompt and the model must answer with exactly one
//! JSON object: `{"tool_calls": [{"name": "...", "arguments": {...}}]}` to call tools, or
//! `{"final": "..."}` to answer. The reply is buffered, parsed (fences and surrounding
//! prose are tolerated) and every call's arguments are validated against the tool schema;
//! the result is turned into ordinary [`Event::ToolCall`]s, so harnesses do not care which
//! protocol ran. Invalid calls are still emitted, with the validation errors in `meta`, so
//! the dispatcher can return them to the model as an error result.

use serde_json::{Value, json};

use crate::error::AiResult;
use crate::provider::Decoder;
use crate::schema::validate;
use crate::sse::SseEvent;
use crate::types::{ChatRequest, Event, Message, Part, StopReason, ToolSpec};

/// System-prompt section describing the protocol and the tools.
pub fn protocol_prompt(tools: &[ToolSpec]) -> String {
    let mut s = String::from(
        "## Tool protocol\n\
         You can call tools. Reply with exactly one JSON object and nothing else:\n\
         - to call tools: {\"tool_calls\": [{\"name\": \"<tool>\", \"arguments\": { ... }}]}\n\
         - to answer the user: {\"final\": \"<your answer>\"}\n\
         Arguments must match the tool's JSON Schema. Tool results come back in the next \
         user message, framed as untrusted data.\n\n### Tools\n",
    );
    for t in tools {
        s.push_str(&format!("- {}: {}\n  schema: {}\n", t.name, t.description, t.schema));
    }
    s
}

/// Rewrite a request for the JSON protocol: tools go into the system prompt and earlier
/// tool calls / results become plain text.
pub fn lower_request(mut req: ChatRequest) -> ChatRequest {
    let proto = protocol_prompt(&req.tools);
    req.system = if req.system.is_empty() { proto } else { format!("{}\n\n{proto}", req.system) };
    req.tools.clear();
    req.messages = req
        .messages
        .into_iter()
        .map(|m| {
            let mut text = String::new();
            let mut calls = Vec::new();
            for p in m.parts {
                match p {
                    Part::Text { text: t } => text.push_str(&t),
                    Part::ToolUse { name, args, .. } => calls.push(json!({"name": name, "arguments": args})),
                    Part::ToolResult { name, content, is_error, .. } => {
                        let status = if is_error { "error" } else { "ok" };
                        text.push_str(&format!("[tool result: {name} ({status})]\n{content}\n"));
                    }
                    Part::Opaque { .. } => {}
                }
            }
            if !calls.is_empty() {
                text = json!({"tool_calls": calls}).to_string();
            }
            Message { role: m.role, parts: vec![Part::Text { text }] }
        })
        .collect();
    req
}

/// One parsed call.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCall {
    pub name: String,
    pub args: Value,
    /// Schema violations (empty = valid).
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JsonReply {
    Calls(Vec<ParsedCall>),
    Final(String),
    /// Not protocol JSON: treated as a plain answer.
    Text(String),
}

/// Parse a model reply.
pub fn parse_reply(text: &str, tools: &[ToolSpec]) -> JsonReply {
    let Some(v) = extract_json(text) else {
        return JsonReply::Text(text.trim().to_string());
    };
    if let Some(f) = v.get("final") {
        return JsonReply::Final(f.as_str().map(str::to_string).unwrap_or_else(|| f.to_string()));
    }
    let raw_calls: Vec<Value> = match (v.get("tool_calls"), v.get("name").or_else(|| v.get("tool"))) {
        (Some(Value::Array(a)), _) => a.clone(),
        (Some(single @ Value::Object(_)), _) => vec![single.clone()],
        (None, Some(_)) => vec![v.clone()],
        _ => return JsonReply::Text(text.trim().to_string()),
    };
    let calls = raw_calls
        .into_iter()
        .map(|c| {
            let name = c.get("name").or_else(|| c.get("tool")).and_then(Value::as_str).unwrap_or_default().to_string();
            let mut args = c.get("arguments").or_else(|| c.get("args")).cloned().unwrap_or_else(|| json!({}));
            // Some models put the arguments object in a string.
            if let Value::String(s) = &args
                && let Ok(parsed) = serde_json::from_str::<Value>(s)
            {
                args = parsed;
            }
            let errors = match tools.iter().find(|t| t.name == name) {
                Some(t) => validate(&t.schema, &args),
                None => vec![format!("unknown tool “{name}”")],
            };
            ParsedCall { name, args, errors }
        })
        .collect();
    JsonReply::Calls(calls)
}

/// First JSON object in `text` (handles ``` fences and prose around it).
pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let bytes = t.as_bytes();
    let mut start = 0;
    while let Some(off) = t[start..].find('{') {
        let s = start + off;
        // Find the matching close brace (string-aware).
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for (i, &b) in bytes.iter().enumerate().skip(s) {
            if in_str {
                match b {
                    _ if esc => esc = false,
                    b'\\' => esc = true,
                    b'"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(&t[s..=i]) {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
        start = s + 1;
    }
    None
}

/// Wraps a backend decoder: buffers the text and converts the protocol reply into events.
pub struct JsonToolDecoder {
    inner: Box<dyn Decoder>,
    tools: Vec<ToolSpec>,
    text: String,
    done: bool,
}

impl JsonToolDecoder {
    pub fn new(inner: Box<dyn Decoder>, tools: Vec<ToolSpec>) -> Self {
        JsonToolDecoder { inner, tools, text: String::new(), done: false }
    }

    fn map(&mut self, evs: Vec<Event>) -> Vec<Event> {
        let mut out = Vec::new();
        for ev in evs {
            if self.done {
                break;
            }
            match ev {
                Event::TextDelta(t) => self.text.push_str(&t),
                Event::Done(stop) => {
                    self.done = true;
                    out.extend(self.resolve(stop));
                }
                Event::Error(e) => {
                    self.done = true;
                    out.push(Event::Error(e));
                }
                other => out.push(other),
            }
        }
        out
    }

    fn resolve(&mut self, stop: StopReason) -> Vec<Event> {
        match parse_reply(&self.text, &self.tools) {
            JsonReply::Calls(calls) if !calls.is_empty() => {
                let mut out: Vec<Event> = calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| Event::ToolCall {
                        id: format!("json_call_{i}"),
                        name: c.name,
                        args: c.args,
                        meta: (!c.errors.is_empty()).then(|| json!({"schema_errors": c.errors})),
                    })
                    .collect();
                out.push(Event::Done(StopReason::ToolUse));
                out
            }
            JsonReply::Final(t) | JsonReply::Text(t) => vec![Event::TextDelta(t), Event::Done(stop)],
            JsonReply::Calls(_) => vec![Event::Done(stop)],
        }
    }
}

impl Decoder for JsonToolDecoder {
    fn event(&mut self, ev: &SseEvent) -> AiResult<Vec<Event>> {
        let evs = self.inner.event(ev)?;
        Ok(self.map(evs))
    }

    fn finish(&mut self) -> Vec<Event> {
        let evs = self.inner.finish();
        self.map(evs)
    }
}
