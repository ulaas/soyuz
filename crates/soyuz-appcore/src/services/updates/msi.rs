//! "Update and restart" for a copy installed by the MSI.
//!
//! Windows Installer replaces `soyuz.exe`, `soyuz-tui.exe` and the FFmpeg DLLs, so no
//! Soyuz process may run while it works, and something has to start Soyuz again
//! afterwards. That something is a copy of the running executable in the update's work
//! folder under the system temp folder, started with [`HELPER_FLAG`] ([`helper_main`]):
//!
//! 1. The app downloads and verifies the MSI, copies itself next to it ([`prepare`]) and
//!    closes normally (pending jobs are asked about like on any quit). On exit it starts
//!    the helper with a pipe as its standard input ([`spawn_helper`]).
//! 2. The helper waits for the pipe to close (the app is gone) and for the installed
//!    executables to be free, hashes the MSI once more, and runs
//!    `msiexec /i <msi> /passive /norestart INSTALLFOLDER="<the install's folder>"`,
//!    plus `ALLUSERS=1` for a per-machine install: Windows Installer shows its progress
//!    window (and the UAC prompt of a per-machine install) and nothing else. The major
//!    upgrade keeps the features the user picked (context menu, PATH).
//! 3. Whatever the outcome, the helper records it ([`write_result`]) and starts the
//!    installed `soyuz.exe`: the new version after a success, the old one (Windows
//!    Installer rolls a failed upgrade back) otherwise. The next start shows the outcome
//!    as a notification ([`take_result`]) and deletes the work folder later.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// First argument of the helper process.
pub const HELPER_FLAG: &str = "--apply-update";
/// The helper's file name in the work folder.
const HELPER_NAME: &str = "soyuz-update.exe";
/// File in the config dir with the outcome of the last installed update.
const RESULT_FILE: &str = "update-result.json";

/// Everything the helper needs to install a verified MSI and come back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallerPlan {
    /// The copy of the app that runs the installer.
    pub helper: PathBuf,
    pub msi: PathBuf,
    /// Lower-case hex SHA-256 the MSI was verified against.
    pub sha256: String,
    /// The folder of the installed copy (the MSI is told to install there again).
    pub install_dir: PathBuf,
    pub per_machine: bool,
    /// The executable to start afterwards.
    pub relaunch: PathBuf,
    /// The version being installed (for the notification).
    pub version: String,
}

impl InstallerPlan {
    /// The helper's arguments ([`InstallerPlan::from_helper_args`] reads them back).
    pub fn helper_args(&self) -> Vec<OsString> {
        vec![
            HELPER_FLAG.into(),
            self.msi.clone().into(),
            self.sha256.clone().into(),
            self.install_dir.clone().into(),
            if self.per_machine { "machine" } else { "user" }.into(),
            self.relaunch.clone().into(),
            self.version.clone().into(),
        ]
    }

    /// The plan from a helper's command line (`args` without the program name), running
    /// as `helper`. `None` when this is not a helper start.
    pub fn from_helper_args(helper: PathBuf, args: &[OsString]) -> Option<InstallerPlan> {
        let [flag, msi, sha256, install_dir, scope, relaunch, version] = args else { return None };
        if flag != HELPER_FLAG {
            return None;
        }
        Some(InstallerPlan {
            helper,
            msi: msi.into(),
            sha256: sha256.to_str()?.to_string(),
            install_dir: install_dir.into(),
            per_machine: match scope.to_str()? {
                "machine" => true,
                "user" => false,
                _ => return None,
            },
            relaunch: relaunch.into(),
            version: version.to_str()?.to_string(),
        })
    }

    /// The arguments of `msiexec.exe`, as one command-line string in the quoting Windows
    /// Installer reads (`PROPERTY="value"`): an unattended install with the progress
    /// window, no restart of Windows, into the folder and scope of this install.
    pub fn msiexec_command_line(&self) -> Result<String, String> {
        let quoted = |p: &Path| -> Result<String, String> {
            let s = p.to_string_lossy();
            // A quote cannot be escaped for Windows Installer, and a trailing backslash
            // would escape the closing one.
            if s.contains('"') || s.is_empty() {
                return Err(tr!("services-update-msi-bad-path", path = s.to_string()));
            }
            Ok(format!("\"{}\"", s.trim_end_matches('\\')))
        };
        let mut line =
            format!("/i {} /passive /norestart INSTALLFOLDER={}", quoted(&self.msi)?, quoted(&self.install_dir)?);
        if self.per_machine {
            line.push_str(" ALLUSERS=1");
        }
        Ok(line)
    }
}

