//! In-app download of the FFmpeg runtime (Phase V4c): the libraries Soyuz loads at run
//! time, fetched from the GitHub release `ffmpeg-runtime-<version>`
//! ([`media::runtime_release_tag`]) of the `[updates] repo` repository.
//!
//! * **Only on a click** (Settings → Preview → Media, or the palette's "Download FFmpeg
//!   runtime"), and only where it makes sense: not inside Flatpak (the sandbox uses the
//!   runtime's FFmpeg) or MSIX (the package carries the libraries) ([`offer`]). An empty
//!   `[updates] repo` turns it off like the update check.
//! * **Its own matcher.** The asset is exactly `ffmpeg-runtime-<ver>-windows-x64.zip`,
//!   `…-linux-x86_64.tar.xz` or `…-macos-universal.tar.xz` (`.github/workflows/release.yml`,
//!   "Publish the FFmpeg runtime release"); the app's own [`super::pick_asset`] never takes
//!   these. Its SHA-256 comes from the release's `SHA256SUMS.txt` and GitHub's asset
//!   digest, which must agree.
//! * **Verified, then unpacked atomically.** The download goes to a temp file in the
//!   config folder (hashed as it streams, then read back through the same handle). Only
//!   the five library files of a supported FFmpeg row plus `LICENSE.md` / `BUILD.txt` /
//!   `SOURCES.txt` are taken, by exact flat name (nothing else, no folders, no links), into
//!   a fresh staging folder that then replaces `<config dir>/ffmpeg` by rename. The media
//!   service is asked to load again ([`media::retry`]): no restart.
//!
//! Work runs on a worker thread with its own small tokio runtime, like the update check;
//! the frontends read [`phase`] and are woken on progress.

use std::io::{Read, Seek as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::Duration;

use parking_lot::Mutex;

use super::{
    Asset, InstallKind, MAX_JSON, RawRelease, client, convert_assets, create_temp_in, expected_sha256, fetch_into,
    get_text, install_kind_for, parse_repo, runtime, walk_tar,
};
use crate::host::Wake;
use crate::services::media;

/// Largest single file accepted from a runtime archive.
const MAX_FILE: u64 = 256 << 20;
/// The folder under the config dir the libraries are unpacked into (searched by
/// `soyuz_media::search_dirs`).
pub const DIR_NAME: &str = "ffmpeg";
/// Leftover staging / old folders and download temp files older than this are removed
/// when a download starts (a crash or power loss midway).
const STALE: Duration = Duration::from_secs(3600);

// ---- names ------------------------------------------------------------------------------

/// The runtime archive for `os` / `arch` (`std::env::consts` values) on the release
/// `tag`, as `release.yml` names it; `None` where no runtime is published.
pub fn asset_name(tag: &str, os: &str, arch: &str) -> Option<String> {
    let suffix = match (os, arch) {
        ("windows", "x86_64") => "windows-x64.zip",
        ("linux", "x86_64") => "linux-x86_64.tar.xz",
        // One universal archive (arm64 + x86_64 slices).
        ("macos", "aarch64" | "x86_64") => "macos-universal.tar.xz",
        _ => return None,
    };
    Some(format!("{tag}-{suffix}"))
}

/// The runtime asset of a release for `os` / `arch`: exactly [`asset_name`], ignoring case.
pub fn pick<'a>(assets: &'a [Asset], tag: &str, os: &str, arch: &str) -> Option<&'a Asset> {
    let want = asset_name(tag, os, arch)?;
    assets.iter().find(|a| a.name.eq_ignore_ascii_case(&want))
}

/// The checksum files of the runtime release for `asset`: `<asset>.sha256` if one is
/// there, then `SHA256SUMS.txt` (`sha256sum -- *` over every file of the release).
pub fn checksum_files<'a>(assets: &'a [Asset], asset: &str) -> Vec<&'a Asset> {
    let own = format!("{asset}.sha256");
    let mut v: Vec<&Asset> = assets
        .iter()
        .filter(|a| a.name.eq_ignore_ascii_case(&own) || a.name.eq_ignore_ascii_case("SHA256SUMS.txt"))
        .collect();
    v.sort_by_key(|a| !a.name.eq_ignore_ascii_case(&own));
    v
}

