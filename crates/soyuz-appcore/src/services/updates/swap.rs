//! In-place update of a copy unpacked from a release archive: the Windows portable zip
//! and the macOS tarball ([`install_archive_to`]).
//!
//! A running program cannot be overwritten on Windows, but it can be renamed. So the new
//! files are unpacked into a staging folder next to the executable (same volume: every
//! step below is a rename), checked, and then swapped in one file at a time: the
//! installed file moves into a backup folder, the new one takes its place, the running
//! executable last ([`swap_in`]). Any failure undoes every step in reverse, so the old
//! version is complete again and still starts. The backup folder (it holds the files the
//! running process still has open) is deleted on the next start ([`remove_leftovers`]).
//!
//! Never touched: `portable-data` (the user's settings), the `portable` marker, and any
//! file the archive does not carry (except FFmpeg libraries of a major the new release
//! no longer ships, which move to the backup too).

use std::io::{Read, Seek as _};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use super::ffmpeg_runtime::new_dir_in;
use super::{MAX_ASSET, MAX_BINARY, sha256_of, walk_tar};

/// Backup folders of finished swaps, next to the executable.
pub(super) const OLD_PREFIX: &str = ".soyuz-update-old-";
/// Staging folders of running (or interrupted) swaps.
pub(super) const NEW_PREFIX: &str = ".soyuz-update-new-";
/// Most files an archive may carry.
const MAX_FILES: usize = 4096;
/// Staging folders older than this belong to an interrupted run.
const STALE: Duration = Duration::from_secs(3600);

/// The release archive being installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// `soyuz-<ver>-windows-<arch>-portable.zip`: `soyuz.exe`, `soyuz-tui.exe`, the
    /// FFmpeg DLLs and the documents, flat.
    WindowsZip,
    /// `soyuz-<ver>-macos-<arch>.tar.gz`: one top folder with `soyuz`, `soyuz-tui`, the
    /// FFmpeg dylibs and the documents.
    MacTarball,
}

impl Payload {
    /// The executables of the archive: the app, then the terminal UI.
    pub fn programs(self) -> [&'static str; 2] {
        match self {
            Payload::WindowsZip => ["soyuz.exe", "soyuz-tui.exe"],
            Payload::MacTarball => ["soyuz", "soyuz-tui"],
        }
    }

    /// An FFmpeg library file name of this platform, of any major.
    fn is_ffmpeg_lib(self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        ["avutil", "swresample", "swscale", "avcodec", "avformat"].iter().any(|lib| {
            let version = match self {
                Payload::WindowsZip => {
                    name.strip_prefix(lib).and_then(|n| n.strip_prefix('-')).and_then(|n| n.strip_suffix(".dll"))
                }
                Payload::MacTarball => name
                    .strip_prefix("lib")
                    .and_then(|n| n.strip_prefix(lib))
                    .and_then(|n| n.strip_prefix('.'))
                    .and_then(|n| n.strip_suffix(".dylib")),
            };
            version.is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
        })
    }

    /// Whether `head` (the first bytes of a file) is a program of this platform for `arch`
    /// (`std::env::consts::ARCH` names; an unknown one is not checked).
    fn is_program(self, head: &[u8], arch: &str) -> bool {
        match self {
            Payload::WindowsZip => {
                if head.len() < 0x40 || &head[..2] != b"MZ" {
                    return false;
                }
                let at = u32::from_le_bytes([head[0x3c], head[0x3d], head[0x3e], head[0x3f]]) as usize;
                let Some(pe) = head.get(at..at + 6) else { return false };
                let machine = u16::from_le_bytes([pe[4], pe[5]]);
                let want = match arch {
                    "x86_64" => Some(0x8664),
                    "aarch64" => Some(0xAA64),
                    "x86" => Some(0x014C),
                    _ => None,
                };
                &pe[..4] == b"PE\0\0" && want.is_none_or(|w| w == machine)
            }
            Payload::MacTarball => {
                let Some(magic) = head.get(..8) else { return false };
                match &magic[..4] {
                    // A universal binary: the slices are checked by the loader.
                    [0xCA, 0xFE, 0xBA, 0xBE] | [0xCA, 0xFE, 0xBA, 0xBF] => true,
                    [0xCF, 0xFA, 0xED, 0xFE] => {
                        let cpu = u32::from_le_bytes([magic[4], magic[5], magic[6], magic[7]]);
                        let want = match arch {
                            "x86_64" => Some(0x0100_0007),
                            "aarch64" => Some(0x0100_000C),
                            _ => None,
                        };
                        want.is_none_or(|w| w == cpu)
                    }
                    _ => false,
                }
            }
        }
    }
}

