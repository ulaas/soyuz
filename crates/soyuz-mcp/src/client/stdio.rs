//! stdio client transport: newline-delimited JSON-RPC over a child process (or any byte
//! stream, for tests).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, bounded};
use parking_lot::Mutex;
use serde_json::{Value, json};
use soyuz_core::OpControl;

use super::{ClientError, Transport};
use crate::jsonrpc;

type Pending = Arc<Mutex<HashMap<String, Sender<Value>>>>;

/// JSON-RPC over a reader / writer pair.
pub struct StreamTransport {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pending: Pending,
    closed: Arc<AtomicBool>,
}

impl StreamTransport {
    pub fn new(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(Box::new(writer)));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        // Weak: the reader thread must not keep the server's stdin open after we are dropped.
        let (p, c, w) = (pending.clone(), closed.clone(), Arc::downgrade(&writer));
        let _ = std::thread::Builder::new().name("soyuz-mcp-client-rx".into()).spawn(move || {
            for line in BufReader::new(reader).lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
                let has_result = msg.get("result").is_some() || msg.get("error").is_some();
                match (msg.get("id"), msg.get("method").and_then(Value::as_str)) {
                    (Some(id), None) if has_result => {
                        if let Some(tx) = p.lock().remove(&jsonrpc::id_key(id)) {
                            let _ = tx.send(msg);
                        }
                    }
                    // Server → client request (legacy servers): we offer no client features.
                    (Some(id), Some(method)) => {
                        let reply = if method == "ping" {
                            jsonrpc::result(id, json!({}))
                        } else {
                            jsonrpc::error(
                                id,
                                &jsonrpc::RpcError::new(
                                    jsonrpc::METHOD_NOT_FOUND,
                                    format!("{method} is not supported"),
                                ),
                            )
                        };
                        if let Some(w) = w.upgrade() {
                            let mut w = w.lock();
                            let _ = w.write_all(format!("{reply}\n").as_bytes()).and_then(|_| w.flush());
                        }
                    }
                    _ => {} // notifications (progress, logging): ignored
                }
            }
            c.store(true, Ordering::SeqCst);
            p.lock().clear();
        });
        StreamTransport { writer, pending, closed }
    }

    fn send(&self, msg: &Value) -> Result<(), ClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(ClientError::Transport("the server closed the connection".into()));
        }
        let mut w = self.writer.lock();
        w.write_all(format!("{msg}\n").as_bytes())
            .and_then(|_| w.flush())
            .map_err(|e| ClientError::Transport(format!("cannot write to the server: {e}")))
    }
}

impl Transport for StreamTransport {
    fn request(&self, msg: Value, timeout: Duration, ctl: &OpControl) -> Result<Value, ClientError> {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let key = jsonrpc::id_key(&id);
        let (tx, rx) = bounded(1);
        self.pending.lock().insert(key.clone(), tx);
        if let Err(e) = self.send(&msg) {
            self.pending.lock().remove(&key);
            return Err(e);
        }
        let deadline = Instant::now() + timeout;
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(v) => return Ok(v),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return Err(ClientError::Transport("the server closed the connection".into()));
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }
            let cancelled = ctl.is_cancelled();
            if cancelled || Instant::now() > deadline {
                self.pending.lock().remove(&key);
                let reason = if cancelled { "cancelled by the user" } else { "timed out" };
                let _ = self.send(&jsonrpc::notification(
                    "notifications/cancelled",
                    json!({"requestId": id, "reason": reason}),
                ));
                return Err(if cancelled { ClientError::Cancelled } else { ClientError::Timeout });
            }
        }
    }

    fn notify(&self, msg: Value) -> Result<(), ClientError> {
        self.send(&msg)
    }
}

/// A server started as a child process.
pub struct StdioTransport {
    inner: StreamTransport,
    child: Mutex<Child>,
}

impl StdioTransport {
    /// Start `command args…` with `env` added (no shell is involved).
    pub fn spawn(
        command: &str,
        args: &[String],
        env: &std::collections::BTreeMap<String, String>,
        server_name: &str,
    ) -> Result<StdioTransport, ClientError> {
        let mut cmd = Command::new(soyuz_fs::expand_env(command));
        cmd.args(args).envs(env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Own process group: a Ctrl+C in the terminal that started Soyuz must not reach the
        // server, and dropping the transport can stop the server's own children too.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| ClientError::Transport(format!("cannot start “{command}”: {e}")))?;
        let stdin = child.stdin.take().ok_or_else(|| ClientError::Transport("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| ClientError::Transport("no stdout".into()))?;
        if let Some(stderr) = child.stderr.take() {
            let name = server_name.to_string();
            let _ = std::thread::Builder::new().name("soyuz-mcp-client-err".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    log::debug!("mcp server {name}: {line}");
                }
            });
        }
        Ok(StdioTransport { inner: StreamTransport::new(stdout, stdin), child: Mutex::new(child) })
    }
}

impl Transport for StdioTransport {
    fn request(&self, msg: Value, timeout: Duration, ctl: &OpControl) -> Result<Value, ClientError> {
        self.inner.request(msg, timeout, ctl)
    }

