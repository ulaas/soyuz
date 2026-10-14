//! The FFmpeg runtime of this process (Phase V1c): where the `soyuz-media` libraries are
//! loaded from, whether they are loaded, and the crash sentinel for files decoded in
//! process.
//!
//! [`configure`] runs with every config change (`AppCore::apply_core_settings`): it
//! remembers `[media] ffmpeg` / `ffmpeg_dir`, and when the libraries are not loaded yet it
//! starts one worker thread (`soyuz-media-load`) that calls `soyuz_media::Runtime::get`
//! over [`soyuz_media::search_dirs`]. A failed attempt is retried only when the folder
//! list changes or [`retry`] asks (the Settings page's button); a successful load is final
//! for the process (the libraries are never unloaded), so turning `[media] ffmpeg` off
//! only hides it: [`runtime`] returns `None` until it is on again. Nothing here blocks the
//! caller: [`runtime`] and [`status`] read memoised state.
//!
//! **Crash sentinel** ([`Sentinel`]): a malformed file can crash the FFmpeg demuxers and
//! decoders, and with in-process decoding that takes the whole app with it. Every
//! automatic decode (thumbnails, the preview pane's poster and metadata) registers the
//! file in `<config dir>/media-inflight` first and removes it when done. A path still
//! listed at the next start is one whose decode crashed the process: it is skipped by the
//! automatic previews (shown as "not previewed, it crashed the media library last time")
//! until the user opens it explicitly (Quick Look), which forgives it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use soyuz_media::{LoadError, Runtime, Versions};

// The frontends reach the few `soyuz-media` types they show through here (no direct
// dependency on the crate).
pub use soyuz_media::{Majors, MediaInfo, lib_file_names};

/// The FFmpeg line Soyuz ships (`third_party/ffmpeg/pins.env`): the runtime assets live on
/// the release tag `ffmpeg-runtime-<version>` ([`runtime_release_tag`]).
pub const BUNDLED_VERSION: &str = "9.0.2";

/// The GitHub release tag that carries the library archives for every platform.
pub fn runtime_release_tag() -> String {
    format!("ffmpeg-runtime-{BUNDLED_VERSION}")
}

/// Where the libraries stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// `[media] ffmpeg = false`.
    Disabled,
    /// Not tried yet, or the worker thread is loading.
    Loading,
    /// Loaded; final for the process.
    Loaded(Arc<LoadedInfo>),
    /// The last attempt failed (retried when the folders change or [`retry`] asks).
    Failed { searched: Vec<PathBuf>, reason: String },
}

/// What a loaded runtime reports (About, Settings).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedInfo {
    pub versions: Versions,
    /// The folder the libraries were loaded from.
    pub dir: PathBuf,
    /// The five library files, in load order.
    pub files: Vec<PathBuf>,
    pub decoders: usize,
    pub encoders: usize,
}

impl LoadedInfo {
    /// `"9.0 (avcodec 63.2.100)"`, with the GPL label when the build is not ours.
    pub fn version_line(&self) -> String {
        let majors = self.versions.majors();
        let line = majors.release().unwrap_or("?");
        let mut s = format!("{line} (avcodec {})", Versions::text(self.versions.avcodec));
        if self.versions.gpl {
            s.push_str(" · GPL build");
        }
        s
    }
}

/// [`State`] plus the folders the next attempt would search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// `search_dirs` for the current config (empty entry = the loader's own path).
    pub dirs: Vec<PathBuf>,
}

struct Cfg {
    enabled: bool,
    ffmpeg_dir: String,
    config_dir: PathBuf,
    /// The folder list of the attempt in progress / last failed (no retry on the same).
    tried: Option<Vec<PathBuf>>,
}

static ENABLED: AtomicBool = AtomicBool::new(true);
static CFG: Mutex<Option<Cfg>> = Mutex::new(None);
static STATE: Mutex<Option<State>> = Mutex::new(None);
/// Bumped per attempt so a late failure never overwrites a newer attempt's state.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// `[media] ffmpeg` and `ffmpeg_dir` (every config change). Starts the load on a worker
/// thread when needed; never blocks.
pub fn configure(media: &soyuz_core::config::Media) {
    configure_in(media.ffmpeg, &media.ffmpeg_dir, soyuz_core::config::config_dir());
}

