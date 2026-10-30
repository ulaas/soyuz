//! Windows default-app resolution through the registry association chain (advapi32,
//! `HKEY_CLASSES_ROOT` merged view) and process creation through `std::process::Command`
//! (`CreateProcessW`). No shell execution API is used.
//!
//! Chain: `HKCU\...\FileExts\<ext>\UserChoiceLatest\ProgId` → `…\UserChoice\ProgId` →
//! `HKCR\<ext>` default → `HKCR\SystemFileAssociations\<ext>` →
//! `HKCR\SystemFileAssociations\<PerceivedType>`; for each ProgId the verb is
//! `shell\(Default)` else `open` else (not for `SystemFileAssociations`, whose verbs only add
//! to a type's menu) the first verb that is not `print` / `printto`, and the command is
//! `shell\<verb>\command`. `AppX*` ProgIds (Store apps) start through their app execution
//! alias when the package declares one; other `AppX*` ProgIds and `DelegateExecute` verbs
//! are COM/UWP-only (not launchable). Opening such a file hands it to `explorer.exe` (a
//! separate process that opens it in its default app; `[open] store_apps_via_explorer`),
//! else to the first registered program Soyuz can start, else fails with "no application
//! is associated". URLs (`.url` files) resolve their scheme through
//! `HKCU\...\Shell\Associations\UrlAssociations\<scheme>\UserChoice` and `HKCR\<scheme>`
//! (`URL Protocol`) the same way.
//!
//! The resolution logic is pure (behind the [`Registry`] trait) and unit-tested everywhere.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

use super::AppInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Hive {
    ClassesRoot,
    CurrentUser,
    LocalMachine,
}

/// Read-only registry access (real: advapi32; tests: in-memory).
pub(crate) trait Registry {
    /// String value (`None` value name = default value), environment variables expanded.
    fn get(&self, hive: Hive, key: &str, value: Option<&str>) -> Option<String>;
    fn has(&self, hive: Hive, key: &str, value: &str) -> bool;
    fn subkeys(&self, hive: Hive, key: &str) -> Vec<String>;
    fn values(&self, hive: Hive, key: &str) -> Vec<String>;
    /// Display string (resolves `@dll,-id` indirect strings in the real registry).
    fn display(&self, hive: Hive, key: &str, value: &str) -> Option<String> {
        self.get(hive, key, Some(value)).filter(|s| !s.starts_with('@'))
    }
    /// The app execution alias (`%LOCALAPPDATA%\Microsoft\WindowsApps\<family>\<alias>.exe`)
    /// that starts packaged app `app_id` (`Microsoft.Paint_8wekyb3d8bbwe!App`), if any.
    fn app_alias(&self, app_id: &str) -> Option<PathBuf> {
        let _ = app_id;
        None
    }
}

const FILE_EXTS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts";
const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
const MUI_CACHE: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache";

/// A resolved verb command of a ProgId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Verb {
    pub command: Option<String>,
    pub delegate: bool,
}

/// The command of `progid`'s default verb (`shell\(Default)`, then `open`, then the first
/// verb that does not print: a type whose only verb is `printto` has no way to open).
pub(crate) fn verb_command(reg: &dyn Registry, progid: &str) -> Option<Verb> {
    verb_command_in(reg, progid, true)
}

/// [`verb_command`]; `any_verb` off = only `shell\(Default)` and `open` count.
fn verb_command_in(reg: &dyn Registry, progid: &str, any_verb: bool) -> Option<Verb> {
    let shell = format!(r"{progid}\shell");
    let mut verbs: Vec<String> = Vec::new();
    if let Some(def) = reg.get(Hive::ClassesRoot, &shell, None) {
        // May be a comma-separated preference list.
        verbs.extend(def.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    }
    verbs.push("open".into());
    if any_verb {
        let opens = |v: &String| !v.eq_ignore_ascii_case("print") && !v.eq_ignore_ascii_case("printto");
        verbs.extend(reg.subkeys(Hive::ClassesRoot, &shell).into_iter().filter(opens).take(1));
    }
    for verb in verbs {
        let key = format!(r"{shell}\{verb}\command");
        let command = reg.get(Hive::ClassesRoot, &key, None).filter(|c| !c.trim().is_empty());
        let delegate = reg.has(Hive::ClassesRoot, &key, "DelegateExecute");
        if command.is_some() || delegate {
            return Some(Verb { command, delegate });
        }
    }
    None
}

/// The association for `ext` (with the dot, any case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Assoc {
    pub progid: String,
    pub command: Option<String>,
    pub launchable: bool,
}

fn is_appx(progid: &str) -> bool {
    progid.len() >= 4 && progid[..4].eq_ignore_ascii_case("appx")
}

/// The app user model id of a Store app's ProgId (`AppX…\Application\AppUserModelID`).
fn appx_app_id(reg: &dyn Registry, progid: &str) -> Option<String> {
    reg.get(Hive::ClassesRoot, &format!(r"{progid}\Application"), Some("AppUserModelID")).filter(|s| !s.is_empty())
}

/// A Store app's name: its registered display name when it is a plain or `@dll,-id`
/// string, else the package name of its app id. Package resource strings
/// (`@{…?ms-resource://…}`) are not resolved.
fn appx_name(reg: &dyn Registry, progid: &str) -> String {
    if let Some(n) = reg.display(Hive::ClassesRoot, &format!(r"{progid}\Application"), "ApplicationName") {
        return n;
    }
    appx_app_id(reg, progid).and_then(|id| app_id_name(&id)).unwrap_or_else(|| progid.to_string())
}

