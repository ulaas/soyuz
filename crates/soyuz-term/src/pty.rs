//! Shell resolution, shell-integration injection, `cd` command quoting, the prompt
//! heuristic used by `send_cd`, and cwd polling. The PTY itself is created by
//! `alacritty_terminal::tty` (ConPTY on Windows, openpty on unix) in `backend.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{TermError, TermSettings};

/// Prefix of the window title emitted by the injected prompt hook; the rest is the cwd.
pub(crate) const CWD_TITLE_MARKER: &str = "soyuz-cwd:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    Zsh,
    Fish,
    /// sh/dash/ksh/anything else: POSIX quoting, no integration.
    Posix,
}

impl ShellKind {
    pub(crate) fn detect(program: &str) -> ShellKind {
        let stem = Path::new(program.trim_matches('"'))
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match stem.as_str() {
            "pwsh" | "powershell" | "pwsh-preview" => ShellKind::PowerShell,
            "cmd" => ShellKind::Cmd,
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            _ => ShellKind::Posix,
        }
    }
}

/// Everything needed to start the child process.
#[derive(Clone, Debug)]
pub(crate) struct ShellCommand {
    pub program: String,
    pub args: Vec<String>,
    pub kind: ShellKind,
    pub env: HashMap<String, String>,
    pub cwd: Option<PathBuf>,
}

/// Resolve program, arguments, environment and working directory from the settings.
pub(crate) fn shell_command(settings: &TermSettings) -> Result<ShellCommand, TermError> {
    let program = match settings.shell.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(requested) => which(requested).ok_or_else(|| TermError::ShellNotFound(requested.to_owned()))?,
        None => default_shell().ok_or_else(|| TermError::ShellNotFound("default shell".to_owned()))?,
    };
    let kind = ShellKind::detect(&program);

    let mut env: HashMap<String, String> = HashMap::new();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    env.insert("TERM_PROGRAM".into(), "soyuz".into());
    for (k, v) in &settings.env {
        env.insert(k.clone(), v.clone());
    }

    let mut args = settings.args.clone();
    if settings.shell_integration {
        inject_integration(kind, &mut args, &mut env, settings);
    }

    let cwd = std::path::absolute(&settings.cwd).ok().filter(|p| p.is_dir());

    Ok(ShellCommand { program, args, kind, env, cwd })
}

fn inject_integration(kind: ShellKind, args: &mut Vec<String>, env: &mut HashMap<String, String>, s: &TermSettings) {
    let user_env = |k: &str| s.env.iter().any(|(key, _)| key.eq_ignore_ascii_case(k));
    match kind {
        ShellKind::PowerShell if args.is_empty() => {
            args.extend(["-NoLogo".into(), "-NoExit".into(), "-EncodedCommand".into(), encode_ps_command(PS_HOOK)]);
        }
        ShellKind::Cmd if !user_env("PROMPT") => {
            // `$E]0;...$E\` is an OSC 0 title terminated by ST; `$P$G` is the visible prompt.
            env.insert("PROMPT".into(), format!("$E]0;{CWD_TITLE_MARKER}$P$E\\$P$G"));
        }
        ShellKind::Bash if args.is_empty() => {
            if let Some(rc) = write_support_file("bashrc", BASH_HOOK) {
                args.extend(["--rcfile".into(), rc.to_string_lossy().into_owned(), "-i".into()]);
            }
        }
        ShellKind::Zsh if !user_env("ZDOTDIR") => {
            let dir = support_dir().map(|d| d.join("zsh"));
            if let Some(dir) = dir.filter(|d| std::fs::create_dir_all(d).is_ok())
                && std::fs::write(dir.join(".zshenv"), ZSH_ENV).is_ok()
                && std::fs::write(dir.join(".zshrc"), ZSH_RC).is_ok()
            {
                let orig = std::env::var("ZDOTDIR").unwrap_or_default();
                env.insert("SOYUZ_ORIG_ZDOTDIR".into(), orig);
                env.insert("ZDOTDIR".into(), dir.to_string_lossy().into_owned());
            }
        }
        ShellKind::Fish if args.is_empty() => {
            args.extend(["-C".into(), FISH_HOOK.into()]);
        }
        _ => {}
    }
}