/// `GET /repos/<owner>/<name>/releases/tags/<tag>`.
pub fn release_url(owner: &str, name: &str, tag: &str) -> String {
    format!("https://api.github.com/repos/{owner}/{name}/releases/tags/{tag}")
}

/// The assets of the release JSON for `tag` (a draft or another tag is refused).
pub fn parse_release(json: &str, tag: &str) -> Result<Vec<Asset>, String> {
    let r: RawRelease =
        serde_json::from_str(json).map_err(|e| tr!("services-update-unexpected-reply", error = e.to_string()))?;
    if r.draft || r.tag_name != tag {
        return Err(tr!("services-ffmpeg-dl-no-release", tag = tag));
    }
    Ok(convert_assets(r.assets))
}

/// A file name the runtime archive may contribute on `os`: a library of a supported
/// FFmpeg row, or one of the texts. Exact, flat names only.
pub fn takes(name: &str, os: &str) -> bool {
    super::FFMPEG_TEXTS.contains(&name)
        || media::Majors::SUPPORTED.iter().any(|m| soyuz_media::lib_file_names_for(*m, os).iter().any(|n| n == name))
}

// ---- where it is offered ----------------------------------------------------------------

/// Whether this copy offers the download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    Offered,
    /// Inside Flatpak: the sandbox's FFmpeg (`codecs-extra`) is used.
    Flatpak,
    /// An MSIX package: it carries the libraries and its folder is read-only.
    Msix,
    /// No runtime is published for this OS / CPU.
    Unsupported,
}

/// [`Offer`] for an executable at `exe` on `os` / `arch`, with environment lookup `env`.
pub fn offer_for(os: &str, arch: &str, exe: &Path, env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Offer {
    if asset_name("t", os, arch).is_none() {
        return Offer::Unsupported;
    }
    if install_kind_for(os, exe, env) == InstallKind::Flatpak {
        return Offer::Flatpak;
    }
    // MSIX packages are installed under `…\WindowsApps\<package>\`.
    if os == "windows" && exe.components().any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps")) {
        return Offer::Msix;
    }
    Offer::Offered
}

/// [`Offer`] of the running copy (computed once).
pub fn offer() -> Offer {
    static OFFER: std::sync::OnceLock<Offer> = std::sync::OnceLock::new();
    *OFFER.get_or_init(|| {
        let exe = std::env::current_exe().unwrap_or_default();
        offer_for(std::env::consts::OS, std::env::consts::ARCH, &exe, |k| std::env::var_os(k))
    })
}

// ---- state ------------------------------------------------------------------------------

/// What the frontends show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Reading the release and its checksums.
    Fetching,
    Downloading {
        asset: String,
        got: u64,
        total: u64,
    },
    /// Verifying and unpacking.
    Installing,
    /// Unpacked into this folder; the media service is loading it.
    Installed(PathBuf),
    Failed(String),
}

impl Phase {
    pub fn busy(&self) -> bool {
        matches!(self, Phase::Fetching | Phase::Downloading { .. } | Phase::Installing)
    }
}

#[derive(Default)]
struct Shared {
    phase: Phase,
    cancel: Option<Arc<AtomicBool>>,
    /// A finished run the next [`pump`] turns into a toast.
    notice: Option<Result<PathBuf, String>>,
}

static STATE: Mutex<Shared> = Mutex::new(Shared { phase: Phase::Idle, cancel: None, notice: None });

/// Current phase (cheap clone).
pub fn phase() -> Phase {
    STATE.lock().phase.clone()
}

fn set_phase(phase: Phase, wake: &Wake) {
    STATE.lock().phase = phase;
    wake();
}

/// Cancel a running download.
pub fn cancel() {
    if let Some(c) = STATE.lock().cancel.as_ref() {
        c.store(true, AtomicOrdering::SeqCst);
    }
}

