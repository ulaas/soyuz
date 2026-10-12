//! Suspend: leave the terminal UI to run a shell, one command line or a text editor in the
//! real terminal, then come back (`FrontendCmd::Suspend`, `[tui] shell_mode =
//! "suspend"`, "Edit config file", "Edit in $EDITOR").
//!
//! The frame loop (`app::run_loop`) runs [`run`] between two frames for the request
//! `App::request_suspend` stored. The sequence:
//! 1. resolve the child ([`editor_spec`] / [`shell_spec`]); nothing found → a toast (and
//!    for the config file the built-in settings editor instead), no suspend;
//! 2. write a pending `config.toml` save (`models::settings::flush`), so an editor sees
//!    the current file;
//! 3. park the input reader thread ([`InputGate::pause`] waits for its acknowledgement):
//!    a reader blocked in `read()` would steal the child's keystrokes;
//! 4. Ctrl+C reaches the child only ([`crate::signals::set_suspended`]: unix drops
//!    `SIGINT`, Windows' console handler swallows `CTRL_C_EVENT`);
//! 5. `TermIo::suspend` (cooked mode, main screen, no mouse / paste / keyboard flags; the
//!    pushed window title stays), a banner for a shell or a command line;
//! 6. [`App::child_runner`](crate::app::App) runs the [`ChildSpec`] (inherited stdio; a
//!    command line waits for Enter afterwards so its output stays readable);
//! 7. `TermIo::resume`, the console's UTF-8 code pages again (a child can `chcp`),
//!    Ctrl+C back to normal, the reader resumes and drops the input that piled up while
//!    the child had the terminal (resizes are kept);
//! 8. repaint everything (`terminal.clear`, `images::invalidate`);
//! 9. an editor of `EditPurpose::Config` reloads the config (`config_watch::reload_now`:
//!    "Configuration reloaded" or the parse error), one of `EditPurpose::Scratch(token)`
//!    is handed back through `App::take_finished_edit`; a non-zero exit of a command or an
//!    editor and a failed start are toasts.
//!
//! Never a shell verb: children start with `std::process::Command` (CreateProcessW /
//! fork-exec). There is no Notepad fallback (owner decision): only console editors.

