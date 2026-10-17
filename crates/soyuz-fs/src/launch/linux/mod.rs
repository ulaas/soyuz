//! Linux / BSD desktop integration without `xdg-open` / `gio`: the MIME type comes from
//! the shared-mime-info database ([`mime`]), the application from `mimeapps.list` and
//! the installed `.desktop` files ([`mimeapps`], [`desktop`]), and the entry is launched
//! directly (D-Bus activation when it asks for it, else its `Exec=` line, wrapped in a
//! terminal for `Terminal=true`). Inside a Flatpak / Snap sandbox the
//! `org.freedesktop.portal.OpenURI` portal is used instead. "Show in folder" asks the
//! file manager over `org.freedesktop.FileManager1`.
//!
//! Parsing is pure and tested on every OS; D-Bus needs Linux (`zbus`).
#![cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]

#[cfg(target_os = "linux")]
pub(crate) mod bus;
pub(crate) mod desktop;
pub(crate) mod icons;
pub(crate) mod mime;
pub(crate) mod mimeapps;
pub(crate) mod uri;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use soyuz_core::{FsError, FsErrorKind, FsResult};

pub(crate) use desktop::{DesktopEntry, Target, parse_desktop, which};
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) use {desktop::exec_tokens, mime::parse_globs2, mimeapps::parse_mimeapps};

/// Best glob match for a file name in a parsed `globs2` (fuzzing entry point).
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn mime_for_name(globs: &[mime::Glob], name: &str) -> Option<String> {
    let db = mime::MimeDb { globs: globs.to_vec(), ..Default::default() };
    db.glob_candidates(name).into_iter().next()
}

use super::AppInfo;

// ---------------------------------------------------------------------------------------
// XDG base directories

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn env_dirs(var: &str, default: &str) -> Vec<PathBuf> {
    let v = std::env::var(var).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string());
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute()) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// `$XDG_DATA_HOME` (`~/.local/share`).
pub(crate) fn data_home() -> PathBuf {
    env_dir("XDG_DATA_HOME").unwrap_or_else(|| crate::path::home().join(".local/share"))
}

/// `$XDG_CONFIG_HOME` (`~/.config`).
pub(crate) fn config_home() -> PathBuf {
    env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| crate::path::home().join(".config"))
}

// ---------------------------------------------------------------------------------------
// Flatpak

/// The Flatpak app id when running inside a Flatpak sandbox (`/.flatpak-info` exists):
/// `$FLATPAK_ID`, else the `name=` of its `[Application]` group.
pub(crate) fn flatpak_id() -> Option<String> {
    static ID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let info = std::fs::read_to_string("/.flatpak-info").ok()?;
        std::env::var("FLATPAK_ID").ok().filter(|s| !s.is_empty()).or_else(|| flatpak_info_name(&info))
    })
    .clone()
}