/// Forget a finished result (the Settings card's close button).
pub fn dismiss() {
    let mut s = STATE.lock();
    if !s.phase.busy() {
        s.phase = Phase::Idle;
    }
}

/// Start the download for the `[updates] repo` repository `repo`. `Err` says in plain
/// words why it cannot start (not offered here, no repository, already running).
pub fn start(repo: &str, wake: Wake) -> Result<(), String> {
    match offer() {
        Offer::Offered => {}
        Offer::Flatpak => return Err(tr!("services-ffmpeg-dl-flatpak").into()),
        Offer::Msix => return Err(tr!("services-ffmpeg-dl-msix").into()),
        Offer::Unsupported => return Err(tr!("services-ffmpeg-dl-unsupported").into()),
    }
    if repo.trim().is_empty() {
        return Err(tr!("services-ffmpeg-dl-no-repo").into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = STATE.lock();
        if s.phase.busy() {
            return Err(tr!("services-ffmpeg-dl-busy").into());
        }
        s.phase = Phase::Fetching;
        s.cancel = Some(cancel.clone());
        s.notice = None;
    }
    wake();
    let repo = repo.to_string();
    let worker_wake = wake.clone();
    let spawned = std::thread::Builder::new().name("soyuz-ffmpeg-download".into()).spawn(move || {
        let wake = worker_wake;
        let res = run(&repo, &cancel, &wake);
        if res.is_ok() {
            media::retry();
        }
        let mut s = STATE.lock();
        s.cancel = None;
        s.phase = match &res {
            Ok(dir) => Phase::Installed(dir.clone()),
            Err(e) => Phase::Failed(e.clone()),
        };
        s.notice = Some(res);
        drop(s);
        wake();
    });
    if let Err(e) = spawned {
        let mut s = STATE.lock();
        s.cancel = None;
        s.phase = Phase::Failed(tr!("services-update-cannot-start", error = e.to_string()));
        drop(s);
        wake();
    }
    Ok(())
}

/// The whole run on the worker: release → checksum → download → unpack.
fn run(repo: &str, cancel: &AtomicBool, wake: &Wake) -> Result<PathBuf, String> {
    let (owner, name) = parse_repo(repo)?;
    let tag = media::runtime_release_tag();
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let config_dir = soyuz_core::config::config_dir();
    std::fs::create_dir_all(&config_dir).map_err(|e| {
        tr!("services-update-cannot-create", path = config_dir.display().to_string(), error = e.to_string())
    })?;
    remove_leftovers(&config_dir);
    let rt = runtime().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = client()?;
        let json = match get_text(&client, &release_url(&owner, &name, &tag), MAX_JSON).await {
            Err(e) if e == tr!("services-update-not-found") => {
                return Err(tr!("services-ffmpeg-dl-no-release", tag = tag.as_str()));
            }
            r => r?,
        };
        let assets = parse_release(&json, &tag)?;
        let asset =
            pick(&assets, &tag, os, arch).ok_or_else(|| tr!("services-ffmpeg-dl-no-asset", tag = tag.as_str()))?;
        let want = expected_sha256(&client, asset, &checksum_files(&assets, &asset.name)).await?;
        set_phase(Phase::Downloading { asset: asset.name.clone(), got: 0, total: asset.size }, wake);
        let (mut file, part) = create_temp_in(&config_dir).map_err(|e| {
            tr!("services-update-cannot-write-file", path = config_dir.display().to_string(), error = e.to_string())
        })?;
        let progress = |got: u64, total: u64| {
            if let Phase::Downloading { got: g, total: t, .. } = &mut STATE.lock().phase {
                *g = got;
                *t = total;
            }
            wake();
        };
        let res = match fetch_into(&client, &asset.url, asset.size, &mut file, cancel, progress).await {
            Ok(have) if have != want => {
                Err(tr!("services-update-checksum-mismatch", expected = &want[..12], got = &have[..12]))
            }
            Ok(_) => {
                set_phase(Phase::Installing, wake);
                install_archive(&mut file, &asset.name, os, &config_dir)
            }
            Err(e) => Err(e),
        };
        drop(file);
        let _ = std::fs::remove_file(&part);
        res
    })
}