    fn notify(&self, msg: Value) -> Result<(), ClientError> {
        self.inner.notify(msg)
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Close stdin (graceful shutdown signal), give the server a moment, then kill.
        *self.inner.writer.lock() = Box::new(std::io::sink());
        let mut child = self.child.lock();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if exited_unreaped(&mut child) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        // The leader is not reaped yet (at most a zombie), so its pid, and with it the
        // group id, cannot have been reused: the group kill only hits the server's own
        // leftovers. Reap afterwards.
        kill_group(&child);
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Has the child exited? On unix this peeks without reaping (`waitid` with `WNOWAIT`),
/// so the pid / process group id stays reserved until [`Child::wait`].
#[cfg(unix)]
fn exited_unreaped(child: &mut Child) -> bool {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let pid = Pid::from_child(child);
    let opts = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    match waitid(WaitId::Pid(pid), opts) {
        Ok(status) => status.is_some(),
        // Already reaped (ECHILD) or unknown: stop waiting; `kill_group` checks again.
        Err(_) => true,
    }
}

#[cfg(not(unix))]
fn exited_unreaped(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(Some(_)))
}

/// Kill whatever is left of the server's process group (`npx` / `uvx` launchers leave the
/// real server as a grandchild). The group id is the child's pid (`process_group(0)`).
/// Only called while the leader is unreaped (so the id cannot have been reused); if it
/// was reaped somehow (`waitid` fails with `ECHILD`), nothing is killed.
#[cfg(unix)]
fn kill_group(child: &Child) {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let pid = Pid::from_child(child);
    let opts = WaitIdOptions::EXITED | WaitIdOptions::STOPPED | WaitIdOptions::CONTINUED;
    if waitid(WaitId::Pid(pid), opts | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT).is_err() {
        return;
    }
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
}

#[cfg(not(unix))]
fn kill_group(_child: &Child) {}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// Dropping the transport stops the server's whole process group, grandchildren included.
    #[test]
    fn drop_kills_the_process_group() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("pid");
        let script = format!("sleep 30 & echo $! > '{}'; wait", pidfile.display());
        let t = StdioTransport::spawn("/bin/sh", &["-c".into(), script], &Default::default(), "test").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(p) = std::fs::read_to_string(&pidfile).ok().and_then(|s| s.trim().parse::<i32>().ok()) {
                break p;
            }
            assert!(Instant::now() < deadline, "the script did not start");
            std::thread::sleep(Duration::from_millis(10));
        };
        let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        assert!(proc_dir.exists());
        drop(t);
        // The orphaned grandchild is reaped by init / the subreaper shortly after the kill.
        let deadline = Instant::now() + Duration::from_secs(5);
        let gone = || {
            std::fs::read_to_string(proc_dir.join("stat")).map_or(true, |s| s.contains(") Z ") || !s.contains("sleep"))
        };
        while !gone() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(gone(), "the grandchild survived the drop");
    }

    /// Exit is detected without reaping: the leader stays a zombie (its pid, and so the
    /// group id, reserved) until the group was killed and `wait` reaps it.
    #[test]
    fn exit_check_does_not_reap() {
        let mut child = Command::new("/bin/true").spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !exited_unreaped(&mut child) {
            assert!(Instant::now() < deadline, "/bin/true did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).expect("still a zombie");
        assert!(stat.contains(") Z "), "{stat}");
        assert!(child.try_wait().unwrap().is_some());
        // Reaped now: the group kill is skipped instead of hitting a possibly reused id.
        kill_group(&child);
    }

    /// The leader exits first (a launcher that forked the real server): the leftover group
    /// is still killed on drop.
    #[test]
    fn drop_kills_the_group_after_the_leader_exited() {
        let d = tempfile::tempdir().unwrap();
        let pidfile = d.path().join("pid");
        let script = format!("sleep 30 & echo $! > '{}'; exit 0", pidfile.display());
        let t = StdioTransport::spawn("/bin/sh", &["-c".into(), script], &Default::default(), "test").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(p) = std::fs::read_to_string(&pidfile).ok().and_then(|s| s.trim().parse::<i32>().ok()) {
                break p;
            }
            assert!(Instant::now() < deadline, "the script did not start");
            std::thread::sleep(Duration::from_millis(10));
        };
        let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
        drop(t);
        let deadline = Instant::now() + Duration::from_secs(5);
        let gone = || {
            std::fs::read_to_string(proc_dir.join("stat")).map_or(true, |s| s.contains(") Z ") || !s.contains("sleep"))
        };
        while !gone() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(gone(), "the grandchild survived the drop");
    }
}
