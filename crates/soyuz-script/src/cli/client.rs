//! Running a CLI command: through the window when one serves the pipe, else headless.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use soyuz_core::{ConflictResolution, SizeStyle, format_size};
use soyuz_jobs::{DeleteMode, EngineConfig, JobEngine, JobKind, JobSpec, QueueMode};

use super::proto::{self, Request};
use super::{Command, Conflict, HELP, Op, exit};

/// Run a parsed command; returns the process exit code.
pub fn run(cmd: Command) -> i32 {
    let config =
        soyuz_core::config::load_or_default(&soyuz_core::config::default_path()).map(|l| l.config).unwrap_or_default();
    let pipe = (config.cli.server_enabled && !cmd.headless).then(proto::pipe_name);
    let history = soyuz_core::config::config_dir().join("history.jsonl");
    let stdout = std::io::stdout();
    let tty = stdout.is_terminal();
    let mut out = stdout.lock();
    run_with(&cmd, pipe.as_deref(), Some(history), &mut out, tty)
}

/// [`run`] with explicit pipe / history / output (tests).
pub fn run_with(cmd: &Command, pipe: Option<&str>, history: Option<PathBuf>, out: &mut dyn Write, tty: bool) -> i32 {
    let mut r = Render { out, json: cmd.json, tty, partial: Vec::new(), last: None, progress_shown: false };
    match &cmd.op {
        Op::Help => {
            let _ = writeln!(r.out, "{HELP}");
            exit::OK
        }
        Op::Ls { path, all } => ls(&mut r, path, *all),
        Op::Copy { .. } | Op::Move { .. } | Op::Trash { .. } => {
            let spec = match spec_for(cmd) {
                Ok(s) => s,
                Err((code, msg)) => {
                    eprintln!("soyuz: {msg}");
                    return code;
                }
            };
            if let Some(code) =
                via_window(&mut r, pipe, &Request::Submit { spec: Box::new(spec.clone()), wait: !cmd.queue })
            {
                return code;
            }
            if cmd.queue {
                eprintln!("soyuz: no Soyuz window is running; running the job here instead of queueing it");
            }
            headless_job(&mut r, spec, history)
        }
        Op::Open(path) | Op::Reveal(path) => {
            if !soyuz_fs::exists_nofollow(path) {
                eprintln!("soyuz: {}: no such file or folder", path.display());
                return exit::NOT_FOUND;
            }
            let reveal = matches!(cmd.op, Op::Reveal(_));
            let req =
                if reveal { Request::Reveal { path: path.clone() } } else { Request::Open { path: path.clone() } };
            if let Some(code) = via_window(&mut r, pipe, &req) {
                return code;
            }
            headless_open(path, reveal)
        }
        Op::Jobs => {
            if let Some(code) = via_window(&mut r, pipe, &Request::Jobs) {
                return code;
            }
            let engine = JobEngine::new(
                EngineConfig { history_path: history, ..EngineConfig::default() },
                Arc::new(soyuz_fs::RealProbe),
                Arc::new(|| {}),
            );
            let v = proto::jobs_json(&engine);
            engine.shutdown(Duration::from_secs(1));
            r.event(&v);
            exit::OK
        }
    }
}

fn conflict(c: Option<Conflict>) -> Option<ConflictResolution> {
    match c? {
        Conflict::Ask => None,
        Conflict::Skip => Some(ConflictResolution::Skip),
        Conflict::Overwrite => Some(ConflictResolution::Overwrite),
        Conflict::Rename => Some(ConflictResolution::KeepBoth),
    }
}