/// `name=` of the `[Application]` group of a `.flatpak-info` file.
fn flatpak_info_name(info: &str) -> Option<String> {
    let mut in_app = false;
    for l in info.lines().map(str::trim) {
        if l.starts_with('[') {
            in_app = l == "[Application]";
        } else if in_app && let Some(v) = l.strip_prefix("name=") {
            return Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// The host's own XDG base folder. Inside Flatpak `$XDG_*_HOME` points at the app's
/// private `~/.var/app/<id>/…`; settings shared with the desktop (`mimeapps.list`,
/// `user-dirs.dirs`, D-Bus service files, the thumbnail cache) live in the host's folder,
/// which Flatpak passes as `$HOST_XDG_*_HOME` when it was set (else the default under
/// `~`, reachable through `--filesystem=host`). Outside Flatpak: the normal folder.
fn host_home(var: &str, default: &str) -> PathBuf {
    if flatpak_id().is_some() {
        return env_dir(&format!("HOST_{var}")).unwrap_or_else(|| crate::path::home().join(default));
    }
    env_dir(var).unwrap_or_else(|| crate::path::home().join(default))
}

/// The desktop's `$XDG_CONFIG_HOME` (see [`host_home`]).
pub(crate) fn host_config_home() -> PathBuf {
    host_home("XDG_CONFIG_HOME", ".config")
}

/// The desktop's `$XDG_DATA_HOME` (see [`host_home`]).
pub(crate) fn host_data_home() -> PathBuf {
    host_home("XDG_DATA_HOME", ".local/share")
}

/// The desktop's `$XDG_CACHE_HOME` (see [`host_home`]).
pub(crate) fn host_cache_home() -> PathBuf {
    host_home("XDG_CACHE_HOME", ".cache")
}

/// `argv` as a command that runs on the host: inside Flatpak through the sandbox's
/// `flatpak-spawn --host` (the `org.freedesktop.Flatpak` Development API, needs
/// `--talk-name=org.freedesktop.Flatpak`), with the working directory and startup token
/// passed explicitly; elsewhere `argv` itself.
fn host_command(argv: &[OsString], cwd: Option<&Path>, token: Option<&str>) -> Command {
    if flatpak_id().is_none() {
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        child_env(&mut c, token);
        return c;
    }
    let mut c = Command::new("flatpak-spawn");
    c.args(flatpak_spawn_args(argv, cwd, token));
    c
}

/// Arguments of `flatpak-spawn` for [`host_command`].
fn flatpak_spawn_args(argv: &[OsString], cwd: Option<&Path>, token: Option<&str>) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec!["--host".into()];
    if let Some(d) = cwd {
        let mut a = OsString::from("--directory=");
        a.push(d.as_os_str());
        v.push(a);
    }
    if let Some(t) = token {
        v.push(format!("--env=XDG_ACTIVATION_TOKEN={t}").into());
        v.push(format!("--env=DESKTOP_STARTUP_ID={t}").into());
    }
    v.extend(argv.iter().cloned());
    v
}

/// Whether `prog` is found on the host's `PATH` (inside Flatpak: `flatpak-spawn --host
/// which`, blocking; call off the UI thread).
fn host_has(prog: &str) -> bool {
    if flatpak_id().is_none() {
        return which(prog).is_some();
    }
    Command::new("flatpak-spawn")
        .args(["--host", "which", prog])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Data dirs, highest precedence first: `$XDG_DATA_HOME`, then `$XDG_DATA_DIRS`.
pub(crate) fn data_dirs() -> Vec<PathBuf> {
    let mut v = vec![data_home()];
    for d in env_dirs("XDG_DATA_DIRS", "/usr/local/share:/usr/share") {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// Config dirs, highest precedence first: `$XDG_CONFIG_HOME`, then `$XDG_CONFIG_DIRS`.
pub(crate) fn config_dirs() -> Vec<PathBuf> {
    let mut v = vec![config_home()];
    for d in env_dirs("XDG_CONFIG_DIRS", "/etc/xdg") {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

// ---------------------------------------------------------------------------------------
// Installed applications

/// One installed desktop entry (the highest-precedence file for its id).
#[derive(Clone, Debug)]
pub(crate) struct AppEntry {
    pub id: String,
    pub file: PathBuf,
    pub entry: DesktopEntry,
}

/// Every installed entry plus the per-data-dir type → ids map.
#[derive(Debug, Default)]
pub(crate) struct AppIndex {
    pub entries: Vec<AppEntry>,
    pub by_id: HashMap<String, usize>,
    pub per_dir: Vec<HashMap<String, Vec<String>>>,
}

impl AppIndex {
    /// Scan `<data dir>/applications` trees (ids: relative path with `/` → `-`).
    pub fn scan(data_dirs: &[PathBuf]) -> (AppIndex, Vec<(PathBuf, Option<SystemTime>)>) {
        let mut idx = AppIndex::default();
        let mut stamps = Vec::new();
        for base in data_dirs {
            let apps = base.join("applications");
            let mut per: HashMap<String, Vec<String>> = HashMap::new();
            if let Ok(t) = std::fs::read_to_string(apps.join("mimeinfo.cache")) {
                per = mimeapps::parse_mimeinfo_cache(&t);
            }
            let mut stack = vec![apps.clone()];
            while let Some(dir) = stack.pop() {
                stamps.push((dir.clone(), std::fs::metadata(&dir).and_then(|m| m.modified()).ok()));
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                let mut files: Vec<PathBuf> = Vec::new();
                for de in rd.flatten() {
                    let p = de.path();
                    if de.file_type().is_ok_and(|t| t.is_dir()) {
                        stack.push(p);
                    } else if p.extension().is_some_and(|e| e == "desktop") {
                        files.push(p);
                    }
                }
                files.sort();
                for p in files {
                    let id = p.strip_prefix(&apps).map(|r| r.to_string_lossy().replace('/', "-")).unwrap_or_default();
                    if idx.by_id.contains_key(&id) {
                        continue;
                    }
                    let Some(entry) = std::fs::read_to_string(&p).ok().and_then(|t| parse_desktop(&t)) else {
                        continue;
                    };
                    for m in &entry.mime_types {
                        let v = per.entry(m.clone()).or_default();
                        if !v.contains(&id) {
                            v.push(id.clone());
                        }
                    }
                    idx.by_id.insert(id.clone(), idx.entries.len());
                    idx.entries.push(AppEntry { id, file: p, entry });
                }
            }
            idx.per_dir.push(per);
        }
        (idx, stamps)
    }

    pub fn get(&self, id: &str) -> Option<&AppEntry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// Exists and can be launched (not `Hidden`, `TryExec` satisfied).
    pub fn usable(&self, id: &str) -> bool {
        self.get(id).is_some_and(|a| a.entry.launchable())
    }
}

struct IndexCache {
    dirs: Vec<PathBuf>,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    checked: Instant,
    index: Arc<AppIndex>,
}

static APPS: parking_lot::Mutex<Option<IndexCache>> = parking_lot::Mutex::new(None);

/// The installed applications (cached; rescanned when an applications dir changes,
/// checked at most every 3 s).
pub(crate) fn app_index() -> Arc<AppIndex> {
    let dirs = data_dirs();
    let mut g = APPS.lock();
    if let Some(c) = g.as_mut()
        && c.dirs == dirs
    {
        if c.checked.elapsed() < Duration::from_secs(3) {
            return c.index.clone();
        }
        c.checked = Instant::now();
        let fresh = c.stamps.iter().all(|(p, t)| std::fs::metadata(p).and_then(|m| m.modified()).ok() == *t)
            && dirs.iter().all(|d| {
                let a = d.join("applications");
                !a.is_dir() || c.stamps.iter().any(|(p, _)| *p == a)
            });
        if fresh {
            return c.index.clone();
        }
    }
    let (index, stamps) = AppIndex::scan(&dirs);
    let index = Arc::new(index);
    *g = Some(IndexCache { dirs, stamps, checked: Instant::now(), index: index.clone() });
    index
}

/// The type chain used for associations: the type and its parents, without the generic
/// `application/octet-stream` (unless that is the type itself).
fn assoc_chain(db: &mime::MimeDb, m: &str) -> Vec<String> {
    let mut chain = db.with_ancestors(m);
    if chain.len() > 1 {
        chain.retain(|t| t != "application/octet-stream");
    }
    chain
}

fn sources(index: &AppIndex) -> Vec<mimeapps::Source> {
    mimeapps::sources(&config_dirs(), &data_dirs(), &desktop::current_desktops(), |i| {
        index.per_dir.get(i).cloned().unwrap_or_default()
    })
}

/// Default application id for a MIME type (with the subclass walk).
pub(crate) fn default_id_for_mime(m: &str) -> Option<String> {
    let index = app_index();
    let db = mime::db();
    mimeapps::default_app(&sources(&index), &assoc_chain(&db, m), &|id| index.usable(id))
}

/// Every application for a MIME type: the default first, then the associated ones that
/// are shown in menus here (`NoDisplay` / `OnlyShowIn` / `NotShowIn` respected).
pub(crate) fn app_ids_for_mime(m: &str) -> Vec<String> {
    let index = app_index();
    let db = mime::db();
    let srcs = sources(&index);
    let chain = assoc_chain(&db, m);
    let desktops = desktop::current_desktops();
    let mut ids = mimeapps::all_apps(&srcs, &chain, &|id| index.usable(id));
    let default = mimeapps::default_app(&srcs, &chain, &|id| index.usable(id));
    ids.retain(|id| Some(id) == default.as_ref() || index.get(id).is_some_and(|a| a.entry.listed(&desktops)));
    ids
}

/// AppInfo from a desktop entry (`id` = desktop file id).
pub(crate) fn app_from_entry(id: &str, e: &DesktopEntry) -> Option<AppInfo> {
    let tokens = desktop::exec_tokens(e.exec.as_deref()?);
    let (exe, args) = tokens.split_first()?;
    Some(AppInfo {
        id: id.to_string(),
        name: if e.name.is_empty() { id.trim_end_matches(".desktop").to_string() } else { e.name.clone() },
        exec: which(&exe.text).unwrap_or_else(|| PathBuf::from(&exe.text)),
        args: args.iter().map(|t| t.text.clone()).collect(),
        icon: e.icon.as_deref().and_then(|i| icons::icon_path(i, 48, 1)),
        launchable: true,
    })
}

/// MIME type of `path` (see [`mime::mime_of`]).
pub(crate) fn mime_of(path: &Path) -> Option<String> {
    mime::mime_of(path)
}

/// Applications for `path`: the default first, then every associated application.
pub(crate) fn apps_for(path: &Path) -> Vec<AppInfo> {
    let Some(m) = mime_of(path) else { return Vec::new() };
    let index = app_index();
    app_ids_for_mime(&m).iter().filter_map(|id| index.get(id)).filter_map(|a| app_from_entry(&a.id, &a.entry)).collect()
}

/// The default application for `path`.
pub(crate) fn default_app_for(path: &Path) -> Option<AppInfo> {
    let m = mime_of(path)?;
    let id = default_id_for_mime(&m)?;
    let index = app_index();
    let a = index.get(&id)?;
    app_from_entry(&a.id, &a.entry)
}

/// Target of a `.desktop` file: `URL=` for links (`file://` decoded), the resolved
/// executable for applications.
pub(crate) fn desktop_target(path: &Path) -> std::io::Result<Option<PathBuf>> {
    let text = std::fs::read_to_string(path)?;
    let Some(e) = parse_desktop(&text) else { return Ok(None) };
    if e.kind == "Link" {
        return Ok(e.url.as_deref().and_then(uri::uri_to_path));
    }
    Ok(e.program().map(|exe| which(&exe).unwrap_or_else(|| PathBuf::from(exe))))
}

/// The icon file for a theme icon name.
pub(crate) fn icon_path(name: &str, size: u32, scale: u32) -> Option<PathBuf> {
    icons::icon_path(name, size, scale)
}

/// Icon file for a MIME type (most specific icon name that the theme has).
pub(crate) fn mime_icon_path(m: &str, size: u32, scale: u32) -> Option<PathBuf> {
    mime::db().icon_names(m).iter().find_map(|n| icons::icon_path(n, size, scale))
}

// ---------------------------------------------------------------------------------------
// Launching

/// Activation token handed to the next launched application (`XDG_ACTIVATION_TOKEN` /
/// `DESKTOP_STARTUP_ID`); single use.
static ACTIVATION_TOKEN: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

pub(crate) fn set_activation_token(token: Option<String>) {
    *ACTIVATION_TOKEN.lock() = token.filter(|t| !t.is_empty());
}

fn take_activation_token() -> Option<String> {
    ACTIVATION_TOKEN.lock().take()
}

/// Environment of a launched child: our own (already used) startup tokens removed, the
/// pending one (if any) set.
fn child_env(cmd: &mut Command, token: Option<&str>) {
    cmd.env_remove("XDG_ACTIVATION_TOKEN").env_remove("DESKTOP_STARTUP_ID");
    if let Some(t) = token {
        cmd.env("XDG_ACTIVATION_TOKEN", t).env("DESKTOP_STARTUP_ID", t);
    }
}

/// Running inside a Flatpak or Snap sandbox (launch through the portal).
pub(crate) fn sandboxed() -> bool {
    Path::new("/.flatpak-info").exists() || std::env::var_os("SNAP").is_some()
}

fn no_handler(op: &'static str, path: &Path, what: &str) -> FsError {
    FsError::with_message(
        FsErrorKind::NoApplication,
        op,
        Some(path),
        format!("no application is associated with {what} (use \u{201c}Open with\u{201d})"),
    )
}

/// Launch a desktop entry for `targets`: D-Bus activation for `DBusActivatable=true`
/// (falling back to `Exec=`), else every command line `Exec=` expands to.
pub(crate) fn launch_entry(app: &AppEntry, targets: &[Target], op: &'static str, path: &Path) -> FsResult<()> {
    let token = take_activation_token();
    #[cfg(target_os = "linux")]
    if app.entry.dbus_activatable {
        match dbus_activate(&app.id, targets, token.as_deref()) {
            Ok(()) => return Ok(()),
            Err(e) => log::debug!("dbus activation of {} failed ({e}); using Exec=", app.id),
        }
    }
    let e = &app.entry;
    let ctx = desktop::ExecContext { icon: e.icon.as_deref(), name: &e.name, location: Some(&app.file) };
    let Some(lines) = e.exec.as_deref().and_then(|x| desktop::expand_exec(x, targets, &ctx)) else {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            op,
            Some(path),
            format!("{} has no command to run", app.id),
        ));
    };
    let cwd = e.path.as_ref().map(PathBuf::from).filter(|p| p.is_dir()).or_else(|| {
        targets.iter().find_map(|t| match t {
            Target::Path(p) => p.parent().filter(|d| d.is_dir()).map(Path::to_path_buf),
            Target::Url(_) => None,
        })
    });
    for (i, argv) in lines.into_iter().enumerate() {
        let argv = if e.terminal {
            wrap_in_terminal(&argv, None).ok_or_else(|| {
                FsError::with_message(FsErrorKind::NotFound, op, Some(path), "no terminal emulator found")
            })?
        } else {
            argv
        };
        // Inside Flatpak the entry is a host (or other Flatpak) program: run it there.
        let cmd = host_command(&argv, cwd.as_deref(), if i == 0 { token.as_deref() } else { None });
        super::spawn_detached(cmd, op, path)?;
    }
    Ok(())
}

/// `org.freedesktop.Application.Open` / `Activate` on the entry's well-known name.
#[cfg(target_os = "linux")]
fn dbus_activate(id: &str, targets: &[Target], token: Option<&str>) -> Result<(), String> {
    use zbus::zvariant::Value;
    let name = id.strip_suffix(".desktop").unwrap_or(id);
    let obj = format!("/{}", name.replace('.', "/").replace('-', "_"));
    let conn = bus::session().ok_or("no session bus")?;
    let mut platform: HashMap<&str, Value<'_>> = HashMap::new();
    if let Some(t) = token {
        platform.insert("activation-token", Value::from(t));
        platform.insert("desktop-startup-id", Value::from(t));
    }
    let uris: Vec<String> = targets
        .iter()
        .map(|t| match t {
            Target::Path(p) => uri::file_uri(p),
            Target::Url(u) => u.clone(),
        })
        .collect();
    let iface = Some("org.freedesktop.Application");
    let r = if uris.is_empty() {
        conn.call_method(Some(name), obj.as_str(), iface, "Activate", &(platform,))
    } else {
        conn.call_method(Some(name), obj.as_str(), iface, "Open", &(uris, platform))
    };
    r.map(drop).map_err(|e| e.to_string())
}

/// Open `path` with its default application.
pub(crate) fn open_default(path: &Path) -> FsResult<()> {
    crate::list::stat(path, true).map_err(|e| FsError { op: "open".into(), ..e })?;
    #[cfg(target_os = "linux")]
    if sandboxed() {
        return portal_open_file(path);
    }
    let Some(m) = mime_of(path) else {
        return Err(no_handler("open", path, "this file"));
    };
    let Some(id) = default_id_for_mime(&m) else {
        return Err(no_handler("open", path, &format!("\u{201c}{m}\u{201d} files")));
    };
    let index = app_index();
    let app = index.get(&id).ok_or_else(|| no_handler("open", path, &format!("\u{201c}{m}\u{201d} files")))?;
    launch_entry(app, &[Target::Path(path.to_path_buf())], "open", path)
}

/// A program to run rather than a document to open: a regular file with an execute bit
/// whose content is an ELF executable (`ET_EXEC`, or `ET_DYN` for PIE binaries and
/// AppImages). `*.so` / `*.so.N` libraries are `ET_DYN` too and are left out by name.
pub(crate) fn is_runnable_binary(path: &Path) -> bool {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    let Ok(md) = std::fs::metadata(path) else { return false };
    if !md.is_file() || md.permissions().mode() & 0o111 == 0 {
        return false;
    }
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if name.ends_with(".so") || name.contains(".so.") {
        return false;
    }
    let mut head = [0u8; 18];
    if std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_err() || head[..4] != *b"\x7fELF" {
        return false;
    }
    // e_type at offset 16, in the byte order EI_DATA (offset 5) names: 2 = big endian.
    let e_type =
        if head[5] == 2 { u16::from_be_bytes([head[16], head[17]]) } else { u16::from_le_bytes([head[16], head[17]]) };
    matches!(e_type, 2 | 3)
}

/// Run a program found by [`is_runnable_binary`], detached, in its own folder (on the
/// host inside Flatpak).
pub(crate) fn run_binary(path: &Path) -> FsResult<()> {
    let token = take_activation_token();
    let cwd = path.parent().filter(|d| d.is_dir());
    let cmd = host_command(&[path.as_os_str().to_owned()], cwd, token.as_deref());
    super::spawn_detached(cmd, "run", path)
}

/// Open `paths` with `app` (a desktop entry when `app.id` names one, else the generic
/// `exec` + `args` launcher).
pub(crate) fn open_with_many(paths: &[PathBuf], app: &AppInfo) -> FsResult<()> {
    let Some(first) = paths.first() else { return Ok(()) };
    for p in paths {
        crate::list::stat(p, true).map_err(|e| FsError { op: "open_with".into(), ..e })?;
    }
    let index = app_index();
    match index.get(&app.id).filter(|a| a.entry.launchable()) {
        Some(entry) => {
            let targets: Vec<Target> = paths.iter().cloned().map(Target::Path).collect();
            launch_entry(entry, &targets, "open_with", first)
        }
        None => {
            for p in paths {
                super::open_with_generic(p, app)?;
            }
            Ok(())
        }
    }
}

/// Open a URL with the handler for its scheme (`x-scheme-handler/<scheme>`); `file:`
/// URLs open the file with its default application.
pub(crate) fn open_url(url: &str) -> FsResult<()> {
    let p = Path::new(url);
    let Some(scheme) = uri::scheme(url) else {
        return Err(FsError::with_message(FsErrorKind::InvalidName, "open", Some(p), "not a valid URL"));
    };
    if scheme == "file" {
        let local = uri::uri_to_path(url)
            .ok_or_else(|| FsError::with_message(FsErrorKind::InvalidName, "open", Some(p), "not a local file URL"))?;
        return open_default(&local);
    }
    #[cfg(target_os = "linux")]
    if sandboxed() {
        return portal_open_uri(url);
    }
    let Some(id) = default_id_for_mime(&format!("x-scheme-handler/{scheme}")) else {
        return Err(no_handler("open", p, &format!("\u{201c}{scheme}:\u{201d} links")));
    };
    let index = app_index();
    let app = index.get(&id).ok_or_else(|| no_handler("open", p, &format!("\u{201c}{scheme}:\u{201d} links")))?;
    launch_entry(app, &[Target::Url(url.to_string())], "open", p)
}

#[cfg(target_os = "linux")]
fn portal_options(token: Option<&str>) -> HashMap<&'static str, zbus::zvariant::Value<'static>> {
    let mut o: HashMap<&'static str, zbus::zvariant::Value<'static>> = HashMap::new();
    if let Some(t) = token {
        o.insert("activation_token", zbus::zvariant::Value::from(t.to_string()));
    }
    o
}

#[cfg(target_os = "linux")]
fn portal_call<B>(method: &str, body: &B, path: &Path) -> FsResult<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let conn = bus::session()
        .ok_or_else(|| FsError::with_message(FsErrorKind::Unsupported, "open", Some(path), "no D-Bus session bus"))?;
    conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.OpenURI"),
        method,
        body,
    )
    .map(drop)
    .map_err(|e| FsError::with_message(FsErrorKind::Io, "open", Some(path), format!("portal: {e}")))
}

/// `OpenURI.OpenFile` with a read-only descriptor (sandboxed builds).
#[cfg(target_os = "linux")]
fn portal_open_file(path: &Path) -> FsResult<()> {
    let f = std::fs::File::open(path).map_err(crate::err::map("open", path))?;
    let token = take_activation_token();
    portal_call("OpenFile", &("", zbus::zvariant::Fd::from(&f), portal_options(token.as_deref())), path)
}

/// `OpenURI.OpenURI` (sandboxed builds).
#[cfg(target_os = "linux")]
fn portal_open_uri(url: &str) -> FsResult<()> {
    let token = take_activation_token();
    portal_call("OpenURI", &("", url, portal_options(token.as_deref())), Path::new(url))
}

// ---------------------------------------------------------------------------------------
// Reveal in the file manager

/// Whether a desktop entry launches this very program (avoid handing folders back to
/// ourselves when Soyuz is the default file manager).
fn is_self(app: &AppEntry) -> bool {
    if app.id.starts_with("soyuz") || flatpak_id().is_some_and(|f| app.id.starts_with(f.as_str())) {
        return true;
    }
    let Ok(me) = std::env::current_exe().and_then(std::fs::canonicalize) else { return false };
    app.entry.program().and_then(|p| which(&p)).and_then(|p| std::fs::canonicalize(p).ok()).is_some_and(|p| p == me)
}

const FILE_MANAGER1: &str = "org.freedesktop.FileManager1";

/// Path of the per-user D-Bus service file for `FileManager1`.
pub(crate) fn file_manager1_service_file() -> PathBuf {
    host_data_home().join("dbus-1/services/org.freedesktop.FileManager1.service")
}

/// `ShowItems([uri], "")` on the session's file manager, unless that would be us.
#[cfg(target_os = "linux")]
fn show_items(path: &Path) -> Result<(), String> {
    let conn = bus::session().ok_or("no session bus")?;
    let owned = bus::has_owner(&conn, FILE_MANAGER1);
    if owned {
        if bus::owner_pid(&conn, FILE_MANAGER1) == Some(std::process::id()) {
            return Err("FileManager1 is served by this process".into());
        }
    } else {
        if !bus::activatable(&conn, FILE_MANAGER1) {
            return Err("no FileManager1 service".into());
        }
        // Our own (opt-in) service file would start another Soyuz (inside Flatpak it runs
        // `flatpak run … <app id>`).
        let me = flatpak_id()
            .unwrap_or_else(|| std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        if !me.is_empty()
            && std::fs::read_to_string(file_manager1_service_file()).is_ok_and(|t| t.contains(me.as_str()))
        {
            return Err("FileManager1 would activate Soyuz itself".into());
        }
    }
    let token = take_activation_token().unwrap_or_default();
    conn.call_method(
        Some(FILE_MANAGER1),
        "/org/freedesktop/FileManager1",
        Some(FILE_MANAGER1),
        "ShowItems",
        &(vec![uri::file_uri(path)], token.as_str()),
    )
    .map(drop)
    .map_err(|e| e.to_string())
}

/// Whether a `FileManager1` service (other than us) is reachable.
#[cfg(target_os = "linux")]
pub(crate) fn file_manager1_available() -> bool {
    let Some(conn) = bus::session() else { return false };
    if bus::has_owner(&conn, FILE_MANAGER1) {
        return bus::owner_pid(&conn, FILE_MANAGER1) != Some(std::process::id());
    }
    bus::activatable(&conn, FILE_MANAGER1)
}

/// Show `path` selected in the system file manager; failing that, open its folder with
/// the default `inode/directory` handler (never Soyuz itself).
pub(crate) fn reveal(path: &Path) -> FsResult<()> {
    crate::list::stat(path, false).map_err(|e| FsError { op: "reveal".into(), ..e })?;
    #[cfg(target_os = "linux")]
    match show_items(path) {
        Ok(()) => return Ok(()),
        Err(e) => log::debug!("FileManager1.ShowItems: {e}"),
    }
    // Sandboxed: the host's folder handlers can't be started from here; the portal
    // opens the folder holding `path` in the desktop's file manager.
    #[cfg(target_os = "linux")]
    if sandboxed() {
        let f = std::fs::File::open(path).map_err(crate::err::map("reveal", path))?;
        let token = take_activation_token();
        return portal_call(
            "OpenDirectory",
            &("", zbus::zvariant::Fd::from(&f), portal_options(token.as_deref())),
            path,
        );
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(path);
    let index = app_index();
    let db = mime::db();
    let srcs = sources(&index);
    let usable = |id: &str| index.get(id).is_some_and(|a| a.entry.launchable() && !is_self(a));
    let chain = assoc_chain(&db, "inode/directory");
    let id = mimeapps::all_apps(&srcs, &chain, &usable).into_iter().next();
    match id.and_then(|id| index.get(&id)) {
        Some(app) => launch_entry(app, &[Target::Path(dir.to_path_buf())], "reveal", path),
        None => Err(no_handler("reveal", path, "folders")),
    }
}

// ---------------------------------------------------------------------------------------
// Terminals

/// How a terminal emulator takes a working directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirArg {
    /// Only through the process's working directory.
    Cwd,
    /// `--flag=DIR`.
    Joined(&'static str),
    /// `--flag DIR`.
    Separate(&'static str),
}

/// A known terminal: arguments before everything else, the working-directory flag and
/// what precedes a command to run (empty = the command follows directly).
#[derive(Clone, Copy, Debug)]
struct TermSpec {
    name: &'static str,
    pre: &'static [&'static str],
    dir: DirArg,
    exec: &'static [&'static str],
}

const TERMINALS: &[TermSpec] = &[
    TermSpec { name: "xdg-terminal-exec", pre: &[], dir: DirArg::Cwd, exec: &[] },
    TermSpec { name: "ptyxis", pre: &["--new-window"], dir: DirArg::Joined("--working-directory="), exec: &["--"] },
    TermSpec { name: "kgx", pre: &[], dir: DirArg::Joined("--working-directory="), exec: &["--"] },
    TermSpec { name: "gnome-terminal", pre: &[], dir: DirArg::Joined("--working-directory="), exec: &["--"] },
    TermSpec { name: "konsole", pre: &[], dir: DirArg::Separate("--workdir"), exec: &["-e"] },
    TermSpec { name: "xfce4-terminal", pre: &[], dir: DirArg::Joined("--working-directory="), exec: &["-x"] },
    TermSpec { name: "alacritty", pre: &[], dir: DirArg::Separate("--working-directory"), exec: &["-e"] },
    TermSpec { name: "kitty", pre: &[], dir: DirArg::Separate("--directory"), exec: &[] },
    TermSpec { name: "foot", pre: &[], dir: DirArg::Joined("--working-directory="), exec: &[] },
    TermSpec { name: "wezterm", pre: &["start"], dir: DirArg::Separate("--cwd"), exec: &["--"] },
    TermSpec { name: "xterm", pre: &[], dir: DirArg::Cwd, exec: &["-e"] },
    TermSpec { name: "x-terminal-emulator", pre: &[], dir: DirArg::Cwd, exec: &["-e"] },
];

/// The order terminals are tried in: `preferred` (user setting), `xdg-terminal-exec`,
/// `$TERMINAL`, then the table. Each item is `(program words, spec)`; unknown programs
/// get `-e` and the working directory only.
fn terminal_candidates(preferred: Option<&str>) -> Vec<(Vec<String>, TermSpec)> {
    terminal_candidates_with(preferred, &host_has)
}

/// [`terminal_candidates`] with `exists` deciding whether a program is installed.
fn terminal_candidates_with(preferred: Option<&str>, exists: &dyn Fn(&str) -> bool) -> Vec<(Vec<String>, TermSpec)> {
    let unknown = TermSpec { name: "", pre: &[], dir: DirArg::Cwd, exec: &["-e"] };
    let spec_of = |prog: &str| {
        let base = Path::new(prog).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        TERMINALS.iter().copied().find(|t| t.name == base).unwrap_or(unknown)
    };
    let mut out: Vec<(Vec<String>, TermSpec)> = Vec::new();
    let mut push = |words: Vec<String>, must_exist: bool| {
        let Some(prog) = words.first() else { return };
        if must_exist && !exists(prog) {
            return;
        }
        if !out.iter().any(|(w, _)| w == &words) {
            let s = spec_of(prog);
            out.push((words, s));
        }
    };
    if let Some(p) = preferred.map(str::trim).filter(|p| !p.is_empty()) {
        push(p.split_whitespace().map(str::to_string).collect(), false);
    }
    push(vec!["xdg-terminal-exec".into()], true);
    if let Ok(t) = std::env::var("TERMINAL") {
        push(t.split_whitespace().map(str::to_string).collect(), true);
    }
    for t in &TERMINALS[1..] {
        push(vec![t.name.to_string()], true);
    }
    out
}

fn term_argv(words: &[String], spec: &TermSpec, dir: Option<&Path>, cmd: Option<&[OsString]>) -> Vec<OsString> {
    let mut v: Vec<OsString> = words.iter().map(OsString::from).collect();
    v.extend(spec.pre.iter().map(OsString::from));
    if let Some(d) = dir {
        match spec.dir {
            DirArg::Cwd => {}
            DirArg::Joined(f) => {
                let mut a = OsString::from(f);
                a.push(d.as_os_str());
                v.push(a);
            }
            DirArg::Separate(f) => {
                v.push(f.into());
                v.push(d.as_os_str().to_owned());
            }
        }
    }
    if let Some(c) = cmd {
        v.extend(spec.exec.iter().map(OsString::from));
        v.extend(c.iter().cloned());
    }
    v
}

/// `argv` wrapped to run inside a terminal window (`Terminal=true` entries).
pub(crate) fn wrap_in_terminal(argv: &[OsString], preferred: Option<&str>) -> Option<Vec<OsString>> {
    let (words, spec) = terminal_candidates(preferred).into_iter().next()?;
    Some(term_argv(&words, &spec, None, Some(argv)))
}

/// Open a terminal window in `dir`.
pub(crate) fn open_terminal_at(dir: &Path, preferred: Option<&str>) -> FsResult<()> {
    let mut last_err = None;
    for (words, spec) in terminal_candidates(preferred) {
        let argv = term_argv(&words, &spec, Some(dir), None);
        let c = host_command(&argv, Some(dir), take_activation_token().as_deref());
        match super::spawn_detached(c, "open_terminal", dir) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        FsError::with_message(FsErrorKind::NotFound, "open_terminal", Some(dir), "no terminal emulator found")
    }))
}

/// Environment variables passed through `pkexec` so graphical programs can reach the
/// user's display.
pub(crate) const ELEVATE_ENV: [&str; 4] = ["DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR"];

/// `pkexec env VAR=… <exe> <args…>`: pkexec clears the environment, `env` restores the
/// display variables.
pub(crate) fn elevate_argv(exe: &Path, args: &[String], get: impl Fn(&str) -> Option<OsString>) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec!["pkexec".into()];
    let vars: Vec<OsString> = ELEVATE_ENV
        .iter()
        .filter_map(|k| {
            get(k).filter(|x| !x.is_empty()).map(|x| {
                let mut a = OsString::from(format!("{k}="));
                a.push(x);
                a
            })
        })
        .collect();
    if !vars.is_empty() {
        v.push(which("env").map_or_else(|| "/usr/bin/env".into(), PathBuf::into_os_string));
        v.extend(vars);
    }
    v.push(exe.as_os_str().to_owned());
    v.extend(args.iter().map(OsString::from));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn desktop_link_target() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("l.desktop");
        std::fs::write(&p, "[Desktop Entry]\nType=Link\nURL=file:///home/u/My%20Docs\n").unwrap();
        assert_eq!(desktop_target(&p).unwrap(), Some(PathBuf::from("/home/u/My Docs")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runnable_binaries() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let put = |name: &str, from: &Path, mode: u32| {
            let p = d.path().join(name);
            std::fs::copy(from, &p).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
            p
        };
        // The test binary itself is an ELF executable.
        let exe = std::env::current_exe().unwrap();
        assert!(is_runnable_binary(&put("tool", &exe, 0o755)));
        assert!(is_runnable_binary(&put("Tool-1.0-x86_64.AppImage", &exe, 0o755)));
        assert!(!is_runnable_binary(&put("no-x-bit", &exe, 0o644)));
        assert!(!is_runnable_binary(&put("libfoo.so", &exe, 0o755)));
        assert!(!is_runnable_binary(&put("libfoo.so.1.2", &exe, 0o755)));
        let script = d.path().join("src.sh");
        std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        assert!(!is_runnable_binary(&put("run.sh", &script, 0o755)));
        assert!(!is_runnable_binary(d.path()));
        assert!(!is_runnable_binary(&d.path().join("missing")));
    }

    #[test]
    fn terminal_argv_shapes() {
        let words = |s: &str| vec![s.to_string()];
        let spec = |n: &str| TERMINALS.iter().copied().find(|t| t.name == n).unwrap();
        let dir = Path::new("/w d");
        let s = |v: Vec<OsString>| v.into_iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(
            s(term_argv(&words("ptyxis"), &spec("ptyxis"), Some(dir), None)),
            ["ptyxis", "--new-window", "--working-directory=/w d"]
        );
        assert_eq!(
            s(term_argv(&words("konsole"), &spec("konsole"), Some(dir), None)),
            ["konsole", "--workdir", "/w d"]
        );
        let cmd = [OsString::from("htop"), OsString::from("-d")];
        assert_eq!(
            s(term_argv(&words("konsole"), &spec("konsole"), None, Some(&cmd))),
            ["konsole", "-e", "htop", "-d"]
        );
        assert_eq!(s(term_argv(&words("kitty"), &spec("kitty"), None, Some(&cmd))), ["kitty", "htop", "-d"]);
        assert_eq!(
            s(term_argv(&words("wezterm"), &spec("wezterm"), Some(dir), Some(&cmd))),
            ["wezterm", "start", "--cwd", "/w d", "--", "htop", "-d"]
        );
        // A user-chosen terminal with its own arguments is kept as given.
        let c = terminal_candidates(Some("my-term --flag"));
        assert_eq!(c[0].0, ["my-term", "--flag"]);
        assert_eq!(c[0].1.exec, ["-e"]);
    }

    #[test]
    fn flatpak_helpers() {
        let info = "[Application]\nname=io.github.ulaas.Soyuz\nruntime=runtime/org.freedesktop.Platform/x86_64/25.08\n\n[Instance]\nname=x\n";
        assert_eq!(flatpak_info_name(info).as_deref(), Some("io.github.ulaas.Soyuz"));
        assert_eq!(flatpak_info_name("[Instance]\nname=x\n"), None);
        let argv = [OsString::from("ptyxis"), OsString::from("--new-window")];
        let s = |v: Vec<OsString>| v.into_iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(
            s(flatpak_spawn_args(&argv, Some(Path::new("/w d")), Some("tok"))),
            [
                "--host",
                "--directory=/w d",
                "--env=XDG_ACTIVATION_TOKEN=tok",
                "--env=DESKTOP_STARTUP_ID=tok",
                "ptyxis",
                "--new-window"
            ]
        );
        assert_eq!(s(flatpak_spawn_args(&argv, None, None)), ["--host", "ptyxis", "--new-window"]);
        // Installed-ness comes from the probe (the host's PATH inside Flatpak).
        let c = terminal_candidates_with(None, &|p| p == "konsole");
        assert_eq!(c.iter().map(|(w, _)| w[0].as_str()).collect::<Vec<_>>(), ["konsole"]);
    }

    #[test]
    fn elevation_passes_display_env() {
        let env = |k: &str| match k {
            "DISPLAY" => Some(OsString::from(":0")),
            "WAYLAND_DISPLAY" => Some(OsString::from("wayland-1")),
            "XAUTHORITY" => Some(OsString::new()),
            _ => None,
        };
        let v: Vec<String> = elevate_argv(Path::new("/usr/bin/gparted"), &["a b".into()], env)
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(v[0], "pkexec");
        assert!(v[1].ends_with("env"), "{v:?}");
        assert_eq!(&v[2..], ["DISPLAY=:0", "WAYLAND_DISPLAY=wayland-1", "/usr/bin/gparted", "a b"]);
        let bare = elevate_argv(Path::new("/x"), &[], |_| None);
        assert_eq!(bare, [OsString::from("pkexec"), OsString::from("/x")]);
    }

    /// `SOYUZ_DBUS_TESTS=1`: resolve against this desktop's real databases and probe the
    /// session bus for `FileManager1` (nothing is launched or shown).
    #[cfg(target_os = "linux")]
    #[test]
    fn real_desktop_probe() {
        if std::env::var_os("SOYUZ_DBUS_TESTS").is_none() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notes.txt");
        std::fs::write(&f, "hi\n").unwrap();
        let png = d.path().join("picture");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").unwrap();
        println!("mime(notes.txt) = {:?}", mime_of(&f));
        println!("mime(picture, sniffed) = {:?}", mime_of(&png));
        println!("mime(dir) = {:?}", mime_of(d.path()));
        for m in
            ["text/plain", "text/x-csrc", "image/png", "x-scheme-handler/https", "inode/directory", "application/pdf"]
        {
            println!("default({m}) = {:?}; all = {:?}", default_id_for_mime(m), app_ids_for_mime(m));
        }
        let apps = apps_for(&f);
        println!("apps_for(notes.txt) = {:?}", apps.iter().map(|a| (&a.id, &a.name, &a.icon)).collect::<Vec<_>>());
        println!("icon(folder, 48) = {:?}", icon_path("folder", 48, 1));
        println!("mime icon(text/plain) = {:?}", mime_icon_path("text/plain", 32, 1));
        println!("theme = {:?}", icons::current_theme_name());
        let t = std::time::Instant::now();
        let fm = file_manager1_available();
        println!("FileManager1 available = {fm} ({:?})", t.elapsed());
        assert_eq!(mime_of(&f).as_deref(), Some("text/plain"));
        assert_eq!(mime_of(&png).as_deref(), Some("image/png"));
        assert!(!apps.is_empty(), "no application for text/plain");
    }

    /// A fake XDG tree: mimeapps + desktop files resolve the default and the list,
    /// including the subclass walk and NoDisplay / Hidden / TryExec handling.
    #[cfg(unix)]
    #[test]
    fn index_and_associations_on_a_fake_tree() {
        let d = tempfile::tempdir().unwrap();
        let data = d.path().join("data");
        let apps = data.join("applications");
        std::fs::create_dir_all(apps.join("sub")).unwrap();
        let w = |n: &str, t: &str| std::fs::write(apps.join(n), t).unwrap();
        w(
            "editor.desktop",
            "[Desktop Entry]\nType=Application\nName=Editor\nExec=sh -c true %F\nMimeType=text/plain;\n",
        );
        w(
            "hidden.desktop",
            "[Desktop Entry]\nType=Application\nName=H\nExec=sh %f\nHidden=true\nMimeType=text/plain;\n",
        );
        w(
            "nodisplay.desktop",
            "[Desktop Entry]\nType=Application\nName=ND\nExec=sh %f\nNoDisplay=true\nMimeType=text/plain;\n",
        );
        w(
            "broken.desktop",
            "[Desktop Entry]\nType=Application\nName=B\nExec=x %f\nTryExec=/nonexistent/soyuz\nMimeType=text/plain;\n",
        );
        std::fs::write(
            apps.join("sub/viewer.desktop"),
            "[Desktop Entry]\nType=Application\nName=V\nExec=sh %u\nMimeType=text/x-csrc;\n",
        )
        .unwrap();
        let (idx, stamps) = AppIndex::scan(std::slice::from_ref(&data));
        assert!(stamps.len() >= 2);
        assert!(idx.get("sub-viewer.desktop").is_some(), "ids use - for subdirectories");
        assert!(!idx.usable("hidden.desktop"));
        assert!(!idx.usable("broken.desktop"));
        assert!(idx.usable("nodisplay.desktop"));
        let srcs = vec![
            mimeapps::Source::List(mimeapps::parse_mimeapps("[Default Applications]\ntext/plain=nodisplay.desktop\n")),
            mimeapps::Source::Installed(idx.per_dir[0].clone()),
        ];
        let chain = vec!["text/x-csrc".to_string(), "text/plain".to_string()];
        let usable = |id: &str| idx.usable(id);
        assert_eq!(mimeapps::default_app(&srcs, &chain, &usable).as_deref(), Some("sub-viewer.desktop"));
        let all = mimeapps::all_apps(&srcs, &chain, &usable);
        assert_eq!(all, ["sub-viewer.desktop", "editor.desktop", "nodisplay.desktop"]);
        let plain = vec!["text/plain".to_string()];
        assert_eq!(mimeapps::default_app(&srcs, &plain, &usable).as_deref(), Some("nodisplay.desktop"));
        let e = idx.get("editor.desktop").unwrap();
        let info = app_from_entry(&e.id, &e.entry).unwrap();
        assert_eq!(info.name, "Editor");
        assert_eq!(info.args, ["-c", "true", "%F"]);
        assert!(info.exec.is_absolute(), "sh resolved via PATH");
    }
}
