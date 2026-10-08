//! OpenAI Responses API (`POST {base}/responses`, `stream: true`). OpenAI's newest models
//! (GPT-6 Astra, GPT-6.1 Sol) require it for tool calling
//! (https://developers.openai.com/api/docs/guides/function-calling, 2026-10).
//!
//! Function tools are `{type: "function", name, description, parameters}`; calls come back
//! as `function_call` output items (`call_id`, `name`, JSON-string `arguments`) and results
//! go in as `{type: "function_call_output", call_id, output}`. Stream events used:
//! `response.output_text.delta`, `response.reasoning_summary_text.delta`,
//! `response.output_item.done` (function_call / reasoning items), `response.completed` /
//! `response.incomplete` (with `usage.input_tokens` / `output_tokens`), `response.failed`
//! and `error`. Requests are stateless (`store: false`); reasoning items are requested with
//! encrypted content and echoed back as [`Part::Opaque`].

use serde_json::{Value, json};

use super::anthropic::ids;
use super::{Backend, Decoder, Endpoint, HttpPlan, Method, base_headers, bearer, parse_args};
use crate::error::{AiError, AiResult};
use crate::sse::SseEvent;
use crate::types::{ChatRequest, Event, Part, Role, StopReason, Usage, Wire};

pub struct OpenAiResponses;

impl Backend for OpenAiResponses {
    fn wire(&self) -> Wire {
        Wire::OpenaiResponses
    }

    fn chat(&self, ep: &Endpoint, req: &ChatRequest) -> AiResult<HttpPlan> {
        let mut headers = base_headers(ep);
        bearer(ep, &mut headers);
        Ok(HttpPlan {
            method: Method::Post,
            url: format!("{}/responses", ep.base_url),
            headers,
            body: Some(encode(req)),
        })
    }

    fn decoder(&self) -> Box<dyn Decoder> {
        Box::new(ResponsesDecoder::default())
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

pub fn encode(req: &ChatRequest) -> Value {
    let mut input = Vec::new();
    for m in &req.messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let mut text = String::new();
        let flush = |text: &mut String, input: &mut Vec<Value>| {
            if !text.is_empty() {
                input.push(json!({"role": role, "content": std::mem::take(text)}));
            }
        };
        for p in &m.parts {
            match p {
                Part::Text { text: t } => text.push_str(t),
                Part::ToolUse { id, name, args, .. } => {
                    flush(&mut text, &mut input);
                    input.push(
                        json!({"type": "function_call", "call_id": id, "name": name, "arguments": args.to_string()}),
                    );
                }
                Part::ToolResult { id, content, .. } => {
                    flush(&mut text, &mut input);
                    input.push(json!({"type": "function_call_output", "call_id": id, "output": content}));
                }
                Part::Opaque { wire: Wire::OpenaiResponses, block } => {
                    flush(&mut text, &mut input);
                    input.push(block.clone());
                }
                Part::Opaque { .. } => {}
                Part::Image { media_type, data } => {
                    let t = std::mem::take(&mut text);
                    input.push(json!({"role": role, "content": [
                        {"type": "input_text", "text": t},
                        {"type": "input_image", "image_url": format!("data:{media_type};base64,{data}")},
                    ]}));
                }
            }
        }
        flush(&mut text, &mut input);
    }
    let mut body = json!({
        "model": req.model,
        "input": input,
        "stream": true,
        "store": false,
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": req.max_tokens,
    });
    if !req.system.is_empty() {
        body["instructions"] = json!(req.system);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.schema}))
                .collect(),
        );
    }
    if let Some(t) = req.temperature {
        body["temperature"] = json!(t);
    }
    body
}

#[derive(Default)]
pub struct ResponsesDecoder {
    saw_call: bool,
    done: bool,
}

fn usage_of(resp: &Value) -> Option<Usage> {
    let u = resp.get("usage")?;
    let n = |p: &str| u.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    let cached = n("/input_tokens_details/cached_tokens");
    Some(Usage {
        input_tokens: n("/input_tokens").saturating_sub(cached),
        output_tokens: n("/output_tokens"),
        cache_read_tokens: cached,
        cache_write_tokens: 0,
    })
}

impl Decoder for ResponsesDecoder {
    fn event(&mut self, ev: &SseEvent) -> AiResult<Vec<Event>> {
        if self.done || ev.data.trim() == "[DONE]" {
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(&ev.data).map_err(|e| AiError::Decode(e.to_string()))?;
        let ty = v.get("type").and_then(Value::as_str).or(ev.event.as_deref()).unwrap_or("");
        let s = |p: &str| v.pointer(p).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut out = Vec::new();
        match ty {
            "response.output_text.delta" => out.push(Event::TextDelta(s("/delta"))),
            "response.reasoning_summary_text.delta" => out.push(Event::ReasoningDelta(s("/delta"))),
            "response.output_item.done" => {
                let item = v.get("item").cloned().unwrap_or(Value::Null);
                match item.get("type").and_then(Value::as_str) {
                    Some("function_call") => {
                        self.saw_call = true;
                        let g = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                        out.push(Event::ToolCall {
                            id: g("call_id"),
                            name: g("name"),
                            args: parse_args(&g("arguments"))?,
                            meta: None,
                        });
                    }
                    Some("reasoning") => {
                        out.push(Event::Opaque(Part::Opaque { wire: Wire::OpenaiResponses, block: item }))
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.incomplete" => {
                self.done = true;
                let resp = v.get("response").cloned().unwrap_or(Value::Null);
                if let Some(u) = usage_of(&resp) {
                    out.push(Event::Usage(u));
                }
                let stop = if ty == "response.incomplete" {
                    match resp.pointer("/incomplete_details/reason").and_then(Value::as_str) {
                        Some("content_filter") => StopReason::Refusal,
                        _ => StopReason::MaxTokens,
                    }
                } else if self.saw_call {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                };
                out.push(Event::Done(stop));
            }
            "response.failed" => {
                self.done = true;
                out.push(Event::Error(AiError::Provider {
                    kind: s("/response/error/code"),
                    message: s("/response/error/message"),
                }));
            }
            "error" => {
                self.done = true;
                let message =
                    v.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| s("/error/message"));
                out.push(Event::Error(AiError::Provider { kind: s("/code"), message }));
            }
            _ => {}
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<Event> {
        if self.done {
            return Vec::new();
        }
        self.done = true;
        vec![Event::Error(AiError::Decode("stream ended before response.completed".into()))]
    }
}
