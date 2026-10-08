//! Provider encoding / decoding against recorded SSE fixtures (no network).

use serde_json::{Value, json};
use soyuz_ai::provider::{
    Backend, Endpoint, Method, anthropic, decode_recorded, gemini, openai_chat, openai_responses,
};
use soyuz_ai::types::{
    ChatRequest, Collected, Event, Message, Part, Role, StopReason, ToolProtocol, ToolSpec, Usage, Wire,
};
use soyuz_ai::{AiError, Secret};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn collect(evs: Vec<Event>) -> Collected {
    let mut c = Collected::default();
    for e in evs {
        c.push(e);
    }
    c
}

fn ep(key: Option<&str>) -> Endpoint {
    Endpoint {
        base_url: "https://example.test".into(),
        key: key.map(Secret::new),
        headers: vec![("x-extra".into(), "1".into())],
    }
}

fn sample_request() -> ChatRequest {
    let mut req = ChatRequest::new(
        "model-x",
        vec![
            Message::user("organize Downloads"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    Part::Opaque {
                        wire: Wire::Anthropic,
                        block: json!({"type": "thinking", "thinking": "", "signature": "sig"}),
                    },
                    Part::Text { text: "Listing.".into() },
                    Part::ToolUse {
                        id: "t1".into(),
                        name: "list_dir".into(),
                        args: json!({"path": "/d"}),
                        meta: Some(json!({"thoughtSignature": "SIG"})),
                    },
                ],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult {
                    id: "t1".into(),
                    name: "list_dir".into(),
                    content: "[]".into(),
                    is_error: false,
                }],
            },
        ],
    );
    req.system = "You are Soyuz.".into();
    req.max_tokens = 1000;
    req.tools = vec![ToolSpec {
        name: "list_dir".into(),
        description: "List".into(),
        schema: json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}),
    }];
    req
}

#[test]
fn anthropic_docs_tool_use_stream() {
    let c = collect(decode_recorded(&mut *anthropic::Anthropic.decoder(), &fixture("anthropic_tool_use.sse")));
    assert_eq!(c.error, None);
    assert_eq!(c.text, "Okay, let's check the weather for San Francisco, CA:");
    assert_eq!(c.tool_calls.len(), 1);
    let (id, name, args, _) = &c.tool_calls[0];
    assert_eq!(id, "toolu_01T1x1fJ34qAmk2tNTrN7Up6");
    assert_eq!(name, "get_weather");
    assert_eq!(args, &json!({"location": "San Francisco, CA"}));
    assert_eq!(c.usage, Usage { input_tokens: 472, output_tokens: 89, ..Default::default() });
    assert_eq!(c.stop, Some(StopReason::ToolUse));
}

#[test]
fn anthropic_thinking_blocks_are_echoed() {
    let c = collect(decode_recorded(&mut *anthropic::Anthropic.decoder(), &fixture("anthropic_thinking.sse")));
    assert_eq!(c.reasoning, "Listing first.");
    assert_eq!(c.tool_calls[0].2, json!({}), "empty input_json means {{}}");
    assert_eq!(c.usage.cache_read_tokens, 800);
    let msg = c.to_message();
    match &msg.parts[0] {
        Part::Opaque { wire: Wire::Anthropic, block } => {
            assert_eq!(block, &json!({"type": "thinking", "thinking": "Listing first.", "signature": "EqQBCgIYAhIM"}))
        }
        p => panic!("expected the thinking block first, got {p:?}"),
    }
    // Re-encoding puts the block back verbatim before the tool_use.
    let mut req = ChatRequest::new("m", vec![Message::user("hi"), msg]);
    req.tool_protocol = ToolProtocol::Native;
    let body = anthropic::encode(&req);
    assert_eq!(body["messages"][1]["content"][0]["type"], "thinking");
    assert_eq!(body["messages"][1]["content"][1]["type"], "tool_use");
}

#[test]
fn anthropic_stream_error_event() {
    let evs = decode_recorded(&mut *anthropic::Anthropic.decoder(), &fixture("anthropic_error.sse"));
    assert!(matches!(evs.last(), Some(Event::Error(AiError::Overloaded(529)))));
    assert!(evs.contains(&Event::TextDelta("Hel".into())));
}

