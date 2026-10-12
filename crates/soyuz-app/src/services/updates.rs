//! Update check against GitHub Releases.
//!
//! * **User-initiated only.** "Check for updates" in Settings → About, or (opt-in,
//!   `[updates] weekly_check`) one background check a week at startup that only shows a
//!   notification. No telemetry: the only request is the public Releases JSON for the
//!   configured repository (`[updates] repo`, empty = disabled).
//! * **Never installs silently.** A download needs a click; it lands in the Downloads
//!   folder, its SHA-256 is verified against the release's `SHA256SUMS*` file (and/or the
//!   asset digest GitHub reports), installers get an Authenticode check, and only then is
//!   the user offered to run the installer.
//! * **Linux.** Releases ship `soyuz-<ver>-linux-<arch>.tar.gz` (`scripts/package.sh`)
//!   with `SHA256SUMS-linux.txt` / `<file>.sha256`. There is no Authenticode, so the
//!   signature is reported as [`Signature::Unknown`]. A copy installed from that tarball
//!   (e.g. under `~/.local`) can replace itself: [`install_tarball`] re-verifies the
//!   download, extracts `bin/soyuz` next to the running executable and renames it over it
//!   atomically; [`request_restart`] + [`take_restart`] / [`exec_restart`] start the new
//!   build. Package-manager, Flatpak and AppImage copies are never overwritten
//!   ([`install_kind`], [`self_update_note`]).
//!
//! Network work runs on short-lived worker threads with their own tiny tokio runtime; the
//! UI reads a shared [`Phase`] snapshot and is woken with `request_repaint`.

use std::cmp::Ordering;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::host::Wake;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use soyuz_fs::integration::Signature;

/// Current app version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Largest Releases JSON we accept.
const MAX_JSON: usize = 4 << 20;
/// Largest checksum file we accept.
const MAX_SUMS: usize = 1 << 20;
/// Largest asset we download.
const MAX_ASSET: u64 = 1 << 30;
/// Interval of the opt-in background check.
const WEEK: Duration = Duration::from_secs(7 * 24 * 3600);
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

fn convert(r: RawRelease) -> Option<Release> {
    if r.draft {
        return None;
    }
    let version = Version::parse(&r.tag_name)?;
    let assets = r
        .assets
        .into_iter()
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
        .collect();
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

/// The asset to download for this OS/arch. Installed copies prefer the `.msi` installer,
/// then a regular `.zip`; portable copies (a `portable` marker next to the exe) prefer the
/// `-portable.zip`, then any `.zip`. Names follow `soyuz-<ver>-<os>-<arch>[-portable].<ext>`.
pub fn pick_asset<'a>(release: &'a Release, os: &str, arch: &str, portable: bool) -> Option<&'a Asset> {
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
        os_words.iter().any(|w| n.contains(w)) && (arch_words.iter().any(|w| n.contains(w)) || n.contains("universal"))
    };
    let is_portable = |a: &Asset| a.name.to_ascii_lowercase().contains("portable");
    // (extension, wants the portable flavour)
    let order: &[(&str, Option<bool>)] = match (os, portable) {
        ("windows", false) => &[(".msi", None), (".zip", Some(false)), (".zip", None)],
        ("windows", true) => &[(".zip", Some(true)), (".zip", None)],
        ("linux", _) => &[(".tar.gz", None), (".tgz", None), (".zip", None)],
        (_, true) => &[(".zip", Some(true)), (".zip", None), (".tar.gz", None)],
        _ => &[(".zip", Some(false)), (".zip", None), (".tar.gz", None), (".dmg", None)],
    };
    order.iter().find_map(|(ext, flavour)| {
        release.assets.iter().find(|a| {
            fits(a) && a.name.to_ascii_lowercase().ends_with(ext) && flavour.is_none_or(|p| is_portable(a) == p)
        })
    })
}

static PORTABLE: AtomicBool = AtomicBool::new(false);

/// Whether this copy runs in portable mode (computed by the check worker, not the UI thread).
pub fn running_portable() -> bool {
    PORTABLE.load(AtomicOrdering::Relaxed)
}

fn detect_portable() {
    let p = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .is_some_and(|d| soyuz_core::config::portable_dir(&d).is_some());
    PORTABLE.store(p, AtomicOrdering::Relaxed);
}