/// [`configure`] with an explicit config folder (tests).
pub fn configure_in(enabled: bool, ffmpeg_dir: &str, config_dir: PathBuf) {
    ENABLED.store(enabled, Ordering::Relaxed);
    let mut cfg = CFG.lock();
    let changed_dir = cfg.as_ref().is_none_or(|c| c.ffmpeg_dir != ffmpeg_dir || c.config_dir != config_dir);
    let tried = if changed_dir { None } else { cfg.as_ref().and_then(|c| c.tried.clone()) };
    *cfg = Some(Cfg { enabled, ffmpeg_dir: ffmpeg_dir.to_string(), config_dir, tried });
    if !enabled {
        *STATE.lock() = Some(State::Disabled);
        return;
    }
    if let Some(rt) = Runtime::loaded() {
        *STATE.lock() = Some(State::Loaded(Arc::new(describe(&rt))));
        return;
    }
    start_load(cfg.as_mut().expect("just set"), false);
}

/// Try again now (the Settings page's button), even with unchanged folders.
pub fn retry() {
    let mut cfg = CFG.lock();
    let Some(c) = cfg.as_mut() else { return };
    if !c.enabled || Runtime::loaded().is_some() {
        return;
    }
    start_load(c, true);
}

fn start_load(c: &mut Cfg, force: bool) {
    let dirs = soyuz_media::search_dirs(&c.ffmpeg_dir, &c.config_dir);
    if !force && c.tried.as_ref() == Some(&dirs) {
        return;
    }
    c.tried = Some(dirs.clone());
    *STATE.lock() = Some(State::Loading);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let searched_dirs = dirs.clone();
    let spawned = std::thread::Builder::new().name("soyuz-media-load".into()).spawn(move || {
        let result = Runtime::get(&dirs);
        let mut state = STATE.lock();
        match result {
            Ok(rt) => {
                log::info!("FFmpeg libraries loaded from {}", rt.dir().display());
                *state = Some(State::Loaded(Arc::new(describe(&rt))));
            }
            Err(e) => {
                if GENERATION.load(Ordering::Relaxed) != generation {
                    return;
                }
                let reason = e.to_string();
                match &e {
                    LoadError::Missing { .. } => log::info!("{reason}"),
                    _ => log::warn!("{reason}"),
                }
                let searched = match e {
                    LoadError::Missing { searched } => searched,
                    _ => dirs,
                };
                *state = Some(State::Failed { searched, reason });
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("cannot start the FFmpeg loader thread: {e}");
        *STATE.lock() = Some(State::Failed { searched: searched_dirs, reason: e.to_string() });
    }
}

fn describe(rt: &Runtime) -> LoadedInfo {
    let versions = rt.versions().clone();
    let names = soyuz_media::lib_file_names(versions.majors());
    let files = names.iter().map(|n| rt.dir().join(n)).collect();
    let codecs = rt.codecs();
    LoadedInfo {
        versions,
        dir: rt.dir().to_path_buf(),
        files,
        decoders: codecs.iter().filter(|c| c.decoder).count(),
        encoders: codecs.iter().filter(|c| c.encoder).count(),
    }
}

/// The loaded libraries, when `[media] ffmpeg` is on and the load succeeded. Never loads
/// and never blocks (a memoised lookup, safe on the UI thread and the workers).
pub fn runtime() -> Option<Arc<Runtime>> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    Runtime::loaded()
}

/// A short tag of the decoder that made a picture, for cache keys (`"ffmpeg-63"`), empty
/// without a runtime. Posters made by one library build are never served for another.
pub fn source_tag() -> String {
    runtime().map(|rt| format!("ffmpeg-{}", rt.versions().majors().avcodec)).unwrap_or_default()
}

/// Where the libraries stand, for Settings and About.
pub fn status() -> Status {
    let cfg = CFG.lock();
    let dirs = match cfg.as_ref() {
        Some(c) => soyuz_media::search_dirs(&c.ffmpeg_dir, &c.config_dir),
        None => Vec::new(),
    };
    let state = match (cfg.as_ref(), Runtime::loaded()) {
        (Some(c), _) if !c.enabled => State::Disabled,
        (_, Some(rt)) => State::Loaded(Arc::new(describe(&rt))),
        _ => STATE.lock().clone().unwrap_or(State::Loading),
    };
    Status { state, dirs }
}

/// The About dialog's "FFmpeg" line (English, for bug reports).
pub fn about_line() -> String {
    match status().state {
        State::Disabled => "off ([media] ffmpeg = false)".into(),
        State::Loading => "loading".into(),
        State::Loaded(i) => format!("{} · {}", i.version_line(), soyuz_fs::strip_verbatim(&i.dir).display()),
        State::Failed { reason, .. } => format!("not loaded ({reason})"),
    }
}

// ---- Crash sentinel ---------------------------------------------------------------------

/// Name of the sentinel file in the config folder.
pub const SENTINEL_FILE: &str = "media-inflight";

/// Files being decoded in process right now, and the ones whose decode crashed the last
/// run (see the module docs). One per process ([`sentinel`]); tests make their own.
pub struct Sentinel {
    file: PathBuf,
    inflight: Mutex<Vec<PathBuf>>,
    crashed: Mutex<HashSet<PathBuf>>,
}

/// Keeps a path in the sentinel file until dropped.
pub struct Inflight<'a> {
    sentinel: &'a Sentinel,
    path: PathBuf,
}

impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        let mut list = self.sentinel.inflight.lock();
        if let Some(i) = list.iter().position(|p| *p == self.path) {
            list.remove(i);
        }
        self.sentinel.write(&list);
    }
}