use std::ffi::OsString;
use std::io::{self, BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use ratatui::Terminal;
use ratatui::backend::Backend;
use soyuz_appcore::host::Host as _;
use soyuz_appcore::models::frontend::{EditPurpose, SuspendRequest};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::config::{Config, TuiSection};
use soyuz_term::shell::ShellKind;

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

/// Runs a [`ChildSpec`] with the terminal's stdio and waits (including the Enter of
/// `wait_key`); returns its exit code (`None`: ended by a signal). `App::child_runner`;
/// tests replace it.
pub type ChildRunner = fn(&ChildSpec) -> io::Result<Option<i32>>;

/// The production [`ChildRunner`]: `std::process::Command` (CreateProcessW / fork-exec,
/// never a shell verb) with inherited stdio. With `wait_key` it prints
/// `tui-suspend-press-key` and reads one line afterwards.
///
/// Windows: a `cmd /d /s /c <line>` is passed with the line as one raw argument in quotes
/// (`/s` strips exactly those), because cmd.exe does not understand the `\"` escapes of
/// the usual argument quoting.
pub fn default_runner(spec: &ChildSpec) -> io::Result<Option<i32>> {
    let mut cmd = std::process::Command::new(&spec.program);
    add_args(&mut cmd, spec);
    cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    let code = cmd.status()?.code();
    if spec.wait_key {
        let mut out = io::stdout();
        let _ = write!(out, "\r\n{} ", tr!("tui-suspend-press-key"));
        let _ = out.flush();
        let mut line = String::new();
        // Ctrl+C here (Windows) or a closed stdin ends the wait too.
        let _ = io::stdin().lock().read_line(&mut line);
    }
    Ok(code)
}

#[cfg(windows)]
fn add_args(cmd: &mut std::process::Command, spec: &ChildSpec) {
    use std::os::windows::process::CommandExt as _;
    let is_cmd = ShellKind::detect(&spec.program.to_string_lossy()) == ShellKind::Cmd;
    let c = spec.args.iter().position(|a| a.eq_ignore_ascii_case("/c"));
    match c {
        Some(i) if is_cmd && i + 2 == spec.args.len() => {
            cmd.args(&spec.args[..=i]);
            let mut raw = OsString::from("\"");
            raw.push(&spec.args[i + 1]);
            raw.push("\"");
            cmd.raw_arg(raw);
        }
        _ => {
            cmd.args(&spec.args);
        }
    }
}

#[cfg(not(windows))]
fn add_args(cmd: &mut std::process::Command, spec: &ChildSpec) {
    cmd.args(&spec.args);
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

    /// Where `name` (a bare name, or a path) is: a path is checked as is, a bare name in
    /// every `PATH` directory; on Windows a name without an extension is looked up as
    /// `.exe`, `.com`, `.cmd` and `.bat` (an extensionless file cannot be started there).
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        let name = name.trim().trim_matches('"');
        if name.is_empty() {
            return None;
        }
        let path = Path::new(name);
        let has_ext = path.extension().is_some();
        let probe = |base: &Path| -> Option<PathBuf> {
            if !self.is_windows || has_ext {
                if (self.is_file)(base) {
                    return Some(base.to_path_buf());
                }
                if self.is_windows {
                    return None;
                }
            }
            if self.is_windows {
                for ext in ["exe", "com", "cmd", "bat"] {
                    let mut s = base.as_os_str().to_owned();
                    s.push(".");
                    s.push(ext);
                    let p = PathBuf::from(s);
                    if (self.is_file)(&p) {
                        return Some(p);
                    }
                }
            }
            None
        };
        let has_sep = name.contains('/') || (self.is_windows && (name.contains('\\') || name.contains(':')));
        if has_sep || path.is_absolute() {
            return probe(path);
        }
        self.path_dirs.iter().filter(|d| !d.as_os_str().is_empty()).find_map(|d| probe(&self.join(d, name)))
    }

    /// `dir` + `name` with this environment's separator (tests run either platform's
    /// environment on both).
    fn join(&self, dir: &Path, name: &str) -> PathBuf {
        let sep = if self.is_windows { '\\' } else { '/' };
        let mut s = dir.as_os_str().to_owned();
        if !s.to_string_lossy().ends_with(['/', sep]) {
            s.push(sep.to_string());
        }
        s.push(name);
        PathBuf::from(s)
    }
}

/// Where [`run`] gets its [`Env`] ([`Env::current`]; tests install another).
static ENV_SOURCE: Mutex<fn() -> Env> = Mutex::new(Env::current as fn() -> Env);

/// Replace where [`run`] reads `$VISUAL` / `$EDITOR` / `PATH` (tests: an editor-less
/// environment without touching the process environment).
pub fn set_env_source(f: fn() -> Env) {
    *ENV_SOURCE.lock() = f;
}

/// The console editors probed on `PATH`, in order (`edit` is Windows only).
pub const EDITOR_PROBES: [&str; 6] = ["nvim", "vim", "nano", "micro", "hx", "vi"];

/// The editor for `path`: `[tui] editor`, `$VISUAL`, `$EDITOR` (each a command line such
/// as `code --wait`; one that cannot be found is skipped), then the first of nvim, vim,
/// nano, micro, hx, vi (and on Windows edit) on `PATH`. The path is the last argument; the
/// editor starts in the file's folder. `Err`: the names tried, in order.
pub fn editor_spec(cfg: &TuiSection, env: &Env, path: &Path) -> Result<ChildSpec, Vec<String>> {
    let mut tried: Vec<String> = Vec::new();
    let spec = |program: PathBuf, rest: &[String]| ChildSpec {
        program,
        args: rest.iter().map(OsString::from).chain(std::iter::once(path.as_os_str().to_owned())).collect(),
        env: Vec::new(),
        cwd: path.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf),
        wait_key: false,
    };
    let configured = [Some(OsString::from(&cfg.editor)), env.visual.clone(), env.editor.clone()];
    for line in configured.into_iter().flatten() {
        let words = split_command(&line.to_string_lossy());
        let Some((program, rest)) = words.split_first() else { continue };
        match env.find(program) {
            Some(p) => return Ok(spec(p, rest)),
            None => push_unique(&mut tried, program),
        }
    }
    let windows_only: &[&str] = if env.is_windows { &["edit"] } else { &[] };
    for name in EDITOR_PROBES.iter().chain(windows_only) {
        match env.find(name) {
            Some(p) => return Ok(spec(p, &[])),
            None => push_unique(&mut tried, name),
        }
    }
    Err(tried)
}

