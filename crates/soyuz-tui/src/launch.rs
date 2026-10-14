//! Process entry shared by the two ways to start the terminal UI:
//!
//! - `soyuz-tui [--safe-mode] [--screen-reader] [PATH]` (this crate's binary,
//!   `src/main.rs`): a console program on every platform, so on Windows it runs inside the
//!   terminal that started it;
//! - `soyuz --tui [--safe-mode] [--screen-reader] [PATH]` (`soyuz-app`'s `main.rs`): the
//!   same session from the GUI binary; a GUI-subsystem release `soyuz.exe` opens a console
//!   window of its own.
//!
//! Both call [`init_logging`] before anything else logs, then [`launch`] /
//! [`launch_with`]. `--screen-reader` turns screen-reader mode on for this run only
//! ([`RunOpts`]); it is never written to the settings.
//!
//! Debug hook: `SOYUZ_TUI_HEADLESS=<w>x<h>` runs the session on an in-memory screen
//! driven by `SOYUZ_SCRIPT` (`crate::devscript::run_headless`), without a console or
//! terminal.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use soyuz_appcore::core::Startup;
use soyuz_appcore::services::crash;

/// `writeln!` to stderr that never panics (a closed pipe or a missing stream is ignored).
macro_rules! err {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

/// Log to a capped file (`<config dir>/soyuz-tui.log`): the screen belongs to the
/// terminal UI. Also installs the crash-report panic hook.
pub fn init_logging() {
    let log = soyuz_core::config::config_dir().join("soyuz-tui.log");
    crash::init_file_logging(&log, crash::LOG_FILE_CAP);
    crash::install_panic_hook();
}

/// Run the terminal UI for `startup` (its `open` path: a folder opens itself, a file
/// opens its folder with the file under the cursor) and return the process exit code.
///
/// Prepares the console ([`soyuz_fs::console::tui_console`]: on Windows the inherited
/// console as is, or a new console window when the process has none, i.e. a
/// GUI-subsystem `soyuz.exe`), refuses redirected stdio, claims the crash-journal slot,
/// loads the configuration (safe defaults with `--safe-mode`) and runs [`crate::run`].
pub fn launch(startup: Startup) -> ExitCode {
    launch_with(startup, RunOpts::default())
}

/// What a run asks for besides the [`Startup`]; nothing here is saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunOpts {
    /// `--screen-reader`: screen-reader mode for this run (`App::sr_override`).
    pub screen_reader: bool,
}

