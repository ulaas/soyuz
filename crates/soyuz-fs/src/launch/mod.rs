//! Launching files and applications. All spawning goes through `std::process::Command`
//! (kernel32 `CreateProcessW` under the hood on Windows; `CommandExt::raw_arg` /
//! `creation_flags` where needed). No shell execution APIs are used.

mod linux;
mod lnk;
mod macos;
mod windows;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;

use crate::baseline;

/// An application that can open a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub exec: PathBuf,
    /// May contain `"%1"` (replaced by the file path; appended when absent).
    pub args: Vec<String>,
    pub icon: Option<PathBuf>,
    /// `false` for UWP / DelegateExecute associations.
    pub launchable: bool,
}

/// Open with the default application.
pub fn open_default(path: &Path) -> FsResult<()> {
    baseline::open_default(path)
}

pub fn open_with(path: &Path, app: &AppInfo) -> FsResult<()> {
    baseline::open_with(path, app)
}

/// Applications registered for `path`'s type. Baseline: empty.
pub fn apps_for(path: &Path) -> Vec<AppInfo> {
    baseline::apps_for(path)
}

/// Open an external terminal in `dir` (`preferred` = executable name / path).
pub fn open_terminal_at(dir: &Path, preferred: Option<&str>) -> FsResult<()> {
    baseline::open_terminal_at(dir, preferred)
}

/// Run `exe` elevated (UAC prompt via an external `powershell Start-Process -Verb RunAs`
/// on Windows, `pkexec` on Linux; see PLAN §4).
pub fn run_elevated(exe: &Path, args: &[String]) -> FsResult<()> {
    baseline::run_elevated(exe, args)
}

/// `explorer.exe /select,`, `open -R`, `xdg-open <parent>`.
pub fn reveal_in_system(path: &Path) -> FsResult<()> {
    baseline::reveal_in_system(path)
}

/// Target of a Windows `.lnk` / `.desktop` `Exec=`; macOS alias → `None`. Baseline: `None`.
pub fn shortcut_target(path: &Path) -> FsResult<Option<PathBuf>> {
    baseline::shortcut_target(path)
}