fn push_unique(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|n| n == name) {
        list.push(name.to_string());
    }
}

/// The shell for `dir` (`[terminal] shell` / `args`, else the platform default
/// `soyuz_term::shell::default_shell`), starting in `dir`. With `line` it runs that line
/// instead of an interactive session (`sh -c`, `cmd /d /s /c`, `pwsh -NoProfile
/// -Command`, by the shell's kind; `[terminal] args` are not used) and waits for Enter
/// afterwards. `Err`: the text of the "no shell" toast.
pub fn shell_spec(cfg: &Config, env: &Env, dir: &Path, line: Option<&str>) -> Result<ChildSpec, String> {
    let configured = cfg.terminal.shell.trim().trim_matches('"');
    let program = if configured.is_empty() {
        PathBuf::from(soyuz_term::shell::default_shell().ok_or_else(|| tr!("tui-suspend-no-shell").to_string())?)
    } else {
        // Not found: start it anyway, the spawn error names it.
        env.find(configured).unwrap_or_else(|| PathBuf::from(configured))
    };
    let args: Vec<OsString> = match line {
        None => cfg.terminal.args.iter().map(OsString::from).collect(),
        Some(line) => {
            let pre: &[&str] = match ShellKind::detect(&program.to_string_lossy()) {
                ShellKind::PowerShell => &["-NoProfile", "-Command"],
                ShellKind::Cmd => &["/d", "/s", "/c"],
                ShellKind::Bash | ShellKind::Zsh | ShellKind::Fish | ShellKind::Posix => &["-c"],
            };
            pre.iter().map(OsString::from).chain(std::iter::once(OsString::from(line))).collect()
        }
    };
    Ok(ChildSpec { program, args, env: Vec::new(), cwd: Some(dir.to_path_buf()), wait_key: line.is_some() })
}

/// Split a configured command line (`code --wait`) into words: whitespace separates,
/// `"…"` and `'…'` group (quotes can join a word: `a"b c"` is `ab c`, `""` is an empty
/// word). Backslashes are literal (Windows paths); no other shell syntax.
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            None => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}

/// An editor run for `EditPurpose::Scratch` that finished: its owner reads `path`
/// back (`App::take_finished_edit`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishedEdit {
    pub token: u64,
    pub path: PathBuf,
    /// The editor's exit code. `None`: no code (ended by a signal), or no editor ran (none
    /// found, or it could not start; a toast said why).
    pub exit: Option<i32>,
}

/// How long [`InputGate::pause`] waits for the reader to park (it polls every 50 ms).
pub const PARK_WAIT: Duration = Duration::from_secs(1);

/// Parks the input reader thread while a child owns the terminal (`App::input_gate`).
///
/// The reader (`app::spawn_reader`) calls [`InputGate::attach_reader`] once and
/// [`InputGate::park`] before each `poll` of 50 ms; [`InputGate::pause`] closes the gate
/// and waits until the reader acknowledges that it is parked (never in the middle of a
/// `read`), [`InputGate::resume`] opens it again.
#[derive(Debug, Default)]
pub struct InputGate {
    state: Mutex<GateState>,
    cv: Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    paused: bool,
    parked: bool,
    readers: usize,
}

impl InputGate {
    /// Stop reading input; returns once the reader is parked (`true`), at once when no
    /// reader is attached (tests, `true`), or `false` after [`PARK_WAIT`] without an
    /// acknowledgement.
    pub fn pause(&self) -> bool {
        let mut s = self.state.lock();
        s.paused = true;
        let deadline = Instant::now() + PARK_WAIT;
        while s.readers > 0 && !s.parked {
            if self.cv.wait_until(&mut s, deadline).timed_out() {
                return s.readers == 0 || s.parked;
            }
        }
        true
    }