#[test]
fn anthropic_truncated_stream_is_an_error() {
    let body = fixture("anthropic_tool_use.sse");
    let cut = &body[..body.len() / 2];
    let evs = decode_recorded(&mut *anthropic::Anthropic.decoder(), cut);
    assert!(matches!(evs.last(), Some(Event::Error(AiError::Decode(_)))));
}

#[test]
fn anthropic_request_encoding() {
    let plan = anthropic::Anthropic.chat(&ep(Some("sk-ant-test")), &sample_request()).unwrap();
    assert_eq!(plan.method, Method::Post);
    assert_eq!(plan.url, "https://example.test/v1/messages");
    let h: Vec<(&str, &str)> = plan.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert!(h.contains(&("x-api-key", "sk-ant-test")));
    assert!(h.contains(&("anthropic-version", "2023-06-01")));
    assert!(h.contains(&("x-extra", "1")));
    let b = plan.body.unwrap();
    assert_eq!(b["stream"], true);
    assert_eq!(b["system"], "You are Soyuz.");
    assert_eq!(b["max_tokens"], 1000);
    assert!(b.get("temperature").is_none(), "sampling params only when set");
    assert_eq!(b["tools"][0]["input_schema"]["required"][0], "path");
    let msgs = b["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(
        msgs[1]["content"][2],
        json!({"type": "tool_use", "id": "t1", "name": "list_dir", "input": {"path": "/d"}})
    );
    assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
    assert_eq!(msgs[2]["content"][0]["tool_use_id"], "t1");
}

#[test]
fn openai_chat_streamed_tool_calls() {
    let dec = openai_chat::OpenAiChat { max_completion_tokens: true };
    let c = collect(decode_recorded(&mut *dec.decoder(), &fixture("openai_chat_tools.sse")));
    assert_eq!(c.error, None);
    assert_eq!(c.tool_calls.len(), 2);
    assert_eq!(c.tool_calls[0].0, "call_abc");
    assert_eq!(c.tool_calls[0].2, json!({"path": "C:/Users"}));
    assert_eq!(c.tool_calls[1].1, "stat");
    assert_eq!(c.usage, Usage { input_tokens: 100, output_tokens: 30, cache_read_tokens: 20, cache_write_tokens: 0 });
    assert_eq!(c.stop, Some(StopReason::ToolUse));
}

#[test]
fn ollama_text_with_reasoning() {
    let dec = openai_chat::OpenAiChat { max_completion_tokens: false };
    let c = collect(decode_recorded(&mut *dec.decoder(), &fixture("ollama_text.sse")));
    assert_eq!(c.text, "Hello there");
    assert_eq!(c.reasoning, "Think.");
    assert_eq!(c.stop, Some(StopReason::EndTurn));
    assert_eq!(c.usage.output_tokens, 2);
}

#[test]
fn openai_chat_request_encoding() {
    let req = sample_request();
    let cloud = openai_chat::encode(&req, true);
    assert_eq!(cloud["max_completion_tokens"], 1000);
    let local = openai_chat::OpenAiChat { max_completion_tokens: false }.chat(&ep(None), &req).unwrap();
    assert_eq!(local.url, "https://example.test/chat/completions");
    assert!(!local.headers.iter().any(|(k, _)| k == "authorization"), "no key, no auth header");
    let b = local.body.unwrap();
    assert_eq!(b["max_tokens"], 1000);
    assert_eq!(b["stream_options"]["include_usage"], true);
    assert_eq!(b["messages"][0], json!({"role": "system", "content": "You are Soyuz."}));
    assert_eq!(b["messages"][2]["tool_calls"][0]["function"]["arguments"], "{\"path\":\"/d\"}");
    assert_eq!(b["messages"][3], json!({"role": "tool", "tool_call_id": "t1", "content": "[]"}));
    assert_eq!(b["tools"][0]["type"], "function");
    let authed = openai_chat::OpenAiChat { max_completion_tokens: true }.chat(&ep(Some("sk-1")), &req).unwrap();
    assert!(authed.headers.iter().any(|(k, v)| k == "authorization" && v == "Bearer sk-1"));
}