impl Sentinel {
    /// Open the sentinel in `dir`: paths left behind by the previous run become the
    /// crashed set, and the file is removed.
    pub fn open(dir: &Path) -> Sentinel {
        let file = dir.join(SENTINEL_FILE);
        let crashed: HashSet<PathBuf> = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect();
        if !crashed.is_empty() {
            log::warn!(
                "the last run crashed while decoding {} file(s); they are skipped by automatic previews: {}",
                crashed.len(),
                crashed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            );
            let _ = std::fs::remove_file(&file);
        }
        Sentinel { file, inflight: Mutex::new(Vec::new()), crashed: Mutex::new(crashed) }
    }

    /// Register `path` as being decoded; `None` when its last decode crashed the process
    /// (the caller skips it). Drop the guard when done.
    pub fn begin(&self, path: &Path) -> Option<Inflight<'_>> {
        if self.crashed(path) {
            return None;
        }
        let mut list = self.inflight.lock();
        list.push(path.to_path_buf());
        self.write(&list);
        Some(Inflight { sentinel: self, path: path.to_path_buf() })
    }

    /// The last decode of `path` crashed the process (and nobody forgave it since).
    pub fn crashed(&self, path: &Path) -> bool {
        self.crashed.lock().contains(path)
    }

    /// The user opened `path` on purpose: decode it again next time.
    pub fn forgive(&self, path: &Path) {
        self.crashed.lock().remove(path);
    }

    /// Paths currently registered (tests).
    pub fn inflight(&self) -> Vec<PathBuf> {
        self.inflight.lock().clone()
    }

    fn write(&self, list: &[PathBuf]) {
        if list.is_empty() {
            let _ = std::fs::remove_file(&self.file);
            return;
        }
        let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
        if let Err(e) = std::fs::write(&self.file, text) {
            log::debug!("cannot write {}: {e}", self.file.display());
        }
    }
}

static SENTINEL: OnceLock<Sentinel> = OnceLock::new();