// ---- unpacking --------------------------------------------------------------------------

/// Unpack the verified runtime archive `file` (`archive_name` says zip or tar.xz) into
/// `<config_dir>/ffmpeg`, replacing what was there in one rename (blocking; the worker
/// calls it). Only [`takes`] names are written; the archive must hold the five libraries
/// of one supported FFmpeg row for `os` and `LICENSE.md`. Returns the folder.
pub fn install_archive(
    file: &mut std::fs::File,
    archive_name: &str,
    os: &str,
    config_dir: &Path,
) -> Result<PathBuf, String> {
    let bad = |e: &dyn std::fmt::Display| tr!("services-ffmpeg-dl-bad-archive", error = e.to_string());
    file.rewind().map_err(|e| bad(&e))?;
    let staging = new_dir_in(config_dir, ".ffmpeg-new-").map_err(|e| {
        tr!("services-update-cannot-create", path = config_dir.display().to_string(), error = e.to_string())
    })?;
    let unpacked = (|| -> Result<(), String> {
        let mut got: Vec<String> = Vec::new();
        let mut take = |name: &str, size: u64, body: &mut dyn Read| -> Result<(), String> {
            let name = name.trim_start_matches("./");
            if !takes(name, os) {
                return Ok(());
            }
            if size > MAX_FILE || got.iter().any(|g| g == name) {
                return Err(bad(&name));
            }
            let path = staging.join(name);
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o644);
            let mut out = opts.open(&path).map_err(|e| {
                tr!("services-update-cannot-write-file", path = path.display().to_string(), error = e.to_string())
            })?;
            let n = std::io::copy(&mut body.take(MAX_FILE + 1), &mut out).map_err(|e| bad(&e))?;
            if n != size {
                return Err(bad(&name));
            }
            out.sync_all().map_err(|e| bad(&e))?;
            got.push(name.to_string());
            Ok(())
        };
        let lower = archive_name.to_ascii_lowercase();
        if lower.ends_with(".zip") {
            let mut zip = zip::ZipArchive::new(&mut *file).map_err(|e| bad(&e))?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(|e| bad(&e))?;
                // Regular files only: folders and links never become anything on disk.
                if !entry.is_file() || entry.is_symlink() {
                    continue;
                }
                let (name, size) = (entry.name().to_string(), entry.size());
                take(&name, size, &mut entry)?;
            }
        } else if lower.ends_with(".tar.xz") {
            let xz = lzma_rust2::XzReader::new(std::io::BufReader::new(&mut *file), true);
            walk_tar(xz, |name, size, body| take(name, size, body))?;
        } else {
            return Err(bad(&archive_name));
        }
        let complete = media::Majors::SUPPORTED
            .iter()
            .any(|m| soyuz_media::lib_file_names_for(*m, os).iter().all(|n| got.contains(n)));
        if !complete || !got.iter().any(|g| g == "LICENSE.md") {
            return Err(tr!("services-ffmpeg-dl-incomplete").into());
        }
        Ok(())
    })();
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    replace_dir(config_dir, &staging)
}

/// Rename `staging` to `<config_dir>/ffmpeg`, moving an existing folder (or link) out of
/// the way first and back if the second rename fails; the old one is deleted after.
fn replace_dir(config_dir: &Path, staging: &Path) -> Result<PathBuf, String> {
    let dest = config_dir.join(DIR_NAME);
    let in_use = |e: &dyn std::fmt::Display| {
        tr!("services-ffmpeg-dl-in-use", path = dest.display().to_string(), error = e.to_string())
    };
    let old = if std::fs::symlink_metadata(&dest).is_ok() {
        let old = match new_name_in(config_dir, ".ffmpeg-old-") {
            Some(o) => o,
            None => {
                let _ = std::fs::remove_dir_all(staging);
                return Err(in_use(&"no free name"));
            }
        };
        if let Err(e) = std::fs::rename(&dest, &old) {
            let _ = std::fs::remove_dir_all(staging);
            return Err(in_use(&e));
        }
        Some(old)
    } else {
        None
    };
    if let Err(e) = std::fs::rename(staging, &dest) {
        if let Some(o) = &old {
            let _ = std::fs::rename(o, &dest);
        }
        let _ = std::fs::remove_dir_all(staging);
        return Err(in_use(&e));
    }
    // `remove_dir_all` removes a link itself, never what it points to.
    if let Some(o) = old
        && let Err(e) = std::fs::remove_dir_all(&o)
    {
        log::warn!("cannot remove the old FFmpeg folder {}: {e}", o.display());
    }
    Ok(dest)
}

