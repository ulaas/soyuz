//! Update check against GitHub Releases, and the one-click update.
//!
//! * **The check.** "Check for updates" (Settings → About & updates, the app menu, the
//!   command palette) and, unless `[updates] auto_check` is off, one background check a
//!   day ([`pump`]). A check is one request for the public Releases JSON of the configured
//!   repository (`[updates] repo`, empty = disabled): no identifiers, no telemetry. A
//!   newer release is announced (a notification with a button, the update page); nothing
//!   is downloaded.
//! * **Nothing is downloaded or installed without a click.** "Update and restart"
//!   ([`install`]) downloads the release file for this copy, verifies it, installs it and
//!   restarts Soyuz, with progress and cancel. "Download" ([`download`]) only saves the
//!   verified file to the Downloads folder.
//! * **Verification.** A download is used only if its SHA-256 equals the one in the
//!   release's checksum file (`SHA256SUMS*` / `<file>.sha256`, fetched over HTTPS) and the
//!   digest GitHub reports for the asset: every source that exists must agree and at
//!   least one must exist ([`expected_sha256`]). The Authenticode signature of an
//!   installer is reported, for information.
//! * **How it installs** depends on how this copy was installed ([`kind`]):
//!   the Linux tarball replaces its files by atomic renames ([`install_tarball_to`]); the
//!   Windows portable zip and the macOS tarball are swapped in place with a full roll
//!   back on failure ([`swap`]); an MSI install runs the next MSI through a helper that
//!   starts Soyuz again ([`msi`]). Copies owned by a package manager (Scoop, Homebrew,
//!   rpm / AUR, Flatpak, AppImage, MSIX) or in a folder Soyuz cannot write to are never
//!   touched: the user is told how to update them ([`how_to_update`]).
//! * **Restart.** [`request_restart`] + [`take_restart`] / [`exec_restart`] start the new
//!   build once the app has closed the normal way (pending jobs are asked about).
//!
//! Network work runs on short-lived worker threads with their own tiny tokio runtime; the
//! UI reads a shared [`Phase`] snapshot and is woken with `request_repaint`.

use std::cmp::Ordering;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::host::Wake;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use soyuz_fs::integration::Signature;

pub mod ffmpeg_runtime;
pub mod kind;
pub mod msi;
pub mod swap;

pub use kind::{
    InstallFacts, InstallKind, asset_for_this_copy, detect_install, how_to_update, install_kind, install_kind_for,
    install_kind_with, pick_asset_for, running_exe, self_update_note,
};
pub use msi::InstallerPlan;

/// Current app version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Largest Releases JSON we accept.
const MAX_JSON: usize = 4 << 20;
/// Largest checksum file we accept.
const MAX_SUMS: usize = 1 << 20;
/// Largest asset we download.
const MAX_ASSET: u64 = 1 << 30;
/// Interval of the automatic background check (`[updates] auto_check`).
const DAY: Duration = Duration::from_secs(24 * 3600);
/// How often a running session looks whether the daily check is due again.
const AUTO_RECHECK: Duration = Duration::from_secs(3600);
/// File in the config dir remembering when the last check ran.
const STAMP_FILE: &str = "update-check.json";

// ---- versions ------------------------------------------------------------------------

/// A semantic version (`MAJOR.MINOR.PATCH[-pre][+build]`; a leading `v` is accepted, missing
/// minor/patch count as 0). Build metadata is ignored for ordering, per SemVer 2.0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut nums = core.split('.');
        let num = |p: Option<&str>| -> Option<u64> {
            match p {
                None => Some(0),
                Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => p.parse().ok(),
                Some(_) => None,
            }
        };
        let major = num(Some(nums.next()?))?;
        let minor = num(nums.next())?;
        let patch = num(nums.next())?;
        if nums.next().is_some() {
            return None;
        }
        let pre = match pre {
            Some(p) => {
                let ids: Vec<String> = p.split('.').map(str::to_string).collect();
                if ids.iter().any(|i| i.is_empty() || !i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                    return None;
                }
                ids
            }
            None => Vec::new(),
        };
        Some(Version { major, minor, patch, pre })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // A release is newer than any of its pre-releases.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

// ---- repository + release JSON -------------------------------------------------------

/// `owner/name` from `owner/name`, `github.com/owner/name` or a `https://github.com/…` URL.
pub fn parse_repo(s: &str) -> Result<(String, String), String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(tr!("services-update-no-repo").into());
    }
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .map(|r| r.strip_prefix("www.").unwrap_or(r))
        .unwrap_or(s);
    let rest = rest.strip_prefix("github.com/").unwrap_or(rest);
    if rest.contains("://") || rest.starts_with("api.github.com") {
        return Err(tr!("services-update-github-only").into());
    }
    let mut parts = rest.trim_end_matches('/').split('/');
    let (Some(owner), Some(name)) = (parts.next(), parts.next()) else {
        return Err(tr!("services-update-expected-owner-name").into());
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    // Extra path segments (`/releases`, `/tree/main`) are tolerated.
    let ok = |p: &str| {
        !p.is_empty() && p.len() <= 100 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    // GitHub account names are letters, digits and hyphens only (so a host like
    // `gitlab.com` is never mistaken for an owner).
    let owner_ok =
        !owner.is_empty() && owner.len() <= 39 && owner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !owner_ok || !ok(name) || name.starts_with('.') {
        return Err(tr!("services-update-bad-repo", repo = s));
    }
    Ok((owner.to_string(), name.to_string()))
}

/// Releases API URL: `/releases/latest` (stable only) or `/releases` (to include pre-releases).
pub fn api_url(owner: &str, name: &str, prereleases: bool) -> String {
    if prereleases {
        format!("https://api.github.com/repos/{owner}/{name}/releases?per_page=20")
    } else {
        format!("https://api.github.com/repos/{owner}/{name}/releases/latest")
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// `sha256:<hex>` (GitHub fills this in for assets uploaded since mid-2025).
    #[serde(default)]
    digest: Option<String>,
}

/// One downloadable file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex SHA-256 reported by GitHub, if any.
    pub sha256: Option<String>,
}

/// A parsed release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub title: String,
    pub notes: String,
    pub url: String,
    pub prerelease: bool,
    pub published: String,
    pub assets: Vec<Asset>,
}

/// The `https://` assets of a release, with GitHub's digest when it reports one.
fn convert_assets(raw: Vec<RawAsset>) -> Vec<Asset> {
    raw.into_iter()
        .filter(|a| a.browser_download_url.starts_with("https://"))
        .map(|a| Asset {
            sha256: a
                .digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:"))
                .filter(|h| is_sha256_hex(h))
                .map(str::to_ascii_lowercase),
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
        })
        .collect()
}

fn convert(r: RawRelease) -> Option<Release> {
    if r.draft {
        return None;
    }
    let version = Version::parse(&r.tag_name)?;
    let assets = convert_assets(r.assets);
    Some(Release {
        title: r.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| r.tag_name.clone()),
        tag: r.tag_name,
        version,
        notes: r.body.unwrap_or_default(),
        url: r.html_url,
        prerelease: r.prerelease,
        published: r.published_at.unwrap_or_default().chars().take(10).collect(),
        assets,
    })
}

/// Parse `/releases/latest` (one object) or `/releases` (an array; the newest non-draft
/// release that `prereleases` allows wins).
pub fn parse_releases(json: &str, prereleases: bool) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| tr!("services-update-unexpected-reply", error = e.to_string()))?;
    let raws: Vec<RawRelease> = if v.is_array() {
        serde_json::from_value(v).map_err(|e| tr!("services-update-unexpected-reply", error = e.to_string()))?
    } else {
        if let Some(msg) = v.get("message").and_then(|m| m.as_str())
            && v.get("tag_name").is_none()
        {
            return Err(tr!("services-update-github-message", message = msg));
        }
        vec![serde_json::from_value(v).map_err(|e| tr!("services-update-unexpected-reply", error = e.to_string()))?]
    };
    raws.into_iter()
        .filter_map(convert)
        .filter(|r| prereleases || !(r.prerelease || r.version.is_prerelease()))
        .max_by(|a, b| a.version.cmp(&b.version))
        .ok_or_else(|| tr!("services-update-no-releases").into())
}

/// Whether `latest` is newer than the running version.
pub fn is_newer(latest: &Version, current: &str) -> bool {
    Version::parse(current).is_none_or(|c| *latest > c)
}

/// The asset to download by hand for this OS/arch. Installed copies prefer the `.msi`
/// installer, then a regular `.zip`; portable copies (a `portable` marker next to the exe)
/// prefer the `-portable.zip`, then any `.zip`. Names follow
/// `soyuz-<ver>-<os>-<arch>[-portable].<ext>` and only names with that `soyuz-<ver>-`
/// prefix are considered: anything else on the release (checksum lists, source tarballs,
/// an `ffmpeg-runtime-*` archive) is never the app. ([`pick_asset_for`] picks for the
/// way a copy was installed.)
pub fn pick_asset<'a>(release: &'a Release, os: &str, arch: &str, portable: bool) -> Option<&'a Asset> {
    // (extension, wants the portable flavour)
    let order: &[(&str, Option<bool>)] = match (os, portable) {
        ("windows", false) => &[(".msi", None), (".zip", Some(false)), (".zip", None)],
        ("windows", true) => &[(".zip", Some(true)), (".zip", None)],
        ("linux", _) => &[(".tar.gz", None), (".tgz", None), (".zip", None)],
        (_, true) => &[(".zip", Some(true)), (".zip", None), (".tar.gz", None)],
        _ => &[(".zip", Some(false)), (".zip", None), (".tar.gz", None), (".dmg", None)],
    };
    pick_in_order(release, os, arch, order)
}

/// The first app archive of `release` for `os` / `arch` in `order`, a list of
/// `(extension, portable flavour wanted)`.
fn pick_in_order<'a>(release: &'a Release, os: &str, arch: &str, order: &[(&str, Option<bool>)]) -> Option<&'a Asset> {
    let prefix = format!("soyuz-{}-", release.version).to_ascii_lowercase();
    let os_words: &[&str] = match os {
        "windows" => &["windows", "win64"],
        "macos" => &["macos", "darwin"],
        other => &[other],
    };
    let arch_words: &[&str] = match arch {
        "x86_64" => &["x64", "x86_64", "amd64"],
        "aarch64" => &["arm64", "aarch64"],
        other => &[other],
    };
    let fits = |a: &Asset| {
        let n = a.name.to_ascii_lowercase();
        n.starts_with(&prefix)
            && os_words.iter().any(|w| n.contains(w))
            && (arch_words.iter().any(|w| n.contains(w)) || n.contains("universal"))
    };
    let is_portable = |a: &Asset| a.name.to_ascii_lowercase().contains("portable");
    order.iter().find_map(|(ext, flavour)| {
        release.assets.iter().find(|a| {
            fits(a) && a.name.to_ascii_lowercase().ends_with(ext) && flavour.is_none_or(|p| is_portable(a) == p)
        })
    })
}

/// Checksum files of a release (`SHA256SUMS*`, `*.sha256`), best match for `asset` first.
pub fn checksum_assets<'a>(release: &'a Release, asset: &str) -> Vec<&'a Asset> {
    let own = format!("{asset}.sha256").to_ascii_lowercase();
    let mut v: Vec<&Asset> = release
        .assets
        .iter()
        .filter(|a| {
            let n = a.name.to_ascii_lowercase();
            n == own || n.starts_with("sha256sums") || n.ends_with(".sha256")
        })
        .collect();
    v.sort_by_key(|a| a.name.to_ascii_lowercase() != own);
    v
}

/// Detached signature files present for `asset` (`.sig`, `.minisig`, `.asc`).
pub fn signature_asset<'a>(release: &'a Release, asset: &str) -> Option<&'a Asset> {
    release.assets.iter().find(|a| [".sig", ".minisig", ".asc"].iter().any(|e| a.name == format!("{asset}{e}")))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The hash for `name` from a `sha256sum`-style file (`<hex>  <name>` or `<hex> *<name>`),