    /// Read input again.
    pub fn resume(&self) {
        self.state.lock().paused = false;
        self.cv.notify_all();
    }

    /// The reader is told to stay parked.
    pub fn is_paused(&self) -> bool {
        self.state.lock().paused
    }

    /// The reader acknowledged the pause and waits.
    pub fn is_parked(&self) -> bool {
        self.state.lock().parked
    }

    /// A reader thread uses this gate: [`InputGate::pause`] waits for it from now on.
    pub fn attach_reader(&self) {
        self.state.lock().readers += 1;
    }

    /// The reader thread ended.
    pub fn detach_reader(&self) {
        let mut s = self.state.lock();
        s.readers = s.readers.saturating_sub(1);
        s.parked = false;
        drop(s);
        self.cv.notify_all();
    }

    /// Reader side, before each poll: while paused, acknowledge and wait. Returns whether
    /// it parked (the input that arrived meanwhile belonged to the child: drop it).
    pub fn park(&self) -> bool {
        let mut s = self.state.lock();
        if !s.paused {
            return false;
        }
        s.parked = true;
        self.cv.notify_all();
        while s.paused {
            self.cv.wait(&mut s);
        }
        s.parked = false;
        true
    }
}

/// Carry out `req` between two frames (`app::run_loop`); see the module docs. `Err` only
/// when the terminal cannot be put back into TUI mode (the loop then ends).
pub fn run<B: Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    io: &mut dyn TermIo,
    req: SuspendRequest,
) -> io::Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let env = (*ENV_SOURCE.lock())();
    let spec = match &req {
        SuspendRequest::Editor { path, purpose } => match editor_spec(&app.core.config.tui, &env, path) {
            Ok(spec) => spec,
            Err(tried) => {
                no_editor(app, path, *purpose, &tried);
                return Ok(());
            }
        },
        SuspendRequest::Shell { dir } => match shell_spec(&app.core.config, &env, dir, None) {
            Ok(spec) => spec,
            Err(e) => {
                app.core.ui.toasts.push(Toast::error(e));
                return Ok(());
            }
        },
        SuspendRequest::Command { dir, line } => match shell_spec(&app.core.config, &env, dir, Some(line)) {
            Ok(spec) => spec,
            Err(e) => {
                app.core.ui.toasts.push(Toast::error(e));
                return Ok(());
            }
        },
    };
    drop(env);
    soyuz_appcore::models::settings::flush(&mut app.core);
    let banner = match &req {
        SuspendRequest::Shell { .. } => Some(tr!("tui-suspend-banner").to_string()),
        SuspendRequest::Command { line, .. } => Some(tr!("tui-suspend-running", command = line.as_str())),
        SuspendRequest::Editor { .. } => None,
    };
    let (result, back) = in_child(app, io, &spec, banner.as_deref());
    // Repaint everything: the child drew on the main screen, graphics images are gone.
    terminal.clear().map_err(io::Error::other)?;
    crate::images::invalidate();
    app.host.redraw();
    back?;
    after(app, &req, &spec, result);
    Ok(())
}

/// No editor: say which names were tried; the config file opens in the settings editor
/// instead, a scratch edit is handed back as finished (nothing changed).
fn no_editor(app: &mut App, path: &Path, purpose: EditPurpose, tried: &[String]) {
    app.core.ui.toasts.push(
        Toast::warning(tr!("tui-suspend-no-editor")).body(tr!("tui-suspend-no-editor-tried", names = tried.join(", "))),
    );
    match purpose {
        EditPurpose::Config => crate::input::settings::open(app, None),
        EditPurpose::Scratch(token) => {
            app.finished_edits.push(FinishedEdit { token, path: path.to_path_buf(), exit: None });
        }
    }
}

