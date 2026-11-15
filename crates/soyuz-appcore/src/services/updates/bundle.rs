//! In-place update of a macOS app bundle (`Soyuz.app`) from the release's disk image
//! ([`install_dmg`]; macOS only).
//!
//! Files inside a signed bundle are never replaced one by one: that would break its seal.
//! The whole bundle is swapped instead, and only for a copy that can hold the update to
//! something ([`super::kind::InstallKind::MacBundle`]: Developer ID signed, not
//! translocated, not on a read-only volume, in a folder this user can write to).
//!
//! 1. The disk image (already checked against the release's SHA-256) is hashed again and
//!    attached read-only, hidden and at a random mount point inside the update's work
//!    folder. It is detached on every path ([`Mounted`]'s drop, `-force` if needed).
//! 2. The image must hold exactly one `.app`, with this app's `CFBundleIdentifier` and the
//!    release's version as `CFBundleShortVersionString`. Its signature must verify
//!    (`codesign --verify --deep --strict`), must come from a Developer ID certificate of
//!    the team that signed the running copy ([`requirement`]), and Gatekeeper must accept
//!    it (`spctl --assess`: notarized).
//! 3. It is copied (`ditto`) to a hidden staging folder next to the installed bundle (same
//!    volume), checked again there, and exchanged with the installed bundle in one atomic
//!    step (`renameatx_np(RENAME_SWAP)` through `soyuz_fs::mv::rename(…, Exchange)`), so
//!    the installed path always holds a complete bundle ([`swap_bundle`]). Where the volume
//!    cannot swap, two renames do it (installed → backup, staged → installed) with roll
//!    back. Any failure leaves the old bundle in place and removes the copy; the old
//!    bundle is then deleted.
//! 4. After the app has quit the normal way, a small detached shell waits for this process
//!    to end and starts the new bundle with `open -n` ([`spawn_relaunch`]); the terminal UI
//!    starts its own new executable instead.
//!
//! Apple's tools are run by their full paths: an app started from Finder gets a minimal
//! `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use super::kind::bundle_of;
use super::sha256_of;

const HDIUTIL: &str = "/usr/bin/hdiutil";
const CODESIGN: &str = "/usr/bin/codesign";
const SPCTL: &str = "/usr/sbin/spctl";
const DITTO: &str = "/usr/bin/ditto";
const PLUTIL: &str = "/usr/bin/plutil";
const OPEN: &str = "/usr/bin/open";
const SH: &str = "/bin/sh";

/// Staging copies of a running (or interrupted) swap: `.<Name>.app.update-<pid>`.
const STAGE_TAG: &str = ".update-";
/// The replaced bundle until it is deleted: `.<Name>.app.old-<pid>`.
const BACKUP_TAG: &str = ".old-";
/// Staging copies older than this belong to an interrupted run.
const STALE: Duration = Duration::from_secs(3600);

fn failed(e: &dyn std::fmt::Display) -> String {
    tr!("services-update-install-failed", error = e.to_string())
}

// ---- pure helpers ----------------------------------------------------------------------

