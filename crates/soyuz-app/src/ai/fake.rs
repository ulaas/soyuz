//! Scripted fake provider (roadmap A8): replays a recorded transcript of model turns —
//! streamed text, tool calls and usage — so harness tests are deterministic and the
//! assistant can be demoed without a model (`SOYUZ_AI_FAKE=<transcript.toml>`).
//!
//! ```toml
//! delay_ms = 30                 # pause between streamed chunks (0 in tests)
//! [[turn]]
//! text = "Let me look at {cwd_name} first."
//! [[turn.call]]
//! name = "list_dir"
//! args = { path = "{cwd}" }
//! [[turn]]
//! text = "I suggest moving the images into a folder."
//! usage = { input = 2400, output = 120 }
//! [[turn.call]]
//! name = "move"
//! args = { sources = ["{cwd}/a.jpg"], dest_dir = "{cwd}/Images" }
//! ```
//!
//! `{name}` placeholders in text and string arguments are replaced from the task's
//! variables (`cwd`, `cwd_name`, `sel0`…, `home`). Each model request consumes one turn;
//! when the script runs out the model ends the task politely.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::Value;
use soyuz_ai::{AiError, ChatRequest, Event, StopReason, ToolProtocol, Usage};

use super::model::{ChatModel, ModelStream};
use super::{CancelFlag, Wake};

/// One scripted model answer.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct FakeTurn {
    pub text: String,
    pub reasoning: String,
    #[serde(rename = "call")]
    pub calls: Vec<FakeCall>,
    pub usage: Option<FakeUsage>,
    /// Fail this request with a provider error instead of answering.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FakeCall {
    pub name: String,
    #[serde(default = "empty_args")]
    pub args: Value,
}

fn empty_args() -> Value {
    Value::Object(Default::default())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct FakeUsage {
    pub input: u64,
    pub output: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct TranscriptFile {
    delay_ms: u64,
    #[serde(rename = "turn")]
    turns: Vec<FakeTurn>,
    model: String,
}

impl FakeTurn {
    pub fn text(t: &str) -> Self {
        FakeTurn { text: t.into(), ..Default::default() }
    }

    pub fn call(mut self, name: &str, args: Value) -> Self {
        self.calls.push(FakeCall { name: name.into(), args });
        self
    }

    pub fn usage(mut self, input: u64, output: u64) -> Self {
        self.usage = Some(FakeUsage { input, output });
        self
    }
}

/// The scripted model.
pub struct FakeModel {
    turns: Mutex<VecDeque<FakeTurn>>,
    delay: Duration,
    model: String,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    local: bool,
}

/// Replace `{var}` placeholders.
fn fill(s: &str, vars: &HashMap<String, String>) -> String {
    if !s.contains('{') {
        return s.to_string();
    }
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

fn fill_value(v: &Value, vars: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => {
            let filled = fill(s, vars);
            // Paths built as "{cwd}/x" use the platform separator.
            let filled =
                if cfg!(windows) && s.starts_with('{') && filled != *s { filled.replace('/', "\\") } else { filled };
            Value::String(filled)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| fill_value(x, vars)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), fill_value(x, vars))).collect()),
        other => other.clone(),
    }
}

impl FakeModel {
    pub fn new(turns: Vec<FakeTurn>) -> Self {
        FakeModel {
            turns: Mutex::new(turns.into()),
            delay: Duration::ZERO,
            model: "fake-transcript".into(),
            requests: Arc::new(Mutex::new(Vec::new())),
            local: true,
        }
    }

    /// Pretend to be a cloud model (redaction applies).
    pub fn cloud(mut self) -> Self {
        self.local = false;
        self
    }

    pub fn with_delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    pub fn from_toml(text: &str, vars: &HashMap<String, String>) -> Result<Self, String> {
        let f: TranscriptFile = toml::from_str(text).map_err(|e| format!("fake transcript: {e}"))?;
        let turns = f
            .turns
            .into_iter()
            .map(|t| FakeTurn {
                text: fill(&t.text, vars),
                reasoning: fill(&t.reasoning, vars),
                calls: t
                    .calls
                    .into_iter()
                    .map(|c| FakeCall { name: c.name, args: fill_value(&c.args, vars) })
                    .collect(),
                usage: t.usage,
                error: t.error,
            })
            .collect();
        let mut m = FakeModel::new(turns).with_delay(Duration::from_millis(f.delay_ms.min(2000)));
        if !f.model.is_empty() {
            m.model = f.model;
        }
        Ok(m)
    }

