//! Window-level actions: new window / move tab to a new window and "Run as
//! administrator". With `[workspace] new_window = "in_process"` (the default) windows are
//! viewports of this process (`models::windows`); otherwise each window is its
//! own process, started as `soyuz --new-window -- <path>` so it neither restores nor
//! overwrites the primary window's session.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use soyuz_core::Location;

use crate::core::AppCore;
use crate::models::toasts::Toast;

/// Folder a new window should open for `loc` (virtual locations fall back to home).
fn window_dir(loc: &Location) -> PathBuf {
    loc.as_fs_path().filter(|p| p.is_absolute()).map(Path::to_path_buf).unwrap_or_else(soyuz_fs::home)
}

/// Arguments for a secondary window showing `dir`.
pub fn new_window_args(dir: &Path, renderer: &str) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["--new-window".into()];
    if renderer == "glow" {
        args.push("--renderer=glow".into());
    }
    args.push("--".into());
    args.push(soyuz_fs::strip_verbatim(dir).into_os_string());
    args
}

/// Start a new Soyuz process showing `loc`. The spawn is local (our own executable, no
/// working directory on a possibly slow share), so it is cheap enough for the UI thread.
pub fn spawn_window(app: &mut AppCore, loc: &Location) -> bool {
    let dir = window_dir(loc);
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            app.ui.toasts.push(Toast::error(tr!("ops-new-window-failed")).body(e.to_string()));
            return false;
        }
    };
    let mut cmd = Command::new(exe);
    cmd.args(new_window_args(&dir, app.renderer)).stdin(Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            // Reap the child so it never lingers as a zombie on Unix.
            let _ = std::thread::Builder::new().name("soyuz-window-reap".into()).spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(e) => {
            app.ui.toasts.push(Toast::error(tr!("ops-new-window-failed")).body(e.to_string()));
            false
        }
    }
}

/// `TabToNewWindow`: open the active tab's folder in a new window, then close the tab
/// here (when it is not the only one).
pub fn tab_to_new_window(app: &mut AppCore) {
    if app.tui_unavailable() {
        return;
    }
    if app.config.workspace.new_window == soyuz_core::config::NewWindowMode::InProcess {
        if app.ws.tabs.len() > 1 {
            let i = app.ws.active;
            app.cmds.push(crate::cmd::Cmd::Workspace(crate::models::workspaces::WsCmd::TabToNewWindow(i)));
        } else {
            app.ui.toasts.push(Toast::info(tr!("ops-window-keeps-tab")));
        }
        return;
    }
    let loc = app.ws.pane().location.clone();

    if !spawn_window(app, &loc) {
        return;
    }
    if app.ws.tabs.len() > 1 {
        let i = app.ws.active;
        crate::ops::close_tab(app, i);
    }
}

/// Whether `path` is something "Run as administrator" can start.
pub fn is_elevatable(path: &Path, mode: Option<u32>) -> bool {
    if cfg!(windows) {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        matches!(ext.as_str(), "exe" | "com" | "bat" | "cmd" | "msi")
    } else {
        mode.is_some_and(|m| m & 0o111 != 0)
    }
}

/// `$platform` of the `ops-elevate-needs` message.
fn elevate_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

/// `name` is an executable file in one of the `PATH` directories (no process spawned).
#[cfg(all(unix, not(target_os = "macos")))]
fn on_path(name: &str, path: Option<&std::ffi::OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_some_and(|p| {
        std::env::split_paths(p)
            .any(|d| std::fs::metadata(d.join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
    })
}

/// `RunElevated`: start the selected program elevated (UAC on Windows via an external
/// PowerShell `Start-Process -Verb RunAs`, `pkexec` on Linux — see `launch::run_elevated`;
/// no shell APIs in-process). The OS elevation prompt is the confirmation.
pub fn run_elevated(app: &mut AppCore) {
    // The elevation prompt (UAC / polkit's text agent) would fight the terminal UI for
    // the screen and the keyboard.
    if app.tui_unavailable() {
        return;
    }
    let pane = app.ws.pane();
    let Some(e) = pane.selected_entries().first().map(|e| (*e).clone()) else {
        app.ui.toasts.push(Toast::info(tr!("ops-elevate-select")));
        return;
    };
    if e.is_dir_like() || !is_elevatable(&e.path, e.mode) {
        app.ui.toasts.push(Toast::info(tr!("ops-elevate-needs", platform = elevate_platform())));
        return;
    }
    // Linux: elevation goes through polkit's `pkexec`; say so plainly when it is missing
    // instead of surfacing a bare "No such file or directory".
    #[cfg(all(unix, not(target_os = "macos")))]
    if !on_path("pkexec", std::env::var_os("PATH").as_deref()) {
        app.ui.toasts.push(Toast::error(tr!("ops-elevate-failed", name = &e.name)).body(tr!("ops-elevate-no-pkexec")));
        return;
    }
    if let Err(err) = soyuz_fs::launch::run_elevated(&e.path, &[]) {
        app.ui.toasts.push(Toast::error(tr!("ops-elevate-failed", name = &e.name)).body(err.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_args_end_options_before_the_path() {
        let a = new_window_args(Path::new("/tmp/-rf"), "wgpu");
        assert_eq!(a, vec!["--new-window", "--", "/tmp/-rf"]);
        let a = new_window_args(Path::new("/x"), "glow");
        assert_eq!(a, vec!["--new-window", "--renderer=glow", "--", "/x"]);
    }

    #[test]
    fn elevatable_kinds() {
        if cfg!(windows) {
            assert!(is_elevatable(Path::new("C:\\a\\setup.EXE"), None));
            assert!(is_elevatable(Path::new("C:\\a\\x.msi"), None));
            assert!(!is_elevatable(Path::new("C:\\a\\notes.txt"), None));
        } else {
            assert!(is_elevatable(Path::new("/usr/bin/x"), Some(0o755)));
            assert!(!is_elevatable(Path::new("/tmp/x.sh"), Some(0o644)));
            assert!(!is_elevatable(Path::new("/tmp/x"), None));
        }
    }

    #[test]
    fn elevate_wording_per_platform() {
        let p = elevate_platform();
        assert_eq!(p == "windows", cfg!(windows));
        assert_eq!(p == "linux", cfg!(target_os = "linux"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn path_lookup() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("pkexec");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o644)).unwrap();
        let path = std::env::join_paths([d.path()]).unwrap();
        assert!(!on_path("pkexec", Some(&path)), "not executable");
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(on_path("pkexec", Some(&path)));
        assert!(!on_path("missing", Some(&path)));
        assert!(!on_path("pkexec", None));
    }
}
