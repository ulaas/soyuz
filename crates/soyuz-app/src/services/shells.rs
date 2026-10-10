//! Shells installed on this machine, for the terminal shell dropdown (Settings →
//! Terminal). Detected once on a background thread (file checks and a registry read for
//! WSL distributions); [`detected`] returns `None` until that finished.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One shell the terminal can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shell {
    /// Shown name (`PowerShell 7`, `Ubuntu (WSL)`).
    pub label: String,
    /// `[terminal] shell` value.
    pub program: String,
    /// `[terminal] args` this entry needs (WSL distributions: `-d <name>`).
    pub args: Vec<String>,
    /// Where it was found (shown as secondary text).
    pub path: String,
    pub kind: ShellKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    PowerShell,
    Cmd,
    Bash,
    Wsl,
    Other,
}

static DETECTED: OnceLock<Vec<Shell>> = OnceLock::new();
static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The detected shells, or `None` while detection runs (it starts on the first call;
/// `wake` is called when it is done).
pub fn detected(wake: impl Fn() + Send + 'static) -> Option<&'static [Shell]> {
    if let Some(v) = DETECTED.get() {
        return Some(v);
    }
    if !STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let spawned = std::thread::Builder::new().name("soyuz-shells".into()).spawn(move || {
            let _ = DETECTED.set(detect(&Env::real()));
            wake();
        });
        if spawned.is_err() {
            let _ = DETECTED.set(Vec::new());
        }
    }
    DETECTED.get().map(Vec::as_slice)
}

type IsFile = Box<dyn Fn(&Path) -> bool>;
type Distros = Box<dyn Fn() -> Vec<String>>;

/// What detection looks at (injectable for tests).
pub struct Env {
    pub is_file: IsFile,
    pub path_dirs: Vec<PathBuf>,
    pub system_root: Option<PathBuf>,
    pub program_files: Vec<PathBuf>,
    pub wsl_distros: Distros,
}

impl Env {
    fn real() -> Env {
        let var = |n: &str| std::env::var_os(n).map(PathBuf::from);
        let mut program_files: Vec<PathBuf> =
            ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"].iter().filter_map(|n| var(n)).collect();
        if let Some(l) = var("LOCALAPPDATA") {
            program_files.push(l.join("Programs"));
        }
        program_files.dedup();
        Env {
            is_file: Box::new(|p| p.is_file()),
            path_dirs: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
            system_root: var("SystemRoot"),
            program_files,
            wsl_distros: Box::new(soyuz_fs::wsl::distros),
        }
    }

    fn which(&self, exe: &str) -> Option<PathBuf> {
        self.path_dirs.iter().map(|d| d.join(exe)).find(|p| (self.is_file)(p))
    }
}

/// Detect the shells (Windows: PowerShell 7, Windows PowerShell, Command Prompt, Git
/// Bash, Nushell, WSL and its distributions; unix: the usual shells).
pub fn detect(env: &Env) -> Vec<Shell> {
    let mut out = Vec::new();
    let mut add = |label: &str, program: &str, args: &[&str], path: &Path, kind: ShellKind| {
        out.push(Shell {
            label: label.to_string(),
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            path: path.display().to_string(),
            kind,
        });
    };
    if cfg!(windows) {
        let pwsh = env
            .which("pwsh.exe")
            .or_else(|| env.program_files.iter().map(|p| p.join(r"PowerShell\7\pwsh.exe")).find(|p| (env.is_file)(p)));
        if let Some(p) = pwsh {
            add("PowerShell 7", "pwsh", &[], &p, ShellKind::PowerShell);
        }
        let sys = env.system_root.clone().map(|r| r.join("System32"));
        let in_sys = |rel: &str| sys.as_ref().map(|s| s.join(rel)).filter(|p| (env.is_file)(p));
        if let Some(p) = in_sys(r"WindowsPowerShell\v1.0\powershell.exe") {
            add("Windows PowerShell", "powershell", &[], &p, ShellKind::PowerShell);
        }
        if let Some(p) = in_sys("cmd.exe") {
            add("Command Prompt", "cmd", &[], &p, ShellKind::Cmd);
        }
        if let Some(p) = env.program_files.iter().map(|p| p.join(r"Git\bin\bash.exe")).find(|p| (env.is_file)(p)) {
            add("Git Bash", &p.display().to_string(), &["--login", "-i"], &p, ShellKind::Bash);
        }
        if let Some(p) = env.which("nu.exe") {
            add("Nushell", "nu", &[], &p, ShellKind::Other);
        }
        if let Some(p) = in_sys("wsl.exe") {
            add("WSL", "wsl.exe", &[], &p, ShellKind::Wsl);
            for d in (env.wsl_distros)() {
                add(&format!("{d} (WSL)"), "wsl.exe", &["-d", &d], &p, ShellKind::Wsl);
            }
        }
    } else {
        for (name, kind) in [
            ("bash", ShellKind::Bash),
            ("zsh", ShellKind::Other),
            ("fish", ShellKind::Other),
            ("nu", ShellKind::Other),
            ("sh", ShellKind::Other),
        ] {
            if let Some(p) = env.which(name) {
                add(name, &p.display().to_string(), &[], &p, kind);
            }
        }
    }
    out
}

/// The entry a `[terminal] shell` / `args` pair selects: WSL distributions by their
/// `-d <name>` argument, other shells by program (name or path, case-insensitive).
pub fn matching<'a>(shells: &'a [Shell], program: &str, args: &[String]) -> Option<&'a Shell> {
    let prog = program.trim();
    let bare = |s: &str| s.strip_suffix(".exe").unwrap_or(s).to_ascii_lowercase();
    let same = |s: &Shell| bare(&s.program) == bare(prog) || s.path.eq_ignore_ascii_case(prog);
    shells
        .iter()
        .filter(|s| same(s))
        .find(|s| s.kind != ShellKind::Wsl || s.args == args)
        .or_else(|| shells.iter().find(|s| same(s) && s.args.is_empty()))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn env(files: &[&str], distros: &[&str]) -> Env {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        let distros: Vec<String> = distros.iter().map(|s| s.to_string()).collect();
        Env {
            is_file: Box::new(move |p| files.iter().any(|f| f == p)),
            path_dirs: vec![PathBuf::from(r"C:\bin")],
            system_root: Some(PathBuf::from(r"C:\Windows")),
            program_files: vec![PathBuf::from(r"C:\Program Files")],
            wsl_distros: Box::new(move || distros.clone()),
        }
    }

    #[test]
    fn detects_windows_shells_and_wsl_distros() {
        let e = env(
            &[
                r"C:\Program Files\PowerShell\7\pwsh.exe",
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                r"C:\Windows\System32\cmd.exe",
                r"C:\Program Files\Git\bin\bash.exe",
                r"C:\Windows\System32\wsl.exe",
            ],
            &["Ubuntu", "Debian"],
        );
        let v = detect(&e);
        let labels: Vec<&str> = v.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            ["PowerShell 7", "Windows PowerShell", "Command Prompt", "Git Bash", "WSL", "Ubuntu (WSL)", "Debian (WSL)"]
        );
        assert_eq!(v[5].args, ["-d", "Ubuntu"]);
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let label = |s: Option<&Shell>| s.map(|s| s.label.clone());
        assert_eq!(label(matching(&v, "wsl.exe", &args(&["-d", "Debian"]))).as_deref(), Some("Debian (WSL)"));
        assert_eq!(label(matching(&v, "wsl", &[])).as_deref(), Some("WSL"));
        assert_eq!(label(matching(&v, "PWSH.exe", &args(&["-NoLogo"]))).as_deref(), Some("PowerShell 7"));
        assert!(matching(&v, "fish", &[]).is_none());
        // Nothing installed: nothing listed.
        assert!(detect(&env(&[], &["Ubuntu"])).is_empty());
    }
}