#[test]
fn openai_responses_stream_and_encoding() {
    let c = collect(decode_recorded(
        &mut *openai_responses::OpenAiResponses.decoder(),
        &fixture("openai_responses_tools.sse"),
    ));
    assert_eq!(c.error, None);
    assert_eq!(c.text, "Checking.");
    assert_eq!(c.tool_calls[0].0, "call_1");
    assert_eq!(c.tool_calls[0].2, json!({"paths": ["/tmp"]}));
    assert_eq!(c.usage, Usage { input_tokens: 200, output_tokens: 25, cache_read_tokens: 100, cache_write_tokens: 0 });
    assert_eq!(c.stop, Some(StopReason::ToolUse));
    assert!(matches!(&c.opaque[0], Part::Opaque { wire: Wire::OpenaiResponses, .. }));

    let mut req = sample_request();
    req.messages.insert(2, c.to_message());
    let b = openai_responses::encode(&req);
    assert_eq!(b["instructions"], "You are Soyuz.");
    assert_eq!(b["store"], false);
    let input = b["input"].as_array().unwrap();
    assert!(input.iter().any(|i| i["type"] == "reasoning"), "reasoning items echoed");
    assert!(input.iter().any(|i| i["type"] == "function_call" && i["call_id"] == "t1"));
    assert!(input.iter().any(|i| i["type"] == "function_call_output" && i["output"] == "[]"));
    assert!(!input.iter().any(|i| i["type"] == "thinking"), "anthropic blocks are not sent to OpenAI");
    assert_eq!(b["tools"][0]["name"], "list_dir");
}

#[test]
fn gemini_function_call_with_signature() {
    let c = collect(decode_recorded(&mut *gemini::Gemini.decoder(), &fixture("gemini_tools.sse")));
    assert_eq!(c.error, None);
    assert_eq!(c.text, "Let me look.");
    let (id, name, args, meta) = &c.tool_calls[0];
    assert!(id.starts_with("gemini_call_"));
    assert_eq!(name, "list_dir");
    assert_eq!(args, &json!({"path": "/home/u/Downloads"}));
    assert_eq!(meta, &Some(json!({"thoughtSignature": "CiQB0e2Kb"})));
    assert_eq!(c.usage, Usage { input_tokens: 50, output_tokens: 42, ..Default::default() });
    assert_eq!(c.stop, Some(StopReason::ToolUse));

    let plan = gemini::Gemini.chat(&ep(Some("AIza")), &sample_request()).unwrap();
    assert_eq!(plan.url, "https://example.test/models/model-x:streamGenerateContent?alt=sse");
    assert!(plan.headers.iter().any(|(k, v)| k == "x-goog-api-key" && v == "AIza"));
    let b = plan.body.unwrap();
    assert_eq!(b["systemInstruction"]["parts"][0]["text"], "You are Soyuz.");
    assert_eq!(b["contents"][1]["role"], "model");
    let call_part = &b["contents"][1]["parts"][1];
    assert_eq!(call_part["functionCall"]["name"], "list_dir");
    assert_eq!(call_part["thoughtSignature"], "SIG");
    assert_eq!(b["contents"][2]["parts"][0]["functionResponse"]["response"], json!({"content": "[]"}));
    assert_eq!(b["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"]["type"], "object");
    let mut bad = sample_request();
    bad.model = "../x?y".into();
    assert!(gemini::Gemini.chat(&ep(None), &bad).is_err());
}

#[test]
fn model_listing_parsers() {
    let v: Value = json!({"data": [{"id": "claude-opus-5-5"}, {"id": "claude-haiku-5-5"}]});
    assert_eq!(anthropic::Anthropic.parse_models(&v), ["claude-opus-5-5", "claude-haiku-5-5"]);
    let g: Value = json!({"models": [{"name": "models/gemini-3.8-flash"}]});
    assert_eq!(gemini::Gemini.parse_models(&g), ["gemini-3.8-flash"]);
}