/// Whether `team` looks like an Apple Developer team identifier (ten upper-case letters
/// or digits), so it can be put into a requirement without quoting surprises.
fn valid_team(team: &str) -> bool {
    team.len() == 10 && team.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// The team identifier from `codesign -dv` output (`TeamIdentifier=…`); `None` for an
/// ad-hoc signed or unsigned program (`not set`, or no such line).
pub fn parse_team(codesign_dv: &str) -> Option<String> {
    codesign_dv
        .lines()
        .find_map(|l| l.trim().strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|t| valid_team(t))
        .map(str::to_string)
}

/// The code requirement an update must satisfy: signed through Apple's chain with a
/// Developer ID Application certificate (the Developer ID intermediate and leaf marker
/// OIDs) issued to `team`, the team of the running copy (so a fork signed by another
/// team updates from its own releases). `None` for a malformed team.
pub fn requirement(team: &str) -> Option<String> {
    valid_team(team).then(|| {
        format!(
            "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
             and certificate leaf[field.1.2.840.113635.100.6.1.13] exists \
             and certificate leaf[subject.OU] = \"{team}\""
        )
    })
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Every `<string>` value that directly follows `<key>{key}</key>` in an XML property
/// list (any depth), in document order.
pub fn plist_strings(xml: &str, key: &str) -> Vec<String> {
    let tag = format!("<key>{key}</key>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&tag) {
        rest = &rest[at + tag.len()..];
        let after = rest.trim_start();
        if let Some(body) = after.strip_prefix("<string>")
            && let Some(end) = body.find("</string>")
        {
            out.push(xml_unescape(&body[..end]));
        }
    }
    out
}

/// The first [`plist_strings`] value of `key`.
pub fn plist_string(xml: &str, key: &str) -> Option<String> {
    plist_strings(xml, key).into_iter().next()
}

/// Hidden siblings of `app` for the swap: (staging copy, backup) for process `pid`.
fn sibling_paths(app: &Path, pid: u32) -> Option<(PathBuf, PathBuf)> {
    let parent = app.parent()?;
    let name = app.file_name()?.to_string_lossy();
    Some((parent.join(format!(".{name}{STAGE_TAG}{pid}")), parent.join(format!(".{name}{BACKUP_TAG}{pid}"))))
}

/// The file operations of [`swap_bundle`] (tests inject failures).
pub struct SwapOps<'a> {
    /// Copy the new bundle to the staging path.
    pub copy: &'a mut dyn FnMut(&Path, &Path) -> Result<(), String>,
    /// Verify the staged copy.
    pub check: &'a mut dyn FnMut(&Path) -> Result<(), String>,
    /// Exchange two paths atomically (`Err`, e.g. `ENOTSUP`: the volume cannot).
    pub exchange: &'a mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
    /// A plain rename, for the fallback.
    pub rename: &'a mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
}

/// The atomic exchange of [`SwapOps`]: `renameatx_np(RENAME_SWAP)` through `soyuz_fs`.
fn real_exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    soyuz_fs::mv::rename(a, b, soyuz_fs::mv::RenameMode::Exchange).map_err(|e| std::io::Error::other(e.to_string()))
}

/// Replace the bundle `installed` with `source` (blocking). `ops.copy` copies `source` to
/// a hidden staging path next to `installed` (same volume) and `ops.check` verifies that
/// copy. Then `ops.exchange` swaps staged and installed in one step, so the installed path
/// always holds a complete bundle; if it cannot, `ops.rename` moves installed → backup and
/// staged → installed, putting the old bundle back when the second move fails. On any
/// failure the old bundle is at `installed` and the staging copy is gone. The old bundle
/// is deleted at the end (what cannot be deleted now is left for [`remove_leftovers`]).
pub fn swap_bundle(installed: &Path, source: &Path, ops: SwapOps<'_>) -> Result<(), String> {
    let SwapOps { copy, check, exchange, rename } = ops;
    let (staged, backup) = sibling_paths(installed, std::process::id()).ok_or_else(|| failed(&"no parent folder"))?;
    for left in [&staged, &backup] {
        if std::fs::symlink_metadata(left).is_ok() {
            std::fs::remove_dir_all(left).map_err(|e| failed(&e))?;
        }
    }
    let drop_staged = |e: String| {
        let _ = std::fs::remove_dir_all(&staged);
        e
    };
    copy(source, &staged).map_err(|e| drop_staged(tr!("services-update-bundle-copy", error = e)))?;
    check(&staged).map_err(drop_staged)?;
    match exchange(&staged, installed) {
        Ok(()) => {
            // The staging path now holds the old bundle.
            if let Err(e) = std::fs::remove_dir_all(&staged) {
                log::warn!("update: the previous app stays at {} until the next start: {e}", staged.display());
            }
            return Ok(());
        }
        Err(e) => log::info!("update: no atomic swap here ({e}); renaming"),
    }
    rename(installed, &backup)
        .map_err(|e| drop_staged(tr!("services-update-bundle-rolled-back", error = e.to_string())))?;
    if let Err(e) = rename(&staged, installed) {
        let put_back = rename(&backup, installed);
        let _ = std::fs::remove_dir_all(&staged);
        return Err(match put_back {
            Ok(()) => tr!("services-update-bundle-rolled-back", error = e.to_string()),
            Err(again) => {
                log::error!("update: cannot put {} back: {again}", installed.display());
                tr!(
                    "services-update-bundle-broken",
                    error = e.to_string(),
                    backup = backup.display().to_string(),
                    name = installed.display().to_string()
                )
            }
        });
    }
    if let Err(e) = std::fs::remove_dir_all(&backup) {
        log::warn!("update: the previous app stays at {} until the next start: {e}", backup.display());
    }
    Ok(())
}

