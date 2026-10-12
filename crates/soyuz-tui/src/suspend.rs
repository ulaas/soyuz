//! Suspend: leave the terminal UI to run a shell, one command line or a text editor in the
//! real terminal, then come back (`FrontendCmd::Suspend`, `[tui] shell_mode =
//! "suspend"`, "Edit config file", "Edit in $EDITOR").
//!
//! The frame loop (`app::run_loop`) runs [`run`] between two frames for the request
//! `App::request_suspend` stored. The full sequence (W1-suspend): flush the config save,
//! park the input reader ([`InputGate`]), let Ctrl+C reach the child only, `TermIo::suspend`,
//! a banner, [`App::child_runner`](crate::app::App) on the [`ChildSpec`], "press Enter"
//! for a command, `TermIo::resume`, re-apply the console's UTF-8 code pages, drain stale
//! input, repaint; an editor of `EditPurpose::Config` reloads the config, one of
//! `EditPurpose::Scratch(token)` is handed back through `App::take_finished_edit`.
//!
//! Skeleton stub (Phase T5 + T6, step S0): W1-suspend fills it. [`run`] only says "not
//! available"; [`default_runner`] is real.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::Backend;
use soyuz_appcore::models::frontend::SuspendRequest;
use soyuz_core::config::{Config, TuiSection};

use crate::app::{App, TermIo};

/// A child program to run in the real terminal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// Wait for Enter after it exits (a command line's output stays readable).
    pub wait_key: bool,
}

/// Runs a [`ChildSpec`] with the terminal's stdio and waits; returns its exit code
/// (`None`: ended by a signal). `App::child_runner`; tests replace it.
pub type ChildRunner = fn(&ChildSpec) -> io::Result<Option<i32>>;

/// The production [`ChildRunner`]: `std::process::Command` (CreateProcessW / fork-exec,
/// never a shell verb) with inherited stdio.
pub fn default_runner(spec: &ChildSpec) -> io::Result<Option<i32>> {
    let mut cmd = std::process::Command::new(&spec.program);
    cmd.args(&spec.args).envs(spec.env.iter().map(|(k, v)| (k, v)));
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    Ok(cmd.status()?.code())
}

/// What the editor and shell resolution look at (tests inject their own).
pub struct Env {
    /// `$VISUAL`.
    pub visual: Option<OsString>,
    /// `$EDITOR`.
    pub editor: Option<OsString>,
    /// The `PATH` directories, in order.
    pub path_dirs: Vec<PathBuf>,
    /// Whether a file exists (the `PATH` probes).
    pub is_file: Box<dyn Fn(&Path) -> bool>,
    pub is_windows: bool,
}

impl Env {
    /// This process's environment.
    pub fn current() -> Env {
        Env {
            visual: std::env::var_os("VISUAL"),
            editor: std::env::var_os("EDITOR"),
            path_dirs: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
            is_file: Box::new(Path::is_file),
            is_windows: cfg!(windows),
        }
    }
}

/// The editor for `path`: `[tui] editor`, `$VISUAL`, `$EDITOR`, then the first of nvim,
/// vim, nano, micro, hx, vi (and edit on Windows) on `PATH`. `Err`: the names tried.
/// Stub: nothing found.
pub fn editor_spec(_cfg: &TuiSection, _env: &Env, _path: &Path) -> Result<ChildSpec, Vec<String>> {
    Err(Vec::new())
}

/// The shell for `dir` (`[terminal] shell` / `args`, else the platform default), running
/// `line` when given (`sh -c`, `cmd /c`, `pwsh -NoProfile -Command`; `wait_key` set).
/// Stub: an error.
pub fn shell_spec(_cfg: &Config, _env: &Env, _dir: &Path, _line: Option<&str>) -> Result<ChildSpec, String> {
    Err(String::new())
}

/// Split a configured command line (`code --wait`) into words, with quotes. Stub:
/// whitespace only.
pub fn split_command(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// An editor run for `EditPurpose::Scratch(token)` that finished: its owner reads `path`
/// back (`App::take_finished_edit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedEdit {
    pub token: u64,
    pub path: PathBuf,
    /// The editor's exit code (`None`: no code, e.g. ended by a signal).
    pub exit: Option<i32>,
}

/// Parks the input reader thread while a child owns the terminal (`App::input_gate`).
/// Stub: no state; `pause` / `resume` do nothing.
#[derive(Debug, Default)]
pub struct InputGate {}

impl InputGate {
    /// Stop reading input; returns once the reader is parked.
    pub fn pause(&self) {}

    /// Read input again.
    pub fn resume(&self) {}

    /// The reader is told to stay parked.
    pub fn is_paused(&self) -> bool {
        false
    }
}

/// Carry out `req` between two frames (`app::run_loop`). Stub: "not available".
pub fn run<B: Backend>(
    app: &mut App,
    _terminal: &mut Terminal<B>,
    _io: &mut dyn TermIo,
    _req: SuspendRequest,
) -> io::Result<()>
where
    B::Error: Send + Sync + 'static,
{
    app.core.tui_unavailable();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runner_reports_the_exit_code() {
        // The test binary itself, asked for a test that does not exist: exits 0.
        let exe = std::env::current_exe().unwrap();
        let spec = ChildSpec {
            program: exe,
            args: vec!["--exact".into(), "no-such-test".into(), "--quiet".into()],
            ..ChildSpec::default()
        };
        assert_eq!(default_runner(&spec).unwrap(), Some(0));
        let missing = ChildSpec { program: "soyuz-no-such-program-here".into(), ..ChildSpec::default() };
        assert!(default_runner(&missing).is_err());
    }
}