/// Get the verified `msi` in `work` ready to install over the copy `exe` belongs to:
/// the running executable is copied into `work` as the helper.
pub fn prepare(
    work: &Path,
    msi: PathBuf,
    sha256: String,
    exe: &Path,
    per_machine: bool,
    version: String,
) -> Result<InstallerPlan, String> {
    let fail = |e: &dyn std::fmt::Display| tr!("services-update-install-failed", error = e.to_string());
    let install_dir = exe.parent().filter(|d| !d.as_os_str().is_empty()).ok_or_else(|| fail(&"no parent folder"))?;
    let helper = work.join(HELPER_NAME);
    std::fs::copy(exe, &helper).map_err(|e| fail(&e))?;
    let plan = InstallerPlan {
        helper,
        msi,
        sha256,
        install_dir: install_dir.to_path_buf(),
        per_machine,
        relaunch: exe.to_path_buf(),
        version,
    };
    // Fail now, not after the app has closed.
    plan.msiexec_command_line()?;
    Ok(plan)
}

/// Start the helper of `plan`, detached, with a pipe as its standard input that stays
/// open until this process ends: that is how the helper knows the app is gone. Call it
/// last, right before exiting.
pub fn spawn_helper(plan: &InstallerPlan) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(&plan.helper)
        .args(plan.helper_args())
        .current_dir(plan.helper.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Never closed by us: the handle goes when the process does.
    std::mem::forget(child.stdin.take());
    Ok(())
}

/// How an installed update ended ([`write_result`] / [`take_result`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    /// The user closed the installer's window or declined the UAC prompt.
    Cancelled,
    /// The MSI changed after it was verified.
    Changed,
    /// Another Soyuz window (or the terminal UI) was still running.
    Busy,
    /// Windows Installer's exit code.
    ExitCode(i32),
    /// The installer could not be started.
    NotStarted(String),
}

impl Outcome {
    fn code(&self) -> String {
        match self {
            Outcome::Installed => "ok".into(),
            Outcome::Cancelled => "cancelled".into(),
            Outcome::Changed => "changed".into(),
            Outcome::Busy => "busy".into(),
            Outcome::ExitCode(n) => format!("exit:{n}"),
            Outcome::NotStarted(e) => format!("spawn:{e}"),
        }
    }

    fn from_code(code: &str) -> Option<Outcome> {
        Some(match code {
            "ok" => Outcome::Installed,
            "cancelled" => Outcome::Cancelled,
            "changed" => Outcome::Changed,
            "busy" => Outcome::Busy,
            c => match c.split_once(':')? {
                ("exit", n) => Outcome::ExitCode(n.parse().ok()?),
                ("spawn", e) => Outcome::NotStarted(e.to_string()),
                _ => return None,
            },
        })
    }

    /// From `msiexec`'s exit code: 0 and 3010 (done, Windows wants a restart later) are a
    /// success, 1602 is the user's cancel.
    pub fn from_exit_code(code: i32) -> Outcome {
        match code {
            0 | 3010 => Outcome::Installed,
            1602 => Outcome::Cancelled,
            n => Outcome::ExitCode(n),
        }
    }

    /// Why the update was not installed, in the user's language (`None` = it was).
    pub fn error(&self) -> Option<String> {
        Some(match self {
            Outcome::Installed => return None,
            Outcome::Cancelled => tr!("services-update-msi-cancelled").into(),
            Outcome::Changed => tr!("services-update-install-changed").into(),
            Outcome::Busy => tr!("services-update-msi-busy").into(),
            Outcome::ExitCode(n) => tr!("services-update-msi-exit", code = n.to_string()),
            Outcome::NotStarted(e) => tr!("services-update-msi-not-started", error = e.as_str()),
        })
    }
}

/// Record how installing `version` ended, in `config_dir`, for the next start.
pub fn write_result(config_dir: &Path, version: &str, outcome: &Outcome) {
    let _ = std::fs::create_dir_all(config_dir);
    let json = serde_json::json!({ "version": version, "outcome": outcome.code() });
    if let Err(e) = std::fs::write(config_dir.join(RESULT_FILE), format!("{json}\n")) {
        log::warn!("update result: {e}");
    }
}

