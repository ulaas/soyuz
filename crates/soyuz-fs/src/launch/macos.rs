//! macOS: `open`, `open -a <app>`, `open -R`. The "Open with" list would need
//! LaunchServices (CoreServices) and is not provided; aliases are not resolved.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::Path;
use std::process::Command;

use soyuz_core::{FsError, FsResult};

use super::AppInfo;

pub(crate) fn open_default(path: &Path) -> FsResult<()> {
    crate::list::stat(path, true).map_err(|e| FsError { op: "open".into(), ..e })?;
    let mut c = Command::new("open");
    c.arg(path);
    super::spawn_detached(c, "open", path)
}

/// `.app` bundles go through `open -a`; plain executables run directly.
pub(crate) fn open_with(path: &Path, app: &AppInfo) -> FsResult<()> {
    if app.exec.extension().is_some_and(|e| e == "app") {
        crate::list::stat(path, true).map_err(|e| FsError { op: "open_with".into(), ..e })?;
        let mut c = Command::new("open");
        c.arg("-a").arg(&app.exec).arg(path);
        return super::spawn_detached(c, "open_with", path);
    }
    super::open_with_generic(path, app)
}