/// or a bare hash when the file holds just one.
pub fn parse_sums(text: &str, name: &str) -> Option<String> {
    let mut bare = None;
    for line in text.lines() {
        let line = line.trim();
        let mut it = line.split_whitespace();
        let (Some(hash), file) = (it.next(), it.next()) else { continue };
        if !is_sha256_hex(hash) {
            continue;
        }
        match file {
            Some(f) if f.trim_start_matches('*').rsplit(['/', '\\']).next() == Some(name) => {
                return Some(hash.to_ascii_lowercase());
            }
            None => bare = Some(hash.to_ascii_lowercase()),
            _ => {}
        }
    }
    bare
}

/// A download file name that cannot escape the target folder.
pub fn safe_file_name(name: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control() && !r#"<>:"/\|?*"#.contains(*c)).collect();
    let n = n.trim().trim_end_matches('.').to_string();
    (!n.is_empty() && n != "." && n != "..").then_some(n)
}

// ---- shared state ----------------------------------------------------------------------

/// What the update page shows.
#[derive(Clone, Debug, Default)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate {
        latest: String,
    },
    Available(Arc<Release>),
    Downloading {
        release: Arc<Release>,
        asset: String,
        got: u64,
        total: u64,
    },
    /// "Download": the verified file is in the Downloads folder.
    Downloaded {
        release: Arc<Release>,
        path: PathBuf,
        sha256: String,
        signature: Signature,
        /// A detached signature file was published but is not verified (no key configured).
        detached_sig: bool,
    },
    /// "Update and restart": the verified download is being installed.
    Installing {
        release: Arc<Release>,
    },
    /// "Update and restart": installed (or, for an MSI install, ready for Windows
    /// Installer); Soyuz restarts to finish.
    Ready {
        release: Arc<Release>,
        restart: Restart,
    },
    Failed(String),
}

impl Phase {
    /// A worker is running.
    pub fn busy(&self) -> bool {
        matches!(self, Phase::Checking | Phase::Downloading { .. } | Phase::Installing { .. })
    }
}

/// Something the next [`pump`] tells the user as a notification.
#[derive(Clone, Debug)]
enum Notice {
    Available(Arc<Release>),
    UpToDate(String),
    CheckFailed(String),
    InstallFailed(String),
    /// How the update installed before this start ended (`error`: why it was not).
    Installed {
        version: String,
        error: Option<String>,
    },
}

struct Shared {
    phase: Phase,
    notices: Vec<Notice>,
    /// When [`pump`] next looks whether the automatic check is due (`None` = not yet
    /// this session).
    next_auto: Option<Instant>,
    /// The version the automatic check has announced this session.
    notified: Option<String>,
    /// The running check also reports its result as a notification.
    announce: bool,
    /// [`Phase::Ready`]: restart at the next [`pump`].
    restart_now: bool,
    cancel: Option<Arc<AtomicBool>>,
}

static STATE: Mutex<Shared> = Mutex::new(Shared {
    phase: Phase::Idle,
    notices: Vec::new(),
    next_auto: None,
    notified: None,
    announce: false,
    restart_now: false,
    cancel: None,
});

/// Whether this process does anything by itself: the automatic check, and the
/// housekeeping after an update. Off until a real app session turns it on
/// ([`enable_background`]), so nothing built on the core (tests, tools) ever reaches the
/// network or touches the install folder on its own.
static BACKGROUND: AtomicBool = AtomicBool::new(false);

/// Turn on the automatic update check (`[updates] auto_check`) and the housekeeping
/// after an update for this process. The app binaries call it once at startup.
pub fn enable_background() {
    BACKGROUND.store(true, AtomicOrdering::Relaxed);
}

/// Whether [`enable_background`] was called.
pub fn background_enabled() -> bool {
    BACKGROUND.load(AtomicOrdering::Relaxed)
}

/// Current phase (cheap clone).
pub fn phase() -> Phase {
    STATE.lock().phase.clone()
}

fn set(phase: Phase, wake: &Wake) {
    STATE.lock().phase = phase;
    wake();
}

/// Forget the result (e.g. after the repo setting changed).
pub fn reset() {
    let mut s = STATE.lock();
    if !s.phase.busy() && !matches!(s.phase, Phase::Ready { .. }) {
        s.phase = Phase::Idle;
    }
}

/// Cancel a running download.
pub fn cancel() {
    if let Some(c) = STATE.lock().cancel.as_ref() {
        c.store(true, AtomicOrdering::SeqCst);
    }
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()
}

fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("soyuz/", env!("CARGO_PKG_VERSION"), " (update check)"))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60 * 30))
        .build()
        .map_err(|e| tr!("services-update-no-client", error = e.to_string()))
}

async fn get_text(client: &reqwest::Client, url: &str, limit: usize) -> Result<String, String> {
    let mut resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| tr!("services-update-network", error = e.to_string()))?;
    let status = resp.status();
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| tr!("services-update-network", error = e.to_string()))? {
        buf.extend_from_slice(&chunk);
        if buf.len() > limit {
            return Err(tr!("services-update-reply-too-large").into());
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    if status.as_u16() == 404 {
        return Err(tr!("services-update-not-found").into());
    }
    if !status.is_success() {
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
            .unwrap_or_default();
        return Err(tr!("services-update-http-status", status = status.to_string(), message = msg).trim().to_string());
    }
    Ok(text)
}

async fn fetch_latest(repo: &str, prereleases: bool) -> Result<Release, String> {
    let (owner, name) = parse_repo(repo)?;
    let client = client()?;
    let json = get_text(&client, &api_url(&owner, &name, prereleases), MAX_JSON).await?;
    parse_releases(&json, prereleases)
}

/// Start a check in the background ("Check for updates"). With `announce` the result is
/// also shown as a notification (for a frontend that has no update page in view).
pub fn check(repo: String, prereleases: bool, announce: bool, wake: Wake) {
    {
        let mut s = STATE.lock();
        if s.phase.busy() {
            return;
        }
        s.phase = Phase::Checking;
        s.announce = announce;
    }
    wake();
    spawn("soyuz-update-check", move || run_check(&repo, prereleases, None, &wake));
}

/// One check on a worker thread; the phase is already [`Phase::Checking`]. `auto` is the
/// phase before an automatic check: its failure is logged and that phase comes back.
fn run_check(repo: &str, prereleases: bool, auto: Option<Phase>, wake: &Wake) {
    detect_install();
    let res = runtime().map_err(|e| e.to_string()).and_then(|rt| rt.block_on(fetch_latest(repo, prereleases)));
    write_stamp();
    let mut s = STATE.lock();
    let announce = std::mem::take(&mut s.announce);
    s.phase = match res {
        Ok(r) if is_newer(&r.version, CURRENT) => {
            let r = Arc::new(r);
            let version = r.version.to_string();
            // The automatic check announces a release once per session; a check asked
            // for from outside the update page, every time.
            if announce || (auto.is_some() && s.notified.as_deref() != Some(version.as_str())) {
                s.notices.push(Notice::Available(r.clone()));
            }
            s.notified = Some(version);
            Phase::Available(r)
        }
        Ok(r) => {
            let latest = r.version.to_string();
            if announce {
                s.notices.push(Notice::UpToDate(latest.clone()));
            }
            Phase::UpToDate { latest }
        }
        Err(e) => match auto {
            Some(before) => {
                log::info!("automatic update check: {e}");
                before
            }
            None => {
                if announce {
                    s.notices.push(Notice::CheckFailed(e.clone()));
                }
                Phase::Failed(e)
            }
        },
    };
    drop(s);
    wake();
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::error!("cannot spawn {name}: {e}");
        STATE.lock().phase = Phase::Failed(tr!("services-update-cannot-start", error = e.to_string()));
    }
}

/// Authenticode exists on Windows only: other platforms and other files report
/// `Unknown` honestly instead of "unsigned". Information for the user; the checksum
/// decides whether a download is used.
fn signature_of(path: &Path) -> Signature {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if cfg!(windows) && (lower.ends_with(".msi") || lower.ends_with(".exe")) {
        soyuz_fs::integration::authenticode(path)
    } else {
        Signature::Unknown
    }
}

/// "Download": save the asset of `release` for this copy into `dir`, verify it, and
/// report [`Phase::Downloaded`]. Nothing is installed.
pub fn download(release: Arc<Release>, dir: PathBuf, wake: Wake) {
    let asset = asset_for_this_copy(&release).cloned();
    let cancel = Arc::new(AtomicBool::new(false));
    let asset = {
        let mut s = STATE.lock();
        if s.phase.busy() {
            return;
        }
        let Some(asset) = asset else {
            s.phase = Phase::Failed(tr!("services-update-no-asset").into());
            drop(s);
            wake();
            return;
        };
        s.cancel = Some(cancel.clone());
        s.phase = Phase::Downloading { release: release.clone(), asset: asset.name.clone(), got: 0, total: asset.size };
        asset
    };
    wake();
    spawn("soyuz-update-download", move || {
        let res = runtime()
            .map_err(|e| e.to_string())
            .and_then(|rt| rt.block_on(download_verified(&release, &asset, &dir, &cancel, &wake)));
        let mut s = STATE.lock();
        s.cancel = None;
        s.phase = match res {
            Ok((path, sha256)) => {
                let signature = signature_of(&path);
                let detached_sig = signature_asset(&release, &asset.name).is_some();
                Phase::Downloaded { release, path, sha256, signature, detached_sig }
            }
            // Cancelled: the offer is back.
            Err(_) if cancel.load(AtomicOrdering::SeqCst) => Phase::Available(release),
            Err(e) => Phase::Failed(e),
        };
        drop(s);
        wake();
    });
}

/// Whether "Update and restart" works for a copy of `kind` in this frontend. The
/// terminal UI leaves an MSI install to the app: Windows Installer's window and the
/// restart belong to the desktop.
pub fn can_install(kind: InstallKind, tui: bool) -> bool {
    kind.self_installs() && !(tui && matches!(kind, InstallKind::WindowsMsi { .. }))
}

/// "Update and restart": download the asset of `release` for this copy, verify it,
/// install it the way this copy was installed and report [`Phase::Ready`]; the next
/// [`pump`] then restarts Soyuz. A copy that does not install itself gets
/// [`Phase::Failed`] with what to do instead; [`cancel`] brings the offer back.
pub fn install(release: Arc<Release>, wake: Wake) {
    let kind = install_kind();
    let asset = asset_for_this_copy(&release).cloned();
    let cancel = Arc::new(AtomicBool::new(false));
    let asset = {
        let mut s = STATE.lock();
        if s.phase.busy() {
            return;
        }
        let refused = match (&asset, how_to_update(kind)) {
            (_, Some(note)) => Some(note),
            (None, None) => Some(tr!("services-update-no-asset").into()),
            (Some(_), None) => None,
        };
        let (Some(asset), None) = (asset, refused.clone()) else {
            s.phase = Phase::Failed(refused.unwrap_or_default());
            drop(s);
            wake();
            return;
        };
        s.cancel = Some(cancel.clone());
        s.phase = Phase::Downloading { release: release.clone(), asset: asset.name.clone(), got: 0, total: asset.size };
        asset
    };
    wake();
    spawn("soyuz-update-install", move || {
        let res = install_worker(&release, &asset, kind, &cancel, &wake);
        let mut s = STATE.lock();
        s.cancel = None;
        s.phase = match res {
            Ok(restart) => {
                s.restart_now = true;
                Phase::Ready { release, restart }
            }
            // Cancelled: the offer is back.
            Err(_) if cancel.load(AtomicOrdering::SeqCst) => Phase::Available(release),
            Err(e) => {
                s.notices.push(Notice::InstallFailed(e.clone()));
                Phase::Failed(e)
            }
        };
        drop(s);
        wake();
    });
}

/// The work of [`install`]: download into a fresh work folder, verify, install.
fn install_worker(
    release: &Arc<Release>,
    asset: &Asset,
    kind: InstallKind,
    cancel: &AtomicBool,
    wake: &Wake,
) -> Result<Restart, String> {
    let fail = |e: &dyn std::fmt::Display| tr!("services-update-install-failed", error = e.to_string());
    let work = swap::new_work_dir().map_err(|e| fail(&e))?;
    let result = (|| {
        let rt = runtime().map_err(|e| e.to_string())?;
        let (path, sha256) = rt.block_on(download_verified(release, asset, &work, cancel, wake))?;
        log::info!("update: {} verified ({sha256}), signature {:?}", asset.name, signature_of(&path));
        set(Phase::Installing { release: release.clone() }, wake);
        let exe = running_exe().map_err(|e| fail(&e))?;
        install_downloaded(&path, &sha256, &exe, kind, &work, &release.version.to_string())
    })();
    match &result {
        // The helper and the MSI stay until Windows Installer is done.
        Ok(Restart::Installer(_)) => {}
        Ok(Restart::Exec(_)) => {
            let _ = std::fs::remove_dir_all(&work);
            // The new build says so when it starts.
            msi::write_result(
                &soyuz_core::config::config_dir(),
                &release.version.to_string(),
                &msi::Outcome::Installed,
            );
        }
        Err(_) => {
            let _ = std::fs::remove_dir_all(&work);
        }
    }
    result
}

