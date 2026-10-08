//! `soyuz-cli`: the console twin of `soyuz cp|mv|rm|trash|ls|open|reveal|jobs` (the
//! release `soyuz.exe` is a GUI-subsystem binary, so shells do not wait for it).
#![forbid(unsafe_code)]

use std::process::ExitCode;

use soyuz_script::cli;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        println!("{}", cli::HELP);
        return ExitCode::SUCCESS;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    match cli::parse(&args, &cwd) {
        Ok(Some(cmd)) => ExitCode::from(cli::run(cmd).clamp(0, 255) as u8),
        Ok(None) => {
            eprintln!("soyuz-cli: unknown command '{}'\n\n{}", args[0], cli::HELP);
            ExitCode::from(cli::exit::USAGE as u8)
        }
        Err(e) => {
            eprintln!("soyuz-cli: {e}\n\nRun `soyuz-cli --help` for usage.");
            ExitCode::from(cli::exit::USAGE as u8)
        }
    }
}
