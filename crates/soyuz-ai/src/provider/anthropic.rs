//! Anthropic Messages API (`POST /v1/messages`, `stream: true`).
//!
//! Verified against https://platform.claude.com/docs/en/build-with-claude/streaming
//! (2026-10): headers `x-api-key` + `anthropic-version: 2023-06-01`; events
//! `message_start` (usage.input_tokens), `content_block_start` (`text` / `tool_use` with
//! `id`, `name`, `input: {}` / `thinking` / `redacted_thinking`), `content_block_delta`
//! (`text_delta`, `input_json_delta.partial_json`, `thinking_delta`, `signature_delta`),
//! `content_block_stop`, `message_delta` (stop_reason, cumulative usage.output_tokens),
//! `message_stop`, `ping`, `error`. Thinking blocks are echoed back unchanged
//! ([`Part::Opaque`]). Sampling parameters are only sent when set (newer models reject them).

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Backend, Decoder, Endpoint, HttpPlan, Method, base_headers, parse_args};
use crate::error::{AiError, AiResult};
use crate::sse::SseEvent;
use crate::types::{ChatRequest, Event, Part, Role, StopReason, Usage, Wire};

pub const API_VERSION: &str = "2023-06-01";

pub struct Anthropic;

impl Backend for Anthropic {
    fn wire(&self) -> Wire {
        Wire::Anthropic
    }

    fn chat(&self, ep: &Endpoint, req: &ChatRequest) -> AiResult<HttpPlan> {
        let mut headers = base_headers(ep);
        headers.push(("anthropic-version".into(), API_VERSION.into()));
        if let Some(k) = &ep.key {
            headers.push(("x-api-key".into(), k.expose().to_string()));
        }
        Ok(HttpPlan {
            method: Method::Post,
            url: format!("{}/v1/messages", ep.base_url),
            headers,
            body: Some(encode(req)),
        })
    }

    fn decoder(&self) -> Box<dyn Decoder> {
        Box::new(AnthropicDecoder::default())
    }

    fn models(&self, ep: &Endpoint) -> HttpPlan {
        let mut headers = base_headers(ep);
        headers.push(("anthropic-version".into(), API_VERSION.into()));
        if let Some(k) = &ep.key {
            headers.push(("x-api-key".into(), k.expose().to_string()));
        }
        HttpPlan { method: Method::Get, url: format!("{}/v1/models?limit=100", ep.base_url), headers, body: None }
    }

    fn parse_models(&self, v: &Value) -> Vec<String> {
        ids(v, "data", "id")
    }
}

pub(crate) fn ids(v: &Value, list: &str, field: &str) -> Vec<String> {
    v.get(list)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|m| m.get(field).and_then(Value::as_str)).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Request body.
pub fn encode(req: &ChatRequest) -> Value {
    let mut messages = Vec::new();
    for m in &req.messages {
        let mut content = Vec::new();
        for p in &m.parts {
            match p {
                Part::Text { text } if !text.is_empty() => content.push(json!({"type": "text", "text": text})),
                Part::Text { .. } => {}
                Part::ToolUse { id, name, args, .. } => {
                    content.push(json!({"type": "tool_use", "id": id, "name": name, "input": args}))
                }
                Part::ToolResult { id, content: c, is_error, .. } => content.push(json!({
                    "type": "tool_result", "tool_use_id": id, "content": c, "is_error": is_error
                })),
                Part::Opaque { wire: Wire::Anthropic, block } => content.push(block.clone()),
                Part::Opaque { .. } => {}
                Part::Image { media_type, data } => content.push(json!({
                    "type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}
                })),
            }
        }
        if content.is_empty() {
            continue;
        }
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        messages.push(json!({"role": role, "content": content}));
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": messages,
        "stream": true,
        // Top-level automatic prompt caching (stable tools + system prefix).
        "cache_control": {"type": "ephemeral"},
    });
    if !req.system.is_empty() {
        body["system"] = json!(req.system);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema}))
                .collect(),
        );
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

enum Block {
    Text,
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    /// Any other block (redacted thinking, server tool blocks): echoed verbatim.
    Other(Value),
}

#[derive(Default)]
pub struct AnthropicDecoder {
    blocks: BTreeMap<u64, Block>,
    usage: Usage,
    stop: Option<StopReason>,
    done: bool,
}

fn stop_reason(s: &str) -> StopReason {
    match s {
        "end_turn" | "stop_sequence" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" | "model_context_window_exceeded" => StopReason::MaxTokens,
        "refusal" => StopReason::Refusal,
        _ => StopReason::Other,
    }
}