fn bad_archive() -> String {
    tr!("services-update-install-bad-archive").into()
}

fn failed(e: &dyn std::fmt::Display) -> String {
    tr!("services-update-install-failed", error = e.to_string())
}

/// The path of an archive entry as a safe relative path: plain names only (no `..`, no
/// root, no drive, no stream suffix). `Err` for anything else.
fn safe_rel(name: &str) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(bad_archive()),
            p if p.contains(':') || p.chars().any(char::is_control) => return Err(bad_archive()),
            p => out.push(p),
        }
    }
    if out.as_os_str().is_empty() || !out.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(bad_archive());
    }
    Ok(out)
}

/// Entries of the archive that are never installed: the user's data folder, the portable
/// marker (the installed one stays) and anything named like our own work folders.
fn skipped(rel: &Path) -> bool {
    let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase());
    let Some(first) = parts.next() else { return true };
    let single = parts.next().is_none();
    first == soyuz_core::config::PORTABLE_DATA_DIR
        || (single && first == soyuz_core::config::PORTABLE_MARKER)
        || first.starts_with(".soyuz-update-")
}

/// Unpack the regular files of `file` (already verified) into the empty folder `staging`.
fn unpack(file: &mut std::fs::File, payload: Payload, staging: &Path) -> Result<(), String> {
    file.rewind().map_err(|e| failed(&e))?;
    let mut count = 0usize;
    let mut total = 0u64;
    let mut take = |name: &str, size: u64, body: &mut dyn Read| -> Result<(), String> {
        let rel = safe_rel(name)?;
        if skipped(&rel) {
            return Ok(());
        }
        count += 1;
        total = total.saturating_add(size);
        if size > MAX_BINARY || count > MAX_FILES || total > 2 * MAX_ASSET {
            return Err(bad_archive());
        }
        let path = staging.join(&rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| failed(&e))?;
        }
        // `create_new`: a name that appears twice is refused, nothing is followed.
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { bad_archive() } else { failed(&e) })?;
        let n = std::io::copy(&mut body.take(MAX_BINARY + 1), &mut out).map_err(|e| failed(&e))?;
        if n != size {
            return Err(bad_archive());
        }
        out.sync_all().map_err(|e| failed(&e))
    };
    match payload {
        Payload::WindowsZip => {
            let mut zip = zip::ZipArchive::new(&mut *file).map_err(|_| bad_archive())?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|_| bad_archive())?;
                // Regular files only: folders and links never become anything on disk.
                if !entry.is_file() || entry.is_symlink() {
                    continue;
                }
                let (name, size) = (entry.name().map_err(|_| bad_archive())?.into_owned(), entry.size());
                take(&name, size, &mut entry)?;
            }
        }
        Payload::MacTarball => {
            let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(&mut *file));
            walk_tar(gz, |name, size, body| take(name, size, body))?;
        }
    }
    Ok(())
}

/// The folder inside `staging` that holds the programs: `staging` itself, or its only
/// subfolder (the tarball's top folder).
fn payload_root(staging: &Path, main: &str) -> Result<PathBuf, String> {
    if staging.join(main).is_file() {
        return Ok(staging.to_path_buf());
    }
    let mut dirs = std::fs::read_dir(staging).map_err(|e| failed(&e))?.flatten().map(|e| e.path());
    match (dirs.next(), dirs.next()) {
        (Some(only), None) if only.join(main).is_file() => Ok(only),
        _ => Err(tr!("services-update-install-no-program", name = main)),
    }
}

/// Every file under `root`, relative to it, sorted.
fn files_under(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let rel = rel.join(e.file_name());
            if e.file_type()?.is_dir() { walk(&e.path(), &rel, out)? } else { out.push(rel) }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, Path::new(""), &mut out)?;
    out.sort();
    Ok(out)
}

/// One step of a swap, for the way back.
enum Step {
    /// `<dir>/<rel>` was moved to `<backup>/<rel>`.
    Backed(PathBuf),
    /// `<payload>/<rel>` was moved to `<dir>/<rel>`.
    Placed(PathBuf),
    /// The folder `<dir>/<rel>` was created.
    Made(PathBuf),
}

