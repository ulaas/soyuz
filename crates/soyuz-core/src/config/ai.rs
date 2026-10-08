//! `[ai]` section (roadmap Phase A1): provider profiles, per-task models, budgets, privacy
//! and tool scopes. Secrets are never stored here: `key` holds a reference
//! (`credential:<name>` for the OS credential store, `env:<VAR>` for an environment
//! variable, empty for none). Consumed by the `soyuz-ai` crate and the app's AI services.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which wire protocol / backend a profile talks to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderKind {
    /// Anthropic Messages API.
    #[default]
    Anthropic,
    /// OpenAI Chat Completions (cloud).
    Openai,
    /// OpenAI Responses API (cloud; needed for tool calling on the newest OpenAI models).
    OpenaiResponses,
    /// Ollama's OpenAI-compatible endpoint.
    Ollama,
    /// LM Studio's OpenAI-compatible server.
    LmStudio,
    /// llama.cpp `llama-server`.
    LlamaCpp,
    Vllm,
    Localai,
    /// Google Gemini `generateContent`.
    Gemini,
    /// Any OpenAI-compatible Chat Completions endpoint with custom headers.
    Custom,
}

impl AiProviderKind {
    pub const ALL: [AiProviderKind; 10] = [
        AiProviderKind::Anthropic,
        AiProviderKind::Openai,
        AiProviderKind::OpenaiResponses,
        AiProviderKind::Gemini,
        AiProviderKind::Ollama,
        AiProviderKind::LmStudio,
        AiProviderKind::LlamaCpp,
        AiProviderKind::Vllm,
        AiProviderKind::Localai,
        AiProviderKind::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AiProviderKind::Anthropic => "Anthropic",
            AiProviderKind::Openai => "OpenAI (Chat Completions)",
            AiProviderKind::OpenaiResponses => "OpenAI (Responses)",
            AiProviderKind::Ollama => "Ollama",
            AiProviderKind::LmStudio => "LM Studio",
            AiProviderKind::LlamaCpp => "llama.cpp server",
            AiProviderKind::Vllm => "vLLM",
            AiProviderKind::Localai => "LocalAI",
            AiProviderKind::Gemini => "Google Gemini",
            AiProviderKind::Custom => "Custom HTTP (OpenAI-compatible)",
        }
    }

    /// Base URL used when the profile leaves `base_url` empty.
    pub fn default_base_url(self) -> &'static str {
        match self {
            AiProviderKind::Anthropic => "https://api.anthropic.com",
            AiProviderKind::Openai | AiProviderKind::OpenaiResponses => "https://api.openai.com/v1",
            AiProviderKind::Ollama => "http://localhost:11434/v1",
            AiProviderKind::LmStudio => "http://localhost:1234/v1",
            AiProviderKind::LlamaCpp => "http://localhost:8080/v1",
            AiProviderKind::Vllm => "http://localhost:8000/v1",
            AiProviderKind::Localai => "http://localhost:8080/v1",
            AiProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            AiProviderKind::Custom => "",
        }
    }

    /// Runs on this machine (no API key needed by default, content never leaves the box).
    pub fn is_local(self) -> bool {
        matches!(
            self,
            AiProviderKind::Ollama
                | AiProviderKind::LmStudio
                | AiProviderKind::LlamaCpp
                | AiProviderKind::Vllm
                | AiProviderKind::Localai
        )
    }
}

/// How tools are offered to the model.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiToolMode {
    /// Native tool calling when the capability probe says it works, else the JSON protocol.
    #[default]
    Auto,
    Native,
    /// Constrained JSON tool protocol in plain text (for models without tool calling).
    Json,
}

/// What the assistant may see by default (A7 refines this per folder).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiPrivacy {
    /// No file information at all.
    None,
    /// Names, sizes, dates and types only; file contents never leave the machine.
    #[default]
    MetadataOnly,
    /// File contents may be sent (still subject to scopes and size caps).
    Contents,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiProfile {
    pub name: String,
    pub kind: AiProviderKind,
    /// Empty = the kind's default.
    pub base_url: String,
    pub model: String,
    /// `credential:<name>` | `env:<VAR>` | empty (no key).
    pub key: String,
    pub tool_mode: AiToolMode,
    /// Monthly spending cap in USD for this profile; 0 = no cap.
    pub monthly_budget_usd: f64,
    /// Extra HTTP headers (custom profiles, proxies). Values are not secret.
    pub headers: std::collections::BTreeMap<String, String>,
    pub max_tokens: u32,
}