const PS_HOOK: &str = r#"$global:__soyuz_prompt = $function:prompt
function global:prompt {
  $out = if ($global:__soyuz_prompt) { & $global:__soyuz_prompt } else { "PS $($executionContext.SessionState.Path.CurrentLocation)$('>' * ($nestedPromptLevel + 1)) " }
  $loc = $executionContext.SessionState.Path.CurrentLocation
  if ($loc.Provider.Name -eq 'FileSystem') { try { $Host.UI.RawUI.WindowTitle = 'soyuz-cwd:' + $loc.ProviderPath } catch {} }
  $out
}
"#;

const BASH_HOOK: &str = r#"[ -f /etc/bash.bashrc ] && . /etc/bash.bashrc
[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"
__soyuz_cwd() { printf '\033]0;soyuz-cwd:%s\007' "$PWD"; }
PROMPT_COMMAND="${PROMPT_COMMAND}"$'\n'"__soyuz_cwd"
"#;

const ZSH_ENV: &str = r#"__soyuz_dir="$ZDOTDIR"
if [ -n "$SOYUZ_ORIG_ZDOTDIR" ]; then ZDOTDIR="$SOYUZ_ORIG_ZDOTDIR"; else unset ZDOTDIR; fi
[ -f "${ZDOTDIR:-$HOME}/.zshenv" ] && . "${ZDOTDIR:-$HOME}/.zshenv"
SOYUZ_ORIG_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$__soyuz_dir"
"#;

const ZSH_RC: &str = r#"if [ -n "$SOYUZ_ORIG_ZDOTDIR" ]; then ZDOTDIR="$SOYUZ_ORIG_ZDOTDIR"; else unset ZDOTDIR; fi
unset SOYUZ_ORIG_ZDOTDIR __soyuz_dir
[ -f "${ZDOTDIR:-$HOME}/.zshrc" ] && . "${ZDOTDIR:-$HOME}/.zshrc"
__soyuz_cwd() { printf '\033]0;soyuz-cwd:%s\007' "$PWD" }
precmd_functions+=(__soyuz_cwd)
"#;

const FISH_HOOK: &str = r"function __soyuz_cwd --on-event fish_prompt; printf '\e]0;soyuz-cwd:%s\a' $PWD; end";

fn support_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("soyuz-term-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn write_support_file(name: &str, content: &str) -> Option<PathBuf> {
    let path = support_dir()?.join(name);
    std::fs::write(&path, content).ok()?;
    Some(path)
}

/// `-EncodedCommand` payload: base64 of the UTF-16LE script (no quoting problems).
pub(crate) fn encode_ps_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Default shell: Windows `pwsh` → `powershell` → `%COMSPEC%` → `cmd.exe`; unix `$SHELL` → `/bin/sh`.
pub(crate) fn default_shell() -> Option<String> {
    if cfg!(windows) {
        if let Some(p) = which("pwsh.exe").or_else(|| which("powershell.exe")) {
            return Some(p);
        }
        if let Some(root) = std::env::var_os("SystemRoot") {
            let ps = PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
            if ps.is_file() {
                return Some(ps.to_string_lossy().into_owned());
            }
        }
        if let Some(comspec) = std::env::var("COMSPEC").ok().filter(|c| Path::new(c).is_file()) {
            return Some(comspec);
        }
        which("cmd.exe")
    } else {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty() && Path::new(s).is_file())
            .or_else(|| Path::new("/bin/sh").is_file().then(|| "/bin/sh".to_owned()))
    }
}

/// `which`-style lookup. Names containing a path separator are checked directly.
pub(crate) fn which(name: &str) -> Option<String> {
    let name = name.trim_matches('"');
    let path = Path::new(name);
    let exts: Vec<String> = if cfg!(windows) && path.extension().is_none() {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        Vec::new()
    };
    let candidates = |base: &Path| -> Option<PathBuf> {
        if base.is_file() {
            return Some(base.to_path_buf());
        }
        exts.iter().map(|e| PathBuf::from(format!("{}{e}", base.display()))).find(|p| p.is_file())
    };
    if path.is_absolute() || name.contains('/') || (cfg!(windows) && name.contains('\\')) {
        return candidates(path).map(|p| p.to_string_lossy().into_owned());
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .filter(|d| !d.as_os_str().is_empty())
        .find_map(|d| candidates(&d.join(name)))
        .map(|p| p.to_string_lossy().into_owned())
}

/// The text typed by `send_cd` (including the trailing `\r`).
pub(crate) fn cd_command(kind: ShellKind, dir: &Path) -> String {
    let s = dir.to_string_lossy();
    match kind {
        ShellKind::PowerShell => {
            let mut q = String::with_capacity(s.len() + 2);
            for c in s.chars() {
                // PowerShell treats typographic single quotes as quotes too.
                if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
                    q.push(c);
                }
                q.push(c);
            }
            format!("Set-Location -LiteralPath '{q}'\r")
        }
        // `"` cannot occur in Windows paths; `/d` also switches drives.
        ShellKind::Cmd => format!("cd /d \"{s}\"\r"),
        ShellKind::Fish => format!("cd '{}'\r", s.replace('\\', "\\\\").replace('\'', "\\'")),
        // Leading space keeps it out of history with `HISTCONTROL=ignorespace`.
        ShellKind::Bash | ShellKind::Zsh | ShellKind::Posix => format!(" cd -- '{}'\r", s.replace('\'', "'\\''")),
    }
}