/// Steps 3–7 of the module docs. Returns the child's result and whether the terminal is
/// back in TUI mode.
fn in_child(
    app: &mut App,
    io: &mut dyn TermIo,
    spec: &ChildSpec,
    banner: Option<&str>,
) -> (io::Result<Option<i32>>, io::Result<()>) {
    let gate = app.input_gate();
    if !gate.pause() {
        log::warn!("tui: the input reader did not park within {PARK_WAIT:?}");
    }
    crate::signals::set_suspended(true);
    let result = match io.suspend() {
        Ok(()) => {
            if let Some(banner) = banner {
                let _ = io.write_raw(format!("\r\n{banner}\r\n\r\n").as_bytes());
            }
            log::info!("tui: suspended for {}", spec.program.display());
            (app.child_runner)(spec)
        }
        Err(e) => Err(e),
    };
    // Even after a failed suspend: whatever was undone is entered again.
    let back = io.resume(&app.core.config.tui);
    soyuz_fs::console::reapply_utf8();
    crate::signals::set_suspended(false);
    gate.resume();
    log::info!("tui: back from {} ({result:?})", spec.program.display());
    (result, back)
}

/// Step 9 of the module docs.
fn after(app: &mut App, req: &SuspendRequest, spec: &ChildSpec, result: io::Result<Option<i32>>) {
    let name = spec.program.file_name().unwrap_or(spec.program.as_os_str()).to_string_lossy().into_owned();
    let code = match result {
        Ok(code) => code,
        Err(e) => {
            app.core
                .ui
                .toasts
                .push(Toast::error(tr!("tui-suspend-failed", program = name.as_str())).body(e.to_string()));
            if let SuspendRequest::Editor { path, purpose: EditPurpose::Scratch(token) } = req {
                app.finished_edits.push(FinishedEdit { token: *token, path: path.clone(), exit: None });
            }
            return;
        }
    };
    match req {
        SuspendRequest::Editor { purpose: EditPurpose::Config, .. } => {
            soyuz_appcore::services::config_watch::reload_now(&mut app.core, &app.host);
            // TODO(W2-settings-editor): rebuild an open settings draft from the reloaded
            // config (no hook in `input::settings` yet; `host.config_changed` fires).
        }
        SuspendRequest::Editor { path, purpose: EditPurpose::Scratch(token) } => {
            app.finished_edits.push(FinishedEdit { token: *token, path: path.clone(), exit: code });
        }
        // An interactive shell's exit code is the last command's: not news.
        SuspendRequest::Shell { .. } => return,
        SuspendRequest::Command { .. } => {}
    }
    if let Some(c) = code.filter(|c| *c != 0) {
        app.core.ui.toasts.push(Toast::warning(tr!("tui-suspend-exit", program = name.as_str(), code = c.to_string())));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

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

    /// An environment with exactly `files` present.
    fn env(windows: bool, visual: Option<&str>, editor: Option<&str>, dirs: &[&str], files: &[&str]) -> Env {
        let files: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        Env {
            visual: visual.map(OsString::from),
            editor: editor.map(OsString::from),
            path_dirs: dirs.iter().map(PathBuf::from).collect(),
            is_file: Box::new(move |p| files.contains(p)),
            is_windows: windows,
        }
    }

    fn tui(editor: &str) -> TuiSection {
        TuiSection { editor: editor.into(), ..TuiSection::default() }
    }

    fn program(r: Result<ChildSpec, Vec<String>>) -> String {
        r.expect("an editor").program.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn editor_order_on_unix() {
        let path = Path::new("/home/me/.config/soyuz/config.toml");
        let all = ["/usr/bin/code", "/usr/bin/hx", "/usr/bin/vim", "/usr/bin/nano", "/usr/bin/vi", "/opt/ed"];
        let e = env(false, Some("hx"), Some("nano -w"), &["/usr/bin"], &all);
        // The config key wins, with its arguments; the path is last; cwd = its folder.
        let s = editor_spec(&tui("code --wait"), &e, path).unwrap();
        assert_eq!(s.program, PathBuf::from("/usr/bin/code"));
        assert_eq!(s.args, vec![OsString::from("--wait"), path.as_os_str().to_owned()]);
        assert_eq!(s.cwd.as_deref(), Some(Path::new("/home/me/.config/soyuz")));
        assert!(!s.wait_key);
        // An absolute configured path.
        assert_eq!(program(editor_spec(&tui("/opt/ed"), &e, path)), "/opt/ed");
        // $VISUAL, then $EDITOR (with arguments).
        assert_eq!(program(editor_spec(&tui(""), &e, path)), "/usr/bin/hx");
        let e = env(false, None, Some("nano -w"), &["/usr/bin"], &all);
        let s = editor_spec(&tui("  "), &e, path).unwrap();
        assert_eq!(s.program, PathBuf::from("/usr/bin/nano"));
        assert_eq!(s.args[0], OsString::from("-w"));
        // A configured editor that is not there is skipped.
        let e = env(false, Some("emacs"), None, &["/usr/bin"], &all);
        assert_eq!(program(editor_spec(&tui("missing"), &e, path)), "/usr/bin/vim");
        // Probes in order: nvim, vim, nano, micro, hx, vi.
        let e = env(false, None, None, &["/bin", "/usr/bin"], &["/usr/bin/vi", "/bin/micro", "/usr/bin/nano"]);
        assert_eq!(program(editor_spec(&tui(""), &e, path)), "/usr/bin/nano");
        // No edit.exe probe off Windows; the error lists the names tried.
        let e = env(false, Some("emacs"), Some("emacs"), &["/usr/bin"], &["/usr/bin/edit"]);
        let tried = editor_spec(&tui("joe"), &e, path).unwrap_err();
        assert_eq!(tried, ["joe", "emacs", "nvim", "vim", "nano", "micro", "hx", "vi"]);
    }

    #[test]
    fn editor_order_on_windows() {
        let path = Path::new(r"C:\Users\me\AppData\Roaming\soyuz\config.toml");
        let dirs = [r"C:\bin", r"C:\tools"];
        // `code` is a .cmd; a bare name never matches an extensionless file.
        let e = env(true, None, None, &dirs, &[r"C:\tools\code.cmd", r"C:\bin\vim", r"C:\tools\nvim.exe"]);
        assert_eq!(program(editor_spec(&tui("code --wait"), &e, path)), "C:/tools/code.cmd");
        assert_eq!(program(editor_spec(&tui(""), &e, path)), "C:/tools/nvim.exe");
        // A quoted path with spaces and its own arguments.
        let e = env(true, None, None, &dirs, &[r"C:\Program Files\Vim\gvim.exe"]);
        let s = editor_spec(&tui(r#""C:\Program Files\Vim\gvim.exe" -f"#), &e, path).unwrap();
        assert_eq!(s.program, PathBuf::from(r"C:\Program Files\Vim\gvim.exe"));
        assert_eq!(s.args[0], OsString::from("-f"));
        // $VISUAL before $EDITOR; an explicit extension is looked up as is.
        let e = env(true, Some("micro.exe"), Some("nano"), &dirs, &[r"C:\bin\micro.exe", r"C:\bin\nano.exe"]);
        assert_eq!(program(editor_spec(&tui(""), &e, path)), "C:/bin/micro.exe");
        // edit.exe is the last probe, Windows only; no Notepad.
        let e = env(true, None, None, &dirs, &[r"C:\bin\edit.exe", r"C:\Windows\notepad.exe"]);
        assert_eq!(program(editor_spec(&tui(""), &e, path)), "C:/bin/edit.exe");
        let e = env(true, None, None, &dirs, &[r"C:\Windows\notepad.exe"]);
        let tried = editor_spec(&tui(""), &e, path).unwrap_err();
        assert_eq!(tried, ["nvim", "vim", "nano", "micro", "hx", "vi", "edit"]);
    }

    fn shell_cfg(shell: &str, args: &[&str]) -> Config {
        let mut c = Config::default();
        c.terminal.shell = shell.into();
        c.terminal.args = args.iter().map(|a| a.to_string()).collect();
        c
    }

    #[test]
    fn shell_spec_per_kind() {
        let e = env(false, None, None, &["/bin"], &["/bin/bash"]);
        let dir = Path::new("/home/me/src");
        // Interactive: the configured args, in the folder, no wait.
        let s = shell_spec(&shell_cfg("bash", &["-l"]), &e, dir, None).unwrap();
        assert_eq!(s.program, PathBuf::from("/bin/bash"));
        assert_eq!(s.args, vec![OsString::from("-l")]);
        assert_eq!(s.cwd.as_deref(), Some(dir));
        assert!(!s.wait_key);
        let line = "ls -la 'a b' && echo \"done\"";
        let cases: [(&str, &[&str]); 7] = [
            ("bash", &["-c"]),
            ("/usr/bin/zsh", &["-c"]),
            ("fish", &["-c"]),
            ("/bin/dash", &["-c"]),
            ("pwsh", &["-NoProfile", "-Command"]),
            (r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe", &["-NoProfile", "-Command"]),
            (r"C:\Windows\system32\cmd.exe", &["/d", "/s", "/c"]),
        ];
        for (shell, pre) in cases {
            // A command ignores the interactive args and waits for Enter.
            let s = shell_spec(&shell_cfg(shell, &["-NoExit"]), &e, dir, Some(line)).unwrap();
            let mut want: Vec<OsString> = pre.iter().map(OsString::from).collect();
            want.push(line.into());
            assert_eq!(s.args, want, "{shell}");
            assert!(s.wait_key);
            assert_eq!(s.cwd.as_deref(), Some(dir));
        }
        // An unknown configured shell is started as given (the spawn error names it).
        let s = shell_spec(&shell_cfg("nosuchsh", &[]), &e, dir, None).unwrap();
        assert_eq!(s.program, PathBuf::from("nosuchsh"));
    }

    #[test]
    fn split_command_quoting() {
        assert_eq!(split_command("code --wait"), ["code", "--wait"]);
        assert_eq!(split_command("  vim   -u  NONE "), ["vim", "-u", "NONE"]);
        assert_eq!(split_command(r#""C:\Program Files\Vim\vim.exe" -f"#), [r"C:\Program Files\Vim\vim.exe", "-f"]);
        assert_eq!(
            split_command("emacs -nw --eval '(setq a \"b c\")'"),
            ["emacs", "-nw", "--eval", "(setq a \"b c\")"]
        );
        assert_eq!(split_command(r#"a"b c"d e"#), ["ab cd", "e"]);
        assert_eq!(split_command(r#"ed "" x"#), ["ed", "", "x"]);
        assert_eq!(split_command(r"C:\tools\hx.exe"), [r"C:\tools\hx.exe"]);
        assert_eq!(split_command("'unterminated arg"), ["unterminated arg"]);
        assert!(split_command("   ").is_empty());
        assert!(split_command("").is_empty());
    }

    #[test]
    fn gate_without_a_reader_does_not_wait() {
        let gate = InputGate::default();
        let start = Instant::now();
        assert!(gate.pause());
        assert!(gate.is_paused());
        assert!(start.elapsed() < PARK_WAIT);
        gate.resume();
        assert!(!gate.is_paused());
        // Not paused: the reader goes on at once.
        assert!(!gate.park());
    }

    #[test]
    fn gate_park_and_unpark_handshake() {
        let gate = Arc::new(InputGate::default());
        gate.attach_reader();
        let (tx, rx) = crossbeam_channel::unbounded::<&'static str>();
        let g = Arc::clone(&gate);
        let reader = std::thread::spawn(move || {
            // A reader loop: poll 5 ms, park when told to.
            let mut parks = 0;
            loop {
                if g.park() {
                    parks += 1;
                    tx.send("resumed").unwrap();
                    if parks == 2 {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            g.detach_reader();
            parks
        });
        for _ in 0..2 {
            let start = Instant::now();
            assert!(gate.pause(), "acknowledged");
            assert!(gate.is_parked());
            assert!(start.elapsed() < PARK_WAIT);
            // Parked: nothing happens until resume.
            assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
            gate.resume();
            assert_eq!(rx.recv_timeout(PARK_WAIT).unwrap(), "resumed");
        }
        assert_eq!(reader.join().unwrap(), 2);
        // The reader is gone: pausing returns at once.
        let start = Instant::now();
        assert!(gate.pause());
        assert!(start.elapsed() < PARK_WAIT);
    }

    #[test]
    fn gate_pause_is_bounded_when_the_reader_hangs() {
        let gate = InputGate::default();
        gate.attach_reader(); // never parks
        let start = Instant::now();
        assert!(!gate.pause());
        let took = start.elapsed();
        assert!(took >= PARK_WAIT && took < PARK_WAIT * 3, "{took:?}");
    }
}