/// [`pick_asset`] for the running copy.
pub fn asset_for_this_copy(release: &Release) -> Option<&Asset> {
    pick_asset(release, std::env::consts::OS, std::env::consts::ARCH, running_portable())
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

/// What the update panel shows.
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
    Downloaded {
        release: Arc<Release>,
        path: PathBuf,
        sha256: String,
        signature: Signature,
        /// A detached signature file was published but is not verified (no key configured).
        detached_sig: bool,
    },
    Failed(String),
}

#[derive(Default)]
struct Shared {
    phase: Phase,
    /// Set by the weekly check when it found an update the user has not been told about.
    pending_notice: Option<String>,
    started_weekly: bool,
    cancel: Option<Arc<AtomicBool>>,
}

static STATE: Mutex<Shared> =
    Mutex::new(Shared { phase: Phase::Idle, pending_notice: None, started_weekly: false, cancel: None });

/// Current phase (cheap clone).
pub fn phase() -> Phase {
    STATE.lock().phase.clone()
}

fn set(phase: Phase, wake: &Wake) {
    STATE.lock().phase = phase;
    wake();
}

fn busy() -> bool {
    matches!(STATE.lock().phase, Phase::Checking | Phase::Downloading { .. })
}

/// Forget the result (e.g. after the repo setting changed).
pub fn reset() {
    let mut s = STATE.lock();
    if !matches!(s.phase, Phase::Checking | Phase::Downloading { .. }) {
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

/// Start a manual check in the background ("Check for updates").
pub fn check(repo: String, prereleases: bool, wake: Wake) {
    if busy() {
        return;
    }
    set(Phase::Checking, &wake);
    spawn("soyuz-update-check", move || {
        detect_portable();
        let res = runtime().map_err(|e| e.to_string()).and_then(|rt| rt.block_on(fetch_latest(&repo, prereleases)));
        write_stamp();
        let phase = match res {
            Ok(r) if is_newer(&r.version, CURRENT) => Phase::Available(Arc::new(r)),
            Ok(r) => Phase::UpToDate { latest: r.version.to_string() },
            Err(e) => Phase::Failed(e),
        };
        set(phase, &wake);
    });
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        log::error!("cannot spawn {name}: {e}");
        STATE.lock().phase = Phase::Failed(tr!("services-update-cannot-start", error = e.to_string()));
    }
}

/// Download `asset` of `release` into `dir`, verify it, and report [`Phase::Downloaded`].
pub fn download(release: Arc<Release>, dir: PathBuf, wake: Wake) {
    if busy() {
        return;
    }
    let Some(asset) = asset_for_this_copy(&release).cloned() else {
        set(Phase::Failed(tr!("services-update-no-asset").into()), &wake);
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = STATE.lock();
        s.cancel = Some(cancel.clone());
        s.phase = Phase::Downloading { release: release.clone(), asset: asset.name.clone(), got: 0, total: asset.size };
    }
    wake();
    spawn("soyuz-update-download", move || {
        let res = runtime()
            .map_err(|e| e.to_string())
            .and_then(|rt| rt.block_on(download_verified(&release, &asset, &dir, &cancel, &wake)));
        STATE.lock().cancel = None;
        let phase = match res {
            Ok((path, sha256)) => {
                let lower = path.to_string_lossy().to_ascii_lowercase();
                // Authenticode exists on Windows only: other platforms report `Unknown`
                // honestly instead of "unsigned".
                let signature = if cfg!(windows) && (lower.ends_with(".msi") || lower.ends_with(".exe")) {
                    soyuz_fs::integration::authenticode(&path)
                } else {
                    Signature::Unknown
                };
                if let Signature::Invalid(hr) = signature {
                    let _ = std::fs::remove_file(&path);
                    Phase::Failed(tr!("services-update-bad-signature", code = format!("0x{hr:08X}")))
                } else {
                    let detached_sig = signature_asset(&release, &asset.name).is_some();
                    Phase::Downloaded { release, path, sha256, signature, detached_sig }
                }
            }
            Err(e) => Phase::Failed(e),
        };
        set(phase, &wake);
    });
}

async fn download_verified(
    release: &Release,
    asset: &Asset,
    dir: &Path,
    cancel: &AtomicBool,
    wake: &Wake,
) -> Result<(PathBuf, String), String> {
    let client = client()?;
    // Expected hash: every source that exists must agree.
    let mut expected: Vec<(String, String)> = Vec::new();
    if let Some(h) = &asset.sha256 {
        expected.push(("GitHub asset digest".into(), h.clone()));
    }
    for sums in checksum_assets(release, &asset.name) {
        let text = get_text(&client, &sums.url, MAX_SUMS).await?;
        if let Some(h) = parse_sums(&text, &asset.name) {
            expected.push((sums.name.clone(), h));
            break;
        }
    }
    if expected.is_empty() {
        return Err(tr!("services-update-no-checksum").into());
    }
    if expected.windows(2).any(|w| w[0].1 != w[1].1) {
        return Err(tr!("services-update-checksums-disagree").into());
    }
    let want = expected[0].1.clone();

    let name = safe_file_name(&asset.name).ok_or(tr!("services-update-bad-asset-name"))?;
    std::fs::create_dir_all(dir)
        .map_err(|e| tr!("services-update-cannot-create", path = dir.display().to_string(), error = e.to_string()))?;
    let part = dir.join(format!("{name}.part"));
    let mut file = std::fs::File::create(&part).map_err(|e| {
        tr!("services-update-cannot-write-file", path = part.display().to_string(), error = e.to_string())
    })?;
    let mut resp =
        client.get(&asset.url).send().await.map_err(|e| tr!("services-update-network", error = e.to_string()))?;
    let cleanup = |e: String| {
        let _ = std::fs::remove_file(&part);
        e
    };
    if !resp.status().is_success() {
        return Err(cleanup(tr!("services-update-download-failed", status = resp.status().to_string())));
    }
    let total = resp.content_length().unwrap_or(asset.size);
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    let mut last_report = std::time::Instant::now();
    loop {
        if cancel.load(AtomicOrdering::SeqCst) {
            drop(file);
            return Err(cleanup(tr!("services-update-cancelled").into()));
        }
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                drop(file);
                return Err(cleanup(tr!("services-update-network", error = e.to_string())));
            }
        };
        got += chunk.len() as u64;
        if got > MAX_ASSET {
            drop(file);
            return Err(cleanup(tr!("services-update-too-large").into()));
        }
        hasher.update(&chunk);
        if let Err(e) = file.write_all(&chunk) {
            drop(file);
            return Err(cleanup(tr!("services-update-cannot-write", error = e.to_string())));
        }
        if last_report.elapsed() > Duration::from_millis(100) {
            last_report = std::time::Instant::now();
            if let Phase::Downloading { got: g, total: t, .. } = &mut STATE.lock().phase {
                *g = got;
                *t = total;
            }
            wake();
        }
    }
    if let Err(e) = file.sync_all() {
        drop(file);
        return Err(cleanup(tr!("services-update-cannot-write", error = e.to_string())));
    }
    drop(file);
    let have = hex(&hasher.finalize());
    if have != want {
        return Err(cleanup(tr!("services-update-checksum-mismatch", expected = &want[..12], got = &have[..12])));
    }
    let target = unique_path(dir, &name);
    std::fs::rename(&part, &target)
        .map_err(|e| cleanup(tr!("services-update-cannot-rename", error = e.to_string())))?;
    Ok((target, have))
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

