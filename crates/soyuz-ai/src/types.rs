//! Provider-neutral chat vocabulary: requests, messages, streamed events and usage.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AiError;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// Which wire protocol produced an opaque block (so it is only echoed back to the same kind).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Wire {
    Anthropic,
    OpenaiChat,
    OpenaiResponses,
    Gemini,
}

/// One piece of a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// A tool call the assistant made (echoed back in history).
    ToolUse {
        id: String,
        name: String,
        args: Value,
        /// Provider-specific data that must travel with the call (e.g. Gemini `thoughtSignature`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
    /// The result of a tool call, sent as user content.
    ToolResult {
        id: String,
        /// Tool name (Gemini's `functionResponse` needs it).
        name: String,
        content: String,
        is_error: bool,
    },
    /// A provider block that must be echoed back verbatim (Anthropic thinking blocks,
    /// OpenAI reasoning items). Other wires skip it.
    Opaque {
        wire: Wire,
        block: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Message { role: Role::User, parts: vec![Part::Text { text: text.into() }] }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Message { role: Role::Assistant, parts: vec![Part::Text { text: text.into() }] }
    }

    /// Concatenated text parts.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// A tool offered to the model (JSON Schema for its arguments).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

/// How tools are offered for this request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ToolProtocol {
    /// The provider's native tool / function calling.
    #[default]
    Native,
    /// The constrained JSON protocol in plain text ([`crate::json_tools`]).
    Json,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub tool_protocol: ToolProtocol,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        ChatRequest {
            model: model.into(),
            system: String::new(),
            messages,
            tools: Vec::new(),
            max_tokens: 4096,
            temperature: None,
            tool_protocol: ToolProtocol::Native,
        }
    }
}

/// Token usage reported by a provider (cumulative for the request).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
        self.cache_write_tokens += o.cache_write_tokens;
    }

    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// Safety refusal / content filter.
    Refusal,
    Other,
}

/// One streamed event. A stream always ends with exactly one `Done` or `Error`.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    TextDelta(String),
    /// Visible reasoning summary text (when the provider returns any); display only.
    ReasoningDelta(String),
    ToolCall {
        id: String,
        name: String,
        args: Value,
        meta: Option<Value>,
    },
    /// Echo this back in the assistant message ([`Part::Opaque`]).
    Opaque(Part),
    /// Cumulative usage so far (the last one wins).
    Usage(Usage),
    Done(StopReason),
    Error(AiError),
}

impl Event {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Event::Done(_) | Event::Error(_))
    }
}

/// Collects a stream into an assistant message (helper for harnesses and tests).
#[derive(Clone, Debug, Default)]
pub struct Collected {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<(String, String, Value, Option<Value>)>,
    pub opaque: Vec<Part>,
    pub usage: Usage,
    pub stop: Option<StopReason>,
    pub error: Option<AiError>,
}

impl Collected {
    pub fn push(&mut self, ev: Event) {
        match ev {
            Event::TextDelta(t) => self.text.push_str(&t),
            Event::ReasoningDelta(t) => self.reasoning.push_str(&t),
            Event::ToolCall { id, name, args, meta } => self.tool_calls.push((id, name, args, meta)),
            Event::Opaque(p) => self.opaque.push(p),
            Event::Usage(u) => self.usage = u,
            Event::Done(s) => self.stop = Some(s),
            Event::Error(e) => self.error = Some(e),
        }
    }

    /// The assistant message to append to history (opaque blocks first, as received).
    pub fn to_message(&self) -> Message {
        let mut parts = self.opaque.clone();
        if !self.text.is_empty() {
            parts.push(Part::Text { text: self.text.clone() });
        }
        for (id, name, args, meta) in &self.tool_calls {
            parts.push(Part::ToolUse { id: id.clone(), name: name.clone(), args: args.clone(), meta: meta.clone() });
        }
        Message { role: Role::Assistant, parts }
    }
}