impl Default for AiProfile {
    fn default() -> Self {
        AiProfile {
            name: String::new(),
            kind: AiProviderKind::Anthropic,
            base_url: String::new(),
            model: String::new(),
            key: String::new(),
            tool_mode: AiToolMode::Auto,
            monthly_budget_usd: 0.0,
            headers: Default::default(),
            max_tokens: 16_000,
        }
    }
}

/// A `(profile, model)` pair for one kind of task; empty fields fall back to the default
/// profile / that profile's model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiTaskModel {
    pub profile: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiTasks {
    /// Cheap / local model for naming and classifying.
    pub fast: AiTaskModel,
    /// Strong model for planning.
    pub strong: AiTaskModel,
}

/// One price-table row; `model` is a glob (`claude-opus-*`). Prices in USD per million tokens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiPrice {
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

/// Which paths AI tools may touch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiScope {
    /// Empty = the home folder.
    pub allowed_roots: Vec<PathBuf>,
    /// Extra blocked glob patterns (matched against the full path, `/` separators).
    pub blocked: Vec<String>,
    /// Allow the built-in sensitive paths (`.ssh`, `.env`, keychains, browser profiles,
    /// wallets). Off by default.
    pub allow_sensitive: bool,
    /// Size cap for `read_text`, KiB.
    pub max_read_kib: u64,
}

impl Default for AiScope {
    fn default() -> Self {
        AiScope { allowed_roots: Vec::new(), blocked: Vec::new(), allow_sensitive: false, max_read_kib: 256 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ai {
    pub enabled: bool,
    /// Profile name used when a task names none.
    pub default_profile: String,
    pub profiles: Vec<AiProfile>,
    pub tasks: AiTasks,
    pub privacy: AiPrivacy,
    pub scope: AiScope,
    /// Seconds to wait for the response headers.
    pub request_timeout_secs: u64,
    /// Seconds without any streamed byte before the request is abandoned.
    pub idle_timeout_secs: u64,
    /// Retries for 429 / overloaded / 5xx before giving up (honours `retry-after`).
    pub max_retries: u32,
    /// Warn when a profile's monthly spend reaches this percentage of its cap.
    pub budget_warn_percent: u32,
    /// Price table (cost accounting); the first matching row wins.
    pub prices: Vec<AiPrice>,
    /// `[ai.mcp]`: the Soyuz MCP server (A3).
    pub mcp: super::mcp::AiMcp,
    /// `[[ai.mcp_clients]]`: MCP clients allowed to drive Soyuz (A3).
    pub mcp_clients: Vec<super::mcp::AiMcpClient>,
    /// `[[ai.mcp_servers]]`: external MCP servers mounted as assistant tools (A4).
    pub mcp_servers: Vec<super::mcp::AiMcpServer>,
}

impl Default for Ai {
    fn default() -> Self {
        Ai {
            enabled: true,
            default_profile: String::new(),
            profiles: Vec::new(),
            tasks: AiTasks::default(),
            privacy: AiPrivacy::MetadataOnly,
            scope: AiScope::default(),
            request_timeout_secs: 60,
            idle_timeout_secs: 120,
            max_retries: 3,
            budget_warn_percent: 80,
            prices: default_prices(),
            mcp: Default::default(),
            mcp_clients: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }
}

/// Built-in prices (USD / MTok, Anthropic list prices verified 2026-10). Users can edit
/// or extend the table; unknown models cost 0 and are shown as "unpriced".
pub fn default_prices() -> Vec<AiPrice> {
    let p = |model: &str, i: f64, o: f64, cr: f64| AiPrice {
        model: model.into(),
        input_per_mtok: i,
        output_per_mtok: o,
        cache_read_per_mtok: cr,
        cache_write_per_mtok: i * 1.25,
    };
    vec![
        p("claude-fable-5*", 10.0, 50.0, 1.0),
        p("claude-opus-5-5*", 4.0, 20.0, 0.2),
        p("claude-opus-*", 5.0, 25.0, 0.5),
        p("claude-sonnet-5*", 2.0, 10.0, 0.2),
        p("claude-sonnet-4*", 3.0, 15.0, 0.3),
        p("claude-haiku-5-5*", 0.1, 0.5, 0.01),
        p("claude-haiku-4*", 1.0, 5.0, 0.1),
    ]
}

impl Ai {
    /// Profile by name (case-sensitive).
    pub fn profile(&self, name: &str) -> Option<&AiProfile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    /// The default profile: `default_profile` when it exists, else the first one.
    pub fn default_profile(&self) -> Option<&AiProfile> {
        self.profile(&self.default_profile).or_else(|| self.profiles.first())
    }
}
