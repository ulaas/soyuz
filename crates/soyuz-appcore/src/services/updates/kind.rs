//! How the running copy of Soyuz was installed, which decides what "Update and restart"
//! may do ([`InstallKind`]):
//!
//! * copies Soyuz may replace itself: the Linux release tarball, the Windows portable zip
//!   (a `portable` marker next to the exe), an MSI install (the next MSI is run into the
//!   same folder and scope) and the macOS tarball in a folder the user can write to;
//! * copies something else owns (Scoop, Homebrew, a distribution package, Flatpak,
//!   AppImage, MSIX) or that sit in a folder Soyuz cannot write to: never touched. The
//!   user is told how to update them ([`how_to_update`]);
//! * anything else (a plain zip without the marker, a macOS app bundle, a build folder):
//!   the download is verified and the user installs it.
//!
//! The decision is a pure function of the executable's path, a few environment variables
//! and three facts read off the UI thread ([`InstallFacts`]), so every case is tested on
//! any host.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use soyuz_fs::integration::MsiInstall;

use super::{Asset, Release};

/// How the running copy was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// Unknown layout: the download is verified, the user installs it.
    Manual,
    /// Linux, unpacked from the release tarball (`~/.local`, `/usr/local`, `/opt`, a
    /// build folder): [`super::install_tarball_to`] updates it in place.
    Tarball,
    /// Windows, unpacked from the portable zip (a `portable` marker next to the exe):
    /// the files are swapped in place ([`super::swap`]).
    WindowsPortable,
    /// Windows, installed by the MSI: the next MSI is run with a progress window
    /// ([`super::msi`]).
    WindowsMsi {
        per_machine: bool,
    },
    /// macOS, unpacked from the release tarball into a folder the user can write to:
    /// swapped in place like the portable zip.
    MacArchive,
    /// Windows, installed by Scoop (`scoop update soyuz`).
    Scoop,
    /// macOS, installed by Homebrew (`brew upgrade --cask soyuz`).
    Homebrew,
    /// Under `/usr` (rpm / AUR / distribution package).
    Package,
    Flatpak,
    AppImage,
    /// Windows, an MSIX package (`…\WindowsApps\…`).
    Msix,
    /// A copy that could update itself, in a folder this user cannot write to.
    ReadOnly,
}

impl InstallKind {
    /// "Update and restart" downloads, verifies, installs and restarts this copy.
    pub fn self_installs(self) -> bool {
        matches!(
            self,
            InstallKind::Tarball
                | InstallKind::WindowsPortable
                | InstallKind::WindowsMsi { .. }
                | InstallKind::MacArchive
        )
    }

    /// Something else updates this copy; Soyuz never downloads or installs for it.
    pub fn managed(self) -> bool {
        !self.self_installs() && self != InstallKind::Manual
    }

    /// `$kind` selector of `services-update-package-managed`.
    pub fn key(self) -> &'static str {
        match self {
            InstallKind::Package => "system",
            InstallKind::Flatpak => "flatpak",
            InstallKind::AppImage => "appimage",
            InstallKind::Scoop => "scoop",
            InstallKind::Homebrew => "homebrew",
            InstallKind::Msix => "msix",
            InstallKind::ReadOnly => "readonly",
            _ => "other",
        }
    }
}

/// What the path alone does not say (read by [`detect_install`] on a worker thread).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallFacts {
    /// A `portable` marker sits next to the executable.
    pub portable: bool,
    /// The MSI installations recorded in the registry (Windows).
    pub msi: Vec<MsiInstall>,
    /// A file can be created in the executable's folder.
    pub writable: bool,
}

impl Default for InstallFacts {
    /// Nothing known: no marker, no MSI, a writable folder.
    fn default() -> Self {
        InstallFacts { portable: false, msi: Vec::new(), writable: true }
    }
}

/// The path components of `p`, lower-case. Split by hand on both separators:
/// `Path::components` only knows the host's.
fn parts(p: &Path) -> Vec<String> {
    p.to_string_lossy().split(['\\', '/']).filter(|c| !c.is_empty()).map(str::to_ascii_lowercase).collect()
}

