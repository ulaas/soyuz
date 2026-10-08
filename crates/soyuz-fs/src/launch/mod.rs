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
/// on Windows, `pkexec` on Linux; see PLAN §4). `Unsupported` on macOS.
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