/// Install the verified download `path` (SHA-256 `sha256`) over the copy `exe` belongs
/// to, the way a copy of `kind` is installed; `work` is the update's work folder.
/// Copies that do not install themselves are refused with what to do instead, and
/// nothing is touched. Blocking.
pub fn install_downloaded(
    path: &Path,
    sha256: &str,
    exe: &Path,
    kind: InstallKind,
    work: &Path,
    version: &str,
) -> Result<Restart, String> {
    let arch = std::env::consts::ARCH;
    match kind {
        InstallKind::Tarball => install_tarball_to(path, sha256, exe, kind).map(Restart::Exec),
        InstallKind::WindowsPortable => {
            swap::install_archive_to(path, sha256, exe, swap::Payload::WindowsZip, arch).map(Restart::Exec)
        }
        InstallKind::MacArchive => {
            swap::install_archive_to(path, sha256, exe, swap::Payload::MacTarball, arch).map(Restart::Exec)
        }
        InstallKind::WindowsMsi { per_machine } => {
            msi::prepare(work, path.to_path_buf(), sha256.to_string(), exe, per_machine, version.to_string())
                .map(|plan| Restart::Installer(Box::new(plan)))
        }
        other => Err(how_to_update(other).unwrap_or_else(|| tr!("services-update-no-asset").into())),
    }
}

async fn download_verified(
    release: &Release,
    asset: &Asset,
    dir: &Path,
    cancel: &AtomicBool,
    wake: &Wake,
) -> Result<(PathBuf, String), String> {
    let client = client()?;
    let want = expected_sha256(&client, asset, &checksum_assets(release, &asset.name)).await?;

    let name = safe_file_name(&asset.name).ok_or(tr!("services-update-bad-asset-name"))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| tr!("services-update-cannot-create", path = dir.display().to_string(), error = e.to_string()))?;
    let part = dir.join(format!("{name}.part"));
    let mut file = std::fs::File::create(&part).map_err(|e| {
        tr!("services-update-cannot-write-file", path = part.display().to_string(), error = e.to_string())
    })?;
    let cleanup = |e: String| {
        let _ = std::fs::remove_file(&part);
        e
    };
    let progress = |got: u64, total: u64| {
        if let Phase::Downloading { got: g, total: t, .. } = &mut STATE.lock().phase {
            *g = got;
            *t = total;
        }
        wake();
    };
    let have = fetch_into(&client, &asset.url, asset.size, &mut file, cancel, progress).await;
    drop(file);
    let have = have.map_err(cleanup)?;
    if have != want {
        return Err(cleanup(tr!("services-update-checksum-mismatch", expected = &want[..12], got = &have[..12])));
    }
    let target = unique_path(dir, &name);
    std::fs::rename(&part, &target)
        .map_err(|e| cleanup(tr!("services-update-cannot-rename", error = e.to_string())))?;
    Ok((target, have))
}

/// The SHA-256 `asset` must have: GitHub's digest and the first checksum file of `sums`
/// that lists it; every source that exists must agree, and at least one must exist.
async fn expected_sha256(client: &reqwest::Client, asset: &Asset, sums: &[&Asset]) -> Result<String, String> {
    let mut expected: Vec<String> = asset.sha256.iter().cloned().collect();
    for file in sums {
        let text = get_text(client, &file.url, MAX_SUMS).await?;
        if let Some(h) = parse_sums(&text, &asset.name) {
            expected.push(h);
            break;
        }
    }
    if expected.is_empty() {
        return Err(tr!("services-update-no-checksum").into());
    }
    if expected.windows(2).any(|w| w[0] != w[1]) {
        return Err(tr!("services-update-checksums-disagree").into());
    }
    Ok(expected.swap_remove(0))
}

/// Stream `url` into `file` (synced at the end), reporting `(got, total)` every 100 ms and
/// stopping when `cancel` is set or more than [`MAX_ASSET`] bytes arrive. Returns the
/// SHA-256 (lower-case hex) of what was written; the caller removes the file on an error.
async fn fetch_into(
    client: &reqwest::Client,
    url: &str,
    size_hint: u64,
    file: &mut std::fs::File,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<String, String> {
    let mut resp = client.get(url).send().await.map_err(|e| tr!("services-update-network", error = e.to_string()))?;
    if !resp.status().is_success() {
        return Err(tr!("services-update-download-failed", status = resp.status().to_string()));
    }
    let total = resp.content_length().unwrap_or(size_hint);
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    let mut last_report = std::time::Instant::now();
    loop {
        if cancel.load(AtomicOrdering::SeqCst) {
            return Err(tr!("services-update-cancelled").into());
        }
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => return Err(tr!("services-update-network", error = e.to_string())),
        };
        got += chunk.len() as u64;
        if got > MAX_ASSET {
            return Err(tr!("services-update-too-large").into());
        }
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|e| tr!("services-update-cannot-write", error = e.to_string()))?;
        if last_report.elapsed() > Duration::from_millis(100) {
            last_report = std::time::Instant::now();
            progress(got, total);
        }
    }
    file.sync_all().map_err(|e| tr!("services-update-cannot-write", error = e.to_string()))?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `dir/name`, or `dir/stem (n).ext` when that exists.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..1000).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

// ---- Linux tarball install -------------------------------------------------------------

/// Largest `bin/soyuz` / `bin/soyuz-tui` accepted from a tarball.
const MAX_BINARY: u64 = 512 << 20;

/// The executables of a release tarball (`bin/<name>`): the GUI binary and the terminal
/// UI (`soyuz-tui`, shipped since 0.3; older archives do not have it).
pub const ARCHIVE_BINARIES: [&str; 2] = ["soyuz", "soyuz-tui"];

/// ELF `e_machine` of this build's architecture (`None` = not checked).
fn elf_machine() -> Option<u16> {
    match std::env::consts::ARCH {
        "x86_64" => Some(62),
        "aarch64" => Some(183),
        "x86" => Some(3),
        "arm" => Some(40),
        "riscv64" => Some(243),
        _ => None,
    }
}

/// Whether `head` (the first bytes of a file) is an ELF executable for `machine`.
fn is_elf_for(head: &[u8], machine: Option<u16>) -> bool {
    if head.len() < 20 || &head[..4] != b"\x7fELF" {
        return false;
    }
    let m =
        if head[5] == 2 { u16::from_be_bytes([head[18], head[19]]) } else { u16::from_le_bytes([head[18], head[19]]) };
    machine.is_none_or(|want| want == m)
}

/// Octal number of a tar header field (NUL / space padded).
fn tar_octal(field: &[u8]) -> Option<u64> {
    let s: String = field.iter().take_while(|b| **b != 0).map(|b| *b as char).collect();
    let s = s.trim();
    if s.is_empty() { Some(0) } else { u64::from_str_radix(s, 8).ok() }
}

/// Walk the uncompressed tar stream `r` and call `each(path, size, body)` for every
/// regular file (`./` stripped, GNU long names and pax `path=` applied); `body` yields at
/// most the entry's data and whatever the callback leaves unread is skipped. Other entry
/// types (directories, links, devices) are never passed on, so nothing can be redirected
/// through a link. Stops at the end-of-archive marker (or the end of the stream).
fn walk_tar(
    mut r: impl std::io::Read,
    mut each: impl FnMut(&str, u64, &mut dyn std::io::Read) -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Read as _;
    let mut header = [0u8; 512];
    let mut long_name: Option<String> = None;
    loop {
        if r.read_exact(&mut header).is_err() || header.iter().all(|b| *b == 0) {
            return Ok(());
        }
        let field = |a: usize, b: usize| String::from_utf8_lossy(&header[a..b]).trim_end_matches('\0').to_string();
        let size = tar_octal(&header[124..136]).ok_or(tr!("services-update-install-bad-archive"))?;
        let kind = header[156];
        let mut name = field(0, 100);
        if &header[257..262] == b"ustar" {
            let prefix = field(345, 500);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        if let Some(n) = long_name.take() {
            name = n;
        }
        let padded = size.div_ceil(512) * 512;
        let mut body = (&mut r).take(padded);
        match kind {
            // GNU long name / pax extended header: names the next entry.
            b'L' | b'x' => {
                if size > 64 << 10 {
                    return Err(tr!("services-update-install-bad-archive").into());
                }
                let mut buf = Vec::new();
                body.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                buf.truncate(size as usize);
                if kind == b'L' {
                    long_name = Some(String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string());
                } else {
                    long_name = String::from_utf8_lossy(&buf).lines().find_map(|l| {
                        l.split_once(' ').and_then(|(_, kv)| kv.strip_prefix("path=")).map(str::to_string)
                    });
                }
                continue;
            }
            b'0' | 0 => {
                let mut data = (&mut body).take(size);
                each(name.trim_start_matches("./"), size, &mut data)?;
            }
            _ => {}
        }
        std::io::copy(&mut body, &mut std::io::sink()).map_err(|e| e.to_string())?;
    }
}

/// `<file>` of a release tarball entry `<top>/bin/<file>` or `bin/<file>`.
fn bin_entry(path: &str) -> Option<&str> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        ["bin", f] | [_, "bin", f] => Some(*f),
        _ => None,
    }
}

/// Copy the `bin/<names[i]>` entries of a release tarball (`<top>/bin/<name>`, or
/// `bin/<name>`) from the uncompressed tar stream `r` into `outs[i]`, and hand every
/// [`SIDE_DIRS`] file to `side(dir index, file name, size, body)`. Only regular files are
/// read; the output files are ours, so entry names can never escape anywhere. Returns
/// the size of each binary, `None` for one the archive does not have. Reads up to the
/// end-of-archive marker (or the end of the stream); a binary that appears twice is
/// refused.
fn extract_binaries(
    r: impl std::io::Read,
    names: &[&str],
    outs: &mut [&mut dyn std::io::Write],
    mut side: impl FnMut(usize, &str, u64, &mut dyn std::io::Read) -> Result<(), String>,
) -> Result<Vec<Option<u64>>, String> {
    debug_assert_eq!(names.len(), outs.len());
    let mut found: Vec<Option<u64>> = vec![None; names.len()];
    walk_tar(r, |path, size, body| {
        if let Some((dir, file)) = side_entry(path) {
            return side(dir, file, size, body);
        }
        let Some(i) = bin_entry(path).and_then(|f| names.iter().position(|n| *n == f)) else { return Ok(()) };
        if size > MAX_BINARY || found[i].is_some() {
            return Err(tr!("services-update-install-bad-archive").into());
        }
        let n = std::io::copy(body, &mut outs[i]).map_err(|e| e.to_string())?;
        if n != size {
            return Err(tr!("services-update-install-bad-archive").into());
        }
        found[i] = Some(n);
        Ok(())
    })?;
    Ok(found)
}

/// A folder of a release tarball whose files are installed next to the binaries (Phase V:
/// the FFmpeg runtime). `dir` is relative to the archive's top folder and to the install
/// root (`<exe dir>/..`, see [`install_root`]).
struct SideDir {
    dir: &'static [&'static str],
    /// The file names taken from the archive (anything else there is ignored).
    takes: fn(&str) -> bool,
    /// The files must be ELF objects for this CPU.
    elf: bool,
    /// Installed files a new archive replaces even when it no longer ships them (an older
    /// FFmpeg major's libraries); everything else in the folder is left alone.
    stale: fn(&str) -> bool,
}

/// `lib/soyuz/` (the FFmpeg libraries; the exe looks in `<exe dir>/../lib/soyuz`) and
/// their licence texts.
const SIDE_DIRS: [SideDir; 2] = [
    SideDir { dir: &["lib", "soyuz"], takes: is_lib_file, elf: true, stale: is_ffmpeg_lib },
    SideDir { dir: &["share", "licenses", "soyuz", "ffmpeg"], takes: is_ffmpeg_text, elf: false, stale: never },
];

