//! `scripts/package.ps1 -DryRun` / `scripts/package.sh --dry-run` /
//! `scripts/ffmpeg-build.sh --dry-run`: the packaging plans name the right files and write
//! nothing. Skipped when PowerShell 7 (`pwsh`) / bash is not installed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // Not `canonicalize`: PowerShell cannot `Push-Location` into a `\\?\` verbatim path.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().and_then(|p| p.parent()).unwrap().to_path_buf()
}

/// The five FFmpeg runtime file names of the bundled majors on this OS (what
/// `soyuz_media::lib_file_names(Majors::BUNDLED)` returns; spelled out so this test needs
/// no dependency on the media crate).
fn ffmpeg_lib_names() -> [&'static str; 5] {
    if cfg!(windows) {
        ["avutil-61.dll", "swresample-7.dll", "swscale-10.dll", "avcodec-63.dll", "avformat-63.dll"]
    } else if cfg!(target_os = "macos") {
        [
            "libavutil.61.dylib",
            "libswresample.7.dylib",
            "libswscale.10.dylib",
            "libavcodec.63.dylib",
            "libavformat.63.dylib",
        ]
    } else {
        ["libavutil.so.61", "libswresample.so.7", "libswscale.so.10", "libavcodec.so.63", "libavformat.so.63"]
    }
}

/// A fake runtime folder: empty library files plus the three texts the scripts require.
fn fake_ffmpeg_dir(dir: &Path, names: &[&str]) {
    for n in names.iter().copied().chain(["LICENSE.md", "BUILD.txt", "SOURCES.txt"]) {
        std::fs::write(dir.join(n), b"").unwrap();
    }
}

/// Git for Windows' bash (never the WSL launcher in System32); `bash` elsewhere.
fn bash() -> Option<PathBuf> {
    if !cfg!(windows) {
        return Some(PathBuf::from("bash"));
    }
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    [pf.join("Git/bin/bash.exe"), pf.join("Git/usr/bin/bash.exe")].into_iter().find(|p| p.is_file())
}

#[test]
fn package_script_dry_run() {
    let root = repo_root();
    let out_dir = tempfile::tempdir().unwrap();
    let ffmpeg_dir = tempfile::tempdir().unwrap();
    fake_ffmpeg_dir(ffmpeg_dir.path(), &ffmpeg_lib_names());
    let run = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(root.join("scripts").join("package.ps1"))
        .args(["-DryRun", "-Portable", "-Msi", "-OutDir"])
        .arg(out_dir.path())
        .arg("-FfmpegDir")
        .arg(ffmpeg_dir.path())
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
        // The FFmpeg runtime: libraries next to the executables, licence texts under ffmpeg/,
        // and the MSI gets the folder too.
        "ffmpeg/LICENSE.md".into(),
        "ffmpeg/BUILD.txt".into(),
        "ffmpeg/SOURCES.txt".into(),
        "FfmpegDir=".into(),
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
    // Every library lands flat next to the executables (`<lib> <- <ffmpeg dir>/<lib>`).
    for lib in ffmpeg_lib_names() {
        let staged = stdout.lines().any(|l| l.trim_start().starts_with(&format!("{lib} ")) && l.contains("<-"));
        assert!(staged, "{lib} is not staged in:\n{stdout}");
    }
    assert_eq!(std::fs::read_dir(out_dir.path()).unwrap().count(), 0, "a dry run must not write anything");
}

#[test]
fn package_sh_dry_run() {
    let Some(bash) = bash() else {
        eprintln!("skipping: bash not available");
        return;
    };
    let root = repo_root();
    let out_dir = tempfile::tempdir().unwrap();
    let ffmpeg_dir = tempfile::tempdir().unwrap();
    // The Linux names whatever the host OS is: package.sh only runs on Linux for real.
    let libs = ["libavutil.so.61", "libswresample.so.7", "libswscale.so.10", "libavcodec.so.63", "libavformat.so.63"];
    fake_ffmpeg_dir(ffmpeg_dir.path(), &libs);
    let out = Command::new(&bash)
        .arg(root.join("scripts").join("package.sh"))
        .args(["--dry-run", "--rpm", "--out-dir"])
        .arg(out_dir.path())
        .arg("--ffmpeg-dir")
        .arg(ffmpeg_dir.path())
        .current_dir(&root)
        .output()
        .expect("run bash");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "package.sh failed:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    for want in [
        "bin/soyuz ",
        "bin/soyuz-tui ",
        "share/licenses/soyuz/ffmpeg/LICENSE.md",
        "share/licenses/soyuz/ffmpeg/BUILD.txt",
        "soyuz_ffmpeg 1",
        "dry run: nothing built or written",
    ] {
        assert!(stdout.contains(want), "missing {want:?} in:\n{stdout}");
    }
    for lib in libs {
        assert!(stdout.contains(&format!("lib/soyuz/{lib}")), "{lib} is not staged in:\n{stdout}");
    }
    assert_eq!(std::fs::read_dir(out_dir.path()).unwrap().count(), 0, "a dry run must not write anything");
}