/// The process's sentinel (opened in the config folder on first use; call it from worker
/// threads, it reads a file the first time).
pub fn sentinel() -> &'static Sentinel {
    SENTINEL.get_or_init(|| Sentinel::open(&soyuz_core::config::config_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loader state is process-wide: tests that configure it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn sentinel_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sentinel::open(dir.path());
        let a = dir.path().join("a.mp4");
        let b = dir.path().join("b.mkv");
        assert!(!s.crashed(&a));
        {
            let _ga = s.begin(&a).expect("not crashed");
            let _gb = s.begin(&b).expect("not crashed");
            let text = std::fs::read_to_string(dir.path().join(SENTINEL_FILE)).unwrap();
            assert!(text.contains("a.mp4") && text.contains("b.mkv"), "{text}");
            assert_eq!(s.inflight().len(), 2);
            drop(_ga);
            let text = std::fs::read_to_string(dir.path().join(SENTINEL_FILE)).unwrap();
            assert!(!text.contains("a.mp4") && text.contains("b.mkv"), "{text}");
        }
        assert!(!dir.path().join(SENTINEL_FILE).exists(), "removed once nothing is in flight");
        assert!(s.inflight().is_empty());
    }

    #[test]
    fn a_leftover_sentinel_blacklists_until_forgiven() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.mp4");
        // The previous run died with the file registered.
        std::fs::write(dir.path().join(SENTINEL_FILE), format!("{}\n\n", bad.display())).unwrap();
        let s = Sentinel::open(dir.path());
        assert!(!dir.path().join(SENTINEL_FILE).exists(), "the leftover file is consumed");
        assert!(s.crashed(&bad));
        assert!(s.begin(&bad).is_none(), "automatic decodes skip it");
        assert!(s.begin(&dir.path().join("fine.mp4")).is_some());
        s.forgive(&bad);
        assert!(!s.crashed(&bad));
        let g = s.begin(&bad);
        assert!(g.is_some(), "an explicit open decodes it again");
        drop(g);
        // A fresh open with nothing left behind: nothing is blacklisted.
        assert!(!Sentinel::open(dir.path()).crashed(&bad));
    }

    #[test]
    fn status_without_libraries_reports_the_searched_folders() {
        let _serial = SERIAL.lock();
        let dir = tempfile::tempdir().unwrap();
        configure_in(true, "", dir.path().to_path_buf());
        let st = status();
        assert!(st.dirs.contains(&dir.path().join("ffmpeg")), "{:?}", st.dirs);
        // The worker reports Missing (no libraries in a temp folder) or the V1a loader's
        // outcome; either way it ends in a state, and nothing blocks the caller.
        let start = std::time::Instant::now();
        while matches!(status().state, State::Loading) && start.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        match status().state {
            State::Failed { searched, reason } => {
                assert_eq!(searched, st.dirs);
                assert!(reason.contains("FFmpeg"), "{reason}");
                assert!(about_line().starts_with("not loaded"), "{}", about_line());
                assert!(runtime().is_none());
                assert_eq!(source_tag(), "");
            }
            State::Loaded(i) => {
                // A development runtime is on this machine (SOYUZ_FFMPEG_DIR / next to the exe).
                assert!(!i.files.is_empty());
                assert!(source_tag().starts_with("ffmpeg-"));
            }
            other => panic!("{other:?}"),
        }
        // Off: hidden, whatever was loaded.
        configure_in(false, "", dir.path().to_path_buf());
        assert_eq!(status().state, State::Disabled);
        assert!(runtime().is_none());
        assert!(about_line().starts_with("off"));
        assert_eq!(runtime_release_tag(), format!("ffmpeg-runtime-{BUNDLED_VERSION}"));
        configure_in(true, "", dir.path().to_path_buf());
    }

    /// With a runtime (`SOYUZ_FFMPEG_DIR`, or the libraries next to the test binary): the
    /// wiring end to end on a generated WAV, no encoder needed. `SOYUZ_MEDIA_TESTS=1`
    /// turns a missing runtime into a failure (the `media` CI job); otherwise it skips.
    #[test]
    fn with_runtime_probe_feeds_the_facts() {
        let _serial = SERIAL.lock();
        let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
        let dir = tempfile::tempdir().unwrap();
        configure_in(true, "", dir.path().to_path_buf());
        let start = std::time::Instant::now();
        while matches!(status().state, State::Loading) && start.elapsed() < std::time::Duration::from_secs(60) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let Some(rt) = runtime() else {
            assert!(!strict, "SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loaded: {:?}", status().state);
            eprintln!("skipped: no FFmpeg runtime");
            return;
        };
        let wav = dir.path().join("t.wav");
        std::fs::write(&wav, crate::services::preview_audio::tests::wav(0.5, 8000, 0.5)).unwrap();
        let info = soyuz_media::probe(&rt, &soyuz_media::Source::File(wav.clone())).expect("probe a WAV");
        let audio = info.audio().expect("one audio stream");
        assert_eq!(audio.sample_rate, 8000);
        assert!(info.duration.is_some_and(|d| (d - 0.5).abs() < 0.05), "{:?}", info.duration);
        let mut facts = Vec::new();
        crate::services::preview_rich::media_facts(&mut facts, &info);
        assert!(facts.iter().any(|(k, v)| k == "Sample rate" && v == "8000 Hz"), "{facts:?}");
        assert!(facts.iter().any(|(k, _)| k == "Audio"), "{facts:?}");
        // The About line and the cache tag name the loaded build.
        assert!(about_line().contains("avcodec"), "{}", about_line());
        assert_eq!(source_tag(), format!("ffmpeg-{}", rt.versions().majors().avcodec));
        // The sentinel wraps the decode and is clean afterwards.
        let s = Sentinel::open(dir.path());
        {
            let _g = s.begin(&wav).unwrap();
            assert!(dir.path().join(SENTINEL_FILE).exists());
        }
        assert!(!dir.path().join(SENTINEL_FILE).exists());
    }

    #[test]
    fn bundled_version_matches_the_pins() {
        let pins = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ffmpeg/pins.env");
        if let Ok(text) = std::fs::read_to_string(&pins) {
            let pinned = text.lines().find_map(|l| l.strip_prefix("FFMPEG_VERSION=")).map(str::trim);
            assert_eq!(pinned, Some(BUNDLED_VERSION), "keep services::media::BUNDLED_VERSION in step with pins.env");
        }
    }
}