/// The licence / build texts that travel with the FFmpeg libraries.
pub const FFMPEG_TEXTS: [&str; 3] = ["LICENSE.md", "BUILD.txt", "SOURCES.txt"];

fn is_ffmpeg_text(name: &str) -> bool {
    FFMPEG_TEXTS.contains(&name)
}

fn never(_: &str) -> bool {
    false
}

/// `lib<x>.so[.<n>…]`, a plain file name.
fn is_lib_file(name: &str) -> bool {
    name.starts_with("lib")
        && name.contains(".so")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
}

/// `lib{avutil,swresample,swscale,avcodec,avformat}.so.<digits>[.<digits>…]`.
fn is_ffmpeg_lib(name: &str) -> bool {
    ["avutil", "swresample", "swscale", "avcodec", "avformat"].iter().any(|lib| {
        name.strip_prefix("lib")
            .and_then(|n| n.strip_prefix(lib))
            .and_then(|n| n.strip_prefix(".so."))
            .is_some_and(|v| v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
    })
}

/// `(index into SIDE_DIRS, file name)` of a tarball entry `<top>/<dir>/<file>` or
/// `<dir>/<file>` that the folder takes.
fn side_entry(path: &str) -> Option<(usize, &str)> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    SIDE_DIRS.iter().enumerate().find_map(|(i, d)| {
        let n = d.dir.len();
        let file = if parts.len() == n + 1 && parts[..n] == *d.dir {
            parts[n]
        } else if parts.len() == n + 2 && parts[1..=n] == *d.dir {
            parts[n + 1]
        } else {
            return None;
        };
        (d.takes)(file).then_some((i, file))
    })
}

/// The install root of a tarball copy: `<root>/bin/<exe>` → `<root>`. `None` when the
/// executable is not in a `bin` folder (a build folder): no side files are installed then.
fn install_root(exe: &Path) -> Option<&Path> {
    let dir = exe.parent()?;
    if dir.file_name()? == "bin" { dir.parent() } else { None }
}

/// A side file being extracted: its [`SIDE_DIRS`] index, name and temp file.
struct SideFile {
    dir: usize,
    name: String,
    temp: (std::fs::File, PathBuf),
}

/// Where each of [`ARCHIVE_BINARIES`] goes for the running executable `exe` (already
/// canonical): the running one replaces `exe` itself (whatever it is called), the
/// others go next to it under their own names. Also returns the index of the running
/// one: `soyuz-tui` when `exe` is called that, `soyuz` otherwise.
fn install_targets(exe: &Path) -> (Vec<PathBuf>, usize) {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let running = ARCHIVE_BINARIES.iter().position(|n| *n == stem).unwrap_or(0);
    let dir = exe.parent().unwrap_or(Path::new(""));
    let targets =
        ARCHIVE_BINARIES.iter().enumerate().map(|(i, n)| if i == running { exe.to_path_buf() } else { dir.join(n) });
    (targets.collect(), running)
}

/// Replace the running executable `exe` with its new build from the downloaded release
/// tarball `archive` (blocking: call it off the UI thread). The archive is hashed again
/// first (it sat in the Downloads folder) and must match `sha256`.
///
/// Every binary of [`ARCHIVE_BINARIES`] in the archive is installed: the running one
/// (`bin/soyuz`, or `bin/soyuz-tui` when `exe` is `soyuz-tui`) over `exe`, the other one
/// next to it ([`install_targets`]). The running one must be in the archive; the other
/// is optional (archives before 0.3 have no `soyuz-tui`). Each is written to a temporary
/// file next to `exe`, checked to be an ELF executable for this CPU, made executable,
/// synced and renamed over its target (atomic; a running process keeps its old inode),
/// `exe` last. Returns the path to restart into (`exe`).
///
/// When `exe` sits in `<root>/bin/`, the archive's [`SIDE_DIRS`] files go the same way
/// into `<root>/lib/soyuz/` (the FFmpeg libraries, ELF-checked, mode 644) and
/// `<root>/share/licenses/soyuz/ffmpeg/`, before the binaries; libraries of another
/// FFmpeg major that the archive no longer ships are removed at the end (other files in
/// those folders are never touched). An archive without them leaves the folders alone.
pub fn install_tarball_to(archive: &Path, sha256: &str, exe: &Path, kind: InstallKind) -> Result<PathBuf, String> {
    if kind != InstallKind::Tarball {
        return Err(tr!("services-update-package-managed", kind = kind.key()));
    }
    let lower = archive.to_string_lossy().to_ascii_lowercase();
    if !(lower.ends_with(".tar.gz") || lower.ends_with(".tgz")) {
        return Err(tr!("services-update-install-not-tarball").into());
    }
    let fail = |e: &dyn std::fmt::Display| tr!("services-update-install-failed", error = e.to_string());
    let no_permission = |dir: &Path, e: &dyn std::fmt::Display| {
        tr!("services-update-install-no-permission", path = dir.display().to_string(), error = e.to_string())
    };
    // One open of the archive: the bytes extracted are the bytes hashed (the file in
    // ~/Downloads cannot be swapped between a check and the extraction).
    let archive_file = std::fs::File::open(archive).map_err(|e| fail(&e))?;
    let exe = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let dir = exe.parent().ok_or_else(|| fail(&"no parent folder"))?;
    let root = install_root(&exe).map(Path::to_path_buf);
    let side_dir = |i: usize| root.as_ref().map(|r| SIDE_DIRS[i].dir.iter().fold(r.clone(), |p, c| p.join(c)));
    let (targets, running) = install_targets(&exe);
    let mut temps: Vec<(std::fs::File, PathBuf)> = Vec::new();
    for _ in ARCHIVE_BINARIES {
        match create_temp_in(dir) {
            Ok(t) => temps.push(t),
            Err(e) => {
                remove_temps(temps);
                return Err(no_permission(dir, &e));
            }
        }
    }
    let mut sides: Vec<SideFile> = Vec::new();
    let result = (|| -> Result<(), String> {
        use std::io::{Read as _, Seek as _};
        // 1. Extract the binaries into the temp files next to the executable and the side
        //    files into temp files in their folders (same filesystem → atomic rename),
        //    hashing the archive on the way through.
        let hashing = HashingReader { inner: archive_file, hasher: Sha256::new() };
        let mut gz = flate2::read::GzDecoder::new(std::io::BufReader::new(hashing));
        let extracted = {
            let mut outs: Vec<&mut dyn std::io::Write> =
                temps.iter_mut().map(|(f, _)| f as &mut dyn std::io::Write).collect();
            extract_binaries(&mut gz, &ARCHIVE_BINARIES, &mut outs, |i, name, size, body| {
                let Some(to) = side_dir(i) else { return Ok(()) };
                if size > MAX_BINARY || sides.iter().any(|s| s.dir == i && s.name == name) {
                    return Err(tr!("services-update-install-bad-archive").into());
                }
                std::fs::create_dir_all(&to).map_err(|e| no_permission(&to, &e))?;
                let temp = create_temp_in(&to).map_err(|e| no_permission(&to, &e))?;
                sides.push(SideFile { dir: i, name: name.to_string(), temp });
                let out = &mut sides.last_mut().expect("just pushed").temp.0;
                let n = std::io::copy(body, out).map_err(|e| fail(&e))?;
                if n != size {
                    return Err(tr!("services-update-install-bad-archive").into());
                }
                Ok(())
            })
        };
        // 2. Hash the rest of the archive (everything below the decoder was hashed as
        //    it was read, buffered bytes included) and compare before anything else.
        let mut hashing = gz.into_inner().into_inner();
        std::io::copy(&mut hashing, &mut std::io::sink()).map_err(|e| fail(&e))?;
        if !hex(&hashing.hasher.finalize()).eq_ignore_ascii_case(sha256) {
            return Err(tr!("services-update-install-changed").into());
        }
        let found = extracted?;
        if found[running].is_none() {
            return Err(tr!("services-update-install-no-binary", name = ARCHIVE_BINARIES[running]));
        }
        let mut head = [0u8; 64];
        for side in &mut sides {
            let out = &mut side.temp.0;
            out.sync_all().map_err(|e| fail(&e))?;
            if SIDE_DIRS[side.dir].elf {
                out.rewind().map_err(|e| fail(&e))?;
                let n = out.read(&mut head).map_err(|e| fail(&e))?;
                if !is_elf_for(&head[..n], elf_machine()) {
                    return Err(tr!("services-update-install-lib-not-elf", name = side.name.as_str()));
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                out.set_permissions(std::fs::Permissions::from_mode(0o644)).map_err(|e| fail(&e))?;
            }
        }
        for (out, _) in temps.iter_mut().zip(&found).filter(|(_, f)| f.is_some()).map(|(t, _)| t) {
            out.sync_all().map_err(|e| fail(&e))?;
            out.rewind().map_err(|e| fail(&e))?;
            let n = out.read(&mut head).map_err(|e| fail(&e))?;
            if !is_elf_for(&head[..n], elf_machine()) {
                return Err(tr!("services-update-install-not-elf").into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                out.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(|e| fail(&e))?;
            }
        }
        // 3. Only verified files replace the installed ones: the side files first, then
        //    the binaries with the running one last.
        for side in &sides {
            let to = side_dir(side.dir).ok_or_else(|| fail(&"no install root"))?;
            std::fs::rename(&side.temp.1, to.join(&side.name)).map_err(|e| fail(&e))?;
        }
        let order = (0..temps.len()).filter(|i| *i != running).chain(Some(running));
        for i in order.filter(|i| found[*i].is_some()) {
            std::fs::rename(&temps[i].1, &targets[i]).map_err(|e| fail(&e))?;
        }
        // 4. Libraries of an FFmpeg major the archive no longer ships (the new binaries
        //    do not load them; a running process keeps its mapped copies).
        for (i, d) in SIDE_DIRS.iter().enumerate() {
            let Some(folder) = side_dir(i).filter(|_| sides.iter().any(|s| s.dir == i)) else { continue };
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if (d.stale)(&name)
                    && !sides.iter().any(|s| s.dir == i && s.name == name)
                    && let Err(err) = std::fs::remove_file(e.path())
                {
                    log::warn!("cannot remove the old library {}: {err}", e.path().display());
                }
            }
        }
        Ok(())
    })();
    // Temp files not renamed (not in the archive, or after a failure) are removed.
    remove_temps(temps);
    remove_temps(sides.into_iter().map(|s| s.temp).collect());
    result.map(|()| exe)
}

/// Close and delete the temp files of [`install_tarball_to`] that are still there.
fn remove_temps(temps: Vec<(std::fs::File, PathBuf)>) {
    for (f, p) in temps {
        drop(f);
        if std::fs::symlink_metadata(&p).is_ok() {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// A reader that hashes everything read through it.
struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: std::io::Read> std::io::Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
}

/// The SHA-256 (lower-case hex) of everything `r` yields.
fn sha256_of(r: impl std::io::Read) -> std::io::Result<String> {
    let mut hashing = HashingReader { inner: r, hasher: Sha256::new() };
    std::io::copy(&mut hashing, &mut std::io::sink())?;
    Ok(hex(&hashing.hasher.finalize()))
}

/// A new temp file `.soyuz-update-<pid>-<random>.tmp` in `dir`, opened read + write.
/// `create_new` (`O_CREAT | O_EXCL`) never opens an existing file and never follows a
/// symlink planted at the name, so nobody can redirect the write.
fn create_temp_in(dir: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    use std::hash::{BuildHasher, Hasher};
    let mut last = None;
    for attempt in 0..8u32 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(attempt);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        let tmp = dir.join(format!(".soyuz-update-{}-{:016x}.tmp", std::process::id(), h.finish()));
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&tmp) {
            Ok(f) => return Ok((f, tmp)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::ErrorKind::AlreadyExists.into()))
}

// ---- restart ---------------------------------------------------------------------------

/// How Soyuz comes back after an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restart {
    /// The new files are in place: start this executable.
    Exec(PathBuf),
    /// An MSI install: start the helper that runs Windows Installer and then Soyuz.
    Installer(Box<InstallerPlan>),
}

impl Restart {
    /// The update is already installed (a restart only switches to it).
    pub fn installed(&self) -> bool {
        matches!(self, Restart::Exec(_))
    }
}

/// A restart asked for by the close in progress (dropped if that close is cancelled).
static RESTART: Mutex<Option<Restart>> = Mutex::new(None);
/// The restart confirmed when the app actually exited ([`arm_restart`]).
static RESTART_ARMED: Mutex<Option<Restart>> = Mutex::new(None);

/// Restart as `restart` says if the window close that the caller now starts (the Quit
/// action) goes through: the app calls [`arm_restart`] on exit and [`cancel_restart`]
/// when it cancels the close; `main` calls [`take_restart`] after the event loop ends.
pub fn request_restart(restart: Restart) {
    *RESTART.lock() = Some(restart);
}

/// The close was cancelled (pending jobs, unsaved documents): forget the restart, so a
/// later, ordinary quit does not start the new binary or the installer. Returns the
/// restart that was pending.
pub fn cancel_restart() -> Option<Restart> {
    RESTART.lock().take()
}

/// The app is exiting: a requested restart now happens.
pub fn arm_restart() {
    if let Some(restart) = RESTART.lock().take() {
        *RESTART_ARMED.lock() = Some(restart);
    }
}

/// The restart to carry out, if one was requested and the app exited.
pub fn take_restart() -> Option<Restart> {
    RESTART_ARMED.lock().take()
}

/// [`Phase::Ready`] whose restart did not happen (the close was cancelled): restart at
/// the next [`pump`].
pub fn restart_now() {
    let mut s = STATE.lock();
    if matches!(s.phase, Phase::Ready { .. }) {
        s.restart_now = true;
    }
}

/// Carry out `restart` and end this process. [`Restart::Exec`] starts the executable
/// with this process's arguments: on unix the process image is replaced (`execv`, same
/// PID), elsewhere a new process is spawned. [`Restart::Installer`] starts the update
/// helper. Returns only on failure.
pub fn exec_restart(restart: &Restart) -> std::io::Error {
    let exe = match restart {
        Restart::Exec(exe) => exe,
        Restart::Installer(plan) => {
            return match msi::spawn_helper(plan) {
                Ok(()) => std::process::exit(0),
                Err(e) => e,
            };
        }
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.exec()
    }
    #[cfg(not(unix))]
    {
        match cmd.spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => e,
        }
    }
}