#[test]
fn ffmpeg_build_script_dry_run() {
    let Some(bash) = bash() else {
        eprintln!("skipping: bash not available");
        return;
    };
    let root = repo_root();
    let out_dir = tempfile::tempdir().unwrap();
    let pins = std::fs::read_to_string(root.join("third_party/ffmpeg/pins.env")).unwrap();
    let pin = |key: &str| {
        pins.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .unwrap_or_else(|| panic!("{key} missing from pins.env"))
            .trim()
            .trim_matches('"')
            .to_string()
    };
    let ffmpeg_version = pin("FFMPEG_VERSION");
    for target in ["linux64", "win64", "macos-universal"] {
        let out = Command::new(&bash)
            .arg(root.join("scripts").join("ffmpeg-build.sh"))
            .arg(target)
            .arg(out_dir.path().join(target))
            .arg("--dry-run")
            .current_dir(&root)
            .output()
            .expect("run bash");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "ffmpeg-build.sh {target} failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Every pinned input with its sum, the LGPL-only configure line and the gates.
        for want in [
            format!("ffmpeg-{ffmpeg_version}.tar.xz"),
            pin("FFMPEG_SHA256"),
            pin("DAV1D_SHA256"),
            pin("OPUS_SHA256"),
            pin("LIBVPX_SHA256"),
            pin("AMF_COMMIT"),
            "--disable-gpl --disable-nonfree --disable-version3".into(),
            "--disable-network".into(),
            "--enable-libdav1d --enable-libopus --enable-libvpx".into(),
            "--enable-shared --disable-static".into(),
            "avcodec_configuration() without --enable-gpl".into(),
            "dry run: nothing downloaded or written".into(),
        ] {
            assert!(stdout.contains(&want), "{target}: missing {want:?} in:\n{stdout}");
        }
        let (libs, hw): (&[&str], &str) = match target {
            "win64" => (&["avcodec-63.dll", "avformat-63.dll"], "--enable-amf --enable-libvpl"),
            "linux64" => (&["libavcodec.so.63", "libavformat.so.63"], "--enable-vaapi"),
            _ => (&["libavcodec.63.dylib", "libavformat.63.dylib"], "--enable-videotoolbox"),
        };
        for want in libs.iter().copied().chain([hw]) {
            assert!(stdout.contains(want), "{target}: missing {want:?} in:\n{stdout}");
        }
        if target == "macos-universal" {
            assert!(
                stdout.contains("slice macos-arm64:")
                    && stdout.contains("slice macos-x86_64:")
                    && stdout.contains("lipo")
            );
        }
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

    // The FFmpeg runtime (Phase V): every package format but the Flatpak bundles the
    // libraries where soyuz-media looks (next to the exe; lib/soyuz on Linux; the .app's
    // Contents/Frameworks), each with the licence texts; the rpm hides the private sonames.
    assert!(wxs.contains("<?ifdef FfmpegDir ?>") && wxs.contains(r#"Include="$(var.FfmpegDir)\*.dll""#));
    assert!(wxs.contains(r#"Source="$(var.FfmpegDir)\LICENSE.md""#));
    assert!(msi.contains("FfmpegDir=") && msix.contains("FfmpegDir"));
    let ps1 = std::fs::read_to_string(root.join("scripts/package.ps1")).unwrap();
    assert!(ps1.contains("FfmpegDir") && ps1.contains("\"ffmpeg/$doc\""));
    assert!(sh.contains("lib/soyuz/") && sh.contains("share/licenses/soyuz/ffmpeg/") && sh.contains("soyuz_ffmpeg"));
    assert!(spec.contains("%{_libdir}/soyuz/lib*.so.*"));
    assert!(spec.contains("__provides_exclude_from ^%{_libdir}/soyuz/.*$"));
    assert!(spec.contains("__requires_exclude ^lib(avcodec|avformat|avutil|swscale|swresample)"));
    assert!(flatpak.contains("codecs-extra"), "the Flatpak documents why it bundles no FFmpeg");
    let deb = std::fs::read_to_string(root.join("crates/soyuz-app/Cargo.toml")).unwrap();
    assert!(deb.contains(r#"["../../ffmpeg-rt/lib*.so.*", "usr/lib/soyuz/", "644"]"#));
    let entitlements = std::fs::read_to_string(root.join("installer/macos/entitlements.plist")).unwrap();
    assert!(entitlements.contains("com.apple.security.cs.disable-library-validation"));
    let release = std::fs::read_to_string(root.join(".github/workflows/release.yml")).unwrap();
    assert!(release.contains("--entitlements installer/macos/entitlements.plist"));
    assert!(release.contains("Contents/Frameworks"));
    // The runtime goes on its own release tag, never on the app's (the update check).
    assert!(release.contains(r#"tag="ffmpeg-runtime-$FFMPEG_VERSION""#) && release.contains("--latest=false"));
    assert!(release.contains("pattern: soyuz-*"));
    // The build inputs are pinned by SHA-256 (or a commit for the sparse AMF checkout).
    let pins = std::fs::read_to_string(root.join("third_party/ffmpeg/pins.env")).unwrap();
    for key in [
        "FFMPEG_SHA256=",
        "DAV1D_SHA256=",
        "OPUS_SHA256=",
        "LIBVPX_SHA256=",
        "NVCODEC_SHA256=",
        "LIBVPL_SHA256=",
        "AMF_COMMIT=",
    ] {
        let line = pins.lines().find(|l| l.starts_with(key)).unwrap_or_else(|| panic!("{key} missing"));
        let value = line[key.len()..].trim();
        assert!(value.len() >= 40 && value.chars().all(|c| c.is_ascii_hexdigit()), "{line}");
    }
    assert!(pins.contains("FFMPEG_MAJORS=\"61 7 10 63 63\""));
}