/// `Microsoft.Windows.Photos_8wekyb3d8bbwe!App` → `Photos`, `Microsoft.WindowsNotepad_…`
/// → `Notepad`.
fn app_id_name(app_id: &str) -> Option<String> {
    let package = app_id.split(['!', '_']).next()?;
    let last = package.rsplit('.').next().filter(|s| !s.is_empty())?;
    let name = last.strip_prefix("Windows").filter(|r| r.starts_with(|c: char| c.is_ascii_uppercase())).unwrap_or(last);
    Some(name.to_string())
}

/// The Store app of `progid` as an Open-with entry: launchable when it has an alias.
fn appx_app(reg: &dyn Registry, progid: &str) -> AppInfo {
    let alias = appx_app_id(reg, progid).and_then(|id| reg.app_alias(&id));
    AppInfo {
        id: progid.to_string(),
        name: appx_name(reg, progid),
        launchable: alias.is_some(),
        args: if alias.is_some() { vec!["\"%1\"".into()] } else { Vec::new() },
        exec: alias.unwrap_or_default(),
        icon: None,
    }
}

pub(crate) fn default_assoc(reg: &dyn Registry, ext: &str) -> Option<Assoc> {
    let ext = ext.to_ascii_lowercase();
    // (ProgId, whether its first verb may open the file)
    let mut candidates: Vec<(String, bool)> = Vec::new();
    for key in [r"UserChoiceLatest\ProgId", "UserChoice"] {
        if let Some(p) = reg.get(Hive::CurrentUser, &format!(r"{FILE_EXTS}\{ext}\{key}"), Some("ProgId")) {
            candidates.push((p, true));
        }
    }
    if let Some(p) = reg.get(Hive::ClassesRoot, &ext, None) {
        candidates.push((p, true));
    }
    candidates.push((format!(r"SystemFileAssociations\{ext}"), false));
    if let Some(pt) = reg.get(Hive::ClassesRoot, &ext, Some("PerceivedType")) {
        candidates.push((format!(r"SystemFileAssociations\{pt}"), false));
    }
    let mut fallback: Option<Assoc> = None;
    for (progid, any_verb) in candidates.into_iter().filter(|(p, _)| !p.is_empty()) {
        let appx = is_appx(&progid);
        if appx && let Some(exe) = appx_app_id(reg, &progid).and_then(|id| reg.app_alias(&id)) {
            let command = format!("\"{}\" \"%1\"", exe.display());
            return Some(Assoc { progid, command: Some(command), launchable: true });
        }
        match verb_command_in(reg, &progid, any_verb) {
            Some(Verb { command: Some(cmd), delegate: false }) if !appx => {
                return Some(Assoc { progid, command: Some(cmd), launchable: true });
            }
            Some(v) => {
                fallback.get_or_insert(Assoc { progid, command: v.command, launchable: false });
            }
            None if appx => {
                fallback.get_or_insert(Assoc { progid, command: None, launchable: false });
            }
            None => {}
        }
    }
    fallback
}

const URL_ASSOC: &str = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations";

/// The scheme of an absolute URL (`https` of `https://x`); `None` for paths (a drive
/// letter `C:` is not a scheme) and malformed input.
pub(crate) fn url_scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let ok = scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then(|| scheme.to_ascii_lowercase())
}

/// Launchable command registered for URL `scheme`: the user's choice first, then the
/// `HKCR\<scheme>` protocol key. COM/UWP-only handlers are not launchable (`None`).
pub(crate) fn url_command(reg: &dyn Registry, scheme: &str) -> Option<String> {
    let mut progids = Vec::new();
    if let Some(p) = reg.get(Hive::CurrentUser, &format!(r"{URL_ASSOC}\{scheme}\UserChoice"), Some("ProgId")) {
        progids.push(p);
    }
    if reg.has(Hive::ClassesRoot, scheme, "URL Protocol") {
        progids.push(scheme.to_string());
    }
    progids.into_iter().filter(|p| !p.is_empty() && !is_appx(p)).find_map(|p| match verb_command(reg, &p) {
        Some(Verb { command: Some(cmd), delegate: false }) => Some(cmd),
        _ => None,
    })
}

