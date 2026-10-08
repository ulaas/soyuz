//! `soyuz-ai`: the AI layer of Soyuz (roadmap Phase A1 + A2).
//!
//! * [`provider`] — streaming chat with tool calling over Anthropic, OpenAI (Chat
//!   Completions and Responses), OpenAI-compatible local servers (Ollama, LM Studio,
//!   llama.cpp, vLLM, LocalAI, custom) and Gemini; retries, timeouts, cancellation, a
//!   capability probe and the JSON tool fallback ([`json_tools`]).
//! * [`audit`] — the append-only `ai-audit.jsonl` log shared by the MCP server and the harness.
//! * [`credentials`] — API keys in the OS credential store (config holds references only).
//! * [`pricing`] — price table, usage ledger and monthly budget caps.
//! * [`tools`] — the typed tool catalog shared by the MCP server (A3) and the in-app
//!   harness (A5): JSON Schemas, risk classes, scopes, dry-run plans and the
//!   [`tools::ToolHost`] trait.
//!
//! The app is not async: requests run on a private tokio runtime and stream [`Event`]s
//! over crossbeam channels. Models never get raw filesystem access: write tools only
//! produce [`tools::Plan`]s holding `JobSpec`s that the host submits after approval.
#![deny(unsafe_code)]

pub mod audit;
pub mod credentials;
pub mod error;
pub mod json_tools;
pub mod pricing;
pub mod provider;
pub mod runtime;
pub mod schema;
pub mod sse;
pub mod tools;
pub mod types;

pub use credentials::Secret;
pub use error::{AiError, AiResult};
pub use pricing::{BudgetStatus, PriceTable, Spend, UsageLedger};
pub use provider::{NetSettings, ProbeReport, Provider, StreamHandle};
pub use runtime::{CancelToken, Wake, no_wake};
pub use types::{ChatRequest, Collected, Event, Message, Part, Role, StopReason, ToolProtocol, ToolSpec, Usage, Wire};
