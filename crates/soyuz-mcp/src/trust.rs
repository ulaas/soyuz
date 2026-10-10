//! Client trust model: who is calling, what it may touch, and whether its
//! write tools need approval in Soyuz.
//!
//! * Clients are listed in `[[ai.mcp_clients]]`. Over stdio a client is named by
//!   `soyuz --mcp --mcp-client <name>` (or, failing that, by the name it reports); over
//!   HTTP it is identified by its bearer token.
//! * An unknown stdio client is registered as **pending**: read-only until the user
//!   approves it in Settings → AI assistant → Connections.
//! * A client's folders are always narrowed to the global `[ai.scope]`.

use std::path::PathBuf;

use soyuz_ai::tools::{Risk, Scope};
use soyuz_core::config::{Ai, AiMcpClient, AiRisk};

/// How a connection reached the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// Headless `soyuz --mcp` on stdin/stdout.
    Stdio,
    /// `soyuz --mcp` attached to the running window over the named pipe.
    Pipe,
    Http,
    /// In-process (tests).
    Memory,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Transport::Stdio => "stdio",
            Transport::Pipe => "pipe",
            Transport::Http => "http",
            Transport::Memory => "memory",
        }
    }
}

pub fn risk_of(r: AiRisk) -> Risk {
    match r {
        AiRisk::Read => Risk::Read,
        AiRisk::Write => Risk::Write,
        AiRisk::Destructive => Risk::Destructive,
    }
}

/// Client names: 1–64 of `a-z 0-9 . _ -` (lower-cased); other characters become `-`.
pub fn sanitize_name(raw: &str) -> String {
    let mut s: String = raw
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c.to_ascii_lowercase() } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let mut s = s.to_string();
    s.truncate(64);
    s
}

/// Effective permissions of one client for one request.
#[derive(Clone, Debug)]
pub struct ClientPolicy {
    pub name: String,
    pub transport: Transport,
    /// Registered but not approved yet: read-only.
    pub pending: bool,
    pub max_risk: Risk,
    pub require_approval: bool,
    pub scope: Scope,
}

impl ClientPolicy {
    /// Policy from a config entry (`None` = unknown client → pending, read-only).
    pub fn new(ai: &Ai, entry: Option<&AiMcpClient>, name: &str, transport: Transport) -> Self {
        let pending = entry.is_none_or(|c| c.pending);
        let max_risk = match entry {
            Some(c) if !c.pending => risk_of(c.max_risk),
            _ => Risk::Read,
        };
        let roots = entry.map(|c| c.allowed_roots.clone()).unwrap_or_default();
        ClientPolicy {
            name: name.to_string(),
            transport,
            pending,
            max_risk,
            require_approval: entry.is_none_or(|c| c.require_approval),
            scope: client_scope(ai, &roots),
        }
    }
}

/// The global scope narrowed to `roots` (empty = the global roots). Roots outside the
/// global scope are dropped; if none remain the client gets no folders at all.
pub fn client_scope(ai: &Ai, roots: &[PathBuf]) -> Scope {
    let global = Scope::from_config(&ai.scope);
    if roots.is_empty() {
        return global;
    }
    let kept: Vec<PathBuf> = roots
        .iter()
        .map(|r| PathBuf::from(soyuz_fs::expand_env(&r.to_string_lossy())))
        .filter(|r| global.check_read(r).is_ok())
        .collect();
    let narrowed = Scope::new(kept, &ai.scope.blocked, ai.scope.allow_sensitive);
    if narrowed.roots().is_empty() {
        log::warn!("ai.mcp_clients: none of {roots:?} is inside [ai.scope]; the client gets no folders");
    }
    narrowed
}

/// Look a client up by (sanitized) name.
pub fn find<'a>(ai: &'a Ai, name: &str) -> Option<&'a AiMcpClient> {
    ai.mcp_clients.iter().find(|c| sanitize_name(&c.name) == name)
}

/// A random bearer token (256 bits, URL-safe).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        // Extremely unlikely; fall back to hashing time + process data.
        let seed = format!("{:?}{:?}{}", std::time::SystemTime::now(), std::thread::current().id(), std::process::id());
        bytes = *blake3::hash(seed.as_bytes()).as_bytes();
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("soyuz_{hex}")
}

/// Constant-time comparison for tokens.
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Credential-store name for a client's HTTP token.
pub fn token_credential(client: &str) -> String {
    let mut n = format!("mcp-client-{}", sanitize_name(client));
    n.truncate(64);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_tokens() {
        assert_eq!(sanitize_name("Claude Desktop"), "claude-desktop");
        assert_eq!(sanitize_name("  VS Code / Copilot "), "vs-code-copilot");
        assert_eq!(sanitize_name("???"), "");
        let t = generate_token();
        assert!(t.starts_with("soyuz_") && t.len() == 70);
        assert_ne!(t, generate_token());
        assert!(token_eq(&t, &t.clone()));
        assert!(!token_eq(&t, "soyuz_x"));
        assert!(soyuz_ai::credentials::valid_name(&token_credential("Claude Desktop")));
    }

    #[test]
    fn unknown_and_pending_clients_are_read_only() {
        let mut ai = Ai::default();
        let p = ClientPolicy::new(&ai, None, "x", Transport::Stdio);
        assert!(p.pending && p.max_risk == Risk::Read && p.require_approval);
        ai.mcp_clients.push(AiMcpClient {
            name: "x".into(),
            pending: true,
            max_risk: AiRisk::Destructive,
            ..Default::default()
        });
        let p = ClientPolicy::new(&ai, find(&ai, "x"), "x", Transport::Stdio);
        assert_eq!(p.max_risk, Risk::Read);
        ai.mcp_clients[0].pending = false;
        let p = ClientPolicy::new(&ai, find(&ai, "x"), "x", Transport::Stdio);
        assert_eq!(p.max_risk, Risk::Destructive);
    }

    #[test]
    fn client_roots_are_narrowed_to_the_global_scope() {
        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("in");
        std::fs::create_dir_all(&inside).unwrap();
        let mut ai = Ai::default();
        ai.scope.allowed_roots = vec![dir.path().to_path_buf()];
        let s = client_scope(&ai, &[inside.clone(), std::env::temp_dir().join("elsewhere-soyuz")]);
        assert_eq!(s.roots().len(), 1);
        assert!(s.check_read(&inside.join("f")).is_ok());
        assert!(s.check_read(&dir.path().join("other")).is_err());
    }
}
