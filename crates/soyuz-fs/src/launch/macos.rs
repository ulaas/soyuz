//! macOS: LaunchServices (`LSOpenFromURLSpec`, `LSCopyApplicationURLsForURL`,
//! `LSCopyDefaultApplicationURLForURL`) and `NSWorkspace` for Finder reveal; no `open`
//! process is spawned. Finder aliases resolve through their bookmark data. Elevation
//! (`run_elevated`) is not offered on macOS.
//!
//! Untested on macOS: compile-checked from Linux only.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

use soyuz_core::FsError;

use crate::err;

/// LaunchServices `OSStatus` → error. `-10814` (`kLSApplicationNotFoundErr`) and `-43`
/// (`fnfErr`) mean nothing can open it.
pub(crate) fn ls_error(op: &'static str, path: &Path, status: i32) -> FsError {
    let msg = match status {
        -10814 | -43 => "no application is associated with this item".to_string(),
        -10826 => "the application may not be launched (no permission)".to_string(),
        _ => format!("LaunchServices error {status}"),
    };
    err::unsupported(op, path, &msg)
}

#[cfg(target_os = "macos")]
mod imp {
    use soyuz_core::FsResult;

    use super::super::AppInfo;
    use super::*;
    use crate::sys::macos::{cf, launch as ls};

    fn url_of(op: &'static str, path: &Path) -> FsResult<core_foundation::url::CFURL> {
        cf::file_url(path).ok_or_else(|| err::unsupported(op, path, "not a valid file URL"))
    }

    pub(crate) fn open_default(path: &Path) -> FsResult<()> {
        crate::list::stat(path, true).map_err(|e| FsError { op: "open".into(), ..e })?;
        let url = url_of("open", path)?;
        ls::open_urls(&[url], None).map_err(|s| ls_error("open", path, s))
    }

    pub(crate) fn open_url(url: &str) -> FsResult<()> {
        let u = cf::string_url(url).ok_or_else(|| err::unsupported("open", Path::new(url), "not a valid URL"))?;
        ls::open_urls(&[u], None).map_err(|s| ls_error("open", Path::new(url), s))
    }

    /// Open several files with an application bundle in one LaunchServices call; plain
    /// executables run directly once per file.
    pub(crate) fn open_with_many(paths: &[PathBuf], app: &AppInfo) -> FsResult<()> {
        let Some(first) = paths.first() else { return Ok(()) };
        if !is_bundle(&app.exec) {
            return paths.iter().try_for_each(|p| super::super::open_with_generic(p, app));
        }
        let mut urls = Vec::with_capacity(paths.len());
        for p in paths {
            crate::list::stat(p, true).map_err(|e| FsError { op: "open_with".into(), ..e })?;
            urls.push(url_of("open_with", p)?);
        }
        let app_url = cf::file_url(&app.exec)
            .ok_or_else(|| err::unsupported("open_with", &app.exec, "not a valid application URL"))?;
        ls::open_urls(&urls, Some(&app_url)).map_err(|s| ls_error("open_with", first, s))
    }

    pub(crate) fn apps_for(path: &Path) -> Vec<AppInfo> {
        let Some(url) = cf::file_url(path) else { return Vec::new() };
        let default = ls::default_app_for_url(&url);
        let mut apps: Vec<PathBuf> = default.iter().cloned().collect();
        for a in ls::apps_for_url(&url) {
            if !apps.contains(&a) {
                apps.push(a);
            }
        }
        apps.iter().map(|a| app_info(a)).collect()
    }

    pub(crate) fn default_app_for(path: &Path) -> Option<AppInfo> {
        let url = cf::file_url(path)?;
        ls::default_app_for_url(&url).map(|a| app_info(&a))
    }

    pub(crate) fn app_info(bundle: &Path) -> AppInfo {
        let b = ls::bundle_info(bundle);
        AppInfo {
            id: b.id.unwrap_or_else(|| bundle.to_string_lossy().into_owned()),
            name: b.name.unwrap_or_else(|| bundle_stem(bundle)),
            exec: bundle.to_path_buf(),
            args: Vec::new(),
            icon: b.icon,
            launchable: true,
        }
    }

    pub(crate) fn reveal(path: &Path) -> FsResult<()> {
        crate::list::stat(path, false).map_err(|e| FsError { op: "reveal".into(), ..e })?;
        if ls::reveal(&[path.to_path_buf()]) {
            Ok(())
        } else {
            Err(err::unsupported("reveal", path, "Finder could not be asked to show the item"))
        }
    }