/// The recorded outcome of the last installed update, read once (the file is removed).
pub fn take_result(config_dir: &Path) -> Option<(String, Outcome)> {
    let path = config_dir.join(RESULT_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let version = v.get("version")?.as_str()?.to_string();
    Some((version, Outcome::from_code(v.get("outcome")?.as_str()?)?))
}

/// The entry of the helper process: when this process was started with [`HELPER_FLAG`],
/// install the update and return the exit code; `None` for an ordinary start. Call it
/// first thing in `main`, before anything reads the arguments, logs or opens a window.
pub fn helper_main() -> Option<i32> {
    let mut args = std::env::args_os();
    let helper = PathBuf::from(args.next()?);
    let args: Vec<OsString> = args.collect();
    if args.first().is_none_or(|a| a != HELPER_FLAG) {
        return None;
    }
    let Some(plan) = InstallerPlan::from_helper_args(helper, &args) else { return Some(2) };
    Some(run_helper(&plan))
}

fn run_helper(plan: &InstallerPlan) -> i32 {
    // 1. The app is gone when its end of the pipe closes.
    let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
    let outcome = if wait_until_free(plan) { install(plan) } else { Outcome::Busy };
    // 2. An MSI install is never portable: this is the app's own config folder.
    write_result(&soyuz_core::config::config_dir(), &plan.version, &outcome);
    let _ = std::fs::remove_file(&plan.msi);
    // 3. Back into Soyuz: the new version, or the old one that is still installed.
    if plan.relaunch.is_file()
        && let Err(e) = std::process::Command::new(&plan.relaunch).current_dir(&plan.install_dir).spawn()
    {
        log::error!("update: cannot start {}: {e}", plan.relaunch.display());
    }
    if outcome == Outcome::Installed { 0 } else { 1 }
}

/// Wait (up to half a minute) until no process runs the installed executables: a
/// running program cannot be opened for writing. `false` when one still runs.
fn wait_until_free(plan: &InstallerPlan) -> bool {
    let programs = [plan.install_dir.join("soyuz.exe"), plan.install_dir.join("soyuz-tui.exe")];
    let free = |p: &Path| match std::fs::OpenOptions::new().write(true).open(p) {
        Ok(_) => true,
        // A per-machine install is not writable for this user: nothing to learn here.
        Err(e) => matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied),
    };
    for _ in 0..150 {
        if programs.iter().all(|p| free(p)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    false
}

/// Hash the MSI once more and run Windows Installer on it, waiting for it to finish.
fn install(plan: &InstallerPlan) -> Outcome {
    let hash = std::fs::File::open(&plan.msi).and_then(super::sha256_of);
    if !hash.is_ok_and(|h| h.eq_ignore_ascii_case(&plan.sha256)) {
        return Outcome::Changed;
    }
    match run_msiexec(plan) {
        Ok(code) => Outcome::from_exit_code(code),
        Err(e) => Outcome::NotStarted(e),
    }
}

#[cfg(windows)]
fn run_msiexec(plan: &InstallerPlan) -> Result<i32, String> {
    use std::os::windows::process::CommandExt;
    let line = plan.msiexec_command_line()?;
    let status = std::process::Command::new(super::msiexec_path())
        .raw_arg(line)
        .current_dir(std::env::temp_dir())
        .status()
        .map_err(|e| e.to_string())?;
    Ok(status.code().unwrap_or(-1))
}

#[cfg(not(windows))]
fn run_msiexec(_plan: &InstallerPlan) -> Result<i32, String> {
    Err("Windows Installer exists on Windows only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> InstallerPlan {
        InstallerPlan {
            helper: PathBuf::from(r"C:\Users\me\AppData\Local\Temp\soyuz-update-1\soyuz-update.exe"),
            msi: PathBuf::from(r"C:\Users\me\AppData\Local\Temp\soyuz-update-1\soyuz-0.8.0-windows-x64.msi"),
            sha256: "ab".repeat(32),
            install_dir: PathBuf::from(r"C:\Users\me\AppData\Local\Programs\Soyuz"),
            per_machine: false,
            relaunch: PathBuf::from(r"C:\Users\me\AppData\Local\Programs\Soyuz\soyuz.exe"),
            version: "0.8.0".into(),
        }
    }

    #[test]
    fn the_installer_keeps_folder_and_scope() {
        let user = plan();
        assert_eq!(
            user.msiexec_command_line().unwrap(),
            r#"/i "C:\Users\me\AppData\Local\Temp\soyuz-update-1\soyuz-0.8.0-windows-x64.msi" /passive /norestart INSTALLFOLDER="C:\Users\me\AppData\Local\Programs\Soyuz""#
        );
        // Per-machine: the same scope again (Windows Installer asks for elevation), a
        // folder with spaces, and no backslash before the closing quote.
        let machine =
            InstallerPlan { per_machine: true, install_dir: PathBuf::from(r"C:\Program Files\Soyuz\"), ..plan() };
        let line = machine.msiexec_command_line().unwrap();
        assert!(line.ends_with(r#"INSTALLFOLDER="C:\Program Files\Soyuz" ALLUSERS=1"#), "{line}");
        // Never interactive, never a silent install without any window, never a reboot.
        for l in [&line, &user.msiexec_command_line().unwrap()] {
            assert!(l.contains(" /passive ") && l.contains(" /norestart ") && !l.contains("/qn"), "{l}");
        }
        assert!(!user.msiexec_command_line().unwrap().contains("ALLUSERS"));
        // A path that cannot be quoted is refused before anything runs.
        let bad = InstallerPlan { install_dir: PathBuf::from("C:\\So\"yuz"), ..plan() };
        assert!(bad.msiexec_command_line().is_err());
    }

    #[test]
    fn helper_arguments_round_trip() {
        for p in [plan(), InstallerPlan { per_machine: true, ..plan() }] {
            let args = p.helper_args();
            assert_eq!(args[0], HELPER_FLAG);
            assert_eq!(InstallerPlan::from_helper_args(p.helper.clone(), &args), Some(p));
        }
        let args = plan().helper_args();
        // An ordinary start, a folder to open, a cut or unknown command line: not a helper.
        assert_eq!(InstallerPlan::from_helper_args(PathBuf::new(), &[]), None);
        assert_eq!(InstallerPlan::from_helper_args(PathBuf::new(), &["C:\\".into()]), None);
        assert_eq!(InstallerPlan::from_helper_args(PathBuf::new(), &args[..5]), None);
        let mut other = args.clone();
        other[4] = "everyone".into();
        assert_eq!(InstallerPlan::from_helper_args(PathBuf::new(), &other), None);
    }

    #[test]
    fn outcomes_survive_the_restart() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(take_result(d.path()), None);
        for o in [
            Outcome::Installed,
            Outcome::Cancelled,
            Outcome::Changed,
            Outcome::Busy,
            Outcome::ExitCode(1603),
            Outcome::NotStarted("no msiexec: here".into()),
        ] {
            write_result(d.path(), "0.8.0", &o);
            assert_eq!(take_result(d.path()), Some(("0.8.0".to_string(), o.clone())));
            assert_eq!(take_result(d.path()), None, "shown once");
            assert_eq!(o.error().is_none(), o == Outcome::Installed);
        }
        assert_eq!(Outcome::from_exit_code(0), Outcome::Installed);
        assert_eq!(Outcome::from_exit_code(3010), Outcome::Installed);
        assert_eq!(Outcome::from_exit_code(1602), Outcome::Cancelled);
        assert_eq!(Outcome::from_exit_code(1603), Outcome::ExitCode(1603));
        assert!(Outcome::ExitCode(1603).error().unwrap().contains("1603"));
        // A damaged file is ignored and removed.
        std::fs::write(d.path().join(RESULT_FILE), b"{").unwrap();
        assert_eq!(take_result(d.path()), None);
        assert!(!d.path().join(RESULT_FILE).exists());
    }

    #[test]
    fn preparing_copies_the_running_program_as_the_helper() {
        let d = tempfile::tempdir().unwrap();
        let (install, work) = (d.path().join("Soyuz"), d.path().join("work"));
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let exe = install.join("soyuz.exe");
        std::fs::write(&exe, b"the app").unwrap();
        let msi = work.join("soyuz-0.8.0-windows-x64.msi");
        std::fs::write(&msi, b"msi").unwrap();
        let p = prepare(&work, msi.clone(), "00".repeat(32), &exe, true, "0.8.0".into()).unwrap();
        assert_eq!(std::fs::read(&p.helper).unwrap(), b"the app");
        assert_eq!(p.helper.parent(), Some(work.as_path()));
        assert_eq!(
            (p.install_dir.as_path(), p.relaunch.as_path(), p.per_machine),
            (install.as_path(), exe.as_path(), true)
        );
        // The installed copy is not touched by preparing.
        assert_eq!(std::fs::read(&exe).unwrap(), b"the app");
        assert_eq!(std::fs::read_dir(&install).unwrap().count(), 1);
        // A changed MSI is never run: the helper's check says so without starting anything.
        assert_eq!(install_outcome_for_test(&p), Outcome::Changed);
    }

    fn install_outcome_for_test(p: &InstallerPlan) -> Outcome {
        // `install` only reaches Windows Installer when the hash matches; "00…" never does.
        install(p)
    }
}