fn read_usage(u: &Value, usage: &mut Usage) {
    let n = |k: &str| u.get(k).and_then(Value::as_u64);
    if let Some(v) = n("input_tokens") {
        usage.input_tokens = v;
    }
    if let Some(v) = n("output_tokens") {
        usage.output_tokens = v;
    }
    if let Some(v) = n("cache_read_input_tokens") {
        usage.cache_read_tokens = v;
    }
    if let Some(v) = n("cache_creation_input_tokens") {
        usage.cache_write_tokens = v;
    }
}

impl Decoder for AnthropicDecoder {
    fn event(&mut self, ev: &SseEvent) -> AiResult<Vec<Event>> {
        if self.done {
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(&ev.data).map_err(|e| AiError::Decode(e.to_string()))?;
        let ty = v.get("type").and_then(Value::as_str).or(ev.event.as_deref()).unwrap_or("");
        let index = v.get("index").and_then(Value::as_u64).unwrap_or(0);
        let mut out = Vec::new();
        match ty {
            "message_start" => {
                if let Some(u) = v.pointer("/message/usage") {
                    read_usage(u, &mut self.usage);
                    out.push(Event::Usage(self.usage));
                }
            }
            "content_block_start" => {
                let cb = v.get("content_block").cloned().unwrap_or(Value::Null);
                let block = match cb.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = cb.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                            out.push(Event::TextDelta(t.to_string()));
                        }
                        Block::Text
                    }
                    Some("tool_use") => Block::ToolUse {
                        id: cb.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                        name: cb.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        json: String::new(),
                    },
                    Some("thinking") => Block::Thinking {
                        thinking: cb.get("thinking").and_then(Value::as_str).unwrap_or_default().to_string(),
                        signature: cb.get("signature").and_then(Value::as_str).unwrap_or_default().to_string(),
                    },
                    _ => Block::Other(cb),
                };
                self.blocks.insert(index, block);
            }
            "content_block_delta" => {
                let d = v.get("delta").cloned().unwrap_or(Value::Null);
                let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                match (d.get("type").and_then(Value::as_str), self.blocks.get_mut(&index)) {
                    (Some("text_delta"), _) => out.push(Event::TextDelta(s("text"))),
                    (Some("input_json_delta"), Some(Block::ToolUse { json, .. })) => json.push_str(&s("partial_json")),
                    (Some("thinking_delta"), Some(Block::Thinking { thinking, .. })) => {
                        let t = s("thinking");
                        thinking.push_str(&t);
                        out.push(Event::ReasoningDelta(t));
                    }
                    (Some("signature_delta"), Some(Block::Thinking { signature, .. })) => {
                        signature.push_str(&s("signature"))
                    }
                    _ => {}
                }
            }
            "content_block_stop" => match self.blocks.remove(&index) {
                Some(Block::ToolUse { id, name, json }) => {
                    out.push(Event::ToolCall { id, name, args: parse_args(&json)?, meta: None });
                }
                Some(Block::Thinking { thinking, signature }) => out.push(Event::Opaque(Part::Opaque {
                    wire: Wire::Anthropic,
                    block: json!({"type": "thinking", "thinking": thinking, "signature": signature}),
                })),
                Some(Block::Other(block)) if !block.is_null() => {
                    out.push(Event::Opaque(Part::Opaque { wire: Wire::Anthropic, block }))
                }
                _ => {}
            },
            "message_delta" => {
                if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop = Some(stop_reason(r));
                }
                if let Some(u) = v.get("usage") {
                    read_usage(u, &mut self.usage);
                    out.push(Event::Usage(self.usage));
                }
            }
            "message_stop" => {
                self.done = true;
                out.push(Event::Done(self.stop.unwrap_or(StopReason::EndTurn)));
            }
            "error" => {
                self.done = true;
                let kind = v.pointer("/error/type").and_then(Value::as_str).unwrap_or("error").to_string();
                let message = v.pointer("/error/message").and_then(Value::as_str).unwrap_or_default().to_string();
                out.push(Event::Error(match kind.as_str() {
                    "overloaded_error" => AiError::Overloaded(529),
                    "rate_limit_error" => AiError::RateLimited { retry_after: None },
                    _ => AiError::Provider { kind, message },
                }));
            }
            _ => {} // ping and future event types
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<Event> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        vec![Event::Error(AiError::Decode("stream ended before message_stop".into()))]
    }
}