// ---- Linux self-update -----------------------------------------------------------------

/// How the running copy was installed (decides whether it may replace itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// Windows / macOS: installers are run by the user ([`run_installer`]).
    Installer,
    /// Linux, unpacked from the release tarball (`~/.local`, `/usr/local`, `/opt`, a
    /// build folder): [`install_tarball`] can update it in place.
    Tarball,
    /// Linux, under `/usr` (rpm / deb / distribution package).
    Package,
    Flatpak,
    AppImage,
}

impl InstallKind {
    /// `$kind` selector of `services-update-package-managed`.
    pub fn key(self) -> &'static str {
        match self {
            InstallKind::Installer | InstallKind::Tarball => "other",
            InstallKind::Package => "system",
            InstallKind::Flatpak => "flatpak",
            InstallKind::AppImage => "appimage",
        }
    }
}

/// [`InstallKind`] of an executable at `exe` on `os`, with environment lookup `env`.
pub fn install_kind_for(os: &str, exe: &Path, env: impl Fn(&str) -> Option<std::ffi::OsString>) -> InstallKind {
    if os != "linux" {
        return InstallKind::Installer;
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    if set("FLATPAK_ID") || exe.starts_with("/app") {
        InstallKind::Flatpak
    } else if set("APPIMAGE") || (set("APPDIR") && env("APPDIR").is_some_and(|d| exe.starts_with(d))) {
        InstallKind::AppImage
    } else if exe.starts_with("/usr") && !exe.starts_with("/usr/local") {
        InstallKind::Package
    } else {
        InstallKind::Tarball
    }
}

/// [`InstallKind`] of the running copy.
pub fn install_kind() -> InstallKind {
    let exe = std::env::current_exe().unwrap_or_default();
    install_kind_for(std::env::consts::OS, &exe, |k| std::env::var_os(k))
}

/// Linux copies that must be updated elsewhere: what to tell the user (`None` = this
/// copy can update itself, or the platform uses installers).
pub fn self_update_note() -> Option<String> {
    match install_kind() {
        InstallKind::Installer | InstallKind::Tarball => None,
        k => Some(tr!("services-update-package-managed", kind = k.key())),
    }
}

/// Largest `bin/soyuz` accepted from a tarball.
const MAX_BINARY: u64 = 512 << 20;

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

/// Copy the `bin/soyuz` entry of a release tarball (`<top>/bin/soyuz`, or `bin/soyuz`)
/// from the uncompressed tar stream `r` into `out`. Only that regular file is read; the
/// output path is ours, so entry names can never escape anywhere.
fn extract_binary(mut r: impl std::io::Read, out: &mut impl std::io::Write) -> Result<u64, String> {
    use std::io::Read as _;
    let mut header = [0u8; 512];
    let mut long_name: Option<String> = None;
    loop {
        if r.read_exact(&mut header).is_err() || header.iter().all(|b| *b == 0) {
            return Err(tr!("services-update-install-no-binary").into());
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
                let rel = name.trim_start_matches("./");
                let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
                let wanted = matches!(parts.as_slice(), ["bin", "soyuz"] | [_, "bin", "soyuz"]);
                if wanted {
                    if size > MAX_BINARY {
                        return Err(tr!("services-update-install-bad-archive").into());
                    }
                    let mut data = body.take(size);
                    let n = std::io::copy(&mut data, out).map_err(|e| e.to_string())?;
                    if n != size {
                        return Err(tr!("services-update-install-bad-archive").into());
                    }
                    return Ok(n);
                }
            }
            _ => {}
        }
        std::io::copy(&mut body, &mut std::io::sink()).map_err(|e| e.to_string())?;
    }
}