/// Split a registry command line into the executable and the raw rest.
/// Handles `"C:\Program Files\x\a.exe" "%1"`, `C:\Windows\notepad.exe %1` and unquoted paths
/// with spaces (`C:\Program Files\x\a.exe %1`, found by the first token ending in an
/// executable extension).
pub(crate) fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        return match rest.find('"') {
            Some(i) => (rest[..i].to_string(), rest[i + 1..].trim_start().to_string()),
            None => (rest.to_string(), String::new()),
        };
    }
    let lower = cmd.to_ascii_lowercase();
    for ext in [".exe", ".com", ".bat", ".cmd"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(ext) {
            let end = from + i + ext.len();
            if end == cmd.len() || cmd.as_bytes()[end] == b' ' {
                return (cmd[..end].to_string(), cmd[end..].trim_start().to_string());
            }
            from = end;
        }
    }
    match cmd.split_once(' ') {
        Some((exe, rest)) => (exe.to_string(), rest.trim_start().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

/// Expand `%1 %L %0 %V` (file), `%W` (its directory), drop `%* %I %D %H` and other `%n`;
/// `%%` → `%`. The file is quoted unless the placeholder already sits inside quotes, or
/// follows Chromium's `--single-argument` (Chrome / Edge register
/// `chrome.exe --single-argument %1`: everything after the switch is one verbatim argument,
/// so quotes would become part of the address, `http://"c/Users/…"`). When no file
/// placeholder occurs, ` "file"` is appended.
pub(crate) fn expand_args(rest: &str, file: &Path) -> String {
    let f = file.to_string_lossy();
    let dir = file.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = String::with_capacity(rest.len() + f.len() + 2);
    let mut used_file = false;
    let mut in_quotes = false;
    let mut it = rest.chars().peekable();
    while let Some(c) = it.next() {
        if c == '"' {
            in_quotes = !in_quotes;
            out.push(c);
            continue;
        }
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(&n) = it.peek() else {
            out.push('%');
            break;
        };
        let verbatim = in_quotes || out.to_ascii_lowercase().contains(SINGLE_ARGUMENT);
        let quoted = |s: &str| if verbatim { s.to_string() } else { format!("\"{s}\"") };
        match n {
            '1' | 'L' | 'l' | '0' | 'V' | 'v' => {
                it.next();
                out.push_str(&quoted(&f));
                used_file = true;
            }
            'W' | 'w' => {
                it.next();
                out.push_str(&quoted(&dir));
            }
            '*' | 'I' | 'i' | 'D' | 'd' | 'H' | 'h' | '2'..='9' => {
                it.next();
            }
            '%' => {
                it.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    let mut s = out.trim_end().to_string();
    if !used_file {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("\"{f}\""));
    }
    s
}

/// Chromium's switch that takes the rest of the command line as one verbatim argument.
const SINGLE_ARGUMENT: &str = "--single-argument";

/// A URL that can go into a registry command line as is: quotes, controls and line
/// breaks could end the argument and smuggle in others.
pub(crate) fn url_is_safe(url: &str) -> bool {
    !url.is_empty() && !url.chars().any(|c| c == '"' || c.is_control())
}

/// The file name of `exe` without its extension. Split by hand on both separators: registry
/// commands are Windows paths, and `Path` only knows `\` on Windows (the tests run everywhere).
fn exe_stem(exe: &str) -> String {
    let name = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    };
    if stem.is_empty() { exe.to_string() } else { stem.to_string() }
}

fn app_from_command(reg: &dyn Registry, id: &str, command: &str, launchable: bool, name_key: Option<&str>) -> AppInfo {
    let (exe, rest) = split_command(&crate::path::expand_env(command));
    let mui = reg.get(Hive::CurrentUser, MUI_CACHE, Some(&format!("{exe}.FriendlyAppName")));
    let name = name_key
        .and_then(|k| {
            reg.display(Hive::ClassesRoot, &format!(r"{k}\Application"), "ApplicationName")
                .or_else(|| reg.display(Hive::ClassesRoot, k, "FriendlyAppName"))
        })
        .or(mui)
        .unwrap_or_else(|| exe_stem(&exe));
    let icon = name_key
        .and_then(|k| reg.get(Hive::ClassesRoot, &format!(r"{k}\DefaultIcon"), None))
        .map(|i| PathBuf::from(i.rsplit_once(',').map_or(i.as_str(), |(p, _)| p).trim_matches('"').to_string()));
    AppInfo {
        id: id.to_string(),
        name,
        exec: PathBuf::from(exe),
        args: if rest.is_empty() { vec!["\"%1\"".into()] } else { vec![rest] },
        icon,
        launchable,
    }
}

/// Applications registered for `ext`: the default first, then OpenWithProgids /
/// OpenWithList entries, deduplicated by executable. Store apps are listed when they have
/// an execution alias; one without (COM-only) only as the default, not launchable (it
/// opens through `open_default`'s hand-off). A package's executable (`…\WindowsApps\
/// <package>\app.exe`) and its alias (`…\WindowsApps\<family>\app.exe`) count as one.
pub(crate) fn apps_for_ext(reg: &dyn Registry, ext: &str) -> Vec<AppInfo> {
    let ext = ext.to_ascii_lowercase();
    let mut out: Vec<AppInfo> = Vec::new();
    let key_of = |a: &AppInfo| {
        let path = a.exec.to_string_lossy().to_ascii_lowercase();
        if path.contains(r"\windowsapps\") { path.rsplit('\\').next().unwrap_or_default().to_string() } else { path }
    };
    let push = |out: &mut Vec<AppInfo>, app: AppInfo| {
        let key = key_of(&app);
        if !out.iter().any(|a| key_of(a) == key && a.launchable == app.launchable) {
            out.push(app);
        }
    };
    if let Some(a) = default_assoc(reg, &ext) {
        match &a.command {
            _ if is_appx(&a.progid) => push(&mut out, appx_app(reg, &a.progid)),
            Some(cmd) => push(&mut out, app_from_command(reg, &a.progid, cmd, a.launchable, Some(&a.progid))),
            None => push(
                &mut out,
                AppInfo {
                    id: a.progid.clone(),
                    name: reg.get(Hive::ClassesRoot, &a.progid, None).unwrap_or_else(|| a.progid.clone()),
                    exec: PathBuf::new(),
                    args: Vec::new(),
                    icon: None,
                    launchable: false,
                },
            ),
        }
    }
    let mut progids = reg.values(Hive::ClassesRoot, &format!(r"{ext}\OpenWithProgids"));
    progids.extend(reg.values(Hive::CurrentUser, &format!(r"{FILE_EXTS}\{ext}\OpenWithProgids")));
    for p in progids {
        if is_appx(&p) {
            let app = appx_app(reg, &p);
            if app.launchable {
                push(&mut out, app);
            }
            continue;
        }
        if let Some(Verb { command: Some(cmd), delegate: false }) = verb_command(reg, &p) {
            push(&mut out, app_from_command(reg, &p, &cmd, true, Some(&p)));
        }
    }
    let mut exes = reg.subkeys(Hive::ClassesRoot, &format!(r"{ext}\OpenWithList"));
    exes.extend(reg.subkeys(Hive::ClassesRoot, &format!(r"SystemFileAssociations\{ext}\OpenWithList")));
    let mru_key = format!(r"{FILE_EXTS}\{ext}\OpenWithList");
    for v in reg.values(Hive::CurrentUser, &mru_key) {
        if v.len() == 1
            && let Some(exe) = reg.get(Hive::CurrentUser, &mru_key, Some(&v))
        {
            exes.push(exe);
        }
    }
    for exe in exes {
        // The most-recently-used list also names Store apps by app id.
        if exe.contains('!') {
            if let Some(alias) = reg.app_alias(&exe) {
                let name = app_id_name(&exe).unwrap_or_else(|| exe.clone());
                let app =
                    AppInfo { id: exe, name, exec: alias, args: vec!["\"%1\"".into()], icon: None, launchable: true };
                push(&mut out, app);
            }
            continue;
        }
        let key = format!(r"Applications\{exe}");
        if reg.has(Hive::ClassesRoot, &key, "NoOpenWith") {
            continue;
        }
        let app = match verb_command(reg, &key) {
            Some(Verb { command: Some(cmd), delegate: false }) => {
                Some(app_from_command(reg, &exe, &cmd, true, Some(&key)))
            }
            _ => reg
                .get(Hive::LocalMachine, &format!(r"{APP_PATHS}\{exe}"), None)
                .or_else(|| reg.get(Hive::CurrentUser, &format!(r"{APP_PATHS}\{exe}"), None))
                .map(|path| {
                    app_from_command(reg, &exe, &format!("\"{}\" \"%1\"", path.trim_matches('"')), true, Some(&key))
                }),
        };
        if let Some(app) = app {
            push(&mut out, app);
        }
    }
    out
}

/// PE header subsystem (2 = GUI, 3 = console) of an executable.
pub(crate) fn pe_subsystem(bytes: &[u8]) -> Option<u16> {
    if bytes.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(bytes.get(0x3C..0x40)?.try_into().ok()?) as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    // COFF header (20 bytes) follows the signature; Subsystem is at offset 68 of the
    // optional header for both PE32 and PE32+.
    let off = pe + 4 + 20 + 68;
    Some(u16::from_le_bytes(bytes.get(off..off + 2)?.try_into().ok()?))
}

#[cfg(windows)]
pub(crate) use imp::*;

#[cfg(windows)]
mod imp {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use soyuz_core::{FsError, FsErrorKind, FsResult};

    use super::{AppInfo, Hive, Registry, apps_for_ext, default_assoc, expand_args, pe_subsystem, split_command};
    use crate::launch::OpenOptions;
    use crate::sys::reparse::{self, Reparse};
    use crate::sys::windows::fs as wfs;
    use crate::sys::windows::registry::{self as wr, Hive as WHive};

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    pub(crate) struct RealRegistry;

    fn h(hive: Hive) -> WHive {
        match hive {
            Hive::ClassesRoot => WHive::ClassesRoot,
            Hive::CurrentUser => WHive::CurrentUser,
            Hive::LocalMachine => WHive::LocalMachine,
        }
    }

    impl Registry for RealRegistry {
        fn get(&self, hive: Hive, key: &str, value: Option<&str>) -> Option<String> {
            wr::get_string(h(hive), key, value)
        }
        fn has(&self, hive: Hive, key: &str, value: &str) -> bool {
            wr::has_value(h(hive), key, value)
        }
        fn subkeys(&self, hive: Hive, key: &str) -> Vec<String> {
            wr::subkeys(h(hive), key)
        }
        fn values(&self, hive: Hive, key: &str) -> Vec<String> {
            wr::value_names(h(hive), key)
        }
        fn display(&self, hive: Hive, key: &str, value: &str) -> Option<String> {
            wr::mui_string(h(hive), key, value)
                .or_else(|| self.get(hive, key, Some(value)).filter(|s| !s.starts_with('@')))
        }
        fn app_alias(&self, app_id: &str) -> Option<PathBuf> {
            let (family, _) = app_id.split_once('!')?;
            if family.is_empty() || family.contains(['\\', '/']) || family.starts_with('.') {
                return None;
            }
            let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Microsoft\WindowsApps").join(family);
            let mut aliases: Vec<PathBuf> = std::fs::read_dir(&dir)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")))
                .collect();
            aliases.sort();
            // An alias names the app it starts; a package may hold several apps.
            aliases.into_iter().find(|p| {
                wfs::get_reparse_data(p).ok().and_then(|b| reparse::parse(&b).ok()).is_some_and(
                    |r| matches!(r, Reparse::AppExecLink { app_id: id, .. } if String::from_utf16_lossy(&id).eq_ignore_ascii_case(app_id)),
                )
            })
        }
    }

    fn plain(p: &Path) -> PathBuf {
        crate::path::strip_verbatim(&crate::path::normalize(p))
    }

    /// Console-subsystem executables get their own console window.
    fn is_console_exe(exe: &Path) -> bool {
        let mut buf = vec![0u8; 4096];
        let Ok(mut f) = std::fs::File::open(exe) else { return false };
        let n = f.read(&mut buf).unwrap_or(0);
        pe_subsystem(&buf[..n]) == Some(3)
    }

    /// Spawn `exe` with a raw (already quoted) argument string.
    pub(crate) fn spawn_raw(
        exe: &Path,
        raw_args: &str,
        cwd: Option<&Path>,
        op: &'static str,
        path: &Path,
    ) -> FsResult<()> {
        let mut c = Command::new(exe);
        if !raw_args.is_empty() {
            c.raw_arg(raw_args);
        }
        if let Some(d) = cwd.filter(|d| d.is_dir()) {
            c.current_dir(d);
        }
        let mut flags = CREATE_NEW_PROCESS_GROUP;
        if is_console_exe(exe) {
            flags |= CREATE_NEW_CONSOLE;
        }
        c.creation_flags(flags);
        super::super::spawn_detached(c, op, path)
    }

    /// `CreateProcessW` refuses a program whose manifest asks for administrator rights.
    const ERROR_ELEVATION_REQUIRED: i32 = 740;

    /// [`spawn_raw`] for a program the user opens; one that needs administrator rights
    /// starts through the UAC prompt (`[open] elevate_when_required`).
    fn spawn_program(exe: &Path, raw_args: &str, cwd: Option<&Path>, opts: &OpenOptions, path: &Path) -> FsResult<()> {
        match spawn_raw(exe, raw_args, cwd, "open", path) {
            Err(e) if e.code == Some(ERROR_ELEVATION_REQUIRED) && opts.elevate_when_required => {
                super::super::run_elevated_raw(exe, raw_args, cwd)
            }
            r => r,
        }
    }

    /// Error for a file type / link whose only handler would need the shell.
    fn no_association(path: &Path, what: &str) -> FsError {
        FsError::with_message(
            FsErrorKind::NoApplication,
            "open",
            Some(path),
            format!("no application is associated with {what} (use \u{201c}Open with\u{201d})"),
        )
    }

    /// Open `url` with the handler registered for its scheme.
    pub(crate) fn open_url(url: &str, path: &Path) -> FsResult<()> {
        if !super::url_is_safe(url) {
            return Err(FsError::with_message(FsErrorKind::InvalidName, "open", Some(path), "not a valid URL"));
        }
        let Some(scheme) = super::url_scheme(url) else {
            return Err(FsError::with_message(FsErrorKind::InvalidName, "open", Some(path), "not a valid URL"));
        };
        let Some(cmd) = super::url_command(&RealRegistry, &scheme) else {
            return Err(no_association(path, &format!("\u{201c}{scheme}:\u{201d} links")));
        };
        let (exe, rest) = split_command(&crate::path::expand_env(&cmd));
        let args = expand_args(&rest, Path::new(url));
        spawn_raw(Path::new(&exe), &args, None, "open", path)
    }

    fn ext_of(p: &Path) -> String {
        p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
    }

    pub(crate) fn open_default(path: &Path, opts: &OpenOptions) -> FsResult<()> {
        open_default_depth(path, opts, 0)
    }

    fn open_default_depth(path: &Path, opts: &OpenOptions, depth: u8) -> FsResult<()> {
        let e = crate::list::stat(path, true).map_err(|e| FsError { op: "open".into(), ..e })?;
        let p = plain(path);
        let dir = p.parent().map(Path::to_path_buf);
        if e.is_dir_like() {
            return spawn_raw(Path::new("explorer.exe"), &format!("\"{}\"", p.display()), None, "open", path);
        }
        match ext_of(&p).as_str() {
            "exe" | "com" | "scr" => spawn_program(&p, "", dir.as_deref(), opts, path),
            "bat" | "cmd" => {
                let mut c = Command::new("cmd.exe");
                c.raw_arg(format!("/c \"\"{}\"\"", p.display()));
                if let Some(d) = &dir {
                    c.current_dir(d);
                }
                c.creation_flags(CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP);
                super::super::spawn_detached(c, "open", path)
            }
            "lnk" if depth < 4 => match crate::launch::lnk::resolve(&p) {
                Ok((Some(target), sc)) if crate::list::exists_nofollow(&target) => {
                    let args = sc.arguments.unwrap_or_default();
                    let is_exe = matches!(ext_of(&target).as_str(), "exe" | "com");
                    if is_exe {
                        let cwd = sc
                            .working_dir
                            .map(|w| PathBuf::from(crate::path::expand_env(&w)))
                            .or_else(|| target.parent().map(Path::to_path_buf));
                        if sc.run_as_admin && opts.elevate_when_required {
                            super::super::run_elevated_raw(&target, &args, cwd.as_deref())
                        } else {
                            spawn_program(&target, &args, cwd.as_deref(), opts, path)
                        }
                    } else {
                        open_default_depth(&target, opts, depth + 1)
                    }
                }
                _ => Err(FsError::with_message(
                    FsErrorKind::NotFound,
                    "open",
                    Some(path),
                    "the shortcut's target could not be found",
                )),
            },
            "url" => {
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                match text.lines().find_map(|l| l.trim().strip_prefix("URL=")) {
                    Some(url) if super::url_is_safe(url.trim()) => open_url(url.trim(), path),
                    _ => {
                        Err(FsError::with_message(FsErrorKind::Corrupt, "open", Some(path), "no URL in this shortcut"))
                    }
                }
            }
            ext => {
                let reg = RealRegistry;
                let assoc = if ext.is_empty() { None } else { default_assoc(&reg, &format!(".{ext}")) };
                match assoc {
                    Some(a) if a.launchable => {
                        let cmd = a.command.unwrap_or_default();
                        let (exe, rest) = split_command(&crate::path::expand_env(&cmd));
                        let args = expand_args(&rest, &p);
                        spawn_program(Path::new(&exe), &args, dir.as_deref(), opts, path)
                    }
                    // A Store / COM-only default: Explorer (its own process) opens the file
                    // in it, as a double-click there would.
                    Some(_) if opts.store_apps_via_explorer => {
                        spawn_raw(Path::new("explorer.exe"), &format!("\"{}\"", p.display()), None, "open", path)
                    }
                    // Else the first registered program we can start ourselves.
                    Some(_) => match apps_for_ext(&reg, &format!(".{ext}")).into_iter().find(|a| a.launchable) {
                        Some(app) => open_with(path, &app),
                        None => Err(no_association(path, &format!("\u{201c}.{ext}\u{201d} files"))),
                    },
                    _ if ext.is_empty() => Err(no_association(path, "files without an extension")),
                    _ => Err(no_association(path, &format!("\u{201c}.{ext}\u{201d} files"))),
                }
            }
        }
    }

    pub(crate) fn open_with(path: &Path, app: &AppInfo) -> FsResult<()> {
        if !app.launchable || app.exec.as_os_str().is_empty() {
            return open_default(path, &OpenOptions::default());
        }
        crate::list::stat(path, true).map_err(|e| FsError { op: "open_with".into(), ..e })?;
        let p = plain(path);
        let mut parts: Vec<String> = Vec::new();
        let mut used = false;
        for a in &app.args {
            if ["%1", "%L", "%l", "%V", "%0"].iter().any(|ph| a.contains(ph)) {
                used = true;
                parts.push(expand_args(a, &p));
            } else if a.contains(char::is_whitespace) && !a.starts_with('"') && !a.contains('%') {
                parts.push(format!("\"{a}\""));
            } else {
                parts.push(a.clone());
            }
        }
        if !used {
            parts.push(format!("\"{}\"", p.display()));
        }
        let exe = PathBuf::from(crate::path::expand_env(&app.exec.to_string_lossy()));
        spawn_raw(&exe, &parts.join(" "), p.parent(), "open_with", path)
    }

    pub(crate) fn apps_for(path: &Path) -> Vec<AppInfo> {
        match path.extension() {
            Some(ext) => super::apps_for_ext(&RealRegistry, &format!(".{}", ext.to_string_lossy())),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct FakeReg {
        values: HashMap<(Hive, String, String), String>,
        keys: HashMap<(Hive, String), Vec<String>>,
        aliases: HashMap<String, PathBuf>,
    }

    impl FakeReg {
        fn set(&mut self, hive: Hive, key: &str, value: &str, data: &str) {
            self.values.insert((hive, key.to_lowercase(), value.to_lowercase()), data.to_string());
            // Register the key chain so subkey enumeration works.
            let parts: Vec<&str> = key.split('\\').collect();
            for i in 1..parts.len() {
                let parent = parts[..i].join("\\").to_lowercase();
                let child = parts[i].to_string();
                let e = self.keys.entry((hive, parent)).or_default();
                if !e.iter().any(|c| c.eq_ignore_ascii_case(&child)) {
                    e.push(child);
                }
            }
        }
    }

    impl Registry for FakeReg {
        fn get(&self, hive: Hive, key: &str, value: Option<&str>) -> Option<String> {
            self.values.get(&(hive, key.to_lowercase(), value.unwrap_or("").to_lowercase())).cloned()
        }
        fn has(&self, hive: Hive, key: &str, value: &str) -> bool {
            self.get(hive, key, Some(value)).is_some()
        }
        fn subkeys(&self, hive: Hive, key: &str) -> Vec<String> {
            self.keys.get(&(hive, key.to_lowercase())).cloned().unwrap_or_default()
        }
        fn values(&self, hive: Hive, key: &str) -> Vec<String> {
            let k = key.to_lowercase();
            self.values
                .keys()
                .filter(|(h, kk, v)| *h == hive && *kk == k && !v.is_empty())
                .map(|(_, _, v)| v.clone())
                .collect()
        }
        fn app_alias(&self, app_id: &str) -> Option<PathBuf> {
            self.aliases.get(app_id).cloned()
        }
    }

    use Hive::*;

    fn sample() -> FakeReg {
        let mut r = FakeReg::default();
        r.set(ClassesRoot, ".txt", "", "txtfile");
        r.set(ClassesRoot, ".txt", "PerceivedType", "text");
        r.set(ClassesRoot, r"txtfile\shell\open\command", "", r"%SystemRoot%\system32\NOTEPAD.EXE %1");
        r.set(CurrentUser, &format!(r"{FILE_EXTS}\.txt\UserChoice"), "ProgId", "Applications\\code.exe");
        r.set(ClassesRoot, r"Applications\code.exe\shell\open\command", "", r#""C:\Program Files\Code\Code.exe" "%1""#);
        r.set(ClassesRoot, r"Applications\code.exe", "FriendlyAppName", "Visual Studio Code");
        r.set(ClassesRoot, r".txt\OpenWithProgids", "txtfile", "");
        r.set(ClassesRoot, r".txt\OpenWithProgids", "AppX4ztfk9wxr86nxmzzq47px0nh0e58b8fw", "");
        r.set(ClassesRoot, r".txt\OpenWithList\wordpad.exe", "", "");
        r.set(
            LocalMachine,
            &format!(r"{APP_PATHS}\wordpad.exe"),
            "",
            r#""C:\Program Files\Windows NT\Accessories\wordpad.exe""#,
        );
        // A UWP-only association.
        r.set(ClassesRoot, ".jpg", "", "AppX43hnxtbyyps62jhe9sqpdzxn1790zetc");
        r.set(ClassesRoot, r"AppX43hnxtbyyps62jhe9sqpdzxn1790zetc\shell\open\command", "DelegateExecute", "{4ED3A719}");
        // Verb preference via shell\(Default).
        r.set(ClassesRoot, ".ps1", "", "Microsoft.PowerShellScript.1");
        r.set(ClassesRoot, r"Microsoft.PowerShellScript.1\shell", "", "Edit");
        r.set(
            ClassesRoot,
            r"Microsoft.PowerShellScript.1\shell\Edit\command",
            "",
            r#""C:\Windows\System32\powershell_ise.exe" "%1""#,
        );
        r.set(ClassesRoot, r"Microsoft.PowerShellScript.1\shell\open\command", "", "notepad.exe \"%1\"");
        r
    }

    #[test]
    fn association_chain() {
        let r = sample();
        let a = default_assoc(&r, ".TXT").unwrap();
        assert_eq!(a.progid, r"Applications\code.exe");
        assert!(a.launchable);
        let j = default_assoc(&r, ".jpg").unwrap();
        assert!(!j.launchable, "DelegateExecute/AppX are not launchable");
        let ps = default_assoc(&r, ".ps1").unwrap();
        assert!(ps.command.unwrap().contains("powershell_ise"));
        assert!(default_assoc(&r, ".nothing").is_none());
        // Without a user choice the HKCR default ProgId wins.
        let mut r2 = sample();
        r2.values.retain(|(h, _, v), _| !(*h == CurrentUser && v == "progid"));
        assert_eq!(default_assoc(&r2, ".txt").unwrap().progid, "txtfile");
    }

    /// `.jpg` as a Windows 11 machine has it: Photos (a COM-only Store app) as the user's
    /// choice, `jpegfile` with only a `printto` verb, menu verbs under
    /// `SystemFileAssociations`, GIMP and Paint (with an alias) as other programs.
    fn photos_machine() -> FakeReg {
        let mut r = FakeReg::default();
        let photos = "AppX4mntx4h978m1v9gtzv0ewksfd6pmwsre";
        let paint = "AppXcesbfs704v2mjbts9dkr42s9vmrhxbkj";
        r.set(CurrentUser, &format!(r"{FILE_EXTS}\.jpg\UserChoice"), "ProgId", "AppX43hnxtbyyps62jhe9sqpdzxn1790zetc");
        r.set(CurrentUser, &format!(r"{FILE_EXTS}\.jpg\UserChoiceLatest\ProgId"), "ProgId", photos);
        for p in [photos, paint] {
            r.set(ClassesRoot, &format!(r"{p}\Shell\open\command"), "DelegateExecute", "{BFEC0C93}");
            r.set(
                ClassesRoot,
                &format!(r"{p}\Application"),
                "ApplicationName",
                "@{Pkg?ms-resource://x/AppDisplayName}",
            );
        }
        r.set(
            ClassesRoot,
            &format!(r"{photos}\Application"),
            "AppUserModelID",
            "Microsoft.Windows.Photos_8wekyb3d8bbwe!App",
        );
        r.set(ClassesRoot, &format!(r"{paint}\Application"), "AppUserModelID", "Microsoft.Paint_8wekyb3d8bbwe!App");
        r.aliases.insert("Microsoft.Paint_8wekyb3d8bbwe!App".into(), PathBuf::from(r"C:\WindowsApps\mspaint.exe"));
        r.set(ClassesRoot, ".jpg", "", "jpegfile");
        r.set(ClassesRoot, ".jpg", "PerceivedType", "image");
        r.set(ClassesRoot, r"jpegfile\shell\printto\command", "", r#""C:\print.exe" /pt "%1" "%2""#);
        r.set(ClassesRoot, r"SystemFileAssociations\.jpg\Shell\PdfCraft.CreatePdf\command", "", r#""C:\pdf.exe" "%1""#);
        r.set(ClassesRoot, r"SystemFileAssociations\image\shell\print\command", "", r#""C:\print.exe" %1"#);
        for p in [photos, paint, "GIMP3.jpg", "jpegfile"] {
            r.set(ClassesRoot, r".jpg\OpenWithProgids", p, "");
        }
        r.set(ClassesRoot, r"GIMP3.jpg\shell\open\command", "", r#""C:\GIMP 3\bin\gimp-3.exe" "%1""#);
        r
    }

    #[test]
    fn store_app_defaults() {
        let mut r = photos_machine();
        // Photos (UserChoiceLatest) is the default; printing and menu verbs never are.
        let a = default_assoc(&r, ".jpg").unwrap();
        assert_eq!(a.progid, "AppX4mntx4h978m1v9gtzv0ewksfd6pmwsre");
        assert!(!a.launchable);
        let apps = apps_for_ext(&r, ".jpg");
        let mut names: Vec<(&str, bool)> = apps.iter().map(|a| (a.name.as_str(), a.launchable)).collect();
        assert_eq!(names[0], ("Photos", false), "the default first");
        names.sort();
        assert_eq!(names, [("Paint", true), ("Photos", false), ("gimp-3", true)]);
        assert!(!apps.iter().any(|a| a.exec.to_string_lossy().contains("print")), "{apps:?}");
        let paint = apps.iter().find(|a| a.name == "Paint").unwrap();
        assert_eq!(paint.exec, PathBuf::from(r"C:\WindowsApps\mspaint.exe"));
        // A Store default with an alias starts directly.
        r.set(
            CurrentUser,
            &format!(r"{FILE_EXTS}\.jpg\UserChoiceLatest\ProgId"),
            "ProgId",
            "AppXcesbfs704v2mjbts9dkr42s9vmrhxbkj",
        );
        let a = default_assoc(&r, ".jpg").unwrap();
        assert!(a.launchable);
        assert_eq!(a.command.as_deref(), Some(r#""C:\WindowsApps\mspaint.exe" "%1""#));
        // Without Store apps, a printing-only ProgId and menu verbs leave no default.
        let mut r = photos_machine();
        r.values.retain(|(h, _, v), _| !(*h == CurrentUser && v == "progid"));
        assert!(default_assoc(&r, ".jpg").is_none());
        assert_eq!(app_id_name("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App").as_deref(), Some("Notepad"));
        assert_eq!(app_id_name("Microsoft.WindowsAlarms_8wekyb3d8bbwe!App").as_deref(), Some("Alarms"));
        assert_eq!(app_id_name("Contoso.Windowsill_abc!App").as_deref(), Some("Windowsill"));
    }

    #[test]
    fn url_scheme_association() {
        assert_eq!(url_scheme("https://example.com/a").as_deref(), Some("https"));
        let url = Path::new("https://example.com/a?b=c&d=e");
        assert_eq!(expand_args("--single-argument %1", url), "--single-argument https://example.com/a?b=c&d=e");
        assert_eq!(expand_args(r#"-osint -url "%1""#, url), r#"-osint -url "https://example.com/a?b=c&d=e""#);
        assert!(url_is_safe("https://example.com/a b"));
        assert!(!url_is_safe("https://x\" --evil"));
        assert!(!url_is_safe("https://x\r\ny"));
        assert!(!url_is_safe(""));
        assert_eq!(url_scheme("MAILTO:x@y").as_deref(), Some("mailto"));
        assert_eq!(url_scheme(r"C:\dir\file.txt"), None, "a drive letter is not a scheme");
        assert_eq!(url_scheme("no scheme"), None);
        let mut r = FakeReg::default();
        r.set(CurrentUser, &format!(r"{URL_ASSOC}\https\UserChoice"), "ProgId", "FirefoxURL");
        r.set(ClassesRoot, r"FirefoxURL\shell\open\command", "", r#""C:\ff\firefox.exe" -osint -url "%1""#);
        r.set(ClassesRoot, "ftp", "URL Protocol", "");
        r.set(ClassesRoot, r"ftp\shell\open\command", "", r#""C:\ftp.exe" %1"#);
        r.set(ClassesRoot, "ms-photos", "URL Protocol", "");
        r.set(ClassesRoot, r"ms-photos\shell\open\command", "DelegateExecute", "{4ED3A719}");
        r.set(CurrentUser, &format!(r"{URL_ASSOC}\mailto\UserChoice"), "ProgId", "AppXabc");
        assert_eq!(url_command(&r, "https").unwrap(), r#""C:\ff\firefox.exe" -osint -url "%1""#);
        assert_eq!(url_command(&r, "ftp").unwrap(), r#""C:\ftp.exe" %1"#);
        assert_eq!(url_command(&r, "ms-photos"), None, "COM-only handler: no association");
        assert_eq!(url_command(&r, "mailto"), None, "UWP-only handler: no association");
        assert_eq!(url_command(&r, "gopher"), None);
    }

    // Windows paths (`C:\…` parents, drive-absolute) only parse as such on Windows.
    #[cfg(windows)]
    #[test]
    fn command_line_splitting_and_expansion() {
        assert_eq!(
            split_command(r#""C:\Program Files\App\a.exe" "%1" /x"#),
            (r"C:\Program Files\App\a.exe".into(), r#""%1" /x"#.into())
        );
        assert_eq!(split_command(r"C:\Windows\notepad.exe %1"), (r"C:\Windows\notepad.exe".into(), "%1".into()));
        assert_eq!(
            split_command(r"C:\Program Files\x\app.exe %1"),
            (r"C:\Program Files\x\app.exe".into(), "%1".into())
        );
        assert_eq!(split_command("notepad"), ("notepad".into(), String::new()));
        let f = Path::new(r"C:\My Docs\a b.txt");
        assert_eq!(expand_args("%1", f), r#""C:\My Docs\a b.txt""#);
        assert_eq!(expand_args(r#""%1""#, f), r#""C:\My Docs\a b.txt""#);
        assert_eq!(expand_args("/n %L %*", f), r#"/n "C:\My Docs\a b.txt""#);
        assert_eq!(expand_args("/dir=%W", f), r#"/dir="C:\My Docs" "C:\My Docs\a b.txt""#);
        assert_eq!(expand_args("-x", f), r#"-x "C:\My Docs\a b.txt""#);
        assert_eq!(expand_args("", f), r#""C:\My Docs\a b.txt""#);
        assert_eq!(expand_args("100%% %1", f), r#"100% "C:\My Docs\a b.txt""#);
        // Chrome / Edge: `--single-argument %1` takes the rest verbatim, so no quotes.
        assert_eq!(expand_args("--single-argument %1", f), r"--single-argument C:\My Docs\a b.txt");
        assert_eq!(expand_args("--Single-Argument %1", f), r"--Single-Argument C:\My Docs\a b.txt");
    }

    // Windows paths (`C:\…` parents, drive-absolute) only parse as such on Windows.
    #[cfg(windows)]
    #[test]
    fn open_with_list() {
        let r = sample();
        let apps = apps_for_ext(&r, ".txt");
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names[0], "Visual Studio Code");
        assert!(names.contains(&"NOTEPAD"), "{names:?}");
        assert!(names.contains(&"wordpad"), "{names:?}");
        assert!(apps.iter().all(|a| a.launchable));
        assert!(!apps.iter().any(|a| a.id.starts_with("AppX")));
        let code = &apps[0];
        assert_eq!(code.exec, PathBuf::from(r"C:\Program Files\Code\Code.exe"));
        assert_eq!(code.args, vec![r#""%1""#.to_string()]);
        let jpg = apps_for_ext(&r, ".jpg");
        assert_eq!(jpg.len(), 1);
        assert!(!jpg[0].launchable);
    }

    #[test]
    fn pe_subsystem_detection() {
        let mut pe = vec![0u8; 512];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        let off = 0x80 + 4 + 20 + 68;
        pe[off] = 3;
        assert_eq!(pe_subsystem(&pe), Some(3));
        pe[off] = 2;
        assert_eq!(pe_subsystem(&pe), Some(2));
        assert_eq!(pe_subsystem(b"not an exe"), None);
        #[cfg(windows)]
        {
            let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            if let Ok(b) = std::fs::read(Path::new(&sys).join("System32").join("cmd.exe")) {
                assert_eq!(pe_subsystem(&b), Some(3));
            }
            if let Ok(b) = std::fs::read(Path::new(&sys).join("notepad.exe")) {
                assert_eq!(pe_subsystem(&b), Some(2));
            }
        }
    }
}