/// The job for a cp / mv / rm command (checks that the sources exist).
pub fn spec_for(cmd: &Command) -> Result<JobSpec, (i32, String)> {
    let missing = |paths: &[PathBuf]| -> Result<(), (i32, String)> {
        match paths.iter().find(|p| !soyuz_fs::exists_nofollow(p)) {
            Some(p) => Err((exit::NOT_FOUND, format!("{}: no such file or folder", p.display()))),
            None => Ok(()),
        }
    };
    let kind = match &cmd.op {
        Op::Copy { sources, dest } | Op::Move { sources, dest } => {
            missing(sources)?;
            let is_dir = soyuz_fs::stat(dest, true).is_ok_and(|e| e.is_dir_like());
            let (dest_dir, rename_to) = if is_dir {
                (dest.clone(), None)
            } else if sources.len() == 1 {
                let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
                let Some(parent) = parent.filter(|p| soyuz_fs::stat(p, true).is_ok_and(|e| e.is_dir_like())) else {
                    return Err((
                        exit::NOT_FOUND,
                        format!("{}: the destination folder does not exist", dest.display()),
                    ));
                };
                let name = dest.file_name().map(|n| n.to_string_lossy().into_owned());
                (parent.to_path_buf(), name)
            } else {
                return Err((exit::USAGE, format!("{}: the destination must be an existing folder", dest.display())));
            };
            let sources = sources.clone();
            if matches!(cmd.op, Op::Copy { .. }) {
                JobKind::Copy { sources, dest_dir, rename_to }
            } else {
                JobKind::Move { sources, dest_dir, rename_to }
            }
        }
        Op::Trash { paths, permanent } => {
            missing(paths)?;
            JobKind::Delete {
                paths: paths.clone(),
                mode: if *permanent { DeleteMode::Permanent } else { DeleteMode::Trash },
            }
        }
        _ => return Err((exit::USAGE, "not a job command".into())),
    };
    let mut spec = JobSpec::new(kind);
    spec.options.conflict = conflict(cmd.conflict);
    if cmd.queue {
        spec.options.queue = QueueMode::Serial;
    }
    Ok(spec)
}

/// Send `req` to the window; `None` when no window serves the pipe.
fn via_window(r: &mut Render<'_>, pipe: Option<&str>, req: &Request) -> Option<i32> {
    let name = pipe?;
    let stream = match soyuz_mcp::pipe::connect(name) {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(e) => {
            log::info!("soyuz: cannot reach the Soyuz window ({e}); running here");
            return None;
        }
    };
    let Ok(mut w) = dup(&stream) else { return None };
    let line = format!("{}{}\n", proto::hello(), serde_json::to_string(req).ok()?);
    if w.write_all(line.as_bytes()).and_then(|_| w.flush()).is_err() {
        return None;
    }
    let reader = BufReader::new(stream);
    let mut got_any = false;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        got_any = true;
        r.event(&v);
    }
    if !got_any {
        eprintln!("soyuz: the Soyuz window closed the connection");
        return Some(exit::UNAVAILABLE);
    }
    Some(r.code())
}

#[cfg(windows)]
fn dup(s: &soyuz_mcp::pipe::ClientStream) -> std::io::Result<soyuz_mcp::pipe::ClientStream> {
    Ok(s.clone())
}

#[cfg(not(windows))]
fn dup(s: &soyuz_mcp::pipe::ClientStream) -> std::io::Result<soyuz_mcp::pipe::ClientStream> {
    s.try_clone()
}

fn headless_job(r: &mut Render<'_>, spec: JobSpec, history: Option<PathBuf>) -> i32 {
    let engine = JobEngine::new(
        EngineConfig { history_path: history, ..EngineConfig::default() },
        Arc::new(soyuz_fs::RealProbe),
        Arc::new(|| {}),
    );
    let id = engine.submit(spec);
    let snap = engine.snapshot(id).map(|s| soyuz_mcp::jobs::snapshot_json(&s));
    r.event(&json!({"event": "submitted", "job": snap.unwrap_or(json!({"job_id": id.0}))}));
    proto::follow(&engine, id, r, true);
    engine.shutdown(Duration::from_secs(5));
    r.code()
}

/// The GUI executable (`soyuz-cli` hands folders to its `soyuz` sibling).
fn gui_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if exe.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("soyuz-cli")) {
        let sib = exe.with_file_name(if cfg!(windows) { "soyuz.exe" } else { "soyuz" });
        return sib.exists().then_some(sib);
    }
    Some(exe)
}

