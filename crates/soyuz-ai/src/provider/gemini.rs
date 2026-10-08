//! Google Gemini `generateContent` streaming
//! (`POST {base}/models/{model}:streamGenerateContent?alt=sse`, header `x-goog-api-key`).
//!
//! Verified against https://ai.google.dev/api/generate-content and
//! https://ai.google.dev/gemini-api/docs/function-calling (2026-10): `contents[]` with
//! roles `user` / `model` and `parts[]` (`text`, `functionCall{id?, name, args}`,
//! `functionResponse{id?, name, response}`), `systemInstruction`, `tools[{functionDeclarations}]`
//! (we send the full JSON Schema as `parametersJsonSchema`), `generationConfig.maxOutputTokens`.
//! Each SSE `data:` is a `GenerateContentResponse` (`candidates[0].content.parts`,
//! `finishReason`, `usageMetadata{promptTokenCount, candidatesTokenCount,
//! thoughtsTokenCount, cachedContentTokenCount}`). Thought signatures attached to
//! function-call parts are kept in the call's `meta` and sent back with it.

use serde_json::{Value, json};

use super::{Backend, Decoder, Endpoint, HttpPlan, Method, base_headers};
use crate::error::{AiError, AiResult};
use crate::sse::SseEvent;
use crate::types::{ChatRequest, Event, Part, Role, StopReason, Usage, Wire};

pub struct Gemini;

fn key_header(ep: &Endpoint, headers: &mut Vec<(String, String)>) {
    if let Some(k) = &ep.key {
        headers.push(("x-goog-api-key".into(), k.expose().to_string()));
    }
}

impl Backend for Gemini {
    fn wire(&self) -> Wire {
        Wire::Gemini
    }

    fn chat(&self, ep: &Endpoint, req: &ChatRequest) -> AiResult<HttpPlan> {
        let model = req.model.strip_prefix("models/").unwrap_or(&req.model);
        if model.is_empty() || model.contains(['/', '?', '#', ' ', '&']) {
            return Err(AiError::Config(format!("invalid Gemini model id “{}”", req.model)));
        }
        let mut headers = base_headers(ep);
        key_header(ep, &mut headers);
        Ok(HttpPlan {
            method: Method::Post,
            url: format!("{}/models/{model}:streamGenerateContent?alt=sse", ep.base_url),
            headers,
            body: Some(encode(req)),
        })
    }

    fn decoder(&self) -> Box<dyn Decoder> {
        Box::new(GeminiDecoder::default())
    }

    fn models(&self, ep: &Endpoint) -> HttpPlan {
        let mut headers = base_headers(ep);
        key_header(ep, &mut headers);
        HttpPlan { method: Method::Get, url: format!("{}/models?pageSize=200", ep.base_url), headers, body: None }
    }

    fn parse_models(&self, v: &Value) -> Vec<String> {
        super::anthropic::ids(v, "models", "name")
            .into_iter()
            .map(|n| n.strip_prefix("models/").map(str::to_string).unwrap_or(n))
            .collect()
    }
}

pub fn encode(req: &ChatRequest) -> Value {
    let mut contents = Vec::new();
    for m in &req.messages {
        let mut parts = Vec::new();
        for p in &m.parts {
            match p {
                Part::Text { text } if !text.is_empty() => parts.push(json!({"text": text})),
                Part::ToolUse { id, name, args, meta } => {
                    let mut call = json!({"name": name, "args": args});
                    if !id.starts_with("gemini_call_") {
                        call["id"] = json!(id);
                    }
                    let mut part = json!({"functionCall": call});
                    if let Some(sig) = meta.as_ref().and_then(|m| m.get("thoughtSignature")) {
                        part["thoughtSignature"] = sig.clone();
                    }
                    parts.push(part);
                }
                Part::ToolResult { id, name, content, is_error } => {
                    let key = if *is_error { "error" } else { "content" };
                    let mut resp = json!({"name": name, "response": {key: content}});
                    if !id.starts_with("gemini_call_") {
                        resp["id"] = json!(id);
                    }
                    parts.push(json!({"functionResponse": resp}));
                }
                Part::Opaque { wire: Wire::Gemini, block } => parts.push(block.clone()),
                _ => {}
            }
        }
        if parts.is_empty() {
            continue;
        }
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "model",
        };
        contents.push(json!({"role": role, "parts": parts}));
    }
    let mut gen_cfg = json!({"maxOutputTokens": req.max_tokens});
    if let Some(t) = req.temperature {
        gen_cfg["temperature"] = json!(t);
    }
    let mut body = json!({"contents": contents, "generationConfig": gen_cfg});
    if !req.system.is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": req.system}]});
    }
    if !req.tools.is_empty() {
        let decls: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "parametersJsonSchema": t.schema}))
            .collect();
        body["tools"] = json!([{"functionDeclarations": decls}]);
        body["toolConfig"] = json!({"functionCallingConfig": {"mode": "AUTO"}});
    }
    body
}