/// Delete what earlier swaps of `app` left next to it: backups of finished swaps, and
/// staging copies of runs interrupted more than an hour ago. Blocking.
pub fn remove_leftovers(app: &Path) {
    let (Some(parent), Some(name)) = (app.parent(), app.file_name()) else { return };
    let name = name.to_string_lossy();
    let (stage, old) = (format!(".{name}{STAGE_TAG}"), format!(".{name}{BACKUP_TAG}"));
    let own = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for e in entries.flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = file.strip_prefix(&stage).or_else(|| file.strip_prefix(&old)) else { continue };
        if pid == own || !pid.bytes().all(|b| b.is_ascii_digit()) || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let aged =
            e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a > STALE);
        if (file.starts_with(&old) || aged)
            && let Err(err) = std::fs::remove_dir_all(e.path())
        {
            log::debug!("update leftover {}: {err}", e.path().display());
        }
    }
}

// ---- Apple's tools -----------------------------------------------------------------------

/// Run `cmd` with no input and its output captured.
fn run(cmd: &mut Command) -> std::io::Result<Output> {
    cmd.stdin(Stdio::null()).output()
}

/// The last non-empty line a tool wrote, for an error message.
fn last_line(out: &Output) -> String {
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

/// Run `cmd`; `Err` with its last message unless it succeeds.
fn run_ok(cmd: &mut Command) -> Result<Output, String> {
    let out = run(cmd).map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out)
    } else {
        let msg = last_line(&out);
        Err(if msg.is_empty() { out.status.to_string() } else { msg })
    }
}

/// The Developer ID team that signed the bundle `app` (`None`: ad-hoc signed, unsigned,
/// or `codesign` failed).
pub fn team_of(app: &Path) -> Option<String> {
    let out = run(Command::new(CODESIGN).args(["-dv", "--verbose=2"]).arg(app)).ok()?;
    // codesign prints the details on stderr.
    parse_team(&format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout)))
}

/// A bundle's `Info.plist` as XML (it may be stored binary).
fn info_plist(app: &Path) -> Result<String, String> {
    let out = run_ok(Command::new(PLUTIL).args(["-convert", "xml1", "-o", "-"]).arg(app.join("Contents/Info.plist")))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A disk image attached by [`attach`]; detached when dropped.
struct Mounted {
    point: PathBuf,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        if run_ok(Command::new(HDIUTIL).args(["detach", "-quiet"]).arg(&self.point)).is_err()
            && let Err(e) = run_ok(Command::new(HDIUTIL).args(["detach", "-quiet", "-force"]).arg(&self.point))
        {
            log::warn!("update: cannot detach {}: {e}", self.point.display());
        }
    }
}

/// Attach `dmg` read-only, without showing it in Finder or opening it, at a random mount
/// point inside `under`.
fn attach(dmg: &Path, under: &Path) -> Result<Mounted, String> {
    let mount = |e: String| tr!("services-update-bundle-mount", error = e);
    let out = run_ok(
        Command::new(HDIUTIL)
            .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-plist", "-mountrandom"])
            .arg(under)
            .arg(dmg),
    )
    .map_err(mount)?;
    let xml = String::from_utf8_lossy(&out.stdout);
    let points: Vec<PathBuf> = plist_strings(&xml, "mount-point").into_iter().map(PathBuf::from).collect();
    match points.as_slice() {
        [one] => Ok(Mounted { point: one.clone() }),
        _ => {
            // Detach whatever did get mounted.
            for p in points {
                drop(Mounted { point: p });
            }
            Err(mount("no single mount point".into()))
        }
    }
}

/// The one app bundle at the root of the mounted image `root`.
fn the_app(root: &Path) -> Result<PathBuf, String> {
    let apps: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| tr!("services-update-bundle-mount", error = e.to_string()))?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("app")))
        .collect();
    match <[PathBuf; 1]>::try_from(apps) {
        Ok([app]) => Ok(app),
        Err(_) => Err(tr!("services-update-bundle-no-app").into()),
    }
}

/// What the new bundle must be.
struct Expect<'a> {
    bundle_id: &'a str,
    version: &'a str,
    team: &'a str,
}

/// The signature of `app` verifies, strictly and with everything nested.
fn verify_signature(app: &Path) -> Result<(), String> {
    run_ok(Command::new(CODESIGN).args(["--verify", "--deep", "--strict"]).arg(app))
        .map(drop)
        .map_err(|e| tr!("services-update-bundle-signature", error = e))
}