/// Move every file of `payload` into `dir`, keeping what it replaces in a new backup
/// folder inside `dir`; `last` (a file name in `dir`: the running executable) goes last.
/// Installed files in `dir` for which `stale` says yes and that the payload does not
/// carry move to the backup as well. `rename` does the moves (`std::fs::rename`; tests
/// inject failures).
///
/// All or nothing: on any failure every step is undone in reverse order and the backup
/// folder removed, so `dir` is as it was. Returns the backup folder (`None` when nothing
/// was replaced).
pub(super) fn swap_in(
    dir: &Path,
    payload: &Path,
    last: &str,
    stale: &dyn Fn(&str) -> bool,
    rename: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<Option<PathBuf>, String> {
    let mut files = files_under(payload).map_err(|e| failed(&e))?;
    if let Some(at) = files.iter().position(|f| f.as_os_str() == last) {
        let running = files.remove(at);
        files.push(running);
    }
    let backup = new_dir_in(dir, OLD_PREFIX).map_err(|e| {
        tr!("services-update-install-no-permission-here", path = dir.display().to_string(), error = e.to_string())
    })?;
    let mut done: Vec<Step> = Vec::new();
    let result = (|| -> std::io::Result<()> {
        for rel in &files {
            let target = dir.join(rel);
            // Folders the new release adds (and their twins in the backup).
            let mut missing = Vec::new();
            let mut up = rel.parent();
            while let Some(p) = up.filter(|p| !p.as_os_str().is_empty()) {
                if !dir.join(p).is_dir() {
                    missing.push(p.to_path_buf());
                }
                up = p.parent();
            }
            for p in missing.into_iter().rev() {
                std::fs::create_dir(dir.join(&p))?;
                done.push(Step::Made(p));
            }
            match std::fs::symlink_metadata(&target) {
                Ok(m) if m.is_dir() => {
                    return Err(std::io::Error::other(format!("{} is a folder", target.display())));
                }
                Ok(_) => {
                    let kept = backup.join(rel);
                    if let Some(p) = kept.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    rename(&target, &kept)?;
                    done.push(Step::Backed(rel.clone()));
                }
                Err(_) => {}
            }
            rename(&payload.join(rel), &target)?;
            done.push(Step::Placed(rel.clone()));
        }
        // Libraries of an FFmpeg major the new release no longer ships.
        for e in std::fs::read_dir(dir)?.flatten() {
            let name = e.file_name();
            let rel = PathBuf::from(&name);
            if e.file_type().is_ok_and(|t| t.is_file()) && stale(&name.to_string_lossy()) && !files.contains(&rel) {
                rename(&e.path(), &backup.join(&rel))?;
                done.push(Step::Backed(rel));
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            // Nothing was replaced (a fresh folder): no backup to keep.
            if !done.iter().any(|s| matches!(s, Step::Backed(_))) {
                let _ = std::fs::remove_dir_all(&backup);
                return Ok(None);
            }
            Ok(Some(backup))
        }
        Err(e) => {
            let mut lost = Vec::new();
            for step in done.into_iter().rev() {
                let undone = match &step {
                    Step::Placed(rel) => rename(&dir.join(rel), &payload.join(rel)),
                    Step::Backed(rel) => rename(&backup.join(rel), &dir.join(rel)),
                    Step::Made(rel) => std::fs::remove_dir(dir.join(rel)),
                };
                if let (Err(err), Step::Placed(rel) | Step::Backed(rel)) = (undone, &step) {
                    log::error!("update: cannot put {} back: {err}", rel.display());
                    lost.push(rel.display().to_string());
                }
            }
            if lost.is_empty() {
                let _ = std::fs::remove_dir_all(&backup);
                Err(tr!("services-update-swap-rolled-back", error = e.to_string()))
            } else {
                // The old files are still in the backup folder: say where.
                Err(tr!(
                    "services-update-swap-broken",
                    error = e.to_string(),
                    files = lost.join(", "),
                    backup = backup.display().to_string()
                ))
            }
        }
    }
}

/// Replace the copy of Soyuz that `exe` belongs to with the release archive `archive`
/// (blocking: call it off the UI thread). The archive is hashed again through the handle
/// it is then unpacked from and must match `sha256`. It must carry the program `exe` is
/// (`soyuz` or `soyuz-tui`, with the platform's extension), and every program in it must
/// be one for this platform and `arch`. Returns the path to restart into (`exe`).
pub fn install_archive_to(
    archive: &Path,
    sha256: &str,
    exe: &Path,
    payload: Payload,
    arch: &str,
) -> Result<PathBuf, String> {
    let mut file = std::fs::File::open(archive).map_err(|e| failed(&e))?;
    if !sha256_of(&mut file).map_err(|e| failed(&e))?.eq_ignore_ascii_case(sha256) {
        return Err(tr!("services-update-install-changed").into());
    }
    let dir = exe.parent().filter(|d| !d.as_os_str().is_empty()).ok_or_else(|| failed(&"no parent folder"))?;
    let programs = payload.programs();
    let running = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // A renamed executable is not what the archive would put back.
    let Some(running) = programs.iter().find(|p| p.eq_ignore_ascii_case(&running)).copied() else {
        return Err(tr!("services-update-install-renamed", name = running));
    };
    let staging = new_dir_in(dir, NEW_PREFIX).map_err(|e| {
        tr!("services-update-install-no-permission-here", path = dir.display().to_string(), error = e.to_string())
    })?;
    let result = (|| -> Result<(), String> {
        unpack(&mut file, payload, &staging)?;
        let root = payload_root(&staging, programs[0])?;
        if !root.join(running).is_file() {
            return Err(tr!("services-update-install-no-program", name = running));
        }
        for program in programs {
            let path = root.join(program);
            if !path.is_file() {
                continue;
            }
            let mut head = Vec::new();
            std::fs::File::open(&path).and_then(|f| f.take(4096).read_to_end(&mut head)).map_err(|e| failed(&e))?;
            if !payload.is_program(&head, arch) {
                return Err(tr!("services-update-install-not-program", name = program));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| failed(&e))?;
            }
        }
        // Old FFmpeg libraries go only when the archive brings its own.
        let brings_ffmpeg = files_under(&root)
            .map_err(|e| failed(&e))?
            .iter()
            .any(|f| f.components().count() == 1 && payload.is_ffmpeg_lib(&f.to_string_lossy()));
        let stale = |name: &str| brings_ffmpeg && payload.is_ffmpeg_lib(name);
        swap_in(dir, &root, running, &stale, &mut |from, to| std::fs::rename(from, to)).map(drop)
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result.map(|()| exe.to_path_buf())
}

/// Delete what earlier updates left next to the executable in `dir`: the backup folders
/// of finished swaps (their files were in use until the old process ended) and staging
/// folders of runs that were interrupted more than an hour ago. Blocking; errors (a file
/// another running copy still uses) are left for the next start.
pub fn remove_leftovers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let old =
            e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a > STALE);
        if (name.starts_with(OLD_PREFIX) || (name.starts_with(NEW_PREFIX) && old))
            && let Err(err) = std::fs::remove_dir_all(e.path())
        {
            log::debug!("update leftover {}: {err}", e.path().display());
        }
    }
}

/// A new, empty work folder for one update under the system temp folder
/// (`soyuz-update-<random>`): the download, and for an MSI install the helper.
pub(super) fn new_work_dir() -> std::io::Result<PathBuf> {
    new_dir_in(&std::env::temp_dir(), WORK_PREFIX)
}

const WORK_PREFIX: &str = "soyuz-update-";

/// Delete work folders ([`new_work_dir`]) older than a day (an update that was cancelled
/// by a crash, or the helper of a finished MSI update).
pub(super) fn remove_old_work_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|a| a > Duration::from_secs(24 * 3600));
        if old
            && e.file_name().to_string_lossy().starts_with(WORK_PREFIX)
            && e.file_type().is_ok_and(|t| t.is_dir())
            && let Err(err) = std::fs::remove_dir_all(e.path())
        {
            log::debug!("update work folder {}: {err}", e.path().display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn sha(bytes: &[u8]) -> String {
        sha256_of(bytes).unwrap()
    }

    /// A minimal PE header for `machine`.
    fn pe(machine: u16, tag: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 0x80];
        v[..2].copy_from_slice(b"MZ");
        v[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        v[0x40..0x44].copy_from_slice(b"PE\0\0");
        v[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
        v.extend_from_slice(tag);
        v
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap().into_inner()
    }

    /// Every file under `dir` as `relative/path` → contents, without our work folders.
    fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        files_under(dir)
            .unwrap()
            .into_iter()
            .map(|f| (f.to_string_lossy().replace('\\', "/"), std::fs::read(dir.join(&f)).unwrap()))
            .collect()
    }

    fn names(dir: &Path) -> Vec<String> {
        tree(dir).into_iter().map(|(n, _)| n).collect()
    }

    /// An installed portable copy of the "old" release in a temp folder.
    fn installed(dir: &Path) {
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::create_dir_all(dir.join("portable-data")).unwrap();
        for (name, data) in [
            ("soyuz.exe", &b"old gui"[..]),
            ("soyuz-tui.exe", b"old tui"),
            ("avcodec-62.dll", b"old codec"),
            ("avutil-60.dll", b"old util"),
            ("README.md", b"old readme"),
            ("docs/USER_GUIDE.md", b"old guide"),
            ("portable", b""),
            ("portable-data/config.toml", b"schema = 7\n"),
            ("notes.txt", b"the user's own file"),
        ] {
            std::fs::write(dir.join(name), data).unwrap();
        }
    }

    fn new_release_zip() -> Vec<u8> {
        zip_of(&[
            ("soyuz.exe", &pe(0x8664, b"new gui")),
            ("soyuz-tui.exe", &pe(0x8664, b"new tui")),
            ("avcodec-63.dll", b"new codec"),
            ("avutil-60.dll", b"new util"),
            ("README.md", b"new readme"),
            ("docs/", b""),
            ("docs/USER_GUIDE.md", b"new guide"),
            ("ffmpeg/LICENSE.md", b"lgpl"),
            // Never installed: the marker, user data, our own work-folder names.
            ("portable", b"from the archive"),
            ("portable-data/config.toml", b"schema = 1\n"),
            (".soyuz-update-old-x/soyuz.exe", b"x"),
        ])
    }

    #[test]
    fn the_portable_zip_replaces_the_copy_in_place() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Soyuz");
        installed(&dir);
        let archive = d.path().join("soyuz-0.8.0-windows-x64-portable.zip");
        let bytes = new_release_zip();
        std::fs::write(&archive, &bytes).unwrap();
        let exe = dir.join("soyuz.exe");

        let new = install_archive_to(&archive, &sha(&bytes), &exe, Payload::WindowsZip, "x86_64").unwrap();
        assert_eq!(new, exe);
        assert_eq!(std::fs::read(&exe).unwrap(), pe(0x8664, b"new gui"));
        assert_eq!(std::fs::read(dir.join("soyuz-tui.exe")).unwrap(), pe(0x8664, b"new tui"));
        assert_eq!(std::fs::read(dir.join("docs/USER_GUIDE.md")).unwrap(), b"new guide");
        assert_eq!(std::fs::read(dir.join("ffmpeg/LICENSE.md")).unwrap(), b"lgpl", "a new folder is created");
        // The user's data, the marker and their own files are exactly as they were.
        assert_eq!(std::fs::read(dir.join("portable-data/config.toml")).unwrap(), b"schema = 7\n");
        assert_eq!(std::fs::read(dir.join("portable")).unwrap(), b"");
        assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"the user's own file");
        // The old files wait in one backup folder (the running ones cannot be deleted
        // yet), the old FFmpeg major included; no staging folder is left.
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".soyuz-update-"))
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(left[0].starts_with(OLD_PREFIX));
        assert_eq!(
            names(&dir.join(&left[0])),
            ["README.md", "avcodec-62.dll", "avutil-60.dll", "docs/USER_GUIDE.md", "soyuz-tui.exe", "soyuz.exe"]
        );
        assert_eq!(std::fs::read(dir.join(&left[0]).join("soyuz.exe")).unwrap(), b"old gui");
        assert!(!dir.join("avcodec-62.dll").exists(), "the old FFmpeg major is gone");

        // The next start deletes the backup and nothing else.
        remove_leftovers(&dir);
        assert_eq!(
            names(&dir),
            [
                "README.md",
                "avcodec-63.dll",
                "avutil-60.dll",
                "docs/USER_GUIDE.md",
                "ffmpeg/LICENSE.md",
                "notes.txt",
                "portable",
                "portable-data/config.toml",
                "soyuz-tui.exe",
                "soyuz.exe"
            ]
        );
    }

    #[test]
    fn a_bad_archive_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Soyuz");
        installed(&dir);
        let before = tree(&dir);
        let exe = dir.join("soyuz.exe");
        let try_with = |name: &str, bytes: &[u8], sha256: &str, exe: &Path| {
            let archive = d.path().join(name);
            std::fs::write(&archive, bytes).unwrap();
            let r = install_archive_to(&archive, sha256, exe, Payload::WindowsZip, "x86_64");
            assert_eq!(tree(&dir), before, "{name}: the installed copy is untouched, nothing left behind");
            r.unwrap_err()
        };
        // Changed after it was verified.
        let good = new_release_zip();
        let e = try_with("changed.zip", &good, &"0".repeat(64), &exe);
        assert_eq!(e, tr!("services-update-install-changed"));
        // Not a zip at all.
        try_with("garbage.zip", b"not a zip", &sha(b"not a zip"), &exe);
        // A program for another processor, or not a program.
        let arm = zip_of(&[("soyuz.exe", &pe(0xAA64, b"arm")), ("soyuz-tui.exe", &pe(0x8664, b"t"))]);
        let e = try_with("arm.zip", &arm, &sha(&arm), &exe);
        assert!(e.contains("soyuz.exe"), "{e}");
        let script = zip_of(&[("soyuz.exe", &pe(0x8664, b"g")), ("soyuz-tui.exe", b"@echo off")]);
        let e = try_with("script.zip", &script, &sha(&script), &exe);
        assert!(e.contains("soyuz-tui.exe"), "{e}");
        // Without the program that runs (the terminal UI updating from an old archive).
        let gui_only = zip_of(&[("soyuz.exe", &pe(0x8664, b"g"))]);
        try_with("gui-only.zip", &gui_only, &sha(&gui_only), &dir.join("soyuz-tui.exe"));
        // No program at all, a path that climbs out, the same name twice.
        let docs = zip_of(&[("README.md", b"x")]);
        try_with("docs.zip", &docs, &sha(&docs), &exe);
        let climbing = zip_of(&[("soyuz.exe", &pe(0x8664, b"g")), ("../evil.exe", b"x")]);
        try_with("climbing.zip", &climbing, &sha(&climbing), &exe);
        assert!(!d.path().join("evil.exe").exists());
        // A renamed executable is not replaced by a file of another name.
        std::fs::rename(&exe, dir.join("files.exe")).unwrap();
        let archive = d.path().join("renamed.zip");
        std::fs::write(&archive, &good).unwrap();
        assert!(
            install_archive_to(&archive, &sha(&good), &dir.join("files.exe"), Payload::WindowsZip, "x86_64").is_err()
        );
        assert_eq!(std::fs::read(dir.join("files.exe")).unwrap(), b"old gui");
    }

    #[test]
    fn a_failed_swap_rolls_back_every_step() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Soyuz");
        installed(&dir);
        let before = tree(&dir);
        let stale = |n: &str| Payload::WindowsZip.is_ffmpeg_lib(n);
        let payload_files: &[(&str, &[u8])] = &[
            ("soyuz.exe", b"new gui"),
            ("soyuz-tui.exe", b"new tui"),
            ("avcodec-63.dll", b"new codec"),
            ("docs/USER_GUIDE.md", b"new guide"),
            ("ffmpeg/LICENSE.md", b"lgpl"),
        ];
        // Fail at every possible rename in turn: each time the folder is as it was.
        let mut failures = 0;
        for fail_at in 0..64 {
            let payload = d.path().join(format!("payload-{fail_at}"));
            for (name, data) in payload_files {
                let p = payload.join(name);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, data).unwrap();
            }
            let mut n = 0;
            let mut rename = |from: &Path, to: &Path| {
                n += 1;
                if n == fail_at + 1 {
                    return Err(std::io::Error::other("the file is in use"));
                }
                std::fs::rename(from, to)
            };
            let r = swap_in(&dir, &payload, "soyuz.exe", &stale, &mut rename);
            std::fs::remove_dir_all(&payload).unwrap();
            match r {
                Err(e) => {
                    failures += 1;
                    assert!(e.contains("the file is in use"), "{e}");
                    assert_eq!(tree(&dir), before, "rename {fail_at} failed: everything is back, no backup folder");
                }
                Ok(backup) => {
                    // Past the last rename: the swap went through, the running exe last.
                    assert!(fail_at >= 9, "{fail_at}");
                    let backup = backup.expect("old files were replaced");
                    assert_eq!(std::fs::read(dir.join("soyuz.exe")).unwrap(), b"new gui");
                    assert_eq!(
                        names(&backup),
                        ["avcodec-62.dll", "avutil-60.dll", "docs/USER_GUIDE.md", "soyuz-tui.exe", "soyuz.exe"]
                    );
                    break;
                }
            }
        }
        // 3 files replaced (2 renames each), 2 new files, 2 stale libraries.
        assert_eq!(failures, 10);
    }

    #[test]
    fn the_running_executable_is_swapped_last() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Soyuz");
        installed(&dir);
        let payload = d.path().join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        for name in ["soyuz.exe", "soyuz-tui.exe", "zz-last-by-name.dll"] {
            std::fs::write(payload.join(name), b"new").unwrap();
        }
        let mut order = Vec::new();
        let mut rename = |from: &Path, to: &Path| {
            if to.parent() == Some(dir.as_path()) {
                order.push(to.file_name().unwrap().to_string_lossy().into_owned());
            }
            std::fs::rename(from, to)
        };
        swap_in(&dir, &payload, "soyuz-tui.exe", &|_| false, &mut rename).unwrap();
        assert_eq!(order, ["soyuz.exe", "zz-last-by-name.dll", "soyuz-tui.exe"]);
    }

    /// What the swap rests on, on the real file system: a program that is running
    /// cannot be overwritten, but it can be moved aside and a new file put under its
    /// name; the old file can be deleted once its process has ended. (A copy of the
    /// system's command interpreter in a temp folder stands in for Soyuz.)
    #[cfg(windows)]
    #[test]
    fn a_running_program_is_moved_aside() {
        use std::process::{Command, Stdio};
        let Some(program) = std::env::var_os("ComSpec").map(PathBuf::from).filter(|p| p.is_file()) else { return };
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Soyuz");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("soyuz.exe");
        std::fs::copy(&program, &exe).unwrap();
        // Waits for input on its pipe: it runs until the pipe closes.
        let mut child =
            Command::new(&exe).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        assert!(std::fs::write(&exe, b"overwritten").is_err(), "a running program cannot be overwritten");

        let payload = d.path().join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("soyuz.exe"), b"new build").unwrap();
        let backup = swap_in(&dir, &payload, "soyuz.exe", &|_| false, &mut |from, to| std::fs::rename(from, to))
            .unwrap()
            .expect("the running program was moved aside");
        assert_eq!(std::fs::read(&exe).unwrap(), b"new build");
        assert!(backup.join("soyuz.exe").is_file());
        assert!(child.try_wait().unwrap().is_none(), "the old process keeps running");
        // Still in use: the backup stays until the process is gone.
        remove_leftovers(&dir);
        assert!(backup.join("soyuz.exe").is_file());

        drop(child.stdin.take());
        let _ = child.kill();
        child.wait().unwrap();
        for _ in 0..50 {
            remove_leftovers(&dir);
            if !backup.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!backup.exists(), "the next start removes the old files");
        assert_eq!(names(&dir), ["soyuz.exe"]);
    }

    #[test]
    fn a_macos_tarball_is_swapped_in_from_its_top_folder() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("soyuz-0.7.5-macos-arm64");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, data) in [("soyuz", &b"old"[..]), ("soyuz-tui", b"old tui"), ("libavcodec.62.dylib", b"old lib")] {
            std::fs::write(dir.join(name), data).unwrap();
        }
        let macho = |tag: &[u8]| {
            let mut v = vec![0xCF, 0xFA, 0xED, 0xFE];
            v.extend_from_slice(&0x0100_000Cu32.to_le_bytes());
            v.extend_from_slice(tag);
            v
        };
        let tar = super::super::tests::tar(&[
            ("soyuz-0.8.0-macos-arm64/", b"", b'5'),
            ("soyuz-0.8.0-macos-arm64/soyuz", &macho(b"gui"), b'0'),
            ("soyuz-0.8.0-macos-arm64/soyuz-tui", &macho(b"tui"), b'0'),
            ("soyuz-0.8.0-macos-arm64/libavcodec.63.dylib", b"new lib", b'0'),
            // A link is never unpacked.
            ("soyuz-0.8.0-macos-arm64/libavutil.61.dylib", b"", b'2'),
            ("soyuz-0.8.0-macos-arm64/ffmpeg/LICENSE.md", b"lgpl", b'0'),
        ]);
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&tar).unwrap();
        let bytes = enc.finish().unwrap();
        let archive = d.path().join("soyuz-0.8.0-macos-arm64.tar.gz");
        std::fs::write(&archive, &bytes).unwrap();

        // An Intel build is not for this (arm64) copy.
        assert!(install_archive_to(&archive, &sha(&bytes), &dir.join("soyuz"), Payload::MacTarball, "x86_64").is_err());
        assert_eq!(std::fs::read(dir.join("soyuz")).unwrap(), b"old");

        install_archive_to(&archive, &sha(&bytes), &dir.join("soyuz-tui"), Payload::MacTarball, "aarch64").unwrap();
        assert_eq!(std::fs::read(dir.join("soyuz")).unwrap(), macho(b"gui"));
        assert_eq!(std::fs::read(dir.join("soyuz-tui")).unwrap(), macho(b"tui"));
        assert_eq!(std::fs::read(dir.join("ffmpeg/LICENSE.md")).unwrap(), b"lgpl");
        assert!(dir.join("libavcodec.63.dylib").is_file() && !dir.join("libavcodec.62.dylib").exists());
        assert!(!dir.join("libavutil.61.dylib").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("soyuz")).unwrap().permissions().mode() & 0o777, 0o755);
        }
        remove_leftovers(&dir);
        assert_eq!(names(&dir), ["ffmpeg/LICENSE.md", "libavcodec.63.dylib", "soyuz", "soyuz-tui"]);
    }

    #[test]
    fn names_and_headers() {
        assert_eq!(safe_rel("docs/USER_GUIDE.md").unwrap(), Path::new("docs").join("USER_GUIDE.md"));
        assert_eq!(safe_rel("./soyuz.exe").unwrap(), PathBuf::from("soyuz.exe"));
        for bad in ["../x", "a/../../x", "C:/x", "x:stream", "", "/", "a\\..\\b"] {
            assert!(safe_rel(bad).is_err(), "{bad}");
        }
        assert!(skipped(Path::new("portable")) && skipped(Path::new("Portable-Data/config.toml")));
        assert!(skipped(Path::new(".soyuz-update-new-1/soyuz.exe")));
        assert!(!skipped(Path::new("docs/portable")) && !skipped(Path::new("portable.txt")));
        let (win, mac) = (Payload::WindowsZip, Payload::MacTarball);
        assert!(win.is_ffmpeg_lib("avcodec-63.dll") && win.is_ffmpeg_lib("SwResample-6.DLL"));
        assert!(
            !win.is_ffmpeg_lib("avcodec.dll")
                && !win.is_ffmpeg_lib("pdfium.dll")
                && !win.is_ffmpeg_lib("avcodec-x.dll")
        );
        assert!(mac.is_ffmpeg_lib("libavutil.61.dylib") && !mac.is_ffmpeg_lib("libpdfium.dylib"));
        assert!(win.is_program(&pe(0x8664, b""), "x86_64") && win.is_program(&pe(0xAA64, b""), "aarch64"));
        assert!(!win.is_program(&pe(0x8664, b""), "aarch64") && !win.is_program(b"MZ", "x86_64"));
        assert!(!win.is_program(b"#!/bin/sh\n", "x86_64"));
        assert!(mac.is_program(&[0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 2], "aarch64"));
        assert!(!mac.is_program(&pe(0x8664, b""), "x86_64"));
    }

    #[test]
    fn leftovers_keep_a_running_swap() {
        let d = tempfile::tempdir().unwrap();
        let staging = d.path().join(format!("{NEW_PREFIX}1"));
        let backup = d.path().join(format!("{OLD_PREFIX}1"));
        let other = d.path().join("portable-data");
        for p in [&staging, &backup, &other] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(d.path().join(format!("{OLD_PREFIX}a-file")), b"not a folder").unwrap();
        remove_leftovers(d.path());
        assert!(staging.exists(), "a staging folder in use (recent) stays");
        assert!(!backup.exists());
        assert!(other.exists() && d.path().join(format!("{OLD_PREFIX}a-file")).exists());
    }
}