/// Prompt heuristic for `send_cd` / Esc handling: nothing typed right of the cursor and
/// the text left of it ends with a prompt character followed by at most one space.
pub(crate) fn looks_like_prompt(left: &str, right: &str) -> bool {
    if !right.trim().is_empty() {
        return false;
    }
    let trimmed = left.trim_end();
    if left.len() - trimmed.len() > 1 {
        return false;
    }
    matches!(trimmed.chars().last(), Some('>' | '$' | '%' | '#' | '❯' | '➜' | 'λ' | '»' | '›'))
}

/// Fallback cwd discovery for shells without integration (Linux `/proc` only).
pub(crate) fn process_cwd(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Parse a title produced by the integration hook.
pub(crate) fn parse_cwd_title(title: &str) -> Option<PathBuf> {
    let rest = title.strip_prefix(CWD_TITLE_MARKER)?.trim();
    (!rest.is_empty()).then(|| PathBuf::from(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_shell_kinds() {
        assert_eq!(ShellKind::detect(r"C:\Program Files\PowerShell\7\pwsh.exe"), ShellKind::PowerShell);
        assert_eq!(ShellKind::detect("powershell.exe"), ShellKind::PowerShell);
        assert_eq!(ShellKind::detect(r"C:\Windows\system32\cmd.exe"), ShellKind::Cmd);
        assert_eq!(ShellKind::detect("/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::detect("/usr/bin/zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::detect("/usr/bin/fish"), ShellKind::Fish);
        assert_eq!(ShellKind::detect("/bin/dash"), ShellKind::Posix);
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // "dir" as UTF-16LE.
        assert_eq!(encode_ps_command("dir"), "ZABpAHIA");
    }

    #[test]
    fn cd_quoting() {
        let p = Path::new(r"C:\it's here");
        assert_eq!(cd_command(ShellKind::PowerShell, p), "Set-Location -LiteralPath 'C:\\it''s here'\r");
        assert_eq!(cd_command(ShellKind::Cmd, p), "cd /d \"C:\\it's here\"\r");
        let u = Path::new("/home/me/it's $HOME");
        assert_eq!(cd_command(ShellKind::Bash, u), " cd -- '/home/me/it'\\''s $HOME'\r");
        assert_eq!(cd_command(ShellKind::Fish, u), "cd '/home/me/it\\'s $HOME'\r");
    }

    #[test]
    fn prompt_heuristic() {
        assert!(looks_like_prompt("PS C:\\Users\\me> ", ""));
        assert!(looks_like_prompt("C:\\Users\\me>", "   "));
        assert!(looks_like_prompt("me@host:~$ ", ""));
        assert!(looks_like_prompt("host% ", ""));
        assert!(looks_like_prompt("root# ", ""));
        assert!(!looks_like_prompt("PS C:\\> git sta", ""));
        assert!(!looks_like_prompt("PS C:\\> ", "tus"));
        assert!(!looks_like_prompt("Compiling foo v0.1.0", ""));
        assert!(!looks_like_prompt("", ""));
    }

    #[test]
    fn cwd_title() {
        assert_eq!(parse_cwd_title("soyuz-cwd:C:\\work"), Some(PathBuf::from("C:\\work")));
        assert_eq!(parse_cwd_title("vim"), None);
        assert_eq!(parse_cwd_title("soyuz-cwd:"), None);
    }

    #[test]
    fn default_shell_exists() {
        let sh = default_shell().expect("a default shell");
        assert!(Path::new(&sh).is_file(), "{sh}");
    }
}