    /// Open a folder in Terminal.app (or `preferred`: an application bundle path / name in
    /// `/Applications` or `/System/Applications/Utilities`; anything else is run as a
    /// program with the folder as its working directory).
    pub(crate) fn open_terminal_at(dir: &Path, preferred: Option<&str>) -> FsResult<()> {
        let dir_url = url_of("open_terminal", dir)?;
        let mut bundles: Vec<PathBuf> = Vec::new();
        if let Some(p) = preferred.map(str::trim).filter(|p| !p.is_empty()) {
            match find_app(p) {
                Some(b) => bundles.push(b),
                None => {
                    let mut c = std::process::Command::new(p);
                    c.current_dir(dir);
                    return super::super::spawn_detached(c, "open_terminal", dir);
                }
            }
        }
        bundles.extend(
            ["/System/Applications/Utilities/Terminal.app", "/Applications/Utilities/Terminal.app"]
                .iter()
                .map(PathBuf::from)
                .filter(|p| p.is_dir()),
        );
        let mut last = None;
        for b in bundles {
            let Some(app_url) = cf::file_url(&b) else { continue };
            match ls::open_urls(std::slice::from_ref(&dir_url), Some(&app_url)) {
                Ok(()) => return Ok(()),
                Err(s) => last = Some(ls_error("open_terminal", dir, s)),
            }
        }
        Err(last.unwrap_or_else(|| FsError::unsupported("open_terminal", Some(dir))))
    }

    /// The target of a Finder alias (`None` for other files).
    pub(crate) fn alias_target(path: &Path) -> Option<PathBuf> {
        super::is_alias_file(path).then(|| ls::resolve_alias(path)).flatten()
    }
}

#[cfg(target_os = "macos")]
pub(crate) use imp::*;

/// An `.app` bundle (a folder), as opposed to a plain executable.
pub(crate) fn is_bundle(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"))
}

/// `Safari` for `/Applications/Safari.app`.
pub(crate) fn bundle_stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// `preferred` as an application bundle: an existing `*.app` path, or a name looked up in
/// the standard application folders (`iTerm`, `Ghostty.app`).
pub(crate) fn find_app(preferred: &str) -> Option<PathBuf> {
    let p = Path::new(preferred);
    if is_bundle(p) && p.is_absolute() {
        return p.is_dir().then(|| p.to_path_buf());
    }
    if p.components().count() != 1 {
        return None;
    }
    let name = if is_bundle(p) { preferred.to_string() } else { format!("{preferred}.app") };
    let home_apps = crate::path::home().join("Applications");
    [
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
        home_apps,
    ]
    .into_iter()
    .map(|d| d.join(&name))
    .find(|c| c.is_dir())
}

/// Finder alias files carry bookmark data starting with `book\0\0\0\0mark\0\0\0\0`; only
/// those are handed to CoreFoundation (cheap check, reads 16 bytes).
pub(crate) fn is_alias_file(path: &Path) -> bool {
    use std::io::Read;
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    if !meta.is_file() || meta.len() < 16 {
        return false;
    }
    let mut head = [0u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && is_bookmark_header(&head)
}

/// The bookmark-data magic (pure).
pub(crate) fn is_bookmark_header(head: &[u8]) -> bool {
    head.len() >= 12 && &head[0..4] == b"book" && &head[8..12] == b"mark"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_names_and_alias_magic() {
        assert!(is_bundle(Path::new("/Applications/Safari.app")));
        assert!(!is_bundle(Path::new("/usr/bin/vim")));
        assert_eq!(bundle_stem(Path::new("/Applications/Visual Studio Code.app")), "Visual Studio Code");
        assert!(is_bookmark_header(b"book\0\0\0\0mark\0\0\0\0"));
        assert!(!is_bookmark_header(b"bplist00"));
        let d = tempfile::tempdir().unwrap();
        let alias = d.path().join("alias");
        std::fs::write(&alias, b"book\0\0\0\0mark\0\0\0\0rest").unwrap();
        assert!(is_alias_file(&alias));
        let plain = d.path().join("plain");
        std::fs::write(&plain, b"hello world, not an alias").unwrap();
        assert!(!is_alias_file(&plain));
        assert!(find_app("definitely/not/an/app").is_none());
    }
}
