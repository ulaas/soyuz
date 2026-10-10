//! MCP settings inside `[ai]`:
//!
//! * `[ai.mcp]` — the Soyuz MCP server: the local named pipe that `soyuz --mcp` attaches
//!   to, and the optional Streamable HTTP transport on 127.0.0.1.
//! * `[[ai.mcp_clients]]` — MCP clients allowed to drive Soyuz (Claude Desktop, IDEs …)
//!   with their folder scope, maximum risk class and approval requirement.
//! * `[[ai.mcp_servers]]` — external MCP servers whose tools the in-app assistant may use.
//!
//! Tokens are never stored here: `token` holds a `credential:<name>` reference.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Risk ceiling of an MCP client / risk class of an external server's tools.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiRisk {
    /// Read-only tools (listing, search, metadata, UI navigation).
    Read,
    /// File changes through the jobs engine (copy, move, rename, create …).
    #[default]
    Write,
    /// Trash, permanent delete, sync with deletions, custom commands.
    Destructive,
}

impl AiRisk {
    pub fn label(self) -> &'static str {
        match self {
            AiRisk::Read => "Read only",
            AiRisk::Write => "Read and write",
            AiRisk::Destructive => "Everything (incl. delete)",
        }
    }
}

/// Where approval cards for MCP clients appear.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiMcpApprovalUi {
    /// In the assistant panel's approvals area (the panel opens when a request arrives).
    #[default]
    Panel,
    /// A modal card over the window (same plan card, one request at a time).
    Dialog,
}

/// `[ai.mcp]`: the Soyuz MCP server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiMcp {
    /// The running app listens on a local named pipe so `soyuz --mcp` can attach and
    /// show actions in the live window. Off = `soyuz --mcp` always runs headless.
    pub server_enabled: bool,
    /// Streamable HTTP transport on 127.0.0.1 (each client needs its own bearer token).
    pub http_enabled: bool,
    /// Port of the HTTP transport on 127.0.0.1.
    pub http_port: u16,
    /// Extra browser origins allowed to call the HTTP endpoint (`http://localhost:<port>`
    /// and `http://127.0.0.1:<port>` are always allowed; requests without `Origin` too).
    pub allowed_origins: Vec<String>,
    /// Register unknown stdio clients as "pending" (read-only until approved). Off =
    /// unknown clients are refused.
    pub register_unknown: bool,
    /// Seconds an approval card waits for an answer before the call is refused.
    pub approval_timeout_secs: u64,
    /// Where MCP approval cards appear.
    pub approval_ui: AiMcpApprovalUi,
}

impl Default for AiMcp {
    fn default() -> Self {
        AiMcp {
            server_enabled: true,
            http_enabled: false,
            http_port: 7863,
            allowed_origins: Vec::new(),
            register_unknown: true,
            approval_timeout_secs: 300,
            approval_ui: AiMcpApprovalUi::Panel,
        }
    }
}

/// `[[ai.mcp_clients]]`: an MCP client allowed to use Soyuz.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiMcpClient {
    /// Identity: the `--mcp-client <name>` argument (stdio) or the name reported by the
    /// client; HTTP clients are identified by their token.
    pub name: String,
    /// Registered automatically on first connect; read-only until approved in Settings.
    pub pending: bool,
    /// Bearer token for the HTTP transport (`credential:<name>`); empty = stdio only.
    pub token: String,
    /// Folders this client may use (empty = `[ai.scope] allowed_roots`). Always narrowed
    /// to the global AI scope.
    pub allowed_roots: Vec<PathBuf>,
    pub max_risk: AiRisk,
    /// Write / destructive tools show an approval card in Soyuz first. Off = they run
    /// without asking (only within `max_risk`).
    pub require_approval: bool,
}

impl Default for AiMcpClient {
    fn default() -> Self {
        AiMcpClient {
            name: String::new(),
            pending: false,
            token: String::new(),
            allowed_roots: Vec::new(),
            max_risk: AiRisk::Write,
            require_approval: true,
        }
    }
}

/// `[[ai.mcp_servers]]`: an external MCP server mounted as extra assistant tools.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiMcpServer {
    /// Name shown in the assistant's tool list.
    pub name: String,
    /// Connect to this server.
    pub enabled: bool,
    /// stdio transport: program to start (no shell is involved) and its arguments.
    pub command: String,
    /// Arguments for `command`.
    pub args: Vec<String>,
    /// Extra environment variables for `command` (values are not secret).
    pub env: BTreeMap<String, String>,
    /// Streamable HTTP transport: endpoint URL (used instead of `command` when set).
    pub url: String,
    /// Bearer token for `url` (`credential:<name>` | `env:<VAR>` | empty).
    pub token: String,
    /// Risk class of this server's tools (they can do anything the server can).
    pub risk: AiRisk,
    /// Treat tools the server marks `readOnlyHint` as read-only (only for servers you trust).
    pub trust_annotations: bool,
    /// Tools above read risk need approval in Soyuz before they run.
    pub require_approval: bool,
    /// Per-request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for AiMcpServer {
    fn default() -> Self {
        AiMcpServer {
            name: String::new(),
            enabled: true,
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            url: String::new(),
            token: String::new(),
            risk: AiRisk::Write,
            trust_annotations: false,
            require_approval: true,
            timeout_secs: 60,
        }
    }
}
