//! OpenAI-compatible Chat Completions (`POST {base}/chat/completions`, `stream: true`).
//! Used for OpenAI and the local servers that speak the same protocol: Ollama
//! (`http://localhost:11434/v1`, tools + `stream_options.include_usage` supported per
//! https://docs.ollama.com/api/openai-compatibility), LM Studio, llama.cpp server, vLLM,
//! LocalAI, and custom endpoints.
//!
//! Stream chunks: `choices[0].delta.content`, `delta.tool_calls[{index, id, type,
//! function{name, arguments}}]` (arguments arrive as string fragments), `finish_reason`,
//! a final chunk with `usage` and empty `choices` when `include_usage` is set, then
//! `data: [DONE]`. Some local servers also stream `delta.reasoning_content` / `reasoning`.

use serde_json::{Value, json};

use super::anthropic::ids;
use super::{Backend, Decoder, Endpoint, HttpPlan, Method, base_headers, bearer, parse_args};
use crate::error::{AiError, AiResult};
use crate::sse::SseEvent;
use crate::types::{ChatRequest, Event, Part, Role, StopReason, Usage, Wire};

pub struct OpenAiChat {
    /// OpenAI cloud uses `max_completion_tokens`; local servers expect `max_tokens`.
    pub max_completion_tokens: bool,
}

impl Backend for OpenAiChat {
    fn wire(&self) -> Wire {
        Wire::OpenaiChat
    }

    fn chat(&self, ep: &Endpoint, req: &ChatRequest) -> AiResult<HttpPlan> {
        let mut headers = base_headers(ep);
        bearer(ep, &mut headers);
        Ok(HttpPlan {
            method: Method::Post,
            url: format!("{}/chat/completions", ep.base_url),
            headers,
            body: Some(encode(req, self.max_completion_tokens)),
        })
    }

    fn decoder(&self) -> Box<dyn Decoder> {
        Box::new(OpenAiChatDecoder::default())
    }

    fn models(&self, ep: &Endpoint) -> HttpPlan {
        let mut headers = base_headers(ep);
        bearer(ep, &mut headers);
        HttpPlan { method: Method::Get, url: format!("{}/models", ep.base_url), headers, body: None }
    }

    fn parse_models(&self, v: &Value) -> Vec<String> {
        ids(v, "data", "id")
    }
}

pub fn encode(req: &ChatRequest, max_completion_tokens: bool) -> Value {
    let mut messages = Vec::new();
    if !req.system.is_empty() {
        messages.push(json!({"role": "system", "content": req.system}));
    }
    for m in &req.messages {
        match m.role {
            Role::User => {
                let mut text = String::new();
                for p in &m.parts {
                    match p {
                        Part::Text { text: t } => text.push_str(t),
                        Part::ToolResult { id, content, .. } => {
                            messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}))
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({"role": "user", "content": text}));
                }
            }
            Role::Assistant => {
                let text = m.text();
                let calls: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolUse { id, name, args, .. } => Some(json!({
                            "id": id, "type": "function",
                            "function": {"name": name, "arguments": args.to_string()}
                        })),
                        _ => None,
                    })
                    .collect();
                let mut msg =
                    json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) }});
                if !calls.is_empty() {
                    msg["tool_calls"] = Value::Array(calls);
                }
                messages.push(msg);
            }
        }
    }
    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    body[if max_completion_tokens { "max_completion_tokens" } else { "max_tokens" }] = json!(req.max_tokens);
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t.name, "description": t.description, "parameters": t.schema
                    }})
                })
                .collect(),
        );
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

#[derive(Default, Debug)]
struct PendingCall {
    index: u64,
    id: String,
    name: String,
    args: String,
}

#[derive(Default)]
pub struct OpenAiChatDecoder {
    calls: Vec<PendingCall>,
    usage: Option<Usage>,
    stop: Option<StopReason>,
    done: bool,
}

fn finish_reason(s: &str) -> StopReason {
    match s {
        "stop" => StopReason::EndTurn,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        "content_filter" => StopReason::Refusal,
        _ => StopReason::Other,
    }
}

