//! `soyuz-tui [--safe-mode] [--screen-reader] [PATH]`: the Soyuz terminal UI as a console program.
//!
//! Unlike `soyuz --tui` (on Windows a GUI-subsystem `soyuz.exe` that opens a console
//! window of its own), this binary is console-subsystem everywhere: it runs in the
//! terminal it is started from, which waits for it to exit. The session itself is the
//! same ([`soyuz_tui::launch`]).
#![forbid(unsafe_code)]

use std::io::Write as _;
use std::process::ExitCode;

use soyuz_tui::launch::{self, Cli};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Help / version / usage errors print to the terminal and never touch the log file
    // or the config folder. Writes ignore errors: a closed pipe (`soyuz-tui -h | head`)
    // must not panic.
    match launch::parse_args(args) {
        Ok(Cli::Help) => {
            let _ = writeln!(std::io::stdout(), "{}", launch::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Cli::Version) => {
            let _ = writeln!(std::io::stdout(), "{}", launch::version_line());
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "soyuz-tui: {e}\n\n{}", launch::USAGE);
            ExitCode::from(2)
        }
        Ok(Cli::Run(startup, opts)) => {
            launch::init_logging();
            launch::launch_with(startup, opts)
        }
    }
}
