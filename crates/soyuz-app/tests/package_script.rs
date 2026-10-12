//! `scripts/package.ps1 -DryRun`: the packaging plan names the right files and
//! writes nothing. Skipped when PowerShell 7 (`pwsh`) is not installed.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // Not `canonicalize`: PowerShell cannot `Push-Location` into a `\\?\` verbatim path.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

#[test]
fn package_script_dry_run() {
    let root = repo_root();
    let out_dir = tempfile::tempdir().unwrap();
    let run = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts").join("package.ps1"))
        .args(["-DryRun", "-Portable", "-Msi", "-OutDir"])
        .arg(out_dir.path())
        .current_dir(&root)
        .output();
    let out = match run {
        Ok(o) => o,
        Err(e) => {
            eprintln!("skipping: pwsh not available ({e})");
            return;
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "package.ps1 failed:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let version = env!("CARGO_PKG_VERSION");
    let portable_extension = if cfg!(windows) { "-portable.zip" } else { "-portable.tar.gz" };
    let exe = |name: &str| if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    for want in [
        format!("soyuz-{version}-"),
        portable_extension.to_string(),
        // Both executables (the app and the terminal UI) are built in one cargo run.
        "-p soyuz-app -p soyuz-tui".into(),
        "LICENSE".into(),
        "THIRD-PARTY-NOTICES.md".into(),
        "README.md".into(),
        "docs/config.example.toml".into(),
        "portable".into(),
        "installer/windows/soyuz.wxs".into(),
        "SHA256SUMS-".into(),
        "dry run: nothing built or written".into(),
    ] {
        assert!(stdout.contains(&want), "missing {want:?} in:\n{stdout}");
    }
    // ... and both are staged from the release folder (`<name> <- target/release/<name>`).
    for name in [exe("soyuz"), exe("soyuz-tui")] {
        let staged = stdout.lines().any(|l| {
            let l = l.replace('\\', "/");
            l.trim_start().starts_with(&format!("{name} "))
                && l.trim_end().ends_with(&format!("<- target/release/{name}"))
        });
        assert!(staged, "{name} is not staged in:\n{stdout}");
    }
    assert_eq!(std::fs::read_dir(out_dir.path()).unwrap().count(), 0, "a dry run must not write anything");
}

#[test]
fn installer_sources_are_consistent() {
    let root = repo_root();
    let wxs = std::fs::read_to_string(root.join("installer/windows/soyuz.wxs")).unwrap();
    // The upgrade code must never change, or major upgrades stop replacing old installs.
    assert!(wxs.contains(r#"UpgradeCode="4d03f89e-9d28-4139-aa95-0e950320af14""#));
    assert!(wxs.contains(r#"Scope="perUserOrMachine""#), "per-user by default, per-machine optional");
    assert!(wxs.contains("<MajorUpgrade"));
    // The terminal UI ships next to soyuz.exe (same folder, so the PATH entry covers it).
    assert!(wxs.contains(r#"Source="$(var.BinDir)\soyuz.exe""#));
    assert!(wxs.contains(r#"Source="$(var.BinDir)\soyuz-tui.exe""#));
    for f in ["README.md", "LICENSE", "THIRD-PARTY-NOTICES.md", "docs/USER_GUIDE.md", "docs/config.example.toml"] {
        assert!(root.join(f).is_file(), "{f} referenced by the installer is missing");
    }
    assert!(root.join("crates/soyuz-app/assets/icon.ico").is_file());
    let appx = std::fs::read_to_string(root.join("installer/msix/AppxManifest.xml")).unwrap();
    assert!(appx.contains(r#"Executable="soyuz.exe""#) && appx.contains("runFullTrust"));
    // `soyuz-tui` from any terminal: its own console-subsystem application and alias.
    assert!(appx.contains(r#"Executable="soyuz-tui.exe""#));
    assert!(appx.contains(r#"Alias="soyuz-tui.exe""#));
    assert!(appx.contains(r#"desktop4:Subsystem="console""#));
    let msix = std::fs::read_to_string(root.join("installer/msix/build-msix.ps1")).unwrap();
    assert!(msix.contains("soyuz-tui.exe"));
    let msi = std::fs::read_to_string(root.join("installer/windows/build-msi.ps1")).unwrap();
    assert!(msi.contains("soyuz-tui.exe"));
    // Linux packages: the tarball, the rpm and the Flatpak carry the terminal UI.
    let sh = std::fs::read_to_string(root.join("scripts/package.sh")).unwrap();
    assert!(sh.contains("bin/soyuz-tui|755") && sh.contains("-p soyuz-tui"));
    let spec = std::fs::read_to_string(root.join("installer/linux/soyuz.spec")).unwrap();
    assert!(spec.contains("%{_bindir}/soyuz-tui"));
    let flatpak = std::fs::read_to_string(root.join("installer/linux/flatpak/io.github.ulaas.Soyuz.yml")).unwrap();
    assert!(flatpak.contains("/app/bin/soyuz-tui"));
}