/// Replace the running executable `exe` with `bin/soyuz` from the downloaded release
/// tarball `archive` (blocking: call it off the UI thread). The archive is hashed again
/// first (it sat in the Downloads folder) and must match `sha256`. The new binary is
/// written to a temporary file next to `exe`, checked to be an ELF executable for this
/// CPU, made executable, synced and renamed over `exe` (atomic; the running process
/// keeps its old inode). Returns the path to restart into.
pub fn install_tarball_to(archive: &Path, sha256: &str, exe: &Path, kind: InstallKind) -> Result<PathBuf, String> {
    if kind != InstallKind::Tarball {
        return Err(tr!("services-update-package-managed", kind = kind.key()));
    }
    let lower = archive.to_string_lossy().to_ascii_lowercase();
    if !(lower.ends_with(".tar.gz") || lower.ends_with(".tgz")) {
        return Err(tr!("services-update-install-not-tarball").into());
    }
    let fail = |e: &dyn std::fmt::Display| tr!("services-update-install-failed", error = e.to_string());
    // One open of the archive: the bytes extracted are the bytes hashed (the file in
    // ~/Downloads cannot be swapped between a check and the extraction).
    let archive_file = std::fs::File::open(archive).map_err(|e| fail(&e))?;
    let exe = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let dir = exe.parent().ok_or_else(|| fail(&"no parent folder"))?;
    let (mut out, tmp) = create_temp_in(dir).map_err(|e| {
        tr!("services-update-install-no-permission", path = dir.display().to_string(), error = e.to_string())
    })?;
    let result = (|| -> Result<(), String> {
        use std::io::{Read as _, Seek as _};
        // 1. Extract bin/soyuz into the temp file next to the executable (same
        //    filesystem → atomic rename), hashing the archive on the way through.
        let hashing = HashingReader { inner: archive_file, hasher: Sha256::new() };
        let mut gz = flate2::read::GzDecoder::new(std::io::BufReader::new(hashing));
        let extracted = extract_binary(&mut gz, &mut out);
        // 2. Hash the rest of the archive (everything below the decoder was hashed as
        //    it was read, buffered bytes included) and compare before anything else.
        let mut hashing = gz.into_inner().into_inner();
        std::io::copy(&mut hashing, &mut std::io::sink()).map_err(|e| fail(&e))?;
        if !hex(&hashing.hasher.finalize()).eq_ignore_ascii_case(sha256) {
            return Err(tr!("services-update-install-changed").into());
        }
        extracted?;
        out.sync_all().map_err(|e| fail(&e))?;
        let mut head = [0u8; 64];
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
        // 3. Only a verified binary replaces the executable.
        std::fs::rename(&tmp, &exe).map_err(|e| fail(&e))
    })();
    drop(out);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(exe)
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

/// [`install_tarball_to`] for the running executable.
pub fn install_tarball(archive: &Path, sha256: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| tr!("services-update-install-failed", error = e.to_string()))?;
    // `/proc/self/exe` of a replaced binary reads `… (deleted)`.
    let exe = exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")).map(PathBuf::from).unwrap_or(exe);
    install_tarball_to(archive, sha256, &exe, install_kind())
}

