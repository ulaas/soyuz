//! `soyuz --mcp` → running window: a local named pipe `\\.\pipe\soyuz-mcp-<user>`.
//!
//! The `soyuz --mcp` process is a thin relay: it sends one hello line
//! (`{"soyuz_mcp_attach":1,"client":…}`), then copies stdin → pipe and pipe → stdout. The
//! window runs the MCP server for the connection, so actions and approval cards appear
//! in the live UI.

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::server::{Server, Session};
use crate::trust::Transport;

/// Pipe name for the current user.
pub fn pipe_name() -> String {
    user_pipe_name("soyuz-mcp")
}

/// `\\.\pipe\<prefix>-<user>`: a per-user pipe name (the CLI companion uses `soyuz-cli`).
pub fn user_pipe_name(prefix: &str) -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let user = crate::trust::sanitize_name(&user);
    format!(r"\\.\pipe\{prefix}-{}", if user.is_empty() { "user" } else { &user })
}

/// The first line on a pipe connection.
pub fn hello(client: Option<&str>) -> String {
    let mut v = json!({"soyuz_mcp_attach": 1, "pid": std::process::id()});
    if let Some(c) = client {
        v["client"] = json!(c);
    }
    let mut s = v.to_string();
    s.push('\n');
    s
}

/// Parse a hello line; returns the client name it names (if any).
pub fn parse_hello(line: &str) -> Option<Option<String>> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("soyuz_mcp_attach").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    Some(v.get("client").and_then(Value::as_str).map(str::to_string))
}

/// A running pipe server (the window side).
pub struct PipeServer {
    stop: Arc<AtomicBool>,
    name: String,
}

impl PipeServer {
    /// Bind `name` and serve each connection with `server` on its own thread.
    pub fn start(name: &str, server: Arc<Server>) -> std::io::Result<PipeServer> {
        PipeServer::serve(name, move |stream| serve_connection(server.clone(), stream))
    }

    /// Bind `name` (current user only, see the DACL in `windows.rs`) and run `handler`
    /// for each connection on its own thread. Used by the MCP server and the CLI companion.
    #[cfg(windows)]
    pub fn serve<F>(name: &str, handler: F) -> std::io::Result<PipeServer>
    where
        F: Fn(ClientStream) + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);
        let mut listener = windows::PipeListener::bind(name)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let pipe = name.to_string();
        std::thread::Builder::new().name("soyuz-pipe".into()).spawn(move || {
            loop {
                let stream = match listener.accept() {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("{pipe}: accept failed: {e}");
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if stop2.load(Ordering::SeqCst) {
                            break;
                        }
                        continue;
                    }
                };
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let handler = handler.clone();
                let _ = std::thread::Builder::new().name("soyuz-pipe-conn".into()).spawn(move || handler(stream));
            }
        })?;
        Ok(PipeServer { stop, name: name.to_string() })
    }

    #[cfg(not(windows))]
    pub fn serve<F>(_name: &str, _handler: F) -> std::io::Result<PipeServer>
    where
        F: Fn(ClientStream) + Send + Sync + 'static,
    {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "the local pipe is Windows-only for now"))
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Unblock the accept loop.
        let _ = connect(&self.name);
    }
}

/// Serve one pipe connection: hello line, then MCP.
pub fn serve_connection<S: Read + Write + Clone + Send + 'static>(server: Arc<Server>, stream: S) {
    let mut reader = BufReader::new(stream.clone());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let Some(client) = parse_hello(&first) else {
        log::warn!("mcp pipe: connection without a hello line; closing");
        return;
    };
    let session = Session::new(Transport::Pipe, client);
    server.serve_lines(reader, Box::new(stream), session);
}

/// A connected client end.
#[cfg(windows)]
pub type ClientStream = windows::PipeStream;

/// Connect to the running window; `Ok(None)` when no window serves the pipe.
#[cfg(windows)]
pub fn connect(name: &str) -> std::io::Result<Option<ClientStream>> {
    windows::connect(name)
}

/// A connected client end (no pipe outside Windows yet).
#[cfg(not(windows))]
pub type ClientStream = std::fs::File;

#[cfg(not(windows))]
pub fn connect(_name: &str) -> std::io::Result<Option<ClientStream>> {
    Ok(None)
}

/// Relay stdin/stdout to a connected pipe until either side closes.
pub fn relay<S: Read + Write + Clone + Send + 'static>(
    stream: S,
    client: Option<&str>,
    stdin: impl Read + Send + 'static,
    stdout: impl Write + Send + 'static,
) -> std::io::Result<()> {
    let mut to_pipe = stream.clone();
    to_pipe.write_all(hello(client).as_bytes())?;
    let (done_tx, done_rx) = crossbeam_channel::bounded::<&'static str>(2);
    let tx = done_tx.clone();
    std::thread::Builder::new().name("soyuz-mcp-stdin".into()).spawn(move || {
        let mut stdin = stdin;
        let _ = std::io::copy(&mut stdin, &mut to_pipe);
        let _ = tx.send("stdin closed");
    })?;
    std::thread::Builder::new().name("soyuz-mcp-stdout".into()).spawn(move || {
        let mut from_pipe = stream;
        let mut out = stdout;
        let mut buf = [0u8; 16 * 1024];
        loop {
            match from_pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out.write_all(&buf[..n]).and_then(|_| out.flush()).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = done_tx.send("window closed the connection");
    })?;
    let why = done_rx.recv().unwrap_or("relay ended");
    log::info!("soyuz --mcp: {why}");
    Ok(())
}
