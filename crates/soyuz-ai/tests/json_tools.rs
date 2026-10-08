//! JSON tool fallback protocol: parsing, schema validation and the stream adapter.

use serde_json::json;
use soyuz_ai::json_tools::{JsonReply, JsonToolDecoder, extract_json, lower_request, parse_reply};
use soyuz_ai::provider::{Backend, decode_recorded, openai_chat};
use soyuz_ai::types::{ChatRequest, Collected, Message, Part, Role, StopReason, ToolSpec};

fn tools() -> Vec<ToolSpec> {
    soyuz_ai::tools::specs(soyuz_ai::tools::Risk::Destructive, soyuz_core::config::AiPrivacy::MetadataOnly)
}

#[test]
fn parses_calls_final_and_prose() {
    let t = tools();
    let reply =
        "Sure!\n```json\n{\"tool_calls\": [{\"name\": \"list_dir\", \"arguments\": {\"path\": \"C:/a\"}}]}\n```";
    match parse_reply(reply, &t) {
        JsonReply::Calls(c) => {
            assert_eq!(c.len(), 1);
            assert_eq!(c[0].name, "list_dir");
            assert!(c[0].errors.is_empty(), "{:?}", c[0].errors);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(parse_reply(r#"{"final": "Done."}"#, &t), JsonReply::Final("Done.".into()));
    assert_eq!(parse_reply("just text", &t), JsonReply::Text("just text".into()));
    // Single-call shorthand and stringified arguments.
    match parse_reply(r#"{"name": "stat", "arguments": "{\"path\": \"/x\"}"}"#, &t) {
        JsonReply::Calls(c) => assert_eq!(c[0].args, json!({"path": "/x"})),
        other => panic!("{other:?}"),
    }
}

#[test]
fn schema_violations_are_reported() {
    let t = tools();
    match parse_reply(
        r#"{"tool_calls": [{"name": "copy", "arguments": {"sources": "nope", "bogus": 1}}, {"name": "rm_rf", "arguments": {}}]}"#,
        &t,
    ) {
        JsonReply::Calls(c) => {
            let e = c[0].errors.join("\n");
            assert!(e.contains("dest_dir"), "{e}");
            assert!(e.contains("expected array"), "{e}");
            assert!(e.contains("unknown field “bogus”"), "{e}");
            assert_eq!(c[1].errors, ["unknown tool “rm_rf”"]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn extract_handles_braces_in_strings() {
    let v = extract_json(r#"noise {"final": "a } b { c"} trailing {"x":1}"#).unwrap();
    assert_eq!(v["final"], "a } b { c");
    assert!(extract_json("{ not json").is_none());
}

#[test]
fn request_lowering_moves_tools_into_the_prompt() {
    let mut req = ChatRequest::new(
        "local",
        vec![
            Message::user("hi"),
            Message {
                role: Role::Assistant,
                parts: vec![Part::ToolUse {
                    id: "1".into(),
                    name: "stat".into(),
                    args: json!({"path": "/x"}),
                    meta: None,
                }],
            },
            Message {
                role: Role::User,
                parts: vec![Part::ToolResult {
                    id: "1".into(),
                    name: "stat".into(),
                    content: "{}".into(),
                    is_error: false,
                }],
            },
        ],
    );
    req.system = "base".into();
    req.tools = tools();
    let low = lower_request(req);
    assert!(low.tools.is_empty());
    assert!(low.system.starts_with("base\n\n## Tool protocol"));
    assert!(low.system.contains("- list_dir:"));
    assert!(low.messages[1].text().contains("\"tool_calls\""));
    assert!(low.messages[2].text().starts_with("[tool result: stat (ok)]"));
}

#[test]
fn decoder_adapter_turns_text_into_tool_calls() {
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"{\\\"tool_calls\\\": [{\\\"name\\\": \\\"stat\\\", \"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\\\"arguments\\\": {\\\"path\\\": \\\"/x\\\"}}]}\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let inner = openai_chat::OpenAiChat { max_completion_tokens: false }.decoder();
    let mut dec = JsonToolDecoder::new(inner, tools());
    let mut c = Collected::default();
    for e in decode_recorded(&mut dec, body.as_bytes()) {
        c.push(e);
    }
    assert_eq!(c.error, None);
    assert_eq!(c.text, "", "protocol JSON is not shown as text");
    assert_eq!(c.tool_calls.len(), 1);
    assert_eq!(c.tool_calls[0].0, "json_call_0");
    assert_eq!(c.tool_calls[0].2, json!({"path": "/x"}));
    assert_eq!(c.tool_calls[0].3, None, "valid call has no schema errors");
    assert_eq!(c.stop, Some(StopReason::ToolUse));
}