    pub fn from_file(path: &Path, vars: &HashMap<String, String>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut m = Self::from_toml(&text, vars)?;
        if m.model == "fake-transcript" {
            m.model = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(m.model);
        }
        Ok(m)
    }

    /// Every request the harness sent (tests inspect prompts and tool results).
    pub fn requests(&self) -> Arc<Mutex<Vec<ChatRequest>>> {
        self.requests.clone()
    }

    pub fn remaining(&self) -> usize {
        self.turns.lock().len()
    }
}

/// Split text into small streamed chunks (word boundaries).
fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, w) in text.split_inclusive(' ').enumerate() {
        cur.push_str(w);
        if i % 3 == 2 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl ChatModel for FakeModel {
    fn profile(&self) -> &str {
        "fake"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn is_local(&self) -> bool {
        self.local
    }

    fn protocol(&self) -> ToolProtocol {
        ToolProtocol::Native
    }

    fn max_tokens(&self) -> u32 {
        4096
    }

    fn stream(&self, req: ChatRequest, wake: Wake) -> ModelStream {
        let input_chars: usize =
            req.system.len() + req.messages.iter().map(|m| format!("{:?}", m.parts).len()).sum::<usize>();
        self.requests.lock().push(req);
        let turn = self.turns.lock().pop_front().unwrap_or_else(|| FakeTurn::text("That's everything I had planned."));
        let (tx, rx) = crossbeam_channel::unbounded();
        let cancel = CancelFlag::new();
        let flag = cancel.clone();
        let delay = self.delay;
        let run = move || {
            let send = |ev: Event| {
                let _ = tx.send(ev);
                wake();
            };
            let sleep = || {
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
            };
            if let Some(e) = turn.error {
                return send(Event::Error(AiError::Provider { kind: "fake".into(), message: e }));
            }
            for c in chunks(&turn.reasoning) {
                if flag.is_cancelled() {
                    return send(Event::Error(AiError::Cancelled));
                }
                send(Event::ReasoningDelta(c));
                sleep();
            }
            for c in chunks(&turn.text) {
                if flag.is_cancelled() {
                    return send(Event::Error(AiError::Cancelled));
                }
                send(Event::TextDelta(c));
                sleep();
            }
            for (i, call) in turn.calls.iter().enumerate() {
                if flag.is_cancelled() {
                    return send(Event::Error(AiError::Cancelled));
                }
                send(Event::ToolCall {
                    id: format!("call_{}_{}", call.name, i),
                    name: call.name.clone(),
                    args: call.args.clone(),
                    meta: None,
                });
            }
            let usage = turn.usage.unwrap_or(FakeUsage {
                input: (input_chars / 4) as u64,
                output: (turn.text.len() / 4 + 30 * turn.calls.len()) as u64,
            });
            send(Event::Usage(Usage { input_tokens: usage.input, output_tokens: usage.output, ..Usage::default() }));
            send(Event::Done(if turn.calls.is_empty() { StopReason::EndTurn } else { StopReason::ToolUse }));
        };
        if delay.is_zero() {
            run();
        } else if let Err(e) = std::thread::Builder::new().name("soyuz-ai-fake".into()).spawn(run) {
            log::warn!("fake model: cannot spawn: {e}");
        }
        ModelStream::new(rx, move || cancel.cancel())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_ai::Message;

    #[test]
    fn replays_transcript_with_vars() {
        let vars: HashMap<String, String> = [("cwd".to_string(), "/tmp/x".to_string())].into();
        let m = FakeModel::from_toml(
            r#"
            [[turn]]
            text = "Looking at {cwd} now"
            [[turn.call]]
            name = "list_dir"
            args = { path = "{cwd}" }
            "#,
            &vars,
        )
        .unwrap();
        let s = m.stream(ChatRequest::new("m", vec![Message::user("hi")]), soyuz_ai::no_wake());
        let mut c = soyuz_ai::Collected::default();
        while let Ok(ev) = s.rx.try_recv() {
            c.push(ev);
        }
        assert!(c.text.starts_with("Looking at "));
        assert_eq!(c.tool_calls.len(), 1);
        assert_eq!(c.stop, Some(StopReason::ToolUse));
        assert_eq!(m.requests().lock().len(), 1);
        assert_eq!(m.remaining(), 0);
        let s = m.stream(ChatRequest::new("m", vec![]), soyuz_ai::no_wake());
        let last: Vec<Event> = s.rx.try_iter().collect();
        assert!(matches!(last.last(), Some(Event::Done(StopReason::EndTurn))));
    }

    #[test]
    fn chunking_keeps_text() {
        let t = "one two three four five six seven";
        assert_eq!(chunks(t).concat(), t);
        assert!(chunks("").is_empty());
    }
}