/// The checks of step 2 (see the module documentation), on the bundle in the image.
fn check_new(app: &Path, want: &Expect<'_>) -> Result<(), String> {
    let info = info_plist(app).map_err(|e| tr!("services-update-bundle-signature", error = e))?;
    let id = plist_string(&info, "CFBundleIdentifier").unwrap_or_default();
    if id != want.bundle_id {
        return Err(tr!("services-update-bundle-other-app", id = id, want = want.bundle_id));
    }
    let version = plist_string(&info, "CFBundleShortVersionString").unwrap_or_default();
    if version != want.version {
        return Err(tr!("services-update-bundle-version", found = version, want = want.version));
    }
    verify_signature(app)?;
    let req = requirement(want.team).ok_or_else(|| tr!("services-update-bundle-team", team = want.team))?;
    run_ok(Command::new(CODESIGN).arg("--verify").arg(format!("-R={req}")).arg(app))
        .map_err(|_| tr!("services-update-bundle-team", team = want.team))?;
    run_ok(Command::new(SPCTL).args(["--assess", "--type", "execute"]).arg(app))
        .map_err(|e| tr!("services-update-bundle-gatekeeper", error = e))?;
    Ok(())
}

/// Replace the app bundle `exe` runs from with the one in the disk image `dmg` (already
/// verified against `sha256`, the release's checksum), which must be `version`; `work` is
/// the update's work folder (the image is mounted inside it). Blocking: call it off the UI
/// thread. Returns the installed bundle.
pub fn install_dmg(dmg: &Path, sha256: &str, exe: &Path, work: &Path, version: &str) -> Result<PathBuf, String> {
    let mut file = std::fs::File::open(dmg).map_err(|e| failed(&e))?;
    if !sha256_of(&mut file).map_err(|e| failed(&e))?.eq_ignore_ascii_case(sha256) {
        return Err(tr!("services-update-install-changed").into());
    }
    drop(file);
    let app = bundle_of(exe).ok_or_else(|| failed(&"not an app bundle"))?.to_path_buf();
    let team = team_of(&app).ok_or_else(|| tr!("services-update-bundle-unsigned"))?;
    let info = info_plist(&app).map_err(|e| failed(&e))?;
    let bundle_id = plist_string(&info, "CFBundleIdentifier").ok_or_else(|| failed(&"no CFBundleIdentifier"))?;

    let mounted = attach(dmg, work)?;
    let new_app = the_app(&mounted.point)?;
    check_new(&new_app, &Expect { bundle_id: &bundle_id, version, team: &team })?;
    swap_bundle(
        &app,
        &new_app,
        SwapOps {
            copy: &mut |from, to| run_ok(Command::new(DITTO).arg("--noqtn").arg(from).arg(to)).map(drop),
            check: &mut |staged| verify_signature(staged),
            exchange: &mut real_exchange,
            rename: &mut |from, to| std::fs::rename(from, to),
        },
    )?;
    drop(mounted);
    log::info!("update: {} replaced by the {version} bundle ({team})", app.display());
    Ok(app)
}

/// Environment the new app is opened with (`open --env`), from this process's `env`:
/// `HOME`, so the new version reads the settings of the copy that was updated (Soyuz's
/// folders hang off it), and in debug builds the update test feed.
fn relaunch_env(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut keys = vec!["HOME"];
    if cfg!(debug_assertions) {
        keys.push("SOYUZ_UPDATE_FEED");
    }
    let mut out = Vec::new();
    for key in keys {
        if let Some(value) = env(key).filter(|v| !v.is_empty()) {
            let mut pair = std::ffi::OsString::from(format!("{key}="));
            pair.push(value);
            out.push("--env".into());
            out.push(pair);
        }
    }
    out
}

/// The command the relaunch helper runs: `open -n [--env K=V …] <app>`.
fn relaunch_command(app: &Path, env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let mut cmd: Vec<std::ffi::OsString> = vec![OPEN.into(), "-n".into()];
    cmd.extend(relaunch_env(env));
    cmd.push(app.into());
    cmd
}

