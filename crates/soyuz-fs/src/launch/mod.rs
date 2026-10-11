//! Launching files and applications. All spawning goes through `std::process::Command`
//! (kernel32 `CreateProcessW` under the hood on Windows; `CommandExt::raw_arg` /
//! `creation_flags` where needed). No shell execution APIs are used in this process.
//!
//! * Windows: registry association chain → `CreateProcessW` (also for `.url` schemes);
//!   UWP / `DelegateExecute` (COM-only) associations and unresolvable shortcuts fail with
//!   a "no application is associated" error — nothing is handed to a shell helper.
//! * Linux/BSD: no `xdg-open` / `gio`: shared-mime-info type detection, `mimeapps.list`
//!   and `.desktop` resolution, then the entry is launched directly (D-Bus activation,
//!   `Exec=`, terminal wrapping); the OpenURI portal inside Flatpak / Snap;
//!   `org.freedesktop.FileManager1` for "show in folder" (see `linux`).
//! * macOS: LaunchServices (`LSOpenFromURLSpec`, `LSCopyApplicationURLsForURL`) and
//!   `NSWorkspace` for Finder reveal; no `open` process (see `macos`).

pub(crate) mod linux;
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
        linux::open_default(path)
    }
}

/// Open a URL (`https:`, `mailto:`, `file:` …) with the application registered for its
/// scheme. Linux / BSD: `x-scheme-handler/<scheme>` (the OpenURI portal when sandboxed);
/// macOS: LaunchServices; Windows: `file:` URLs only (other schemes go through the app's
/// own URL opener there).
pub fn open_url(url: &str) -> FsResult<()> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::open_url(url)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_url(url)
    }
    #[cfg(windows)]
    {
        match url.strip_prefix("file:///") {
            Some(rest) => open_default(&linux::uri::path_from_bytes(linux::uri::percent_decode(rest))),
            None => Err(err::unsupported("open", Path::new(url), "only file: URLs are opened here")),
        }
    }
}

/// Open several files with one application (one process when the application takes
/// several files, else one per file).
pub fn open_with_many(paths: &[PathBuf], app: &AppInfo) -> FsResult<()> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::open_with_many(paths, app)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_with_many(paths, app)
    }
    #[cfg(windows)]
    {
        paths.iter().try_for_each(|p| open_with(p, app))
    }
}

/// The application [`open_default`] would use for `path` (Linux / BSD / macOS; `None` on
/// Windows or when nothing is associated).
pub fn default_app_for(path: &Path) -> Option<AppInfo> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::default_app_for(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::default_app_for(path)
    }
    #[cfg(windows)]
    {
        let _ = path;
        None
    }
}

/// The MIME type of `path` from the shared-mime-info database (names, then content
/// sniffing; directories are `inode/directory`). Linux / BSD only; `None` elsewhere.
pub fn mime_type(path: &Path) -> Option<String> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::mime_of(path)
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

/// The file of a freedesktop icon-theme icon (`org.gnome.TextEditor`, `folder`, or an
/// absolute path) at `size` logical px and `scale`, from the user's icon theme, its
/// parents, `hicolor` and `pixmaps`. PNG, SVG or XPM; Linux / BSD only.
pub fn icon_path(name: &str, size: u32, scale: u32) -> Option<PathBuf> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::icon_path(name, size, scale)
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let _ = (name, size, scale);
        None
    }
}

/// The theme icon file for a MIME type (`text/x-csrc` → `text-x-csrc`, its generic
/// icon, …). Linux / BSD only.
pub fn mime_icon_path(mime: &str, size: u32, scale: u32) -> Option<PathBuf> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::mime_icon_path(mime, size, scale)
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let _ = (mime, size, scale);
        None
    }
}

/// Whether a desktop file manager (other than this process) answers
/// `org.freedesktop.FileManager1` on the session bus, i.e. whether [`reveal_in_system`]
/// can select the item. Linux only; blocks for at most a few seconds (background thread).
pub fn system_file_manager_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::file_manager1_available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        cfg!(any(windows, target_os = "macos"))
    }
}

/// The local path of a `file://` URI (percent-decoded, non-UTF-8 safe; scheme matched
/// case-insensitively); `None` for other schemes / remote hosts. Windows: `file:///C:/x`
/// → `C:\x` and `file://host/share/x` → `\\host\share\x`. For command-line arguments received through a desktop
/// entry's `%U` (`soyuz.desktop` uses `Exec=soyuz %U`).
pub fn path_from_file_uri(uri: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        linux::uri::uri_to_windows_path(uri).map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        linux::uri::uri_to_path(uri)
    }
}

