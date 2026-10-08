//! Launching files and applications. All spawning goes through `std::process::Command`
//! (kernel32 `CreateProcessW` under the hood on Windows; `CommandExt::raw_arg` /
//! `creation_flags` where needed). No shell execution APIs are used in this process.
//!
//! * Windows: registry association chain → `CreateProcessW`; UWP / `DelegateExecute`
//!   associations and unresolvable shortcuts fall back to the external
//!   `rundll32 url.dll,FileProtocolHandler <path>` process.
//! * Linux/BSD: `xdg-open`, then `gio open`; Open-with list from `.desktop` files.
//! * macOS: `open` / `open -a`.

mod linux;
pub(crate) mod lnk;
mod macos;
mod windows;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use soyuz_core::{FsError, FsResult};

use crate::err;

/// An application that can open a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub exec: PathBuf,
    /// May contain `"%1"` (replaced by the file path; appended when absent). On Windows the
    /// entries are raw command-line fragments (registry commands keep their own quoting).
    pub args: Vec<String>,
    pub icon: Option<PathBuf>,
    /// `false` for UWP / DelegateExecute associations.
    pub launchable: bool,
}

/// Open with the default application.
pub fn open_default(path: &Path) -> FsResult<()> {
    #[cfg(windows)]
    {
        windows::open_default(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_default(path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        crate::list::stat(path, true).map_err(|e| FsError { op: "open".into(), ..e })?;
        let mut first_err = None;
        for (prog, pre) in [("xdg-open", None), ("gio", Some("open"))] {
            let mut c = Command::new(prog);
            if let Some(p) = pre {
                c.arg(p);
            }
            c.arg(path);
            match spawn_detached(c, "open", path) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        let mut e = first_err.unwrap_or_else(|| FsError::unsupported("open", Some(path)));
        e.message = format!("{} (install xdg-utils to open files)", e.message);
        Err(e)
    }
}

/// Open `path` with a specific application (from [`apps_for`] or user-defined).
pub fn open_with(path: &Path, app: &AppInfo) -> FsResult<()> {
    #[cfg(windows)]
    {
        windows::open_with(path, app)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_with(path, app)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        open_with_generic(path, app)
    }
}

/// Spawn `app.exec` with `app.args`, substituting the file for `%1 %L %f %F %u %U`
/// (appended when no placeholder occurs).
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn open_with_generic(path: &Path, app: &AppInfo) -> FsResult<()> {
    crate::list::stat(path, true).map_err(|e| FsError { op: "open_with".into(), ..e })?;
    let mut cmd = Command::new(&app.exec);
    let p = path.to_string_lossy();
    let mut substituted = false;
    for a in &app.args {
        if ["%1", "%L", "%f", "%F", "%u", "%U"].iter().any(|ph| a.contains(ph)) {
            substituted = true;
            let r = a
                .replace("%1", &p)
                .replace("%L", &p)
                .replace("%f", &p)
                .replace("%F", &p)
                .replace("%u", &p)
                .replace("%U", &p);
            cmd.arg(r);
        } else {
            cmd.arg(a);
        }
    }
    if !substituted {
        cmd.arg(path);
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    spawn_detached(cmd, "open_with", path)
}

/// Applications registered for `path`'s type (default first). Empty on macOS.
pub fn apps_for(path: &Path) -> Vec<AppInfo> {
    #[cfg(windows)]
    {
        windows::apps_for(path)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = path;
        Vec::new()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::apps_for(path)
    }
}

/// Spawn and reap in the background so no zombie is left behind on unix.
pub(crate) fn spawn_detached(mut cmd: Command, op: &'static str, path: &Path) -> FsResult<()> {
    match cmd.spawn() {
        Ok(mut child) => std::thread::Builder::new()
            .name("soyuz-reap".into())
            .spawn(move || {
                let _ = child.wait();
            })
            .map(drop)
            .map_err(err::map(op, path)),
        Err(e) => {
            let mut fe = err::io(op, path, e);
            if fe.kind == soyuz_core::FsErrorKind::NotFound {
                fe.message = format!("program not found: {}", cmd.get_program().to_string_lossy());
            }
            Err(fe)
        }
    }
}

/// Open an external terminal in `dir` (`preferred` = executable name / path).
pub fn open_terminal_at(dir: &Path, preferred: Option<&str>) -> FsResult<()> {
    let dir_plain = crate::path::strip_verbatim(dir);
    let dir = dir_plain.as_path();
    let mut candidates: Vec<Command> = Vec::new();
    if let Some(p) = preferred.filter(|p| !p.trim().is_empty()) {
        let mut c = Command::new(p);
        c.current_dir(dir);
        candidates.push(c);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        let mut wt = Command::new("wt.exe");
        wt.arg("-d").arg(dir).current_dir(dir);
        candidates.push(wt);
        for exe in ["pwsh.exe", "powershell.exe", "cmd.exe"] {
            let mut c = Command::new(exe);
            c.current_dir(dir).creation_flags(CREATE_NEW_CONSOLE);
            candidates.push(c);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg("-a").arg("Terminal").arg(dir);
        candidates.push(c);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut names: Vec<String> = Vec::new();
        if let Ok(t) = std::env::var("TERMINAL") {
            names.push(t);
        }
        names.extend(
            [
                "x-terminal-emulator",
                "gnome-terminal",
                "konsole",
                "xfce4-terminal",
                "kgx",
                "alacritty",
                "kitty",
                "wezterm",
                "foot",
                "xterm",
            ]
            .map(String::from),
        );
        for n in names {
            let mut c = Command::new(n);
            c.current_dir(dir);
            candidates.push(c);
        }
    }
    let mut last_err = None;
    for cmd in candidates {
        match spawn_detached(cmd, "open_terminal", dir) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| FsError::unsupported("open_terminal", Some(dir))))
}

/// Run `exe` elevated (UAC prompt via an external `powershell Start-Process -Verb RunAs`
/// on Windows, `pkexec` on Linux; see PLAN §4). `Unsupported` on macOS.
pub fn run_elevated(exe: &Path, args: &[String]) -> FsResult<()> {
    #[cfg(windows)]
    {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let mut script = format!(
            "Start-Process -Verb RunAs -FilePath {}",
            quote(&crate::path::strip_verbatim(exe).to_string_lossy())
        );
        if !args.is_empty() {
            let list: Vec<String> = args.iter().map(|a| quote(a)).collect();
            script.push_str(&format!(" -ArgumentList {}", list.join(",")));
        }
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script]);
        spawn_detached(c, "run_elevated", exe)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c = Command::new("pkexec");
        c.arg(exe).args(args);
        spawn_detached(c, "run_elevated", exe)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = args;
        Err(err::unsupported("run_elevated", exe, "elevation is not supported on this platform"))
    }
}

/// `explorer.exe /select,`, `open -R`, `xdg-open <parent>`.
pub fn reveal_in_system(path: &Path) -> FsResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("explorer.exe");
        c.raw_arg(format!("/select,\"{}\"", crate::path::strip_verbatim(&crate::path::normalize(path)).display()));
        spawn_detached(c, "reveal", path)
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        spawn_detached(c, "reveal", path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        let mut c = Command::new("xdg-open");
        c.arg(dir);
        spawn_detached(c, "reveal", path)
    }
}

/// Target of a Windows `.lnk` (MS-SHLLINK parser, any OS) / `.desktop` `Exec=` or `URL=`
/// (unix); macOS aliases and other files → `None`.
pub fn shortcut_target(path: &Path) -> FsResult<Option<PathBuf>> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "lnk" => match lnk::resolve(path) {
            Ok((target, _)) => Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(e) => Err(err::io("shortcut_target", path, e)),
        },
        "desktop" if cfg!(unix) => linux::desktop_target(path).map_err(err::map("shortcut_target", path)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_targets() {
        let d = tempfile::tempdir().unwrap();
        let l = d.path().join("app.lnk");
        std::fs::write(&l, lnk::tests::sample_lnk(r"C:\Tools\app.exe", "")).unwrap();
        assert_eq!(shortcut_target(&l).unwrap(), Some(PathBuf::from(r"C:\Tools\app.exe")));
        let junk = d.path().join("junk.lnk");
        std::fs::write(&junk, b"garbage").unwrap();
        assert_eq!(shortcut_target(&junk).unwrap(), None);
        assert_eq!(shortcut_target(&d.path().join("plain.txt")).unwrap(), None);
        assert!(shortcut_target(&d.path().join("missing.lnk")).is_err());
    }

    #[test]
    fn open_missing_file_fails_cleanly() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("missing.txt");
        assert_eq!(open_default(&p).unwrap_err().kind, soyuz_core::FsErrorKind::NotFound);
        let app = AppInfo {
            id: "x".into(),
            name: "x".into(),
            exec: PathBuf::from("definitely-not-a-program-soyuz"),
            args: vec![],
            icon: None,
            launchable: true,
        };
        std::fs::write(&p, b"x").unwrap();
        assert!(open_with(&p, &app).is_err());
    }
}
