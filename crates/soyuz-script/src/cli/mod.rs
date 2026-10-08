//! The CLI companion: `soyuz cp|mv|rm|trash|ls|open|reveal|jobs …`.
//!
//! With a Soyuz window running (and `[cli] server_enabled`), job commands are sent to
//! it over the local named pipe `\\.\pipe\soyuz-cli-<user>` ([`proto`]): they show in
//! the activity panel with progress, conflict prompts are answered in the window, and
//! the command waits and streams progress (or returns at once with `--queue`).
//! Without a window, the command runs on a private headless engine (same undo history)
//! and conflicts follow `--conflict` (default: skip and report).

mod client;
pub mod proto;

use std::path::{Path, PathBuf};

pub use client::run;

/// Exit codes.
pub mod exit {
    pub const OK: i32 = 0;
    /// The job failed or some items failed.
    pub const FAILED: i32 = 1;
    pub const USAGE: i32 = 2;
    pub const CANCELLED: i32 = 3;
    /// No engine / window could take the request.
    pub const UNAVAILABLE: i32 = 4;
    /// A source path does not exist.
    pub const NOT_FOUND: i32 = 5;
}

pub const SUBCOMMANDS: &[&str] = &["cp", "mv", "rm", "trash", "ls", "open", "reveal", "jobs"];

pub const HELP: &str = "\
Usage: soyuz <command> [options] [paths]

File commands (run as jobs: progress, conflict handling, undo in Soyuz):
  cp <src>... <dest>      Copy into the folder <dest> (one source: <dest> may be a new name)
  mv <src>... <dest>      Move into the folder <dest> (one source: <dest> may be a new name)
  trash <path>...         Move to the trash (undoable)
  rm <path>...            Same as trash; with --permanent deletes for good
Other commands:
  ls [path]               List a folder (-a: include hidden items)
  open <path>             Open a folder in Soyuz, or a file with its default app
  reveal <path>           Show the item selected in its folder
  jobs                    Running jobs and recent history

Options:
  --queue                 Hand the job to the Soyuz window and return at once (runs after
                          the jobs already queued)
  --headless              Never use the running window; run on a private engine
  --conflict <mode>       Name conflicts: ask (in the window), skip, overwrite, rename
  --permanent             rm: delete permanently instead of moving to the trash
  --json                  Machine-readable output (one JSON object per line)
  -a, --all               ls: include hidden items
  -h, --help              Print this help

When a Soyuz window is open, jobs appear in its activity panel and wait there for your
answers; otherwise conflicts are skipped unless --conflict says otherwise.
Exit codes: 0 ok, 1 failed / some items failed, 2 usage, 3 cancelled, 4 unavailable,
5 path not found. (`soyuz -- <path>` opens a folder literally named like a command.)
On Windows, `soyuz-cli.exe` is the console twin of these commands.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Ask,
    Skip,
    Overwrite,
    Rename,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Copy { sources: Vec<PathBuf>, dest: PathBuf },
    Move { sources: Vec<PathBuf>, dest: PathBuf },
    Trash { paths: Vec<PathBuf>, permanent: bool },
    Ls { path: PathBuf, all: bool },
    Open(PathBuf),
    Reveal(PathBuf),
    Jobs,
    Help,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub op: Op,
    pub queue: bool,
    pub headless: bool,
    pub json: bool,
    pub conflict: Option<Conflict>,
}

/// Parse a CLI command line (without the program name). `Ok(None)`: not a CLI
/// command (the GUI handles the arguments). `cwd` resolves relative paths.
pub fn parse(args: &[String], cwd: &Path) -> Result<Option<Command>, String> {
    let Some(sub) = args.first() else { return Ok(None) };
    if !SUBCOMMANDS.contains(&sub.as_str()) {
        return Ok(None);
    }
    let mut cmd = Command { op: Op::Jobs, queue: false, headless: false, json: false, conflict: None };
    let mut paths: Vec<PathBuf> = Vec::new();
    let (mut permanent, mut all, mut help) = (false, false, false);
    let mut it = args[1..].iter();
    let mut literal = false;
    while let Some(a) = it.next() {
        if literal || !a.starts_with('-') || a == "-" {
            paths.push(absolute(cwd, a));
            continue;
        }
        match a.as_str() {
            "--" => literal = true,
            "--queue" => cmd.queue = true,
            "--headless" => cmd.headless = true,
            "--json" => cmd.json = true,
            "--permanent" => permanent = true,
            "-a" | "--all" => all = true,
            "-h" | "--help" => help = true,
            "--conflict" => {
                let v = it.next().ok_or("--conflict needs a value (ask, skip, overwrite, rename)")?;
                cmd.conflict = Some(conflict(v)?);
            }
            other => match other.strip_prefix("--conflict=") {
                Some(v) => cmd.conflict = Some(conflict(v)?),
                None => return Err(format!("unknown option '{other}' for `soyuz {sub}`")),
            },
        }
    }
    if help {
        cmd.op = Op::Help;
        return Ok(Some(cmd));
    }
    let one = |paths: &mut Vec<PathBuf>, what: &str| -> Result<PathBuf, String> {
        match paths.len() {
            1 => Ok(paths.remove(0)),
            0 => Err(format!("`soyuz {sub}` needs {what}")),
            _ => Err(format!("`soyuz {sub}` takes one {what}")),
        }
    };
    cmd.op = match sub.as_str() {
        "cp" | "mv" => {
            if paths.len() < 2 {
                return Err(format!("`soyuz {sub}` needs at least one source and a destination"));
            }
            let dest = paths.pop().unwrap_or_default();
            if sub == "cp" { Op::Copy { sources: paths, dest } } else { Op::Move { sources: paths, dest } }
        }
        "rm" | "trash" => {
            if paths.is_empty() {
                return Err(format!("`soyuz {sub}` needs at least one path"));
            }
            if permanent && sub == "trash" {
                return Err("`soyuz trash` always uses the trash; use `soyuz rm --permanent`".into());
            }
            Op::Trash { paths, permanent }
        }
        "ls" => {
            let path = if paths.is_empty() { cwd.to_path_buf() } else { one(&mut paths, "folder")? };
            Op::Ls { path, all }
        }
        "open" => Op::Open(one(&mut paths, "path")?),
        "reveal" => Op::Reveal(one(&mut paths, "path")?),
        _ => {
            if !paths.is_empty() {
                return Err("`soyuz jobs` takes no paths".into());
            }
            Op::Jobs
        }
    };
    if permanent && !matches!(cmd.op, Op::Trash { .. }) {
        return Err("--permanent only applies to `soyuz rm`".into());
    }
    if cmd.conflict.is_some() && !matches!(cmd.op, Op::Copy { .. } | Op::Move { .. }) {
        return Err("--conflict only applies to `soyuz cp` and `soyuz mv`".into());
    }
    Ok(Some(cmd))
}

fn conflict(v: &str) -> Result<Conflict, String> {
    Ok(match v.to_ascii_lowercase().as_str() {
        "ask" => Conflict::Ask,
        "skip" => Conflict::Skip,
        "overwrite" | "replace" => Conflict::Overwrite,
        "rename" | "keep-both" | "keep_both" => Conflict::Rename,
        other => return Err(format!("unknown conflict mode '{other}' (ask, skip, overwrite, rename)")),
    })
}

fn absolute(cwd: &Path, p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    let joined = if path.is_absolute() { path } else { cwd.join(path) };
    normalize(&joined)
}

/// Lexically resolve `.` and `..` (no disk access).
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests;