/// `msiexec.exe` of this Windows installation.
fn msiexec_path() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("msiexec.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("msiexec.exe"))
}

/// Start a downloaded installer with its own windows (`msiexec /i`), for a copy that
/// does not update itself. Only ever called from an explicit click.
pub fn run_installer(path: &Path) -> std::io::Result<()> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".msi") {
        std::process::Command::new(msiexec_path()).arg("/i").arg(path).spawn().map(drop)
    } else if lower.ends_with(".exe") {
        std::process::Command::new(path).spawn().map(drop)
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "not an installer"))
    }
}

// ---- automatic check -------------------------------------------------------------------

fn stamp_path() -> PathBuf {
    soyuz_core::config::config_dir().join(STAMP_FILE)
}

fn read_stamp() -> Option<SystemTime> {
    let text = std::fs::read_to_string(stamp_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(v.get("last_check")?.as_u64()?))
}

fn write_stamp() {
    // Only a real app session keeps the stamp (never a test's check in the real config folder).
    if !background_enabled() {
        return;
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = stamp_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, format!("{{\"last_check\": {secs}}}\n")) {
        log::debug!("update stamp {}: {e}", path.display());
    }
}

/// Whether the automatic check is due given the last check time: a day after it (never
/// when the clock went backwards).
pub fn check_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|t| now.duration_since(t).is_ok_and(|d| d >= DAY))
}

/// What the automatic check asks for with these settings: `(repository, pre-releases)`,
/// `None` when it is turned off (`auto_check = false`, or no repository).
pub fn auto_check_target(u: &soyuz_core::config::Updates) -> Option<(String, bool)> {
    (u.auto_check && !u.repo.trim().is_empty()).then(|| (u.repo.clone(), u.include_prereleases))
}

/// The background work of a session, on a worker thread: once (`first`) the
/// housekeeping after an update, then the automatic check when one is due.
fn background(first: bool, auto: Option<(String, bool)>, wake: Wake) {
    if first {
        let kind = detect_install();
        if matches!(kind, InstallKind::WindowsPortable | InstallKind::MacArchive)
            && let Ok(exe) = running_exe()
            && let Some(dir) = exe.parent()
        {
            swap::remove_leftovers(dir);
        }
        swap::remove_old_work_dirs();
        if let Some((version, outcome)) = msi::take_result(&soyuz_core::config::config_dir()) {
            STATE.lock().notices.push(Notice::Installed { version, error: outcome.error() });
            wake();
        }
    }
    let Some((repo, prereleases)) = auto else { return };
    if !check_due(read_stamp(), SystemTime::now()) {
        return;
    }
    let before = {
        let mut s = STATE.lock();
        // Never in the way of what the user is doing or has been offered.
        if !matches!(s.phase, Phase::Idle | Phase::UpToDate { .. } | Phase::Available(_) | Phase::Failed(_)) {
            return;
        }
        std::mem::replace(&mut s.phase, Phase::Checking)
    };
    run_check(&repo, prereleases, Some(before), &wake);
}

/// The notification for `notice`.
fn toast_for(notice: Notice, tui: bool) -> crate::models::toasts::Toast {
    use crate::models::toasts::Toast;
    use soyuz_core::config::Action;
    match notice {
        Notice::Available(release) => {
            let kind = install_kind();
            let title = tr!("services-update-available", version = release.version.to_string());
            let mut toast = if can_install(kind, tui) {
                Toast::info(title)
                    .body(tr!("services-update-available-install"))
                    .action(tr!("services-update-action-install"), Action::InstallUpdate)
            } else {
                let body = match how_to_update(kind) {
                    Some(note) if kind.managed() => note,
                    _ => tr!("services-update-available-body").into(),
                };
                Toast::info(title).body(body).action(tr!("services-update-action-details"), Action::InstallUpdate)
            };
            // Time to read it and decide.
            toast.duration = Some(20.0);
            toast
        }
        Notice::UpToDate(latest) => Toast::info(tr!("services-update-up-to-date", version = latest)),
        Notice::CheckFailed(e) => Toast::warning(tr!("services-update-check-failed")).body(e),
        Notice::InstallFailed(e) => Toast::error(tr!("services-update-not-installed")).body(e),
        Notice::Installed { version, error: None } => {
            Toast::success(tr!("services-update-installed", version = version))
        }
        Notice::Installed { version, error: Some(e) } => Toast::error(tr!("services-update-not-installed")).body(tr!(
            "services-update-not-installed-body",
            version = version,
            current = CURRENT,
            error = e
        )),
    }
}

/// `Action::CheckForUpdates`: start a check. The GUI opens Settings → About & updates,
/// where the result shows; the terminal UI gets it as a notification.
pub fn check_from_action(app: &mut crate::core::AppCore) {
    use crate::models::toasts::Toast;
    let tui = app.frontend_kind == crate::core::FrontendKind::Tui;
    let u = &app.config.updates;
    if u.repo.trim().is_empty() {
        app.ui.toasts.push(Toast::warning(tr!("services-update-check-failed")).body(tr!("services-update-no-repo")));
        return;
    }
    check(u.repo.clone(), u.include_prereleases, tui, app.wake.clone());
    if !tui {
        app.frontend
            .push(crate::models::frontend::FrontendCmd::OpenSettings(Some(crate::models::settings::Section::About)));
    }
}

/// `Action::InstallUpdate` (the button of the "update available" notification, the
/// palette): "Update and restart" where this copy installs itself, else what to do
/// instead. Without a known newer release it checks first.
pub fn install_from_action(app: &mut crate::core::AppCore) {
    use crate::models::toasts::Toast;
    let tui = app.frontend_kind == crate::core::FrontendKind::Tui;
    let release = match phase() {
        Phase::Available(r) => r,
        Phase::Ready { .. } => return restart_now(),
        _ => return check_from_action(app),
    };
    let kind = install_kind();
    let version = release.version.to_string();
    if can_install(kind, tui) {
        install(release, app.wake.clone());
        if tui {
            app.ui.toasts.push(
                Toast::info(tr!("services-update-installing", version = version))
                    .body(tr!("services-update-installing-body")),
            );
        }
    } else if tui {
        let how = match kind {
            InstallKind::WindowsMsi { .. } => tr!("services-update-tui-msi").to_string(),
            k => how_to_update(k).unwrap_or_default(),
        };
        let mut toast =
            Toast::info(tr!("services-update-available", version = version)).body(format!("{how}\n{}", release.url));
        toast.duration = Some(20.0);
        app.ui.toasts.push(toast);
    }
    if !tui {
        app.frontend
            .push(crate::models::frontend::FrontendCmd::OpenSettings(Some(crate::models::settings::Section::About)));
    }
}

/// Open the release page `url` in the browser registered for it, on a worker thread
/// (`soyuz_fs::launch::open_url`: registry or MIME lookups and a process spawn stay off
/// the UI thread). Only `https://` pages are opened; failures come back as a toast.
pub fn open_release_page(app: &mut crate::core::AppCore, url: &str) {
    use crate::models::toasts::Toast;
    if !url.starts_with("https://") {
        return;
    }
    let postman = app.services.mailbox.postman();
    let url = url.to_string();
    let spawned = std::thread::Builder::new().name("soyuz-update-link".into()).spawn(move || {
        if let Err(e) = soyuz_fs::launch::open_url(&url) {
            postman.send(crate::services::mailbox::UiMsg::Notice(
                Toast::error(tr!("settings-updates-open-failed")).body(format!("{url}\n{e}")),
            ));
        }
    });
    if let Err(e) = spawned {
        app.ui.toasts.push(Toast::error(tr!("settings-updates-open-failed")).body(e.to_string()));
    }
}

/// The palette offers "Update and restart" only while a newer release is known.
pub fn palette_shows(a: &soyuz_core::config::Action) -> bool {
    *a != soyuz_core::config::Action::InstallUpdate || matches!(phase(), Phase::Available(_) | Phase::Ready { .. })
}