/// A Windows folder path in one spelling: no verbatim prefix, backslashes, no trailing
/// separator, lower-case.
fn win_norm(p: &str) -> String {
    let p = p.strip_prefix(r"\\?\").unwrap_or(p).replace('/', "\\");
    p.trim_end_matches('\\').to_ascii_lowercase()
}

/// The folder of a Windows executable path, normalized ([`win_norm`]).
fn win_dir(exe: &Path) -> String {
    let s = exe.to_string_lossy().replace('/', "\\");
    win_norm(s.rsplit_once('\\').map_or("", |(dir, _)| dir))
}

/// Where the MSI installs when the user does not pick a folder.
fn default_msi_dir(per_machine: bool, env: &dyn Fn(&str) -> Option<OsString>) -> Option<String> {
    let (root, tail) = if per_machine {
        (env("ProgramW6432").or_else(|| env("ProgramFiles"))?, "Soyuz")
    } else {
        (env("LOCALAPPDATA")?, r"Programs\Soyuz")
    };
    let root = win_norm(&root.to_string_lossy());
    (!root.is_empty()).then(|| win_norm(&format!(r"{root}\{tail}")))
}

fn windows_kind(exe: &Path, env: &dyn Fn(&str) -> Option<OsString>, facts: &InstallFacts) -> InstallKind {
    let parts = parts(exe);
    if parts.iter().any(|c| c == "windowsapps") {
        return InstallKind::Msix;
    }
    let dir = win_dir(exe);
    let under = |root: Option<OsString>| {
        root.map(|r| win_norm(&r.to_string_lossy()))
            .is_some_and(|r| !r.is_empty() && (dir == r || dir.starts_with(&format!(r"{r}\"))))
    };
    // `<scoop>\apps\soyuz\<version or current>\soyuz.exe`: Scoop unpacks the portable zip
    // (marker included), so this comes before the marker.
    if parts.windows(2).any(|w| w[0] == "scoop" && w[1] == "apps") || under(env("SCOOP")) || under(env("SCOOP_GLOBAL"))
    {
        return InstallKind::Scoop;
    }
    if facts.portable {
        return if facts.writable { InstallKind::WindowsPortable } else { InstallKind::ReadOnly };
    }
    // An MSI install is the copy in the folder the installer recorded (or, for installers
    // that did not record it yet, the default folder of that scope). Another copy on a
    // computer that also has an MSI install is not it.
    for m in &facts.msi {
        let recorded = m.install_dir.as_ref().map(|d| win_norm(&d.to_string_lossy()));
        let here = match recorded {
            Some(d) => d == dir,
            None => default_msi_dir(m.per_machine, env).is_some_and(|d| d == dir),
        };
        if here {
            return InstallKind::WindowsMsi { per_machine: m.per_machine };
        }
    }
    InstallKind::Manual
}

fn macos_kind(exe: &Path, facts: &InstallFacts) -> InstallKind {
    let s = exe.to_string_lossy();
    if s.contains("/Caskroom/") || s.contains("/Cellar/") || s.starts_with("/opt/homebrew/") {
        InstallKind::Homebrew
    } else if s.contains(".app/Contents/") {
        // A signed, notarized bundle: replacing files inside it would break its seal.
        InstallKind::Manual
    } else if exe.starts_with("/usr") && !exe.starts_with("/usr/local") {
        InstallKind::Package
    } else if facts.writable {
        InstallKind::MacArchive
    } else {
        InstallKind::ReadOnly
    }
}

fn linux_kind(exe: &Path, env: &dyn Fn(&str) -> Option<OsString>, facts: &InstallFacts) -> InstallKind {
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if set("FLATPAK_ID") || exe.starts_with("/app") {
        InstallKind::Flatpak
    } else if set("APPIMAGE") || (set("APPDIR") && env("APPDIR").is_some_and(|d| exe.starts_with(d))) {
        InstallKind::AppImage
    } else if exe.starts_with("/usr") && !exe.starts_with("/usr/local") {
        InstallKind::Package
    } else if facts.writable {
        InstallKind::Tarball
    } else {
        InstallKind::ReadOnly
    }
}

/// [`InstallKind`] of an executable at `exe` on `os`, with environment lookup `env` and
/// the `facts` read from the system.
pub fn install_kind_with(
    os: &str,
    exe: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    facts: &InstallFacts,
) -> InstallKind {
    match os {
        "windows" => windows_kind(exe, &env, facts),
        "macos" => macos_kind(exe, facts),
        "linux" => linux_kind(exe, &env, facts),
        _ => InstallKind::Manual,
    }
}

/// [`install_kind_with`] from the path and the environment alone (no marker, no MSI, a
/// writable folder): enough to tell the package-managed kinds.
pub fn install_kind_for(os: &str, exe: &Path, env: impl Fn(&str) -> Option<OsString>) -> InstallKind {
    install_kind_with(os, exe, env, &InstallFacts::default())
}

/// The running executable: on unix with symbolic links resolved (Homebrew starts the
/// cask through a link) and without the ` (deleted)` that `/proc/self/exe` adds once the
/// binary was replaced.
pub fn running_exe() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let exe = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map(PathBuf::from).unwrap_or(exe);
    if cfg!(unix) { Ok(std::fs::canonicalize(&exe).unwrap_or(exe)) } else { Ok(exe) }
}

static DETECTED: Mutex<Option<InstallKind>> = Mutex::new(None);

/// Work out the [`InstallKind`] of the running copy and remember it. Reads the registry
/// and, for a copy that would replace its own files, creates (and removes) a temp file
/// next to the executable: worker threads only.
pub fn detect_install() -> InstallKind {
    if let Some(k) = *DETECTED.lock() {
        return k;
    }
    let exe = running_exe().unwrap_or_default();
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut facts = InstallFacts {
        portable: soyuz_core::config::portable_dir(&dir).is_some(),
        msi: soyuz_fs::integration::msi_installs(),
        writable: true,
    };
    let classify = |facts: &InstallFacts| install_kind_with(std::env::consts::OS, &exe, |k| std::env::var_os(k), facts);
    let mut kind = classify(&facts);
    // Only a copy that swaps its own files needs the folder; a package manager's folder
    // is not even probed.
    if matches!(kind, InstallKind::Tarball | InstallKind::WindowsPortable | InstallKind::MacArchive)
        && !dir_writable(&dir)
    {
        facts.writable = false;
        kind = classify(&facts);
    }
    log::info!("update: this copy is {kind:?} ({})", exe.display());
    *DETECTED.lock() = Some(kind);
    kind
}

/// [`InstallKind`] of the running copy: what [`detect_install`] found, or, before it ran,
/// what the path and the environment say.
pub fn install_kind() -> InstallKind {
    if let Some(k) = *DETECTED.lock() {
        return k;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    install_kind_for(std::env::consts::OS, &exe, |k| std::env::var_os(k))
}

/// Whether a file can be created in `dir` (one is created and removed).
pub fn dir_writable(dir: &Path) -> bool {
    match super::create_temp_in(dir) {
        Ok((file, path)) => {
            drop(file);
            let _ = std::fs::remove_file(path);
            true
        }
        Err(_) => false,
    }
}

/// What to tell the user about a copy that does not update itself: how its package
/// manager updates it, or why the download has to be installed by hand. `None` for a copy
/// "Update and restart" works on.
pub fn how_to_update(kind: InstallKind) -> Option<String> {
    match kind {
        k if k.self_installs() => None,
        InstallKind::Manual if cfg!(target_os = "macos") => Some(tr!("services-update-manual", kind = "bundle")),
        InstallKind::Manual => Some(tr!("services-update-manual", kind = "other")),
        k => Some(tr!("services-update-package-managed", kind = k.key())),
    }
}

/// [`how_to_update`] for the running copy.
pub fn self_update_note() -> Option<String> {
    how_to_update(install_kind())
}

/// The asset of `release` for a copy of `kind` on `os` / `arch`. A copy that updates
/// itself takes exactly the format its install path understands (the MSI for an MSI
/// install, the portable zip for a portable copy, the tarball on Linux and macOS);
/// other copies get what [`super::pick_asset`] offers for a download by hand.
pub fn pick_asset_for<'a>(release: &'a Release, os: &str, arch: &str, kind: InstallKind) -> Option<&'a Asset> {
    let order: &[(&str, Option<bool>)] = match kind {
        InstallKind::WindowsMsi { .. } => &[(".msi", None)],
        InstallKind::WindowsPortable | InstallKind::Scoop => &[(".zip", Some(true))],
        InstallKind::Tarball | InstallKind::MacArchive | InstallKind::Homebrew => &[(".tar.gz", None), (".tgz", None)],
        _ => return super::pick_asset(release, os, arch, false),
    };
    super::pick_in_order(release, os, arch, order)
}

/// [`pick_asset_for`] for the running copy.
pub fn asset_for_this_copy(release: &Release) -> Option<&Asset> {
    pick_asset_for(release, std::env::consts::OS, std::env::consts::ARCH, install_kind())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<OsString> {
        None
    }

    fn facts(portable: bool, writable: bool) -> InstallFacts {
        InstallFacts { portable, msi: Vec::new(), writable }
    }

    #[test]
    fn linux_kinds() {
        let k = |exe: &str| install_kind_for("linux", Path::new(exe), none);
        assert_eq!(k("/usr/bin/soyuz"), InstallKind::Package);
        assert_eq!(k("/usr/local/bin/soyuz"), InstallKind::Tarball);
        assert_eq!(k("/home/me/.local/bin/soyuz"), InstallKind::Tarball);
        assert_eq!(k("/app/bin/soyuz"), InstallKind::Flatpak);
        let flatpak = |v: &str| (v == "FLATPAK_ID").then(|| "io.github.x".into());
        assert_eq!(install_kind_for("linux", Path::new("/opt/x"), flatpak), InstallKind::Flatpak);
        let appimage = |v: &str| (v == "APPIMAGE").then(|| "/home/me/Soyuz.AppImage".into());
        assert_eq!(
            install_kind_for("linux", Path::new("/tmp/.mount_x/usr/bin/soyuz"), appimage),
            InstallKind::AppImage
        );
        // A tarball unpacked as root: this user cannot replace it.
        assert_eq!(
            install_kind_with("linux", Path::new("/usr/local/bin/soyuz"), none, &facts(false, false)),
            InstallKind::ReadOnly
        );
        assert_eq!(install_kind_for("freebsd", Path::new("/usr/local/bin/soyuz"), none), InstallKind::Manual);
    }

    #[test]
    fn windows_kinds() {
        let env = |k: &str| -> Option<OsString> {
            match k {
                "LOCALAPPDATA" => Some(r"C:\Users\me\AppData\Local".into()),
                "ProgramFiles" => Some(r"C:\Program Files".into()),
                _ => None,
            }
        };
        let k = |exe: &str, f: &InstallFacts| install_kind_with("windows", Path::new(exe), env, f);
        let plain = facts(false, true);
        let portable = facts(true, true);
        // A zip unpacked somewhere, a build folder: nothing Soyuz may replace by itself.
        assert_eq!(k(r"C:\Tools\Soyuz\soyuz.exe", &plain), InstallKind::Manual);
        assert_eq!(k(r"E:\src\soyuz\target\release\soyuz.exe", &plain), InstallKind::Manual);
        // The portable zip, also on a stick; read-only media cannot be updated.
        assert_eq!(k(r"D:\Apps\Soyuz\soyuz.exe", &portable), InstallKind::WindowsPortable);
        assert_eq!(k(r"D:\Apps\Soyuz\soyuz.exe", &facts(true, false)), InstallKind::ReadOnly);
        // Scoop unpacks the portable zip: it wins over the marker, whatever the spelling.
        for exe in [
            r"C:\Users\me\scoop\apps\soyuz\current\soyuz.exe",
            r"C:\Users\me\Scoop\Apps\soyuz\0.7.5\soyuz-tui.exe",
            r"C:\ProgramData\scoop\apps\soyuz\current\soyuz.exe",
            "C:/Users/me/scoop/apps/soyuz/current/soyuz.exe",
        ] {
            assert_eq!(k(exe, &portable), InstallKind::Scoop, "{exe}");
        }
        let scoop_env = |v: &str| (v == "SCOOP").then(|| OsString::from(r"D:\pkg"));
        assert_eq!(
            install_kind_with("windows", Path::new(r"d:\PKG\x\soyuz\current\soyuz.exe"), scoop_env, &portable),
            InstallKind::Scoop
        );
        assert_eq!(
            install_kind_with("windows", Path::new(r"D:\pkgs\soyuz.exe"), scoop_env, &portable),
            InstallKind::WindowsPortable,
            "a folder that only starts with the same letters is not inside it"
        );
        // MSIX packages.
        assert_eq!(k(r"C:\Program Files\WindowsApps\Soyuz_1.0_x64__abc\soyuz.exe", &portable), InstallKind::Msix);

        // MSI installs: an installer that recorded its folder is matched by it.
        let msi = |per_machine: bool, dir: Option<&str>| InstallFacts {
            msi: vec![MsiInstall { per_machine, install_dir: dir.map(PathBuf::from) }],
            ..InstallFacts::default()
        };
        let user = InstallKind::WindowsMsi { per_machine: false };
        let machine = InstallKind::WindowsMsi { per_machine: true };
        assert_eq!(k(r"D:\Soyuz\soyuz.exe", &msi(false, Some(r"D:\Soyuz\"))), user);
        assert_eq!(k(r"d:\soyuz\SOYUZ.EXE", &msi(true, Some(r"D:\Soyuz"))), machine);
        assert_eq!(k(r"\\?\D:\Soyuz\soyuz.exe", &msi(true, Some(r"D:\Soyuz"))), machine);
        // Another copy on a computer that has an MSI install is not the MSI install.
        assert_eq!(k(r"C:\Tools\Soyuz\soyuz.exe", &msi(false, Some(r"D:\Soyuz\"))), InstallKind::Manual);
        // Installers up to 0.7.5 recorded no folder: the default folder of the scope.
        assert_eq!(k(r"C:\Users\me\AppData\Local\Programs\Soyuz\soyuz.exe", &msi(false, None)), user);
        assert_eq!(k(r"C:\Program Files\Soyuz\soyuz-tui.exe", &msi(true, None)), machine);
        assert_eq!(k(r"C:\Program Files\Soyuz\soyuz.exe", &msi(false, None)), InstallKind::Manual);
        assert_eq!(k(r"D:\Custom\soyuz.exe", &msi(false, None)), InstallKind::Manual);
        // Both scopes installed: the copy that runs decides.
        let both = InstallFacts {
            msi: vec![
                MsiInstall { per_machine: false, install_dir: None },
                MsiInstall { per_machine: true, install_dir: None },
            ],
            ..InstallFacts::default()
        };
        assert_eq!(k(r"C:\Program Files\Soyuz\soyuz.exe", &both), machine);
        assert_eq!(k(r"C:\Users\me\AppData\Local\Programs\Soyuz\soyuz.exe", &both), user);
    }

    #[test]
    fn macos_kinds() {
        let k = |exe: &str, writable: bool| install_kind_with("macos", Path::new(exe), none, &facts(false, writable));
        assert_eq!(k("/Users/me/Apps/soyuz-0.7.5-macos-arm64/soyuz", true), InstallKind::MacArchive);
        assert_eq!(k("/opt/soyuz/soyuz", false), InstallKind::ReadOnly);
        assert_eq!(k("/opt/homebrew/Caskroom/soyuz/0.7.5/soyuz-0.7.5-macos-arm64/soyuz", true), InstallKind::Homebrew);
        assert_eq!(k("/usr/local/Caskroom/soyuz/0.7.5/soyuz-0.7.5-macos-arm64/soyuz", true), InstallKind::Homebrew);
        assert_eq!(k("/Applications/Soyuz.app/Contents/MacOS/soyuz", true), InstallKind::Manual);
    }

    #[test]
    fn only_some_kinds_install_themselves() {
        use InstallKind::*;
        let own =
            [Tarball, WindowsPortable, WindowsMsi { per_machine: false }, WindowsMsi { per_machine: true }, MacArchive];
        let managed = [Scoop, Homebrew, Package, Flatpak, AppImage, Msix, ReadOnly];
        for k in own {
            assert!(k.self_installs() && !k.managed(), "{k:?}");
            assert_eq!(how_to_update(k), None, "{k:?}");
        }
        for k in managed {
            assert!(!k.self_installs() && k.managed(), "{k:?}");
            let note = how_to_update(k).unwrap_or_else(|| panic!("{k:?} says how to update"));
            assert!(!note.is_empty(), "{k:?}");
            assert_ne!(k.key(), "other", "{k:?} has its own text");
        }
        assert!(how_to_update(Scoop).unwrap().contains("scoop update soyuz"));
        assert!(how_to_update(Homebrew).unwrap().contains("brew upgrade"));
        assert!(how_to_update(Flatpak).unwrap().contains("flatpak update"));
        assert!(!Manual.self_installs() && !Manual.managed());
        assert!(how_to_update(Manual).is_some());
    }

    fn release(names: &[&str]) -> Release {
        Release {
            tag: "v0.8.0".into(),
            version: super::super::Version::parse("0.8.0").unwrap(),
            title: String::new(),
            notes: String::new(),
            url: String::new(),
            prerelease: false,
            published: String::new(),
            assets: names
                .iter()
                .map(|n| Asset {
                    name: (*n).into(),
                    url: format!("https://example.invalid/{n}"),
                    size: 1,
                    sha256: None,
                })
                .collect(),
        }
    }

    #[test]
    fn each_install_kind_takes_its_own_asset() {
        use InstallKind::*;
        let r = release(&[
            "soyuz-0.8.0-windows-x64.zip",
            "soyuz-0.8.0-windows-x64-portable.zip",
            "soyuz-0.8.0-windows-x64.msi",
            "soyuz-0.8.0-linux-x86_64.tar.gz",
            "soyuz-0.8.0-macos-arm64.tar.gz",
            "soyuz-0.8.0-macos-universal.zip",
            "SHA256SUMS-windows.txt",
            "ffmpeg-runtime-9.0.2-windows-x64.zip",
        ]);
        let name = |os, arch, kind| pick_asset_for(&r, os, arch, kind).map(|a| a.name.as_str());
        let win = |kind| name("windows", "x86_64", kind);
        assert_eq!(win(WindowsMsi { per_machine: false }), Some("soyuz-0.8.0-windows-x64.msi"));
        assert_eq!(win(WindowsMsi { per_machine: true }), Some("soyuz-0.8.0-windows-x64.msi"));
        assert_eq!(win(WindowsPortable), Some("soyuz-0.8.0-windows-x64-portable.zip"));
        assert_eq!(win(Scoop), Some("soyuz-0.8.0-windows-x64-portable.zip"));
        assert_eq!(win(Manual), Some("soyuz-0.8.0-windows-x64.msi"));
        assert_eq!(win(Msix), Some("soyuz-0.8.0-windows-x64.msi"));
        assert_eq!(name("linux", "x86_64", Tarball), Some("soyuz-0.8.0-linux-x86_64.tar.gz"));
        assert_eq!(name("linux", "x86_64", Package), Some("soyuz-0.8.0-linux-x86_64.tar.gz"));
        assert_eq!(name("linux", "aarch64", Tarball), None);
        assert_eq!(name("macos", "aarch64", MacArchive), Some("soyuz-0.8.0-macos-arm64.tar.gz"));
        assert_eq!(name("macos", "aarch64", Homebrew), Some("soyuz-0.8.0-macos-arm64.tar.gz"));
        assert_eq!(name("macos", "aarch64", Manual), Some("soyuz-0.8.0-macos-universal.zip"), "the app bundle");
        assert_eq!(name("macos", "x86_64", MacArchive), None, "no Intel tarball: nothing to swap in");

        // A copy that installs itself never falls back to another format: an MSI install
        // is not "updated" with a zip, a portable copy not with the installer's zip.
        let r = release(&["soyuz-0.8.0-windows-x64.zip", "soyuz-0.8.0-linux-x86_64.zip"]);
        assert_eq!(pick_asset_for(&r, "windows", "x86_64", WindowsMsi { per_machine: false }), None);
        assert_eq!(pick_asset_for(&r, "windows", "x86_64", WindowsPortable), None);
        assert_eq!(pick_asset_for(&r, "linux", "x86_64", Tarball), None);
    }
}