/// Result of the last background install started with [`install_async`]
/// (`None` = none finished yet).
static INSTALLED: Mutex<Option<Result<PathBuf, String>>> = Mutex::new(None);
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Run [`install_tarball`] on a worker thread; poll [`install_status`].
pub fn install_async(archive: PathBuf, sha256: String, wake: Wake) {
    if INSTALLING.swap(true, AtomicOrdering::SeqCst) {
        return;
    }
    *INSTALLED.lock() = None;
    let spawned = std::thread::Builder::new().name("soyuz-update-install".into()).spawn(move || {
        let r = install_tarball(&archive, &sha256);
        *INSTALLED.lock() = Some(r);
        INSTALLING.store(false, AtomicOrdering::SeqCst);
        wake();
    });
    if let Err(e) = spawned {
        INSTALLING.store(false, AtomicOrdering::SeqCst);
        *INSTALLED.lock() = Some(Err(tr!("services-update-cannot-start", error = e.to_string())));
    }
}

/// `(installing, last result)` of [`install_async`].
pub fn install_status() -> (bool, Option<Result<PathBuf, String>>) {
    (INSTALLING.load(AtomicOrdering::SeqCst), INSTALLED.lock().clone())
}

/// A restart asked for by the close in progress (dropped if that close is cancelled).
static RESTART: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The restart confirmed when the app actually exited ([`arm_restart`]).
static RESTART_ARMED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Restart into `exe` if the window close that the caller now starts (by sending
/// `ViewportCommand::Close`) goes through: the app calls [`arm_restart`] on exit and
/// [`cancel_restart`] when it cancels the close; `main` calls [`take_restart`] after
/// the event loop ends.
pub fn request_restart(exe: PathBuf) {
    *RESTART.lock() = Some(exe);
}

/// The close was cancelled (pending jobs, unsaved documents): forget the restart, so a
/// later, ordinary quit does not start the new binary. Returns whether one was pending.
pub fn cancel_restart() -> bool {
    RESTART.lock().take().is_some()
}

/// The app is exiting: a requested restart now happens.
pub fn arm_restart() {
    if let Some(exe) = RESTART.lock().take() {
        *RESTART_ARMED.lock() = Some(exe);
    }
}

/// The executable to restart into, if a restart was requested and the app exited.
pub fn take_restart() -> Option<PathBuf> {
    RESTART_ARMED.lock().take()
}

/// Start `exe` with this process's arguments: on unix the process image is replaced
/// (`execv`, same PID), elsewhere a new process is spawned. Returns only on failure.
pub fn exec_restart(exe: &Path) -> std::io::Error {
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

/// Start the downloaded installer (`msiexec /i`) — only ever called from an explicit click.
pub fn run_installer(path: &Path) -> std::io::Result<()> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".msi") {
        let msiexec = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join("System32").join("msiexec.exe"))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("msiexec.exe"));
        std::process::Command::new(msiexec).arg("/i").arg(path).spawn().map(drop)
    } else if lower.ends_with(".exe") {
        std::process::Command::new(path).spawn().map(drop)
    } else {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "not an installer"))
    }
}

// ---- weekly check --------------------------------------------------------------------

fn stamp_path() -> PathBuf {
    soyuz_core::config::config_dir().join(STAMP_FILE)
}

fn read_stamp() -> Option<SystemTime> {
    let text = std::fs::read_to_string(stamp_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(v.get("last_check")?.as_u64()?))
}

fn write_stamp() {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = stamp_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, format!("{{\"last_check\": {secs}}}\n")) {
        log::debug!("update stamp {}: {e}", path.display());
    }
}

/// Whether a weekly check is due given the last check time.
pub fn weekly_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|t| now.duration_since(t).is_ok_and(|d| d >= WEEK))
}