#[derive(Default)]
pub struct GeminiDecoder {
    calls: usize,
    usage: Option<Usage>,
    finish: Option<String>,
    blocked: bool,
    done: bool,
}

impl Decoder for GeminiDecoder {
    fn event(&mut self, ev: &SseEvent) -> AiResult<Vec<Event>> {
        if self.done {
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(&ev.data).map_err(|e| AiError::Decode(e.to_string()))?;
        if let Some(err) = v.get("error") {
            self.done = true;
            return Ok(vec![Event::Error(AiError::Provider {
                kind: err.get("status").and_then(Value::as_str).unwrap_or("error").to_string(),
                message: err.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            })]);
        }
        let mut out = Vec::new();
        if v.pointer("/promptFeedback/blockReason").is_some() {
            self.blocked = true;
        }
        if let Some(cand) = v.pointer("/candidates/0") {
            for part in cand.pointer("/content/parts").and_then(Value::as_array).into_iter().flatten() {
                if let Some(fc) = part.get("functionCall") {
                    let id = match fc.get("id").and_then(Value::as_str) {
                        Some(id) if !id.is_empty() => id.to_string(),
                        _ => format!("gemini_call_{}", self.calls),
                    };
                    self.calls += 1;
                    let meta = part.get("thoughtSignature").map(|s| json!({"thoughtSignature": s}));
                    out.push(Event::ToolCall {
                        id,
                        name: fc.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                        args: fc.get("args").cloned().unwrap_or_else(|| json!({})),
                        meta,
                    });
                } else if let Some(t) = part.get("text").and_then(Value::as_str) {
                    if part.get("thought").and_then(Value::as_bool) == Some(true) {
                        out.push(Event::ReasoningDelta(t.to_string()));
                    } else if !t.is_empty() {
                        out.push(Event::TextDelta(t.to_string()));
                    }
                }
            }
            if let Some(r) = cand.get("finishReason").and_then(Value::as_str) {
                self.finish = Some(r.to_string());
            }
        }
        if let Some(u) = v.get("usageMetadata") {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("cachedContentTokenCount");
            self.usage = Some(Usage {
                input_tokens: n("promptTokenCount").saturating_sub(cached),
                output_tokens: n("candidatesTokenCount") + n("thoughtsTokenCount"),
                cache_read_tokens: cached,
                cache_write_tokens: 0,
            });
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<Event> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        let mut out = Vec::new();
        if let Some(u) = self.usage {
            out.push(Event::Usage(u));
        }
        let stop = match self.finish.as_deref() {
            _ if self.blocked => StopReason::Refusal,
            Some("STOP") | None if self.calls > 0 => StopReason::ToolUse,
            Some("STOP") => StopReason::EndTurn,
            Some("MAX_TOKENS") => StopReason::MaxTokens,
            Some("SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII") => StopReason::Refusal,
            None => {
                out.push(Event::Error(AiError::Decode("stream ended without a finish reason".into())));
                return out;
            }
            Some(_) => StopReason::Other,
        };
        out.push(Event::Done(stop));
        out
    }
}