/// [`launch`] with the run-only options `opts`. With `SOYUZ_TUI_HEADLESS` set the session
/// runs headless (`crate::devscript::run_headless`): no console, no terminal checks.
pub fn launch_with(startup: Startup, opts: RunOpts) -> ExitCode {
    if let Some(size) = crate::devscript::headless_size(|k| std::env::var_os(k)) {
        crash::startup(startup.safe_mode);
        let config = if startup.safe_mode { crash::safe_config() } else { soyuz_appcore::core::load_config() };
        return crate::devscript::run_headless(config, startup, opts.screen_reader, size);
    }
    // Windows: the process console, or a new console window for a GUI-subsystem
    // release build (never the parent shell's console: both would read the keyboard).
    // Restores the console modes / code pages when dropped.
    let _console = match soyuz_fs::console::tui_console() {
        Ok(c) => c,
        Err(e) => {
            err!("soyuz: cannot open a console for the terminal UI: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !soyuz_fs::console::is_terminal_session() {
        err!(
            "soyuz: the terminal UI needs an interactive terminal (stdin and stdout must not be redirected; TERM must not be dumb)"
        );
        return ExitCode::from(2);
    }
    // Before the job engine exists: claim the crash-journal slot, collect what to offer.
    crash::startup(startup.safe_mode);
    let config = if startup.safe_mode { crash::safe_config() } else { soyuz_appcore::core::load_config() };
    // Installs the legacy-terminal keymap overlay itself when the kitty keyboard protocol
    // is not used (it knows after querying the terminal), before the core is built.
    crate::run(config, startup, opts.screen_reader)
}

/// What the `soyuz-tui` command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    Run(Startup, RunOpts),
    Help,
    Version,
}

/// `soyuz-tui --help`.
pub const USAGE: &str = "\
Usage: soyuz-tui [--safe-mode] [--screen-reader] [PATH]

Soyuz in the terminal it is started from (the same terminal UI as `soyuz --tui`).

Arguments:
  [PATH]         Folder to open (a file opens its folder with the file selected);
                 `file://` URIs are accepted

Options:
  --safe-mode    Default settings; no session restore, folder watching, thumbnails or git
  --screen-reader
                 Screen-reader mode for this run (plain lines, no box drawing; the
                 setting stays as it is)
  -h, --help     Print help
  -V, --version  Print version
  --             End of options: the next argument is a path";

/// `soyuz-tui --version`.
pub fn version_line() -> String {
    format!("soyuz-tui {} ({})", env!("CARGO_PKG_VERSION"), soyuz_appcore::GIT_SHA)
}

/// A command-line path: `file://` URIs (desktop files pass `%U`) are decoded.
pub fn arg_path(arg: String) -> PathBuf {
    if arg.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:"))
        && let Some(p) = soyuz_fs::launch::path_from_file_uri(&arg)
    {
        return p;
    }
    PathBuf::from(arg)
}

/// Parse the `soyuz-tui` arguments (without the program name). `--help` / `--version`
/// win wherever they appear before `--`.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut startup = Startup::default();
    let mut opts = RunOpts::default();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            "--safe-mode" => startup.safe_mode = true,
            "--screen-reader" => opts.screen_reader = true,
            "--" => paths.extend(it.by_ref().map(arg_path)),
            other if other.starts_with('-') && other != "-" => return Err(format!("unexpected argument '{other}'")),
            _ => paths.push(arg_path(arg)),
        }
    }
    if paths.len() > 1 {
        return Err("the terminal UI opens one folder (got several paths)".into());
    }
    startup.open = paths.into_iter().next();
    Ok(Cli::Run(startup, opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> Result<Cli, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    fn run(open: Option<&str>, safe_mode: bool) -> Result<Cli, String> {
        Ok(Cli::Run(Startup { open: open.map(PathBuf::from), safe_mode, ..Default::default() }, RunOpts::default()))
    }

    #[test]
    fn paths_and_safe_mode() {
        assert_eq!(parse(&[]), run(None, false));
        assert_eq!(parse(&["dir"]), run(Some("dir"), false));
        assert_eq!(parse(&["--safe-mode"]), run(None, true));
        assert_eq!(parse(&["dir", "--safe-mode"]), run(Some("dir"), true));
        assert_eq!(parse(&["--safe-mode", "C:\\data"]), run(Some("C:\\data"), true));
        // Never a secondary window: the TUI keeps its own session.
        let Ok(Cli::Run(s, _)) = parse(&["dir"]) else { panic!("parse") };
        assert!(!s.secondary);
    }

    #[test]
    fn double_dash_ends_options() {
        assert_eq!(parse(&["--", "-dash"]), run(Some("-dash"), false));
        assert_eq!(parse(&["--", "--help"]), run(Some("--help"), false));
        assert_eq!(parse(&["--safe-mode", "--", "--safe-mode"]), run(Some("--safe-mode"), true));
        assert_eq!(parse(&["--"]), run(None, false));
        // A lone `-` is a path, not an option.
        assert_eq!(parse(&["-"]), run(Some("-"), false));
    }

    #[test]
    fn screen_reader_is_a_run_option() {
        let Ok(Cli::Run(s, opts)) = parse(&["--screen-reader", "dir"]) else { panic!("parse") };
        assert!(opts.screen_reader);
        assert_eq!(s.open, Some(PathBuf::from("dir")));
        let Ok(Cli::Run(_, opts)) = parse(&["dir"]) else { panic!("parse") };
        assert!(!opts.screen_reader);
        assert_eq!(parse(&["--", "--screen-reader"]), run(Some("--screen-reader"), false));
        assert!(USAGE.contains("--screen-reader"));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse(&["-h"]), Ok(Cli::Help));
        assert_eq!(parse(&["dir", "--version"]), Ok(Cli::Version));
        assert_eq!(parse(&["-V"]), Ok(Cli::Version));
        assert!(version_line().starts_with(&format!("soyuz-tui {} (", env!("CARGO_PKG_VERSION"))));
        assert!(USAGE.starts_with("Usage: soyuz-tui"));
    }

    #[test]
    fn errors() {
        assert!(parse(&["--bogus"]).is_err());
        assert!(parse(&["-x"]).is_err());
        // Options of the GUI binary are not ours.
        assert!(parse(&["--tui"]).is_err());
        assert!(parse(&["--renderer", "glow"]).is_err());
        assert!(parse(&["--new-window"]).is_err());
        let e = parse(&["a", "b"]).unwrap_err();
        assert!(e.contains("one folder"), "{e}");
        assert!(parse(&["a", "--", "b"]).is_err());
    }

    #[test]
    fn file_uris() {
        let Ok(Cli::Run(s, _)) = parse(&["file:///tmp/a%20b"]) else { panic!("parse") };
        if cfg!(windows) {
            assert!(s.open.is_some());
        } else {
            assert_eq!(s.open, Some(PathBuf::from("/tmp/a b")));
        }
        let Ok(Cli::Run(s, _)) = parse(&["sftp://host/x"]) else { panic!("parse") };
        assert_eq!(s.open, Some(PathBuf::from("sftp://host/x")));
    }
}
