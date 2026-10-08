//! `soyuz --mcp [--mcp-client <name>] [--mcp-headless]`: the stdio entry point.
//!
//! When a Soyuz window is running (and `[ai.mcp] server_enabled`), this process attaches
//! to it over the named pipe and relays stdin/stdout, so tool calls show in the live UI
//! and write tools get an approval card. Otherwise it serves MCP itself, headless.
//! Nothing but MCP messages is ever written to stdout; logs go to stderr.

use std::io::{BufReader, Write};

use crate::headless::HeadlessBackend;
use crate::server::{Server, Session};
use crate::trust::Transport;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliOptions {
    /// `--mcp-client <name>`: the client's identity in `[[ai.mcp_clients]]`.
    pub client: Option<String>,
    /// `--mcp-headless`: never attach to a running window.
    pub headless: bool,
}

/// Run until stdin closes; returns the process exit code.
pub fn run(opts: CliOptions) -> i32 {
    let config_path = soyuz_core::config::default_path();
    let ai = match soyuz_core::config::load_or_default(&config_path) {
        Ok(l) => l.config.ai,
        Err(e) => {
            log::warn!("{}: {e}; using default AI settings", config_path.display());
            soyuz_core::config::Ai::default()
        }
    };
    if !ai.enabled {
        log::warn!("AI features are disabled ([ai] enabled = false); every tool call will be refused");
    }
    if !opts.headless && ai.mcp.server_enabled {
        let name = crate::pipe::pipe_name();
        match crate::pipe::connect(&name) {
            Ok(Some(stream)) => {
                log::info!("soyuz --mcp: attached to the running Soyuz window ({name})");
                let res = crate::pipe::relay(stream, opts.client.as_deref(), std::io::stdin(), std::io::stdout());
                return if res.is_ok() { 0 } else { 1 };
            }
            Ok(None) => log::info!("soyuz --mcp: no Soyuz window is running; serving headless"),
            Err(e) => log::warn!("soyuz --mcp: cannot attach to the Soyuz window ({e}); serving headless"),
        }
    }
    let history = soyuz_core::config::config_dir().join("history.jsonl");
    let backend = HeadlessBackend::new(ai, Some(config_path), Some(history), soyuz_ai::audit::default_path());
    let server = Server::new(backend.clone());
    let session = Session::new(Transport::Stdio, opts.client);
    let stdout: Box<dyn Write + Send> = Box::new(std::io::stdout());
    server.serve_lines(BufReader::new(std::io::stdin()), stdout, session);
    backend.engine().shutdown(std::time::Duration::from_secs(5));
    0
}

/// Parse the MCP flags out of the command line (`Some` when `--mcp` is present).
pub fn parse(args: &[String]) -> Result<Option<CliOptions>, String> {
    if !args.iter().any(|a| a == "--mcp") {
        return Ok(None);
    }
    let mut opts = CliOptions::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mcp" => {}
            "--mcp-headless" => opts.headless = true,
            "--mcp-client" => {
                let v = it.next().ok_or("--mcp-client needs a name")?;
                opts.client = Some(v.clone());
            }
            other => {
                if let Some(v) = other.strip_prefix("--mcp-client=") {
                    opts.client = Some(v.to_string());
                } else {
                    return Err(format!("unexpected argument '{other}' with --mcp"));
                }
            }
        }
    }
    Ok(Some(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Option<CliOptions>, String> {
        parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn flags() {
        assert_eq!(p(&["C:\\x"]), Ok(None));
        assert_eq!(p(&["--mcp"]), Ok(Some(CliOptions::default())));
        let o = p(&["--mcp", "--mcp-client", "claude-desktop", "--mcp-headless"]).unwrap().unwrap();
        assert_eq!(o.client.as_deref(), Some("claude-desktop"));
        assert!(o.headless);
        assert_eq!(p(&["--mcp", "--mcp-client=x"]).unwrap().unwrap().client.as_deref(), Some("x"));
        assert!(p(&["--mcp", "--mcp-client"]).is_err());
        assert!(p(&["--mcp", "C:\\x"]).is_err());
    }
}