/// Start a detached shell that waits for this process to end and then opens `app` as a
/// new instance ([`relaunch_command`]). No terminal, no inherited input or output, its
/// own process group.
pub fn spawn_relaunch(app: &Path) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    Command::new(SH)
        .arg("-c")
        .arg(r#"pid="$1"; shift; while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done; exec "$@""#)
        .arg("soyuz-relaunch")
        .arg(std::process::id().to_string())
        .args(relaunch_command(app, &|k| std::env::var_os(k)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_and_requirement() {
        let dv = "Executable=/Applications/Soyuz.app/Contents/MacOS/soyuz\nIdentifier=dev.soyuz.explorer\n\
                  Authority=Developer ID Application: Aydemir Sahin (N6FA79GV97)\nTeamIdentifier=N6FA79GV97\n";
        assert_eq!(parse_team(dv).as_deref(), Some("N6FA79GV97"));
        assert_eq!(parse_team("Signature=adhoc\nTeamIdentifier=not set\n"), None, "ad-hoc");
        assert_eq!(parse_team("code object is not signed at all"), None);
        assert_eq!(parse_team("TeamIdentifier=x\" or true"), None);
        assert_eq!(
            requirement("N6FA79GV97").unwrap(),
            "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate \
             leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"N6FA79GV97\""
        );
        assert_eq!(requirement("n6fa79gv97"), None);
        assert_eq!(requirement("ABC\" or 1"), None);
    }

    #[test]
    fn the_relaunch_keeps_the_settings_folder() {
        let env = |k: &str| match k {
            "HOME" => Some("/Users/me test".into()),
            "SOYUZ_UPDATE_FEED" => Some("http://127.0.0.1:1/r.json".into()),
            _ => None,
        };
        let cmd = relaunch_command(Path::new("/Applications/Soyuz.app"), &env);
        let mut want: Vec<std::ffi::OsString> =
            vec!["/usr/bin/open".into(), "-n".into(), "--env".into(), "HOME=/Users/me test".into()];
        if cfg!(debug_assertions) {
            want.extend(["--env".into(), "SOYUZ_UPDATE_FEED=http://127.0.0.1:1/r.json".into()]);
        }
        want.push("/Applications/Soyuz.app".into());
        assert_eq!(cmd, want);
        assert_eq!(relaunch_command(Path::new("/A.app"), &|_| None), ["/usr/bin/open", "-n", "/A.app"]);
    }

    #[test]
    fn plist_values() {
        let info = "<plist><dict>\n  <key>CFBundleIdentifier</key>\n  <string>dev.soyuz.explorer</string>\n\
                    <key>CFBundleShortVersionString</key><string>0.8.4</string>\n\
                    <key>NSHumanReadableCopyright</key><string>A &amp; B</string>\n\
                    <key>LSArchitecturePriority</key><array><string>arm64</string></array></dict></plist>";
        assert_eq!(plist_string(info, "CFBundleIdentifier").as_deref(), Some("dev.soyuz.explorer"));
        assert_eq!(plist_string(info, "CFBundleShortVersionString").as_deref(), Some("0.8.4"));
        assert_eq!(plist_string(info, "NSHumanReadableCopyright").as_deref(), Some("A & B"));
        assert_eq!(plist_string(info, "LSArchitecturePriority"), None, "not a string");
        assert_eq!(plist_string(info, "CFBundleVersion"), None);
        // `hdiutil attach -plist`: one mount point among the image's entities.
        let attach = "<dict><key>system-entities</key><array>\
                      <dict><key>content-hint</key><string>GUID_partition_scheme</string></dict>\
                      <dict><key>mount-point</key><string>/tmp/w/dmg.Ab12</string></dict></array></dict>";
        assert_eq!(plist_strings(attach, "mount-point"), ["/tmp/w/dmg.Ab12"]);
    }

    /// A fake bundle at `app` whose main program says `tag`.
    fn fake_bundle(app: &Path, tag: &str) {
        for (rel, data) in [
            ("Contents/Info.plist", tag),
            ("Contents/MacOS/soyuz", tag),
            ("Contents/MacOS/soyuz-tui", tag),
            ("Contents/Frameworks/libavcodec.63.dylib", tag),
        ] {
            let p = app.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        }
    }

    fn read(app: &Path) -> String {
        std::fs::read_to_string(app.join("Contents/MacOS/soyuz")).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
        let ok = Command::new(DITTO).arg(from).arg(to).status().map_err(|e| e.to_string())?.success();
        if ok { Ok(()) } else { Err("ditto failed".into()) }
    }

    fn no_exchange(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(45)) // ENOTSUP
    }

    fn plain_rename(a: &Path, b: &Path) -> std::io::Result<()> {
        std::fs::rename(a, b)
    }

    /// The bundle at `installed` and `source`, the old and the new, in a temp folder.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let apps = d.path().join("Applications");
        let installed = apps.join("Soyuz.app");
        let source = d.path().join("image/Soyuz.app");
        fake_bundle(&installed, "old");
        std::fs::write(installed.join("Contents/Frameworks/libavcodec.62.dylib"), "old major").unwrap();
        fake_bundle(&source, "new");
        std::fs::write(apps.join("notes.txt"), "the user's").unwrap();
        (d, apps, installed, source)
    }

    fn assert_new(apps: &Path, installed: &Path, source: &Path) {
        assert_eq!(read(installed), "new");
        assert_eq!(std::fs::read_to_string(installed.join("Contents/MacOS/soyuz-tui")).unwrap(), "new");
        // The bundle is replaced as a whole: nothing of the old one is mixed in.
        assert!(!installed.join("Contents/Frameworks/libavcodec.62.dylib").exists());
        assert_eq!(names(apps), ["Soyuz.app", "notes.txt"], "no staging copy, no backup");
        assert_eq!(read(source), "new", "the image is only read");
    }

    fn assert_old(apps: &Path, installed: &Path, what: &str) {
        assert_eq!(read(installed), "old", "{what}");
        assert!(installed.join("Contents/Frameworks/libavcodec.62.dylib").exists(), "{what}");
        assert_eq!(names(apps), ["Soyuz.app", "notes.txt"], "{what}");
    }

    /// On this volume (APFS) the real swap exchanges the bundles in one step: the
    /// installed path is never empty, and no plain rename is needed.
    #[test]
    fn the_whole_bundle_is_swapped_atomically() {
        let (_d, apps, installed, source) = setup();
        let mut renames = 0;
        swap_bundle(
            &installed,
            &source,
            SwapOps {
                copy: &mut copy_tree,
                check: &mut |_| Ok(()),
                exchange: &mut real_exchange,
                rename: &mut |a, b| {
                    renames += 1;
                    std::fs::rename(a, b)
                },
            },
        )
        .unwrap();
        assert_new(&apps, &installed, &source);
        assert_eq!(renames, 0, "the atomic swap did it");
    }

    /// A volume without the swap call: two renames, the installed bundle first.
    #[test]
    fn without_the_swap_call_two_renames_do_it() {
        let (_d, apps, installed, source) = setup();
        let mut renames = Vec::new();
        swap_bundle(
            &installed,
            &source,
            SwapOps {
                copy: &mut copy_tree,
                check: &mut |_| Ok(()),
                exchange: &mut no_exchange,
                rename: &mut |a, b| {
                    renames.push((a.to_path_buf(), b.to_path_buf()));
                    std::fs::rename(a, b)
                },
            },
        )
        .unwrap();
        assert_new(&apps, &installed, &source);
        assert_eq!(renames.len(), 2);
        assert_eq!(renames[0].0, installed);
        assert_eq!(renames[1].1, installed);
    }

    #[test]
    fn a_failed_swap_keeps_the_old_bundle() {
        let (_d, apps, installed, source) = setup();

        // The copy fails half way: nothing at the installed path changes.
        let e = swap_bundle(
            &installed,
            &source,
            SwapOps {
                copy: &mut |_, to| {
                    std::fs::create_dir_all(to.join("Contents")).unwrap();
                    Err("disk full".into())
                },
                check: &mut |_| Ok(()),
                exchange: &mut real_exchange,
                rename: &mut plain_rename,
            },
        )
        .unwrap_err();
        assert!(e.contains("disk full"), "{e}");
        assert_old(&apps, &installed, "copy");

        // The copy does not verify.
        let e = swap_bundle(
            &installed,
            &source,
            SwapOps {
                copy: &mut copy_tree,
                check: &mut |_| Err("bad seal".into()),
                exchange: &mut real_exchange,
                rename: &mut plain_rename,
            },
        )
        .unwrap_err();
        assert_eq!(e, "bad seal");
        assert_old(&apps, &installed, "check");

        // No swap call, and either rename fails: the old bundle is back, the copy gone.
        for fail_at in 0..2 {
            let mut n = 0;
            let e = swap_bundle(
                &installed,
                &source,
                SwapOps {
                    copy: &mut copy_tree,
                    check: &mut |_| Ok(()),
                    exchange: &mut no_exchange,
                    rename: &mut |a, b| {
                        n += 1;
                        if n == fail_at + 1 {
                            return Err(std::io::Error::other("busy"));
                        }
                        std::fs::rename(a, b)
                    },
                },
            )
            .unwrap_err();
            assert!(e.contains("busy"), "{e}");
            assert_old(&apps, &installed, &format!("rename {fail_at}"));
        }

        // The second rename fails and so does putting the old bundle back: the message
        // says where it is, and it is still complete there.
        let mut n = 0;
        let e = swap_bundle(
            &installed,
            &source,
            SwapOps {
                copy: &mut copy_tree,
                check: &mut |_| Ok(()),
                exchange: &mut no_exchange,
                rename: &mut |a, b| {
                    n += 1;
                    if n >= 2 {
                        return Err(std::io::Error::other("busy"));
                    }
                    std::fs::rename(a, b)
                },
            },
        )
        .unwrap_err();
        let (_, backup) = sibling_paths(&installed, std::process::id()).unwrap();
        assert!(e.contains(&backup.display().to_string()), "{e}");
        assert_eq!(read(&backup), "old");
        assert!(!installed.exists());
    }

    /// The real tools on a disk image made here: it is attached hidden and read-only, the
    /// one app in it is found, a wrong version is refused before anything is copied, and
    /// the image is detached again.
    #[test]
    fn a_disk_image_is_checked_and_always_detached() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        let app = root.join("Soyuz.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/MacOS/soyuz"), "#!/bin/sh\n").unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>dev.soyuz.explorer</string>\
             <key>CFBundleShortVersionString</key><string>0.8.4</string></dict></plist>\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("/Applications", root.join("Applications")).unwrap();
        let dmg = d.path().join("test.dmg");
        let made = Command::new(HDIUTIL)
            .args(["create", "-quiet", "-fs", "HFS+", "-format", "UDZO", "-volname", "Soyuz test", "-srcfolder"])
            .arg(&root)
            .arg(&dmg)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("hdiutil create is not available here; skipped");
            return;
        }
        let work = d.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let point = {
            let mounted = attach(&dmg, &work).unwrap();
            assert!(mounted.point.starts_with(work.canonicalize().unwrap()), "{}", mounted.point.display());
            let found = the_app(&mounted.point).unwrap();
            assert_eq!(found.file_name().unwrap(), "Soyuz.app", "the /Applications link is not an app");
            assert!(std::fs::write(mounted.point.join("x"), "x").is_err(), "read-only");
            let want = |bundle_id, version| Expect { bundle_id, version, team: "N6FA79GV97" };
            let e = check_new(&found, &want("dev.soyuz.other", "0.8.4")).unwrap_err();
            assert!(e.contains("dev.soyuz.explorer"), "{e}");
            let e = check_new(&found, &want("dev.soyuz.explorer", "0.8.5")).unwrap_err();
            assert!(e.contains("0.8.4") && e.contains("0.8.5"), "{e}");
            // Right name and version, but not signed.
            let e = check_new(&found, &want("dev.soyuz.explorer", "0.8.4")).unwrap_err();
            assert!(e.contains("code signature"), "{e}");
            mounted.point.clone()
        };
        let info = Command::new(HDIUTIL).arg("info").output().unwrap();
        assert!(!String::from_utf8_lossy(&info.stdout).contains(&*point.to_string_lossy()), "detached");
        assert_eq!(the_app(d.path()).unwrap_err(), tr!("services-update-bundle-no-app"));
    }

    #[test]
    fn leftovers_of_other_runs_are_removed() {
        let d = tempfile::tempdir().unwrap();
        let app = d.path().join("Soyuz.app");
        fake_bundle(&app, "cur");
        let mine = d.path().join(format!(".Soyuz.app.update-{}", std::process::id()));
        let running = d.path().join(".Soyuz.app.update-1");
        let old = d.path().join(".Soyuz.app.old-2");
        let other = d.path().join(".Other.app.old-3");
        for p in [&mine, &running, &old, &other] {
            std::fs::create_dir_all(p).unwrap();
        }
        remove_leftovers(&app);
        assert!(mine.exists() && running.exists(), "a recent staging copy may be another run's");
        assert!(!old.exists());
        assert!(other.exists() && app.exists());
    }
}