fn headless_open(path: &Path, reveal: bool) -> i32 {
    let is_dir = soyuz_fs::stat(path, true).is_ok_and(|e| e.is_dir_like());
    let res = if !reveal && !is_dir {
        soyuz_fs::launch::open_default(path).map_err(|e| e.to_string())
    } else {
        // A new window opens folders, and files with the file selected.
        match gui_exe() {
            Some(exe) => std::process::Command::new(exe).arg(path).spawn().map(drop).map_err(|e| e.to_string()),
            None => Err("cannot find the Soyuz executable".into()),
        }
    };
    match res {
        Ok(()) => exit::OK,
        Err(e) => {
            eprintln!("soyuz: {}: {e}", path.display());
            exit::FAILED
        }
    }
}

fn ls(r: &mut Render<'_>, path: &Path, all: bool) -> i32 {
    let opts = soyuz_fs::ListOptions { include_hidden: all, ..Default::default() };
    let mut entries = match soyuz_fs::read_dir(path, &opts) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("soyuz: {e}");
            return if e.kind == soyuz_core::FsErrorKind::NotFound { exit::NOT_FOUND } else { exit::FAILED };
        }
    };
    entries.sort_by(|a, b| b.is_dir_like().cmp(&a.is_dir_like()).then(soyuz_core::natural_cmp(&a.name, &b.name, true)));
    for e in &entries {
        if r.json {
            let mut m = serde_json::Map::new();
            m.insert("name".into(), json!(e.name));
            m.insert("path".into(), json!(soyuz_fs::strip_verbatim(&e.path)));
            m.insert("is_dir".into(), json!(e.is_dir_like()));
            m.insert("size".into(), json!(e.size));
            m.insert("modified".into(), json!(e.modified.map(|t| soyuz_core::time::serde_rfc3339::to_string(&t))));
            let _ = writeln!(r.out, "{}", Value::Object(m));
        } else {
            let size = if e.is_dir_like() { "<dir>".to_string() } else { format_size(e.size, SizeStyle::Binary) };
            let when =
                e.modified.map(|t| soyuz_core::format_time(t, &soyuz_core::DateFormat::default())).unwrap_or_default();
            let name =
                if e.is_dir_like() { format!("{}{}", e.name, std::path::MAIN_SEPARATOR) } else { e.name.clone() };
            let _ = writeln!(r.out, "{size:>11}  {when:<18}  {name}");
        }
    }
    exit::OK
}

/// Renders protocol events (human text or JSON lines) and remembers the outcome.
/// Also a `Write` sink for [`proto::follow`] in headless mode.
pub(crate) struct Render<'a> {
    out: &'a mut dyn Write,
    json: bool,
    tty: bool,
    partial: Vec<u8>,
    last: Option<Value>,
    progress_shown: bool,
}