impl OpenAiChatDecoder {
    fn flush_calls(&mut self, out: &mut Vec<Event>) -> AiResult<()> {
        for (n, c) in std::mem::take(&mut self.calls).into_iter().enumerate() {
            let id = if c.id.is_empty() { format!("call_{n}") } else { c.id };
            out.push(Event::ToolCall { id, name: c.name, args: parse_args(&c.args)?, meta: None });
        }
        Ok(())
    }

    fn end(&mut self) -> AiResult<Vec<Event>> {
        let mut out = Vec::new();
        let had_calls = !self.calls.is_empty();
        self.flush_calls(&mut out)?;
        if let Some(u) = self.usage {
            out.push(Event::Usage(u));
        }
        let stop = match self.stop {
            Some(s) => s,
            None if had_calls => StopReason::ToolUse,
            None => StopReason::EndTurn,
        };
        out.push(Event::Done(if had_calls && stop == StopReason::EndTurn { StopReason::ToolUse } else { stop }));
        self.done = true;
        Ok(out)
    }
}

impl Decoder for OpenAiChatDecoder {
    fn event(&mut self, ev: &SseEvent) -> AiResult<Vec<Event>> {
        if self.done {
            return Ok(Vec::new());
        }
        if ev.data.trim() == "[DONE]" {
            return self.end();
        }
        let v: Value = serde_json::from_str(&ev.data).map_err(|e| AiError::Decode(e.to_string()))?;
        if let Some(err) = v.get("error") {
            self.done = true;
            let message = err.get("message").and_then(Value::as_str).unwrap_or("error").to_string();
            let kind = err.get("type").and_then(Value::as_str).unwrap_or("error").to_string();
            return Ok(vec![Event::Error(AiError::Provider { kind, message })]);
        }
        let mut out = Vec::new();
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let n = |p: &str| u.pointer(p).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("/prompt_tokens_details/cached_tokens");
            self.usage = Some(Usage {
                input_tokens: n("/prompt_tokens").saturating_sub(cached),
                output_tokens: n("/completion_tokens"),
                cache_read_tokens: cached,
                cache_write_tokens: 0,
            });
        }
        let Some(choice) = v.pointer("/choices/0") else {
            return Ok(out);
        };
        if let Some(d) = choice.get("delta") {
            if let Some(t) = d.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                out.push(Event::TextDelta(t.to_string()));
            }
            for k in ["reasoning_content", "reasoning"] {
                if let Some(t) = d.get(k).and_then(Value::as_str).filter(|t| !t.is_empty()) {
                    out.push(Event::ReasoningDelta(t.to_string()));
                }
            }
            for tc in d.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                let id = tc.get("id").and_then(Value::as_str).unwrap_or_default();
                let index = match tc.get("index").and_then(Value::as_u64) {
                    Some(i) => i,
                    // No index (some local servers): a new id starts a new call.
                    None => match self.calls.last() {
                        Some(last) if id.is_empty() || last.id == id => last.index,
                        Some(last) => last.index + 1,
                        None => 0,
                    },
                };
                let pos = match self.calls.iter().position(|c| c.index == index) {
                    Some(p) => p,
                    None => {
                        self.calls.push(PendingCall { index, ..Default::default() });
                        self.calls.len() - 1
                    }
                };
                let call = &mut self.calls[pos];
                if !id.is_empty() {
                    call.id = id.to_string();
                }
                if let Some(n) = tc.pointer("/function/name").and_then(Value::as_str) {
                    call.name.push_str(n);
                }
                match tc.pointer("/function/arguments") {
                    Some(Value::String(a)) => call.args.push_str(a),
                    // A few servers send the arguments as an object.
                    Some(obj @ Value::Object(_)) => call.args = obj.to_string(),
                    _ => {}
                }
            }
        }
        if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop = Some(finish_reason(r));
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<Event> {
        if self.done {
            return Vec::new();
        }
        // Some servers close without `[DONE]`; accept it when a finish reason was seen.
        if self.stop.is_some() {
            return self.end().unwrap_or_else(|e| vec![Event::Error(e)]);
        }
        self.done = true;
        vec![Event::Error(AiError::Decode("stream ended without a finish reason".into()))]
    }
}