/// Per-frame hook: starts the opt-in weekly check once per session and turns its result
/// into a notification.
pub fn pump(app: &mut crate::core::AppCore) {
    let notice = {
        let mut s = STATE.lock();
        if !s.started_weekly {
            s.started_weekly = true;
            let u = &app.config.updates;
            if u.weekly_check && !u.repo.trim().is_empty() {
                let (repo, pre, wake) = (u.repo.clone(), u.include_prereleases, app.wake.clone());
                drop(s);
                spawn("soyuz-update-weekly", move || {
                    detect_portable();
                    if !weekly_due(read_stamp(), SystemTime::now()) || busy() {
                        return;
                    }
                    STATE.lock().phase = Phase::Checking;
                    let res = runtime().map_err(|e| e.to_string()).and_then(|rt| rt.block_on(fetch_latest(&repo, pre)));
                    write_stamp();
                    let mut s = STATE.lock();
                    s.phase = match res {
                        Ok(r) if is_newer(&r.version, CURRENT) => {
                            s.pending_notice = Some(r.version.to_string());
                            Phase::Available(Arc::new(r))
                        }
                        Ok(r) => Phase::UpToDate { latest: r.version.to_string() },
                        Err(e) => {
                            log::info!("weekly update check: {e}");
                            Phase::Idle
                        }
                    };
                    drop(s);
                    wake();
                });
                return;
            }
        }
        s.pending_notice.take()
    };
    if let Some(v) = notice {
        app.ui.toasts.push(
            crate::models::toasts::Toast::info(tr!("services-update-available", version = v.to_string()))
                .body(tr!("services-update-available-body")),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn file_names_and_weekly_schedule() {
        assert_eq!(safe_file_name("soyuz-1.0.0.msi").as_deref(), Some("soyuz-1.0.0.msi"));
        assert_eq!(safe_file_name(r"..\..\evil.msi").as_deref(), Some("....evil.msi"));
        assert_eq!(safe_file_name(".."), None);
        assert_eq!(safe_file_name("a/b:c"), Some("abc".into()));

        let now = SystemTime::now();
        assert!(weekly_due(None, now));
        assert!(!weekly_due(Some(now - Duration::from_secs(3600)), now));
        assert!(weekly_due(Some(now - WEEK), now));
        assert!(!weekly_due(Some(now + Duration::from_secs(60)), now), "clock skew: not due");

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
        // The older x64 spelling still matches.
        let mut old = parse_releases(FIXTURE, false).unwrap();
        old.assets.push(Asset {
            name: "soyuz-0.1.0-linux-x64.tar.gz".into(),
            url: "https://x/y".into(),
            size: 1,
            sha256: None,
        });
        assert_eq!(pick_asset(&old, "linux", "x86_64", false).unwrap().name, "soyuz-0.1.0-linux-x64.tar.gz");
        let sums: Vec<&str> = checksum_assets(&r, &a.name).iter().map(|s| s.name.as_str()).collect();
        assert_eq!(sums[0], "soyuz-0.2.0-linux-x86_64.tar.gz.sha256", "own .sha256 first");
        assert!(sums.contains(&"SHA256SUMS-linux.txt"));
    }

    #[test]
    fn install_kinds() {
        let none = |_: &str| None;
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
        assert_eq!(install_kind_for("windows", Path::new("C:/x/soyuz.exe"), none), InstallKind::Installer);
    }

    /// A minimal ustar archive with `entries` (name, data, type flag).
    fn tar(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
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
        let mut out = Vec::new();
        assert_eq!(extract_binary(&t[..], &mut out).unwrap(), bin.len() as u64);
        assert_eq!(out, bin);
        // Nested deeper, a symlink, or missing: not taken.
        let t = tar(&[("a/b/bin/soyuz", b"x", b'0'), ("top/bin/soyuz", b"", b'2')]);
        assert!(extract_binary(&t[..], &mut Vec::new()).is_err());
        assert!(extract_binary(&b"short"[..], &mut Vec::new()).is_err());
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
        assert_eq!(take_restart(), None);
        request_restart(PathBuf::from("/x/soyuz"));
        assert_eq!(take_restart(), None, "not before the app exits");
        arm_restart();
        assert_eq!(take_restart(), Some(PathBuf::from("/x/soyuz")));
        assert_eq!(take_restart(), None);
        // A cancelled close forgets the restart: a later ordinary exit does not restart.
        request_restart(PathBuf::from("/x/soyuz"));
        assert!(cancel_restart());
        assert!(!cancel_restart());
        arm_restart();
        assert_eq!(take_restart(), None);
    }
}