/// A path `<dir>/<prefix><random>` that does not exist yet.
fn new_name_in(dir: &Path, prefix: &str) -> Option<PathBuf> {
    use std::hash::{BuildHasher, Hasher};
    (0..8u32).find_map(|attempt| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(attempt);
        h.write_u32(std::process::id());
        let p = dir.join(format!("{prefix}{:016x}", h.finish()));
        std::fs::symlink_metadata(&p).is_err().then_some(p)
    })
}

/// A new, empty folder `<dir>/<prefix><random>` (`create_dir` never reuses one).
fn new_dir_in(dir: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    for _ in 0..8 {
        let p = new_name_in(dir, prefix).ok_or(std::io::ErrorKind::AlreadyExists)?;
        match std::fs::create_dir(&p) {
            Ok(()) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::ErrorKind::AlreadyExists.into())
}

/// Staging / old folders and download temp files of an interrupted run (older than an
/// hour, so a second Soyuz downloading right now keeps its own).
fn remove_leftovers(config_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(config_dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let ours = name.starts_with(".ffmpeg-new-")
            || name.starts_with(".ffmpeg-old-")
            || (name.starts_with(".soyuz-update-") && name.ends_with(".tmp"));
        let old =
            e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a > STALE);
        if ours && old {
            let p = e.path();
            let r = if e.file_type().is_ok_and(|t| t.is_dir()) {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
            if let Err(err) = r {
                log::debug!("leftover {}: {err}", p.display());
            }
        }
    }
}

// ---- frontends --------------------------------------------------------------------------

/// The palette action: start, or say in a toast why not.
pub fn start_from_action(app: &mut crate::core::AppCore) {
    use crate::models::toasts::Toast;
    if media::runtime().is_some() {
        app.ui.toasts.push(Toast::info(tr!("services-ffmpeg-dl-loaded")));
        return;
    }
    let toast = match start(&app.config.updates.repo, app.wake.clone()) {
        Ok(()) => Toast::info(tr!("services-ffmpeg-dl-started")).body(tr!("services-ffmpeg-dl-started-body")),
        Err(e) => Toast::warning(tr!("services-ffmpeg-dl-cannot")).body(e),
    };
    app.ui.toasts.push(toast);
}

/// The palette offers the action only where the download is offered and nothing is
/// loaded yet.
pub fn palette_shows(a: &soyuz_core::config::Action) -> bool {
    *a != soyuz_core::config::Action::DownloadFfmpegRuntime || (offer() == Offer::Offered && media::runtime().is_none())
}

/// Per-frame: a finished run becomes a toast.
pub fn pump(app: &mut crate::core::AppCore) {
    use crate::models::toasts::Toast;
    let Some(res) = STATE.lock().notice.take() else { return };
    let toast = match res {
        Ok(dir) => Toast::success(tr!("services-ffmpeg-dl-done"))
            .body(tr!("services-ffmpeg-dl-done-body", dir = soyuz_fs::strip_verbatim(&dir).display().to_string())),
        Err(e) if e == tr!("services-update-cancelled") => Toast::info(tr!("services-ffmpeg-dl-cancelled")),
        Err(e) => Toast::error(tr!("services-ffmpeg-dl-failed")).body(e),
    };
    app.ui.toasts.push(toast);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    const TAG: &str = "ffmpeg-runtime-9.0.2";

    fn asset(n: &str) -> Asset {
        Asset { name: n.into(), url: format!("https://example.invalid/{n}"), size: 1, sha256: None }
    }

    /// Every file the `ffmpeg-runtime-<ver>` release carries (release.yml).
    fn release_assets() -> Vec<Asset> {
        [
            "ffmpeg-runtime-9.0.2-windows-x64.zip",
            "ffmpeg-runtime-9.0.2-linux-x86_64.tar.xz",
            "ffmpeg-runtime-9.0.2-macos-universal.tar.xz",
            "BUILD-ffmpeg-runtime-9.0.2-windows-x64.txt",
            "LICENSE.md",
            "SOURCES.txt",
            "ffmpeg-9.0.2.tar.xz",
            "ffmpeg-9.0.2.tar.xz.asc",
            "dav1d-1.5.1.tar.xz",
            "SHA256SUMS.txt",
        ]
        .into_iter()
        .map(asset)
        .collect()
    }

    #[test]
    fn the_matcher_takes_exactly_the_platform_archive() {
        let a = release_assets();
        let name = |os, arch| pick(&a, TAG, os, arch).map(|a| a.name.clone());
        assert_eq!(name("windows", "x86_64").as_deref(), Some("ffmpeg-runtime-9.0.2-windows-x64.zip"));
        assert_eq!(name("linux", "x86_64").as_deref(), Some("ffmpeg-runtime-9.0.2-linux-x86_64.tar.xz"));
        assert_eq!(name("macos", "aarch64").as_deref(), Some("ffmpeg-runtime-9.0.2-macos-universal.tar.xz"));
        assert_eq!(name("macos", "x86_64").as_deref(), Some("ffmpeg-runtime-9.0.2-macos-universal.tar.xz"));
        assert_eq!(name("windows", "aarch64"), None);
        assert_eq!(name("linux", "aarch64"), None);
        // Another runtime version is never taken.
        assert_eq!(pick(&a, "ffmpeg-runtime-9.0.3", "windows", "x86_64"), None);
        let sums: Vec<&str> =
            checksum_files(&a, "ffmpeg-runtime-9.0.2-windows-x64.zip").iter().map(|a| a.name.as_str()).collect();
        assert_eq!(sums, ["SHA256SUMS.txt"]);
        let mut b = a.clone();
        b.push(asset("ffmpeg-runtime-9.0.2-windows-x64.zip.sha256"));
        assert_eq!(
            checksum_files(&b, "ffmpeg-runtime-9.0.2-windows-x64.zip")[0].name,
            "ffmpeg-runtime-9.0.2-windows-x64.zip.sha256"
        );
        assert_eq!(release_url("o", "n", TAG), "https://api.github.com/repos/o/n/releases/tags/ffmpeg-runtime-9.0.2");
        assert_eq!(media::runtime_release_tag(), format!("ffmpeg-runtime-{}", media::BUNDLED_VERSION));
    }

    #[test]
    fn the_app_updater_never_picks_a_runtime_archive() {
        // The runtime release, as if it were the app's latest release.
        let r = super::super::Release {
            tag: TAG.into(),
            version: super::super::Version::parse("9.0.2").unwrap(),
            title: String::new(),
            notes: String::new(),
            url: String::new(),
            prerelease: false,
            published: String::new(),
            assets: release_assets(),
        };
        for (os, arch) in [("windows", "x86_64"), ("linux", "x86_64"), ("macos", "aarch64")] {
            for portable in [false, true] {
                assert_eq!(super::super::pick_asset(&r, os, arch, portable), None, "{os}/{arch}");
            }
        }
    }

    #[test]
    fn release_json() {
        let json = format!(
            r#"{{"tag_name":"{TAG}","draft":false,"assets":[
                {{"name":"ffmpeg-runtime-9.0.2-linux-x86_64.tar.xz","browser_download_url":"https://github.com/x","size":5,"digest":"sha256:{}"}},
                {{"name":"evil.zip","browser_download_url":"http://github.com/x","size":5}}]}}"#,
            "A".repeat(64)
        );
        let a = parse_release(&json, TAG).unwrap();
        assert_eq!(a.len(), 1, "plain http assets are dropped");
        assert_eq!(a[0].sha256.as_deref(), Some("a".repeat(64).as_str()));
        assert!(parse_release(&json, "ffmpeg-runtime-9.0.3").is_err());
        assert!(parse_release(&json.replace("\"draft\":false", "\"draft\":true"), TAG).is_err());
        assert!(parse_release("<html>", TAG).is_err());
    }

    #[test]
    fn offered_except_in_flatpak_msix_and_unsupported_targets() {
        let none = |_: &str| None;
        assert_eq!(offer_for("windows", "x86_64", Path::new(r"C:\Soyuz\soyuz.exe"), none), Offer::Offered);
        assert_eq!(
            offer_for(
                "windows",
                "x86_64",
                Path::new(r"C:\Program Files\WindowsApps\Soyuz_1.0_x64__abc\soyuz.exe"),
                none
            ),
            Offer::Msix
        );
        assert_eq!(offer_for("windows", "aarch64", Path::new(r"C:\Soyuz\soyuz.exe"), none), Offer::Unsupported);
        assert_eq!(offer_for("linux", "x86_64", Path::new("/home/me/.local/bin/soyuz"), none), Offer::Offered);
        assert_eq!(offer_for("linux", "x86_64", Path::new("/app/bin/soyuz"), none), Offer::Flatpak);
        let flatpak = |v: &str| (v == "FLATPAK_ID").then(|| "io.github.x".into());
        assert_eq!(offer_for("linux", "x86_64", Path::new("/opt/soyuz/bin/soyuz"), flatpak), Offer::Flatpak);
        assert_eq!(
            offer_for("macos", "aarch64", Path::new("/Applications/Soyuz.app/Contents/MacOS/soyuz"), none),
            Offer::Offered
        );
    }

    #[test]
    fn only_runtime_files_are_taken() {
        assert!(takes("avcodec-63.dll", "windows"));
        assert!(takes("avcodec-61.dll", "windows"), "FFmpeg 7.1 row");
        assert!(takes("libavformat.so.63", "linux"));
        assert!(takes("libavutil.61.dylib", "macos"));
        assert!(takes("LICENSE.md", "linux"));
        for bad in ["avcodec-63.dll", "../avcodec-63.dll", "x/LICENSE.md", "avcodec-63.dll:ads", "evil.dll", "README"] {
            assert!(!takes(bad, "linux"), "{bad}");
        }
        assert!(!takes("libavcodec.so.63", "windows"));
    }

    fn libs(os: &str) -> Vec<String> {
        soyuz_media::lib_file_names_for(media::Majors::BUNDLED, os).to_vec()
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

    fn tar_xz(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let tar = super::super::tests::tar(entries);
        let mut w = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
        w.write_all(&tar).unwrap();
        w.finish().unwrap()
    }

    fn file_with(dir: &Path, bytes: &[u8]) -> std::fs::File {
        let p = dir.join("download.tmp");
        std::fs::write(&p, bytes).unwrap();
        std::fs::OpenOptions::new().read(true).write(true).open(p).unwrap()
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn a_zip_runtime_is_unpacked_flat_and_replaces_the_folder() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config");
        std::fs::create_dir_all(cfg.join(DIR_NAME)).unwrap();
        std::fs::write(cfg.join(DIR_NAME).join("avcodec-62.dll"), b"old").unwrap();
        let names = libs("windows");
        let mut entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"dll"[..])).collect();
        entries.extend([
            ("LICENSE.md", &b"lgpl"[..]),
            ("BUILD.txt", &b"build"[..]),
            ("include/", &b""[..]),
            ("include/libavcodec/avcodec.h", &b"h"[..]),
            ("../evil.dll", &b"x"[..]),
            ("readme.exe", &b"x"[..]),
        ]);
        let mut f = file_with(d.path(), &zip_of(&entries));
        let dir = install_archive(&mut f, "ffmpeg-runtime-9.0.2-windows-x64.zip", "windows", &cfg).unwrap();
        assert_eq!(dir, cfg.join(DIR_NAME));
        let mut want: Vec<String> = names.clone();
        want.extend(["BUILD.txt".to_string(), "LICENSE.md".to_string()]);
        want.sort();
        assert_eq!(listing(&dir), want, "only the runtime files, the old folder replaced");
        assert_eq!(listing(&cfg), [DIR_NAME], "no staging or old folder left");
        assert!(!d.path().join("evil.dll").exists());
    }

    #[test]
    fn a_tar_xz_runtime_skips_links_and_needs_every_library() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().to_path_buf();
        let names = libs("linux");
        let mut entries: Vec<(String, Vec<u8>, u8)> =
            names.iter().map(|n| (format!("./{n}"), b"\x7fELF so".to_vec(), b'0')).collect();
        entries.push(("./".into(), Vec::new(), b'5'));
        entries.push(("./LICENSE.md".into(), b"lgpl".to_vec(), b'0'));
        entries.push(("./SOURCES.txt".into(), b"src".to_vec(), b'0'));
        let as_refs = |e: &[(String, Vec<u8>, u8)]| -> Vec<u8> {
            let v: Vec<(&str, &[u8], u8)> = e.iter().map(|(n, d, k)| (n.as_str(), d.as_slice(), *k)).collect();
            tar_xz(&v)
        };
        let mut f = file_with(d.path(), &as_refs(&entries));
        let dir = install_archive(&mut f, "ffmpeg-runtime-9.0.2-linux-x86_64.tar.xz", "linux", &cfg).unwrap();
        assert_eq!(listing(&dir).len(), 7);
        assert_eq!(std::fs::read(dir.join(&names[3])).unwrap(), b"\x7fELF so");

        // A library that is a symlink (pointing out of the folder) is not a library:
        // the archive is incomplete and the installed folder stays as it was.
        let mut linked = entries.clone();
        linked[3] = (format!("./{}", names[3]), b"".to_vec(), b'2');
        let mut f = file_with(d.path(), &as_refs(&linked));
        let err = install_archive(&mut f, "x.tar.xz", "linux", &cfg).unwrap_err();
        assert_eq!(err, tr!("services-ffmpeg-dl-incomplete"));
        assert_eq!(listing(&dir).len(), 7);
        assert_eq!(std::fs::read(dir.join(&names[3])).unwrap(), b"\x7fELF so");

        // No licence text: refused too. A file twice: refused.
        let mut f = file_with(
            d.path(),
            &as_refs(&entries.iter().filter(|e| !e.0.ends_with("LICENSE.md")).cloned().collect::<Vec<_>>()),
        );
        assert!(install_archive(&mut f, "x.tar.xz", "linux", &cfg).is_err());
        let mut twice = entries.clone();
        twice.push(entries[0].clone());
        let mut f = file_with(d.path(), &as_refs(&twice));
        assert!(install_archive(&mut f, "x.tar.xz", "linux", &cfg).is_err());
        // Not an archive: refused, nothing left behind.
        let mut f = file_with(d.path(), b"not xz at all");
        assert!(install_archive(&mut f, "x.tar.xz", "linux", &cfg).is_err());
        assert!(install_archive(&mut f, "x.rar", "linux", &cfg).is_err());
        let left: Vec<String> = listing(&cfg).into_iter().filter(|n| n.starts_with(".ffmpeg")).collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn leftovers_of_an_interrupted_run_are_removed_when_old() {
        let d = tempfile::tempdir().unwrap();
        let fresh = d.path().join(".ffmpeg-new-0000000000000001");
        std::fs::create_dir(&fresh).unwrap();
        remove_leftovers(d.path());
        assert!(fresh.exists(), "a run in progress (recent) is left alone");
        assert!(new_dir_in(d.path(), ".ffmpeg-new-").unwrap().is_dir());
    }
}