impl Render<'_> {
    fn clear_progress(&mut self) {
        if self.progress_shown {
            let _ = write!(self.out, "\r\x1b[2K");
            self.progress_shown = false;
        }
    }

    fn event(&mut self, v: &Value) {
        self.last = Some(v.clone());
        if self.json {
            let _ = writeln!(self.out, "{v}");
            let _ = self.out.flush();
            return;
        }
        let job = v.get("job").cloned().unwrap_or(Value::Null);
        let label = job.get("label").and_then(Value::as_str).unwrap_or("job").to_string();
        match v.get("event").and_then(Value::as_str).unwrap_or("") {
            "submitted" => {
                let id = job.get("job_id").and_then(Value::as_u64).unwrap_or(0);
                let _ = writeln!(self.out, "{label} (job {id})");
            }
            "progress" => {
                let state = job.get("state").and_then(Value::as_str).unwrap_or("");
                if state == "waiting_for_user" || job.get("waiting_for_user").and_then(Value::as_bool) == Some(true) {
                    self.clear_progress();
                    let _ = writeln!(self.out, "Waiting for your answer in the Soyuz window…");
                } else if self.tty {
                    let line = progress_line(&job);
                    let _ = write!(self.out, "\r\x1b[2K{line}");
                    self.progress_shown = true;
                }
            }
            "finished" => {
                self.clear_progress();
                let _ = writeln!(self.out, "{}", finished_line(&job));
                if let Some(errs) = job.pointer("/summary/errors").and_then(Value::as_array) {
                    for e in errs {
                        let _ = writeln!(
                            self.out,
                            "  {}: {}",
                            e.get("path").and_then(Value::as_str).unwrap_or(""),
                            e.get("error").and_then(Value::as_str).unwrap_or("")
                        );
                    }
                }
            }
            "jobs" => {
                let jobs = v.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default();
                if jobs.is_empty() {
                    let _ = writeln!(self.out, "No running jobs.");
                }
                for j in &jobs {
                    let _ = writeln!(
                        self.out,
                        "#{:<5} {:<16} {}",
                        j.get("job_id").and_then(Value::as_u64).unwrap_or(0),
                        j.get("state").and_then(Value::as_str).unwrap_or(""),
                        progress_line(j)
                    );
                }
                let hist = v.pointer("/history/entries").and_then(Value::as_array).cloned().unwrap_or_default();
                if !hist.is_empty() {
                    let _ = writeln!(self.out, "\nRecent:");
                }
                for h in hist {
                    let when = h.get("when").and_then(Value::as_str).unwrap_or("");
                    let label = h.get("label").and_then(Value::as_str).unwrap_or("");
                    let kind = h.get("kind").and_then(Value::as_str).unwrap_or("");
                    let undone = if h.get("undone").and_then(Value::as_bool) == Some(true) { " (undone)" } else { "" };
                    let what = if label.is_empty() { kind } else { label };
                    let _ = writeln!(self.out, "  {:<20} {what}{undone}", when.get(..19).unwrap_or(when));
                }
            }
            "ok" => {}
            "error" => {
                self.clear_progress();
                eprintln!("soyuz: {}", v.get("message").and_then(Value::as_str).unwrap_or("error"));
            }
            _ => {}
        }
        let _ = self.out.flush();
    }

    /// Exit code from the last event.
    fn code(&self) -> i32 {
        let Some(v) = &self.last else { return exit::UNAVAILABLE };
        match v.get("event").and_then(Value::as_str) {
            Some("error") => exit::FAILED,
            Some("finished") => match v.pointer("/job/state").and_then(Value::as_str) {
                Some("done") => {
                    if v.pointer("/job/summary/items_failed").and_then(Value::as_u64).unwrap_or(0) > 0 {
                        exit::FAILED
                    } else {
                        exit::OK
                    }
                }
                Some("cancelled") => exit::CANCELLED,
                _ => exit::FAILED,
            },
            _ => exit::OK,
        }
    }
}

impl Write for Render<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.partial.extend_from_slice(buf);
        while let Some(i) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                self.event(&v);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

fn progress_line(job: &Value) -> String {
    let label = job.get("label").and_then(Value::as_str).unwrap_or("");
    let n = |k: &str| job.get(k).and_then(Value::as_u64).unwrap_or(0);
    let (done, total) =
        if n("bytes_total") > 0 { (n("bytes_done"), n("bytes_total")) } else { (n("items_done"), n("items_total")) };
    let pct = match (done * 100).checked_div(total) {
        Some(p) => format!("{:>3}%", p.min(100)),
        None => " …".into(),
    };
    let amount = if n("bytes_total") > 0 {
        format!("{} / {}", format_size(done, SizeStyle::Binary), format_size(total, SizeStyle::Binary))
    } else if total > 0 {
        format!("{done} / {total} items")
    } else {
        String::new()
    };
    format!("[{pct}] {label}  {amount}")
}

fn finished_line(job: &Value) -> String {
    let label = job.get("label").and_then(Value::as_str).unwrap_or("job");
    match job.get("state").and_then(Value::as_str) {
        Some("done") => {
            let n = |k: &str| job.pointer(&format!("/summary/{k}")).and_then(Value::as_u64).unwrap_or(0);
            let mut s = format!("Done: {label} — {} item(s)", n("items_ok"));
            if n("bytes") > 0 {
                s.push_str(&format!(", {}", format_size(n("bytes"), SizeStyle::Binary)));
            }
            if n("items_skipped") > 0 {
                s.push_str(&format!(", {} skipped", n("items_skipped")));
            }
            if n("items_failed") > 0 {
                s.push_str(&format!(", {} failed", n("items_failed")));
            }
            s
        }
        Some("cancelled") => format!("Cancelled: {label}"),
        _ => format!("Failed: {label}: {}", job.get("error").and_then(Value::as_str).unwrap_or("unknown error")),
    }
}