/// Per-frame hook. In an app session ([`enable_background`]): once, the housekeeping
/// after an update; then, while `[updates] auto_check` is on, the daily check (looked at
/// once an hour). Always: results become notifications, and an installed update
/// restarts Soyuz through the ordinary Quit (pending jobs are asked about).
pub fn pump(app: &mut crate::core::AppCore) {
    ffmpeg_runtime::pump(app);
    let (notices, restart) = {
        let mut s = STATE.lock();
        let now = Instant::now();
        if background_enabled() && s.next_auto.is_none_or(|t| now >= t) {
            let first = s.next_auto.is_none();
            s.next_auto = Some(now + AUTO_RECHECK);
            let auto = auto_check_target(&app.config.updates);
            if first || auto.is_some() {
                let wake = app.wake.clone();
                drop(s);
                spawn("soyuz-update-auto", move || background(first, auto, wake));
                s = STATE.lock();
            }
        }
        let restart_now = std::mem::take(&mut s.restart_now);
        let restart = match &s.phase {
            Phase::Ready { restart, .. } if restart_now => Some(restart.clone()),
            _ => None,
        };
        (std::mem::take(&mut s.notices), restart)
    };
    let tui = app.frontend_kind == crate::core::FrontendKind::Tui;
    for notice in notices {
        app.ui.toasts.push(toast_for(notice, tui));
    }
    if let Some(restart) = restart {
        request_restart(restart);
        app.cmds.push(crate::cmd::Cmd::Action(soyuz_core::config::Action::Quit));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local HTTP server on 127.0.0.1 that answers `GET <path>` with the bytes of
    /// `routes` (404 otherwise) and counts the requests per path. No test talks to the
    /// real network.
    fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead as _, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().push(path.clone());
                let reply = match routes.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let mut r =
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                                .into_bytes();
                        r.extend_from_slice(body);
                        r
                    }
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                let _ = stream.write_all(&reply);
            }
        });
        (base, seen)
    }

    fn sha_of(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// A release whose asset `name` and checksum list `sums` live on the mock server.
    fn mock_release(base: &str, name: &str, digest: Option<String>, sums: bool) -> (Arc<Release>, Asset) {
        let asset = Asset { name: name.into(), url: format!("{base}/{name}"), size: 0, sha256: digest };
        let mut assets = vec![asset.clone()];
        if sums {
            assets.push(Asset {
                name: "SHA256SUMS-windows.txt".into(),
                url: format!("{base}/SHA256SUMS-windows.txt"),
                size: 0,
                sha256: None,
            });
        }
        let release = Release {
            tag: "v9.0.0".into(),
            version: v("9.0.0"),
            title: "Soyuz 9.0.0".into(),
            notes: String::new(),
            url: format!("{base}/release"),
            prerelease: false,
            published: String::new(),
            assets,
        };
        (Arc::new(release), asset)
    }

    fn fetch(release: &Release, asset: &Asset, dir: &Path) -> Result<(PathBuf, String), String> {
        let cancel = AtomicBool::new(false);
        let wake = crate::host::no_wake();
        runtime().unwrap().block_on(download_verified(release, asset, dir, &cancel, &wake))
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn a_download_is_kept_only_when_every_checksum_matches() {
        const NAME: &str = "soyuz-9.0.0-windows-x64-portable.zip";
        let body = b"the release archive".to_vec();
        let good = sha_of(&body);
        let other = sha_of(b"something else");
        let sums = |hash: &str| format!("{hash}  {NAME}\n{}  another-file.zip\n", "1".repeat(64)).into_bytes();
        let d = tempfile::tempdir().unwrap();

        // The checksum file and GitHub's digest agree with the download: kept.
        let (base, seen) = serve(vec![
            ("/soyuz-9.0.0-windows-x64-portable.zip", body.clone()),
            ("/SHA256SUMS-windows.txt", sums(&good)),
        ]);
        let (release, asset) = mock_release(&base, NAME, Some(good.clone()), true);
        let (path, sha) = fetch(&release, &asset, d.path()).unwrap();
        assert_eq!((sha.as_str(), std::fs::read(&path).unwrap()), (good.as_str(), body.clone()));
        assert_eq!(files_in(d.path()), [NAME], "no partial file left");
        // The digest alone is enough when the release has no checksum file.
        let (release, asset) = mock_release(&base, NAME, Some(good.clone()), false);
        assert!(fetch(&release, &asset, d.path()).is_ok());
        // The two sources disagree: refused before anything is downloaded.
        let before = seen.lock().len();
        let (release, asset) = mock_release(&base, NAME, Some(other.clone()), true);
        assert_eq!(fetch(&release, &asset, d.path()).unwrap_err(), tr!("services-update-checksums-disagree"));
        assert_eq!(seen.lock()[before..], ["/SHA256SUMS-windows.txt"], "the archive was not requested");
        // No checksum anywhere: refused, nothing downloaded.
        let (release, asset) = mock_release(&base, NAME, None, false);
        let before = seen.lock().len();
        assert_eq!(fetch(&release, &asset, d.path()).unwrap_err(), tr!("services-update-no-checksum"));
        assert_eq!(seen.lock().len(), before);

        // The download is not what the checksum file promises: deleted, never installed.
        let d = tempfile::tempdir().unwrap();
        let (base, _) = serve(vec![
            ("/soyuz-9.0.0-windows-x64-portable.zip", b"tampered".to_vec()),
            ("/SHA256SUMS-windows.txt", sums(&good)),
        ]);
        let (release, asset) = mock_release(&base, NAME, None, true);
        let err = fetch(&release, &asset, d.path()).unwrap_err();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(files_in(d.path()).is_empty(), "the bad download is gone");
        // The same with GitHub's digest as the only source.
        let (release, asset) = mock_release(&base, NAME, Some(good.clone()), false);
        assert!(fetch(&release, &asset, d.path()).is_err());
        assert!(files_in(d.path()).is_empty());
        // A checksum file that does not list the archive is no checksum.
        let (base, _) = serve(vec![
            ("/soyuz-9.0.0-windows-x64-portable.zip", body.clone()),
            ("/SHA256SUMS-windows.txt", sums(&good).split_off(NAME.len() + 67)),
        ]);
        let (release, asset) = mock_release(&base, NAME, None, true);
        assert_eq!(fetch(&release, &asset, d.path()).unwrap_err(), tr!("services-update-no-checksum"));
        // A missing archive is an error, not an empty file.
        let (release, asset) = mock_release(&base, "soyuz-9.0.0-windows-arm64-portable.zip", Some(good), false);
        assert!(fetch(&release, &asset, d.path()).is_err());
        assert!(files_in(d.path()).is_empty());
    }

    #[test]
    fn a_verified_portable_zip_is_installed_and_a_managed_copy_is_never_touched() {
        use std::io::Write as _;
        // The new release: a portable zip with a (minimal) program for this processor.
        let machine: u16 = match std::env::consts::ARCH {
            "aarch64" => 0xAA64,
            "x86" => 0x014C,
            _ => 0x8664,
        };
        let mut program = vec![0u8; 0x80];
        program[..2].copy_from_slice(b"MZ");
        program[0x3c] = 0x40;
        program[0x40..0x44].copy_from_slice(b"PE\0\0");
        program[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["soyuz.exe", "soyuz-tui.exe"] {
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&program).unwrap();
        }
        let body = w.finish().unwrap().into_inner();
        const NAME: &str = "soyuz-9.0.0-windows-x64-portable.zip";
        let sums = format!("{}  {NAME}\n", sha_of(&body)).into_bytes();
        let (base, _) = serve(vec![("/soyuz-9.0.0-windows-x64-portable.zip", body), ("/SHA256SUMS-windows.txt", sums)]);
        let (release, asset) = mock_release(&base, NAME, None, true);

        // An installed copy (dummy files in a temp folder, never the real install).
        let d = tempfile::tempdir().unwrap();
        let (install, work) = (d.path().join("Soyuz"), d.path().join("work"));
        std::fs::create_dir_all(install.join("portable-data")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let exe = install.join("soyuz.exe");
        for (name, data) in
            [("soyuz.exe", "old"), ("soyuz-tui.exe", "old"), ("portable", ""), ("portable-data/config.toml", "mine")]
        {
            std::fs::write(install.join(name), data).unwrap();
        }
        let (path, sha) = fetch(&release, &asset, &work).unwrap();

        // Copies something else owns, or that cannot be written: refused, untouched.
        use InstallKind::*;
        for kind in [Scoop, Homebrew, Package, Flatpak, AppImage, Msix, ReadOnly, Manual] {
            let err = install_downloaded(&path, &sha, &exe, kind, &work, "9.0.0").unwrap_err();
            assert_eq!(Some(err), how_to_update(kind), "{kind:?}");
            assert_eq!(std::fs::read(&exe).unwrap(), b"old", "{kind:?}");
            assert_eq!(files_in(&install), ["portable", "portable-data", "soyuz-tui.exe", "soyuz.exe"], "{kind:?}");
        }
        // A download that changed after it was verified is not installed either.
        assert!(install_downloaded(&path, &"0".repeat(64), &exe, WindowsPortable, &work, "9.0.0").is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");

        // The portable copy: swapped in place, restart into the same path.
        let restart = install_downloaded(&path, &sha, &exe, WindowsPortable, &work, "9.0.0").unwrap();
        assert_eq!(restart, Restart::Exec(exe.clone()));
        assert!(restart.installed());
        assert_eq!(std::fs::read(&exe).unwrap(), program);
        assert_eq!(std::fs::read(install.join("soyuz-tui.exe")).unwrap(), program);
        assert_eq!(std::fs::read(install.join("portable-data/config.toml")).unwrap(), b"mine");

        // An MSI install: nothing is installed in the app's process. The plan names the
        // same folder and scope, and the installed files are as they were.
        std::fs::write(&exe, b"msi install").unwrap();
        let msi_path = work.join("soyuz-9.0.0-windows-x64.msi");
        std::fs::write(&msi_path, b"msi").unwrap();
        let restart =
            install_downloaded(&msi_path, &sha_of(b"msi"), &exe, WindowsMsi { per_machine: true }, &work, "9.0.0")
                .unwrap();
        let Restart::Installer(plan) = &restart else { panic!("{restart:?}") };
        assert!(!restart.installed());
        assert_eq!(
            (plan.install_dir.as_path(), plan.per_machine, plan.version.as_str()),
            (install.as_path(), true, "9.0.0")
        );
        assert_eq!(plan.relaunch, exe);
        assert!(plan.helper.starts_with(&work) && plan.helper.is_file());
        assert_eq!(std::fs::read(&exe).unwrap(), b"msi install");
    }

    #[test]
    fn the_automatic_check_is_on_by_default_and_daily() {
        let mut u = soyuz_core::config::Updates::default();
        assert!(u.auto_check, "on for everyone who never chose");
        assert_eq!(auto_check_target(&u), Some((soyuz_core::config::DEFAULT_UPDATE_REPO.to_string(), false)));
        u.include_prereleases = true;
        assert_eq!(auto_check_target(&u).map(|t| t.1), Some(true));
        // The Settings switch, and an empty repository, turn it off.
        u.auto_check = false;
        assert_eq!(auto_check_target(&u), None);
        let empty = soyuz_core::config::Updates { repo: "  ".into(), ..Default::default() };
        assert_eq!(auto_check_target(&empty), None);
        // Nothing built on the core checks by itself: only the app binaries turn it on.
        assert!(!background_enabled());

        let now = SystemTime::now();
        assert!(check_due(None, now));
        assert!(!check_due(Some(now - Duration::from_secs(3600)), now));
        assert!(!check_due(Some(now - DAY + Duration::from_secs(60)), now));
        assert!(check_due(Some(now - DAY), now));
        assert!(check_due(Some(now - 7 * DAY), now));
        assert!(!check_due(Some(now + Duration::from_secs(60)), now), "clock skew: not due");
    }

    #[test]
    fn only_some_copies_update_from_the_terminal_ui() {
        use InstallKind::*;
        for kind in [Tarball, WindowsPortable, MacArchive] {
            assert!(can_install(kind, false) && can_install(kind, true), "{kind:?}");
        }
        let msi = WindowsMsi { per_machine: false };
        assert!(can_install(msi, false) && !can_install(msi, true));
        for kind in [Manual, Scoop, Homebrew, Package, Flatpak, AppImage, Msix, ReadOnly] {
            assert!(!can_install(kind, false) && !can_install(kind, true), "{kind:?}");
        }
    }

    const FIXTURE: &str = include_str!("updates_fixture.json");

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn version_parse_and_compare() {
        assert_eq!(v("v1.2.3"), Version { major: 1, minor: 2, patch: 3, pre: vec![] });
        assert_eq!(v("2"), v("2.0.0"));
        assert_eq!(v("1.0.0+build.5"), v("1.0.0"));
        for bad in ["", "x1.0", "1..2", "1.2.3.4", "1.2.3-", "1.2.3-a..b", "1.-2.3"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        // SemVer 2.0 §11 precedence example, in ascending order.
        let order = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0",
            "10.0.0",
        ];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
        assert!(is_newer(&v("0.2.0"), "0.1.0"));
        assert!(!is_newer(&v("0.1.0"), "0.1.0"));
        assert!(!is_newer(&v("0.1.0-rc.1"), "0.1.0"));
        assert!(is_newer(&v("0.1.0"), "0.1.0-rc.1"));
        assert_eq!(v("v1.2.3-rc.1").to_string(), "1.2.3-rc.1");
    }

    #[test]
    fn repo_forms() {
        let want = Ok(("soyuz-dev".to_string(), "soyuz".to_string()));
        for s in [
            "soyuz-dev/soyuz",
            " https://github.com/soyuz-dev/soyuz ",
            "https://github.com/soyuz-dev/soyuz.git",
            "github.com/soyuz-dev/soyuz/",
            "https://www.github.com/soyuz-dev/soyuz/releases",
        ] {
            assert_eq!(parse_repo(s), want, "{s}");
        }
        for s in ["", "soyuz", "https://gitlab.com/a/b", "a/../b", "a b/c", "../x"] {
            assert!(parse_repo(s).is_err(), "{s}");
        }
        assert_eq!(api_url("o", "n", false), "https://api.github.com/repos/o/n/releases/latest");
        assert!(api_url("o", "n", true).ends_with("/repos/o/n/releases?per_page=20"));
    }

    #[test]
    fn parses_release_fixture() {
        // The fixture is a `/releases` list: a draft, a newer pre-release and a stable release.
        let stable = parse_releases(FIXTURE, false).unwrap();
        assert_eq!(stable.tag, "v0.2.0");
        assert_eq!(stable.title, "Soyuz 0.2.0");
        assert_eq!(stable.published, "2026-09-30");
        assert!(stable.notes.contains("Default file manager"));
        assert_eq!(stable.assets.len(), 4);
        let msi = pick_asset(&stable, "windows", "x86_64", false).unwrap();
        assert_eq!(msi.name, "soyuz-0.2.0-windows-x64.msi");
        assert_eq!(msi.sha256.as_deref(), Some("4f2b7a5d0d1c6f4a1c9e3b2a8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d9e"));
        assert_eq!(pick_asset(&stable, "linux", "x86_64", false), None);
        let sums = checksum_assets(&stable, &msi.name);
        assert_eq!(sums[0].name, "SHA256SUMS-windows.txt");
        assert!(signature_asset(&stable, &msi.name).is_some());

        let pre = parse_releases(FIXTURE, true).unwrap();
        assert_eq!(pre.tag, "v0.3.0-rc.1");
        assert!(pre.prerelease);

        // `/releases/latest` returns a single object.
        let one = serde_json::to_string(&serde_json::from_str::<serde_json::Value>(FIXTURE).unwrap()[2]).unwrap();
        assert_eq!(parse_releases(&one, false).unwrap().tag, "v0.2.0");
        assert_eq!(parse_releases(r#"{"message":"Not Found"}"#, false).unwrap_err(), "GitHub: Not Found");
        assert!(parse_releases("[]", false).is_err());
        assert!(parse_releases("<html>", false).is_err());
    }

    #[test]
    fn zip_is_the_fallback_asset() {
        let mut r = parse_releases(FIXTURE, false).unwrap();
        r.assets.retain(|a| !a.name.ends_with(".msi"));
        assert_eq!(pick_asset(&r, "windows", "x86_64", false).unwrap().name, "soyuz-0.2.0-windows-x64.zip");
        assert_eq!(pick_asset(&r, "windows", "aarch64", false), None);
        // Portable copies take the portable zip, installed copies never do when there's a choice.
        let mut p = r.clone();
        p.assets.push(Asset {
            name: "soyuz-0.2.0-windows-x64-portable.zip".into(),
            url: "https://example.invalid/p.zip".into(),
            size: 1,
            sha256: None,
        });
        assert_eq!(pick_asset(&p, "windows", "x86_64", true).unwrap().name, "soyuz-0.2.0-windows-x64-portable.zip");
        assert_eq!(pick_asset(&p, "windows", "x86_64", false).unwrap().name, "soyuz-0.2.0-windows-x64.zip");
        let full = parse_releases(FIXTURE, false).unwrap();
        assert_eq!(pick_asset(&full, "windows", "x86_64", true).unwrap().name, "soyuz-0.2.0-windows-x64.zip");
        assert_eq!(pick_asset(&full, "macos", "aarch64", false), None, "darwin never matches windows assets");
    }

    #[test]
    fn only_the_app_archives_of_the_release_are_picked() {
        // An FFmpeg runtime (its own release tag, but a copy of it or a source tarball may
        // sit next to the app) names an OS and an arch and ends like an app archive: the
        // `soyuz-<version>-` prefix keeps every matcher away from it.
        let mut r = parse_releases(FIXTURE, false).unwrap();
        r.assets.clear();
        for n in [
            "ffmpeg-runtime-9.0.2-windows-x64.zip",
            "ffmpeg-runtime-9.0.2-windows-x64-portable.zip",
            "ffmpeg-runtime-9.0.2-macos-universal.zip",
            "ffmpeg-runtime-9.0.2-linux-x86_64.tar.xz",
            "ffmpeg-runtime-9.0.2-linux-x86_64.tar.gz",
            "ffmpeg-9.0.2.tar.xz",
            "soyuz-0.1.0-windows-x64.zip",
            "SOYUZ-0.2.0-windows-x64.msi.sig",
        ] {
            r.assets.push(Asset { name: n.into(), url: format!("https://example.invalid/{n}"), size: 1, sha256: None });
        }
        for (os, arch) in [("windows", "x86_64"), ("macos", "aarch64"), ("macos", "x86_64"), ("linux", "x86_64")] {
            for portable in [false, true] {
                assert_eq!(pick_asset(&r, os, arch, portable), None, "{os}/{arch} portable={portable}");
            }
        }
        // The app's own archives are still found, whatever their case.
        r.assets.push(Asset { name: "Soyuz-0.2.0-macOS-universal.zip".into(), url: "u".into(), size: 1, sha256: None });
        r.assets.push(Asset { name: "soyuz-0.2.0-windows-x64.zip".into(), url: "u".into(), size: 1, sha256: None });
        assert_eq!(pick_asset(&r, "macos", "aarch64", false).unwrap().name, "Soyuz-0.2.0-macOS-universal.zip");
        assert_eq!(pick_asset(&r, "windows", "x86_64", true).unwrap().name, "soyuz-0.2.0-windows-x64.zip");
    }

    #[test]
    fn checksum_file_parsing() {
        let h1 = "a".repeat(64);
        let h2 = "B".repeat(64);
        let text = format!("{h1}  soyuz-0.2.0-windows-x64.zip\n{h2} *dist/soyuz-0.2.0-windows-x64.msi\n# comment\n");
        assert_eq!(parse_sums(&text, "soyuz-0.2.0-windows-x64.zip"), Some(h1.clone()));
        assert_eq!(parse_sums(&text, "soyuz-0.2.0-windows-x64.msi"), Some(h2.to_ascii_lowercase()));
        assert_eq!(parse_sums(&text, "other.zip"), None);
        assert_eq!(parse_sums(&format!("{h1}\n"), "anything"), Some(h1));
        assert_eq!(parse_sums("deadbeef  x.zip", "x.zip"), None);
    }

    #[test]
    fn file_names() {
        assert_eq!(safe_file_name("soyuz-1.0.0.msi").as_deref(), Some("soyuz-1.0.0.msi"));
        assert_eq!(safe_file_name(r"..\..\evil.msi").as_deref(), Some("....evil.msi"));
        assert_eq!(safe_file_name(".."), None);
        assert_eq!(safe_file_name("a/b:c"), Some("abc".into()));

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.msi"), dir.path().join("a.msi"));
        std::fs::write(dir.path().join("a.msi"), b"").unwrap();
        assert_eq!(unique_path(dir.path(), "a.msi"), dir.path().join("a (1).msi"));
    }

    /// Live request against a real public repository (network): run with `--ignored`.
    #[test]
    #[ignore]
    fn live_github_latest() {
        let rt = runtime().unwrap();
        let r = rt.block_on(fetch_latest("BurntSushi/ripgrep", false)).unwrap();
        assert!(r.version >= v("14.0.0"), "{r:?}");
        assert!(!r.assets.is_empty());
        let asset = pick_asset(&r, "windows", "x86_64", false).expect("ripgrep ships a windows x64 zip");
        let client = client().unwrap();
        let sums = checksum_assets(&r, &asset.name);
        assert!(asset.sha256.is_some() || !sums.is_empty(), "no checksum source for {}", asset.name);
        if let Some(s) = sums.first() {
            let text = rt.block_on(get_text(&client, &s.url, MAX_SUMS)).unwrap();
            assert!(parse_sums(&text, &asset.name).is_some(), "{text}");
        }
    }

    fn linux_release() -> Release {
        let mut r = parse_releases(FIXTURE, false).unwrap();
        for n in ["soyuz-0.2.0-linux-x86_64.tar.gz", "soyuz-0.2.0-linux-x86_64.tar.gz.sha256", "SHA256SUMS-linux.txt"] {
            r.assets.push(Asset { name: n.into(), url: format!("https://example.invalid/{n}"), size: 1, sha256: None });
        }
        r
    }

    #[test]
    fn linux_tarball_asset_and_checksums() {
        let r = linux_release();
        let a = pick_asset(&r, "linux", "x86_64", false).unwrap();
        assert_eq!(a.name, "soyuz-0.2.0-linux-x86_64.tar.gz");
        assert_eq!(pick_asset(&r, "linux", "x86_64", true).unwrap().name, a.name, "portable copies too");
        assert_eq!(pick_asset(&r, "linux", "aarch64", false), None);
        // The older x64 spelling still matches (the asset of the release being considered).
        let mut old = parse_releases(FIXTURE, false).unwrap();
        old.assets.push(Asset {
            name: "soyuz-0.2.0-linux-x64.tar.gz".into(),
            url: "https://x/y".into(),
            size: 1,
            sha256: None,
        });
        assert_eq!(pick_asset(&old, "linux", "x86_64", false).unwrap().name, "soyuz-0.2.0-linux-x64.tar.gz");
        let sums: Vec<&str> = checksum_assets(&r, &a.name).iter().map(|s| s.name.as_str()).collect();
        assert_eq!(sums[0], "soyuz-0.2.0-linux-x86_64.tar.gz.sha256", "own .sha256 first");
        assert!(sums.contains(&"SHA256SUMS-linux.txt"));
    }

    /// A minimal ustar archive with `entries` (name, data, type flag).
    pub(super) fn tar(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data, kind) in entries {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..107].copy_from_slice(b"0000755");
            h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
            h[156] = *kind;
            h[257..262].copy_from_slice(b"ustar");
            out.extend_from_slice(&h);
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.extend_from_slice(&[0u8; 1024]);
        out
    }

    fn elf() -> Vec<u8> {
        let mut e = vec![0u8; 64];
        e[..4].copy_from_slice(b"\x7fELF");
        e[4] = 2;
        e[5] = 1;
        let m = elf_machine().unwrap_or(62);
        e[18..20].copy_from_slice(&m.to_le_bytes());
        e.extend_from_slice(b"new build");
        e
    }

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    /// [`extract_binaries`] of `soyuz` and `soyuz-tui` into memory.
    #[allow(clippy::type_complexity)]
    fn extract(r: &[u8]) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>), String> {
        let (mut gui, mut tui) = (Vec::new(), Vec::new());
        let found = extract_binaries(r, &ARCHIVE_BINARIES, &mut [&mut gui, &mut tui], |_, _, _, _| Ok(()))?;
        Ok((found[0].map(|_| gui), found[1].map(|_| tui)))
    }

    #[test]
    fn tar_extraction() {
        let bin = elf();
        let long = format!("soyuz-0.2.0-linux-x86_64/{}/x", "d".repeat(120));
        let t = tar(&[
            ("soyuz-0.2.0-linux-x86_64/", b"", b'5'),
            ("././@LongLink", long.as_bytes(), b'L'),
            ("soyuz-0.2.0-linux-x86_64/ddd", b"long-named file", b'0'),
            ("soyuz-0.2.0-linux-x86_64/share/doc/soyuz/README.md", b"readme", b'0'),
            ("soyuz-0.2.0-linux-x86_64/bin/soyuz", &bin, b'0'),
        ]);
        assert_eq!(extract(&t[..]).unwrap(), (Some(bin.clone()), None));
        // Both binaries, in any order.
        let t = tar(&[("top/bin/soyuz-tui", b"tui", b'0'), ("top/bin/soyuz", b"gui", b'0')]);
        assert_eq!(extract(&t[..]).unwrap(), (Some(b"gui".to_vec()), Some(b"tui".to_vec())));
        // Nested deeper, a symlink, another name, or missing: not taken.
        let t = tar(&[
            ("a/b/bin/soyuz", b"x", b'0'),
            ("top/bin/soyuz", b"", b'2'),
            ("top/bin/soyuz-tuix", b"x", b'0'),
            ("top/soyuz-tui", b"x", b'0'),
        ]);
        assert_eq!(extract(&t[..]).unwrap(), (None, None));
        assert_eq!(extract(&b"short"[..]).unwrap(), (None, None));
        // The same binary twice: refused.
        let t = tar(&[("top/bin/soyuz", b"a", b'0'), ("bin/soyuz", b"b", b'0')]);
        assert!(extract(&t[..]).is_err());
        assert!(is_elf_for(&bin, elf_machine()));
        assert!(!is_elf_for(b"#!/bin/sh\n echo hi ........", None));
        let mut other = bin.clone();
        other[18..20].copy_from_slice(&0xBEEFu16.to_le_bytes());
        assert!(!is_elf_for(&other, Some(62)));
    }

    #[test]
    fn tarball_install_replaces_the_executable() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("bin").join("soyuz");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"old build").unwrap();
        let archive = d.path().join("soyuz-0.2.0-linux-x86_64.tar.gz");
        std::fs::write(&archive, gz(&tar(&[("soyuz-0.2.0-linux-x86_64/bin/soyuz", &elf(), b'0')]))).unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&archive).unwrap()));

        // Package-managed copies and tampered downloads are refused, nothing changes.
        assert!(install_tarball_to(&archive, &sha, &exe, InstallKind::Package).is_err());
        assert!(install_tarball_to(&archive, &"0".repeat(64), &exe, InstallKind::Tarball).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old build");

        let new = install_tarball_to(&archive, &sha, &exe, InstallKind::Tarball).unwrap();
        assert_eq!(new, std::fs::canonicalize(&exe).unwrap());
        assert_eq!(std::fs::read(&exe).unwrap(), elf());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let leftovers: Vec<_> =
            std::fs::read_dir(exe.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "no temp file left: {leftovers:?}");

        // Not an ELF binary for this CPU: refused, temp file removed.
        let bad = d.path().join("bad.tar.gz");
        std::fs::write(&bad, gz(&tar(&[("x/bin/soyuz", b"#!/bin/sh\necho pwned\n", b'0')]))).unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&bad).unwrap()));
        assert!(install_tarball_to(&bad, &sha, &exe, InstallKind::Tarball).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), elf());
        assert_eq!(std::fs::read_dir(exe.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn install_targets_follow_the_running_binary() {
        let (t, running) = install_targets(Path::new("/opt/soyuz/bin/soyuz"));
        assert_eq!(running, 0);
        assert_eq!(t, vec![PathBuf::from("/opt/soyuz/bin/soyuz"), PathBuf::from("/opt/soyuz/bin/soyuz-tui")]);
        let (t, running) = install_targets(Path::new("/opt/soyuz/bin/soyuz-tui"));
        assert_eq!(running, 1);
        assert_eq!(t, vec![PathBuf::from("/opt/soyuz/bin/soyuz"), PathBuf::from("/opt/soyuz/bin/soyuz-tui")]);
        // A renamed GUI binary is replaced in place.
        let (t, running) = install_targets(Path::new("/x/soyuz-nightly"));
        assert_eq!(running, 0);
        assert_eq!(t, vec![PathBuf::from("/x/soyuz-nightly"), PathBuf::from("/x/soyuz-tui")]);
    }

    #[test]
    fn tarball_install_swaps_both_binaries() {
        let tui_elf = || {
            let mut e = elf();
            e.extend_from_slice(b" (tui)");
            e
        };
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let (gui, tui) = (bin.join("soyuz"), bin.join("soyuz-tui"));
        std::fs::write(&gui, b"old gui").unwrap();
        std::fs::write(&tui, b"old tui").unwrap();
        let archive = d.path().join("soyuz-0.3.0-linux-x86_64.tar.gz");
        std::fs::write(
            &archive,
            gz(&tar(&[
                ("soyuz-0.3.0-linux-x86_64/bin/soyuz", &elf(), b'0'),
                ("soyuz-0.3.0-linux-x86_64/bin/soyuz-tui", &tui_elf(), b'0'),
            ])),
        )
        .unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&archive).unwrap()));

        // Running the terminal UI: it is replaced (and restarted), the GUI next to it too.
        let new = install_tarball_to(&archive, &sha, &tui, InstallKind::Tarball).unwrap();
        assert_eq!(new, std::fs::canonicalize(&tui).unwrap());
        assert_eq!(std::fs::read(&gui).unwrap(), elf());
        assert_eq!(std::fs::read(&tui).unwrap(), tui_elf());
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 2, "no temp file left");

        // An archive without soyuz-tui (before 0.3) still updates the GUI and leaves the
        // terminal UI alone; the terminal UI cannot update from it.
        std::fs::write(&gui, b"old gui").unwrap();
        let old = d.path().join("soyuz-0.2.0-linux-x86_64.tar.gz");
        std::fs::write(&old, gz(&tar(&[("soyuz-0.2.0-linux-x86_64/bin/soyuz", &elf(), b'0')]))).unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&old).unwrap()));
        assert!(install_tarball_to(&old, &sha, &tui, InstallKind::Tarball).is_err());
        assert_eq!(std::fs::read(&gui).unwrap(), b"old gui");
        install_tarball_to(&old, &sha, &gui, InstallKind::Tarball).unwrap();
        assert_eq!(std::fs::read(&gui).unwrap(), elf());
        assert_eq!(std::fs::read(&tui).unwrap(), tui_elf());
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 2, "no temp file left");

        // One bad binary: nothing is replaced.
        std::fs::write(&gui, b"old gui").unwrap();
        let bad = d.path().join("bad.tar.gz");
        std::fs::write(&bad, gz(&tar(&[("x/bin/soyuz", &elf(), b'0'), ("x/bin/soyuz-tui", b"#!/bin/sh\n", b'0')])))
            .unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&bad).unwrap()));
        assert!(install_tarball_to(&bad, &sha, &gui, InstallKind::Tarball).is_err());
        assert_eq!(std::fs::read(&gui).unwrap(), b"old gui");
        assert_eq!(std::fs::read(&tui).unwrap(), tui_elf());
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 2, "no temp file left");
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn tarball_install_updates_the_ffmpeg_libraries() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("soyuz");
        let bin = root.join("bin");
        let libs = root.join("lib").join("soyuz");
        let texts = root.join("share").join("licenses").join("soyuz").join("ffmpeg");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&libs).unwrap();
        let exe = bin.join("soyuz");
        std::fs::write(&exe, b"old build").unwrap();
        // The previous release's FFmpeg 8 libraries, plus files that are not ours to remove.
        for f in ["libavcodec.so.62", "libavutil.so.60", "libavcodec.so.63", "libother.so.1", "notes.txt"] {
            std::fs::write(libs.join(f), b"old").unwrap();
        }
        let lib = |tag: &[u8]| {
            let mut e = elf();
            e.extend_from_slice(tag);
            e
        };
        let (codec, util) = (lib(b"avcodec 63"), lib(b"avutil 61"));
        let archive = d.path().join("soyuz-0.4.0-linux-x86_64.tar.gz");
        std::fs::write(
            &archive,
            gz(&tar(&[
                ("soyuz-0.4.0-linux-x86_64/bin/soyuz", &elf(), b'0'),
                ("soyuz-0.4.0-linux-x86_64/lib/soyuz/libavcodec.so.63", &codec, b'0'),
                ("soyuz-0.4.0-linux-x86_64/lib/soyuz/libavutil.so.61", &util, b'0'),
                // A link, a nested file and a non-library are never installed.
                ("soyuz-0.4.0-linux-x86_64/lib/soyuz/libswscale.so.10", b"", b'2'),
                ("soyuz-0.4.0-linux-x86_64/lib/soyuz/sub/libx.so.1", &codec, b'0'),
                ("soyuz-0.4.0-linux-x86_64/lib/soyuz/README", b"x", b'0'),
                ("soyuz-0.4.0-linux-x86_64/share/licenses/soyuz/ffmpeg/LICENSE.md", b"lgpl", b'0'),
            ])),
        )
        .unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&archive).unwrap()));
        install_tarball_to(&archive, &sha, &exe, InstallKind::Tarball).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), elf());
        assert_eq!(
            names_in(&libs),
            ["libavcodec.so.63", "libavutil.so.61", "libother.so.1", "notes.txt"],
            "new libraries in, the old major out, foreign files kept, no temp file left"
        );
        assert_eq!(std::fs::read(libs.join("libavcodec.so.63")).unwrap(), codec);
        assert_eq!(std::fs::read(texts.join("LICENSE.md")).unwrap(), b"lgpl");
        assert_eq!(names_in(&bin), ["soyuz"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(libs.join("libavutil.so.61")).unwrap().permissions().mode() & 0o777, 0o644);
        }

        // A library that is not ELF for this CPU: nothing changes anywhere.
        std::fs::write(&exe, b"old build").unwrap();
        let bad = d.path().join("bad.tar.gz");
        std::fs::write(
            &bad,
            gz(&tar(&[("x/bin/soyuz", &elf(), b'0'), ("x/lib/soyuz/libavcodec.so.64", b"#!/bin/sh\n", b'0')])),
        )
        .unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&bad).unwrap()));
        let err = install_tarball_to(&bad, &sha, &exe, InstallKind::Tarball).unwrap_err();
        assert!(err.contains("libavcodec.so.64"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old build");
        assert_eq!(names_in(&libs), ["libavcodec.so.63", "libavutil.so.61", "libother.so.1", "notes.txt"]);

        // An archive without libraries (before Phase V) leaves the folder alone.
        let plain = d.path().join("soyuz-0.3.0-linux-x86_64.tar.gz");
        std::fs::write(&plain, gz(&tar(&[("x/bin/soyuz", &elf(), b'0')]))).unwrap();
        let sha = hex(&Sha256::digest(std::fs::read(&plain).unwrap()));
        install_tarball_to(&plain, &sha, &exe, InstallKind::Tarball).unwrap();
        assert_eq!(names_in(&libs), ["libavcodec.so.63", "libavutil.so.61", "libother.so.1", "notes.txt"]);

        // An executable outside a `bin` folder (a build folder) gets no side files.
        let dev = d.path().join("target");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("soyuz"), b"old").unwrap();
        install_tarball_to(
            &archive,
            &hex(&Sha256::digest(std::fs::read(&archive).unwrap())),
            &dev.join("soyuz"),
            InstallKind::Tarball,
        )
        .unwrap();
        assert_eq!(names_in(&dev), ["soyuz"]);
        assert!(!d.path().join("lib").exists());
    }

    #[test]
    fn side_file_names() {
        assert_eq!(side_entry("top/lib/soyuz/libavcodec.so.63"), Some((0, "libavcodec.so.63")));
        assert_eq!(side_entry("lib/soyuz/libavcodec.so.63"), Some((0, "libavcodec.so.63")));
        assert_eq!(side_entry("top/share/licenses/soyuz/ffmpeg/BUILD.txt"), Some((1, "BUILD.txt")));
        for none in [
            "top/lib/soyuz/sub/libx.so",
            "a/b/lib/soyuz/libavcodec.so.63",
            "top/lib/soyuz/README",
            "top/lib/soyuz/..",
            "top/share/licenses/soyuz/ffmpeg/evil.sh",
            "top/lib/other/libavcodec.so.63",
        ] {
            assert_eq!(side_entry(none), None, "{none}");
        }
        assert!(is_ffmpeg_lib("libavcodec.so.62") && is_ffmpeg_lib("libswresample.so.5.1.100"));
        for not in ["libavcodec.so", "libavcodec.so.", "libavcodec.so.6x", "libother.so.1", "libavcodecx.so.1"] {
            assert!(!is_ffmpeg_lib(not), "{not}");
        }
        assert_eq!(install_root(Path::new("/opt/soyuz/bin/soyuz")), Some(Path::new("/opt/soyuz")));
        assert_eq!(install_root(Path::new("/src/target/release/soyuz")), None);
    }

    #[test]
    fn temp_files_are_new_and_distinct() {
        let d = tempfile::tempdir().unwrap();
        let (_a, pa) = create_temp_in(d.path()).unwrap();
        let (_b, pb) = create_temp_in(d.path()).unwrap();
        assert_ne!(pa, pb);
        assert!(pa.file_name().unwrap().to_string_lossy().starts_with(".soyuz-update-"));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 2);
    }

    #[test]
    fn restart_request_round_trip() {
        let exec = || Restart::Exec(PathBuf::from("/x/soyuz"));
        assert_eq!(take_restart(), None);
        request_restart(exec());
        assert_eq!(take_restart(), None, "not before the app exits");
        arm_restart();
        assert_eq!(take_restart(), Some(exec()));
        assert_eq!(take_restart(), None);
        // A cancelled close forgets the restart: a later ordinary exit does not restart.
        request_restart(exec());
        assert_eq!(cancel_restart(), Some(exec()));
        assert_eq!(cancel_restart(), None);
        arm_restart();
        assert_eq!(take_restart(), None);
    }
}