/// The `file://` URI of a local path (percent-encoded).
pub fn file_uri(path: &Path) -> String {
    linux::uri::file_uri(path)
}

/// An activation token (Wayland `xdg_activation_v1` / X11 startup id) to hand to the next
/// application launched from here, so it may take focus. Single use; Linux / BSD only.
pub fn set_activation_token(token: Option<String>) {
    #[cfg(not(any(windows, target_os = "macos")))]
    linux::set_activation_token(token);
    #[cfg(any(windows, target_os = "macos"))]
    let _ = token;
}

/// Open `path` with a specific application (from [`apps_for`] or user-defined).
pub fn open_with(path: &Path, app: &AppInfo) -> FsResult<()> {
    #[cfg(windows)]
    {
        windows::open_with(path, app)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_with_many(&[path.to_path_buf()], app)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::open_with_many(&[path.to_path_buf()], app)
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
        if let Some(r) = substitute_placeholders(a, &p) {
            substituted = true;
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

/// `arg` with every `%1 %L %f %F %u %U` replaced by `path` in a single pass (a path that
/// itself contains e.g. `%U` is inserted verbatim, never substituted again); `None` when
/// `arg` has no placeholder.
#[cfg_attr(windows, allow(dead_code))]
fn substitute_placeholders(arg: &str, path: &str) -> Option<String> {
    let mut out = String::with_capacity(arg.len() + path.len());
    let mut found = false;
    let mut rest = arg;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.chars().next() {
            Some('1' | 'L' | 'f' | 'F' | 'u' | 'U') => {
                out.push_str(path);
                found = true;
                rest = &after[1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    found.then_some(out)
}

/// Applications registered for `path`'s type (default first). macOS: every LaunchServices
/// role, `exec` is the `.app` bundle.
pub fn apps_for(path: &Path) -> Vec<AppInfo> {
    #[cfg(windows)]
    {
        windows::apps_for(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::apps_for(path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::apps_for(path)
    }
}

/// Spawn and reap in the background so no zombie is left behind on unix. On unix the
/// child gets its own process group (no signals from our terminal / group reach it) and
/// no stdin.
pub(crate) fn spawn_detached(mut cmd: Command, op: &'static str, path: &Path) -> FsResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0).stdin(std::process::Stdio::null());
    }
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

/// Open an external terminal in `dir` (`preferred` = executable name / path, may carry
/// arguments on Linux). Linux / BSD: `xdg-terminal-exec`, `$TERMINAL`, then the known
/// emulators (ptyxis, kgx, gnome-terminal, konsole, …) with their working-directory flag.
/// macOS: `preferred` as an app bundle (path or name) or program, else Terminal.app, via
/// LaunchServices.
pub fn open_terminal_at(dir: &Path, preferred: Option<&str>) -> FsResult<()> {
    let dir_plain = crate::path::strip_verbatim(dir);
    let dir = dir_plain.as_path();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux::open_terminal_at(dir, preferred)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open_terminal_at(dir, preferred)
    }
    #[cfg(not(unix))]
    {
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
        let mut last_err = None;
        for cmd in candidates {
            match spawn_detached(cmd, "open_terminal", dir) {
                Ok(()) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| FsError::unsupported("open_terminal", Some(dir))))
    }
}

/// Quote `s` as a PowerShell single-quoted string literal. PowerShell treats the
/// typographic quotes `‘ ’ ‚ ‛` exactly like `'`, so every one of them is doubled too;
/// nothing else is special inside single quotes (no `$`, backtick or subexpression).
pub fn powershell_quote(s: &str) -> String {
    let mut q = String::with_capacity(s.len() + 2);
    q.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            q.push(c);
        }
        q.push(c);
    }
    q.push('\'');
    q
}

/// Encode a script for `powershell.exe -EncodedCommand` (base64 of UTF-16LE). This keeps
/// the script away from both the CRT and PowerShell's own command-line parsing.
pub fn powershell_encoded_command(script: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Quote one argument for a Windows command line as the MSVC CRT /
/// `CommandLineToArgvW` parse it (backslashes before a `"` are doubled).
pub fn windows_quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// The `Start-Process -Verb RunAs` script for [`run_elevated`]. `-ArgumentList` gets one
/// pre-quoted command-line string (Start-Process joins list items with bare spaces, which
/// would split arguments containing spaces).
#[cfg_attr(not(windows), allow(dead_code))]
fn elevate_script(exe: &str, args: &[String]) -> String {
    let mut script = format!("Start-Process -Verb RunAs -FilePath {}", powershell_quote(exe));
    if !args.is_empty() {
        let line: Vec<String> = args.iter().map(|a| windows_quote_arg(a)).collect();
        script.push_str(&format!(" -ArgumentList {}", powershell_quote(&line.join(" "))));
    }
    script
}

/// Run `exe` elevated (UAC prompt via an external `powershell Start-Process -Verb RunAs`
/// on Windows, `pkexec` on Linux). `Unsupported` on macOS: there is no supported
/// non-deprecated way to start an elevated process without a privileged helper tool
/// (`AuthorizationExecuteWithPrivileges` is deprecated); use `sudo` in a terminal.
pub fn run_elevated(exe: &Path, args: &[String]) -> FsResult<()> {
    #[cfg(windows)]
    {
        let script = elevate_script(&crate::path::strip_verbatim(exe).to_string_lossy(), args);
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand"])
            .arg(powershell_encoded_command(&script));
        spawn_detached(c, "run_elevated", exe)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let argv = linux::elevate_argv(exe, args, |k| std::env::var_os(k));
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        spawn_detached(c, "run_elevated", exe)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = args;
        Err(err::unsupported("run_elevated", exe, "elevation is not supported on this platform"))
    }
}

/// `explorer.exe /select,`; macOS: `NSWorkspace activateFileViewerSelectingURLs`; Linux / BSD: `org.freedesktop.FileManager1`
/// `ShowItems`, else the parent folder with the default folder handler (never Soyuz).
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
        macos::reveal(path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        linux::reveal(path)
    }
}

/// Target of a Windows `.lnk` (MS-SHLLINK parser, any OS) / `.desktop` `Exec=` or `URL=`
/// (unix); a Finder alias (macOS, bookmark data); other files → `None`.
pub fn shortcut_target(path: &Path) -> FsResult<Option<PathBuf>> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "lnk" => match lnk::resolve(path) {
            Ok((target, _)) => Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(e) => Err(err::io("shortcut_target", path, e)),
        },
        "desktop" if cfg!(unix) => linux::desktop_target(path).map_err(err::map("shortcut_target", path)),
        #[cfg(target_os = "macos")]
        _ => Ok(macos::alias_target(path)),
        #[cfg(not(target_os = "macos"))]
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_substituted_in_one_pass() {
        let sub = |a: &str, p: &str| substitute_placeholders(a, p);
        assert_eq!(sub("%f", "/t/100%Ugly.txt").as_deref(), Some("/t/100%Ugly.txt"));
        assert_eq!(sub("--file=%U", "C:\\a%L%f%1.txt").as_deref(), Some("--file=C:\\a%L%f%1.txt"));
        assert_eq!(sub("%1 and %L", "x").as_deref(), Some("x and x"));
        assert_eq!(sub("100%", "x"), None);
        assert_eq!(sub("--no-placeholder %d", "x"), None);
        assert_eq!(sub("50%%f", "x").as_deref(), Some("50%x"));
    }

    #[test]
    fn powershell_quoting_covers_typographic_quotes() {
        assert_eq!(powershell_quote("a b"), "'a b'");
        assert_eq!(powershell_quote("it's"), "'it''s'");
        assert_eq!(powershell_quote("it’s"), "'it’’s'");
        assert_eq!(powershell_quote("‘x‛‚"), "'‘‘x‛‛‚‚'");
        assert_eq!(powershell_quote("$(calc) `n"), "'$(calc) `n'");
        assert_eq!(powershell_quote(""), "''");
    }

    #[test]
    fn encoded_command_is_utf16le_base64() {
        assert_eq!(powershell_encoded_command("dir"), "ZABpAHIA");
        assert_eq!(powershell_encoded_command(""), "");
        assert_eq!(powershell_encoded_command("a"), "YQA=");
    }

    #[test]
    fn windows_arg_quoting() {
        assert_eq!(windows_quote_arg("plain"), "plain");
        assert_eq!(windows_quote_arg(""), "\"\"");
        assert_eq!(windows_quote_arg("a b"), "\"a b\"");
        assert_eq!(windows_quote_arg(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(windows_quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(windows_quote_arg(r#"a\"b c"#), r#""a\\\"b c""#);
    }

    #[test]
    fn elevate_script_quotes_hostile_names() {
        let s = elevate_script(r"C:\it’s; calc ‘.exe", &["a b".into(), "x’;calc;’".into()]);
        assert_eq!(
            s,
            "Start-Process -Verb RunAs -FilePath 'C:\\it’’s; calc ‘‘.exe' -ArgumentList '\"a b\" x’’;calc;’’'"
        );
    }

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
