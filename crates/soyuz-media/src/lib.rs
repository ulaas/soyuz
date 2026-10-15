//! The FFmpeg libraries, loaded at run time (Phase V).
//!
//! Soyuz links no FFmpeg code: `avutil`, `swresample`, `swscale`, `avcodec` and
//! `avformat` are shared libraries found at run time ([`search_dirs`]) and bound through
//! `libloading`. Everything `unsafe` lives under `sys/` (the one allow-listed module of
//! this crate, see `scripts/unsafe-allowlist.txt`); the rest of the workspace sees only
//! this safe API:
//!
//! - [`Runtime`]: the loaded libraries (one per process, never unloaded) with their
//!   [`Versions`]; [`Runtime::get`] loads them on first use, [`Runtime::loaded`] never loads.
//! - [`info::probe`]: container, streams, chapters, tags and cover art of a file.
//! - [`frame::grab_frame`]: one RGBA frame (poster frames, thumbnails).
//! - [`audio::AudioReader`] / [`audio::peaks`]: decoded f32 samples and waveform peaks.
//! - [`player::Player`]: audio / video playback with an audio-clock master, frames handed
//!   to a [`player::FrameSink`] as RGBA or as planes ([`yuv::YuvFrame`], for a GPU
//!   colour conversion); hardware decoding through [`hw`] (`[media] hw_decode`).
//! - [`transcode::run`] / [`transcode::plan`] with the built-in [`presets`]: convert,
//!   extract audio, trim, remux, save a frame; driven by `soyuz-jobs` (`JobKind::Media`).
//!
//! Without libraries every call fails with [`MediaError::NotLoaded`] and the callers keep
//! their pure-Rust fallbacks (metadata from `preview_media`, Symphonia audio).
//!
//! Supported releases (one row of [`Majors::SUPPORTED`] at a time, mixed sets are refused):
//! the bundled 9.0 (avcodec 63) plus the distro / Flatpak-runtime 8.x (62) and 7.1 (61).
//!
//! Tests: `cargo test -p soyuz-media` runs without libraries;
//! `SOYUZ_FFMPEG_DIR=<folder> SOYUZ_MEDIA_TESTS=1` adds the with-library suites
//! (`tests/with_libs.rs`, `tests/player.rs`, `tests/transcode.rs`), which fail instead of
//! skipping when no runtime loads.

#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

#[allow(unsafe_code)]
pub(crate) mod sys;

pub mod audio;
pub(crate) mod clock;
pub(crate) mod decode;
pub mod fixtures;
pub mod frame;
pub mod hw;
pub mod info;
pub mod mux;
pub mod player;
pub mod presets;
pub(crate) mod queue;
pub mod transcode;
pub mod yuv;

pub use audio::{AudioReader, peaks};
pub use frame::{FrameRequest, grab_frame};
pub use info::{Chapter, ColorInfo, Hdr, MediaInfo, Source, StreamInfo, StreamKind, probe};
pub use mux::{audio_copy_container, container_accepts, container_for_ext, ext_for_container};
pub use player::{
    AudioOutput, AudioSink, FramePixels, FrameSink, HwPref, PREROLL, Player, PlayerCmd, PlayerOptions, PlayerState,
    preroll_frames,
};
pub use presets::{AudioEnc, Preset, PresetId, VideoEnc, presets};
pub use transcode::{AudioCodecChoice, Cancelled, ImageFormat, MediaOp, Outcome, PlanInfo, Progress, plan, run};
pub use yuv::{Transfer, YuvColor, YuvFrame, YuvImage, YuvLayout, YuvMatrix};

/// Errors of the media functions.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// No FFmpeg libraries are loaded ([`Runtime::get`] failed or was never called).
    #[error("the FFmpeg libraries are not loaded")]
    NotLoaded,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The container / codec / operation is not available in the loaded build.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The file has no stream of this kind.
    #[error("no {0} stream")]
    NoStream(StreamKind),
    /// An FFmpeg call failed (`code` is the negative `AVERROR`, `what` its `av_strerror`).
    #[error("FFmpeg: {what} ({code})")]
    Av { code: i32, what: String },
    /// The caller's `check` closure asked to stop.
    #[error("cancelled")]
    Cancelled,
    /// The caller's `stale` closure said the result is no longer wanted.
    #[error("stale")]
    Stale,
}

/// Why the libraries could not be loaded.
#[derive(Clone, Debug, thiserror::Error)]
pub enum LoadError {
    /// No complete set of libraries in any of the folders searched (an empty entry stands
    /// for the dynamic loader's own search path, see [`LOADER_PATH`]).
    #[error("FFmpeg libraries not found (searched {})", describe_dirs(.searched))]
    Missing { searched: Vec<PathBuf> },
    /// A set was found but its majors are not a row of [`Majors::SUPPORTED`].
    #[error("unsupported FFmpeg libraries: {}", .found.majors().describe())]
    Unsupported { found: Versions },
    /// The five libraries belong to different releases.
    #[error("mismatched FFmpeg libraries: {}", .0.majors().describe())]
    Mismatch(Versions),
    /// A library file exists but could not be loaded or bound.
    #[error("cannot load {}: {why}", .lib.display())]
    Broken { lib: PathBuf, why: String },
    /// The load-time self-test (decode of a generated WAV / AVI) failed.
    #[error("FFmpeg self-test failed: {0}")]
    SelfTest(String),
}

fn describe_dirs(dirs: &[PathBuf]) -> String {
    let names: Vec<String> = dirs
        .iter()
        .map(
            |d| if d.as_os_str().is_empty() { "the loader's search path".to_string() } else { d.display().to_string() },
        )
        .collect();
    if names.is_empty() { "nowhere".to_string() } else { names.join(", ") }
}

/// The major version of each library in one FFmpeg release (`avcodec_version() >> 16`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Majors {
    pub avutil: u32,
    pub swresample: u32,
    pub swscale: u32,
    pub avcodec: u32,
    pub avformat: u32,
}

impl Majors {
    /// FFmpeg 9.0 (the bundled build).
    pub const V9: Majors = Majors { avutil: 61, swresample: 7, swscale: 10, avcodec: 63, avformat: 63 };
    /// FFmpeg 8.x (Ubuntu 26.04).
    pub const V8: Majors = Majors { avutil: 60, swresample: 6, swscale: 9, avcodec: 62, avformat: 62 };
    /// FFmpeg 7.1 (Debian 13, Fedora 43, the Flatpak 25.08 runtime).
    pub const V7: Majors = Majors { avutil: 59, swresample: 5, swscale: 8, avcodec: 61, avformat: 61 };
    /// Every accepted row, newest first; the loader tries their file names in this order.
    pub const SUPPORTED: [Majors; 3] = [Majors::V9, Majors::V8, Majors::V7];
    /// The row Soyuz ships (`third_party/ffmpeg/pins.env`).
    pub const BUNDLED: Majors = Majors::V9;

    /// The release line of a supported row (`"9.0"`), `None` for anything else.
    pub fn release(&self) -> Option<&'static str> {
        match *self {
            Majors::V9 => Some("9.0"),
            Majors::V8 => Some("8.x"),
            Majors::V7 => Some("7.1"),
            _ => None,
        }
    }

    /// `"avutil 61, swresample 7, swscale 10, avcodec 63, avformat 63"`.
    pub fn describe(&self) -> String {
        format!(
            "avutil {}, swresample {}, swscale {}, avcodec {}, avformat {}",
            self.avutil, self.swresample, self.swscale, self.avcodec, self.avformat
        )
    }
}

/// What the loaded libraries report about themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Versions {
    /// `avutil_version()` (major in the high 16 bits, then minor and micro).
    pub avutil: u32,
    pub swresample: u32,
    pub swscale: u32,
    pub avcodec: u32,
    pub avformat: u32,
    /// `avcodec_configuration()`: the configure line of the build.
    pub configuration: String,
    /// The build was configured with `--enable-gpl` or `--enable-nonfree`: a user-supplied
    /// library, labelled in About; never what Soyuz ships.
    pub gpl: bool,
}

impl Versions {
    /// The major of each library.
    pub fn majors(&self) -> Majors {
        Majors {
            avutil: self.avutil >> 16,
            swresample: self.swresample >> 16,
            swscale: self.swscale >> 16,
            avcodec: self.avcodec >> 16,
            avformat: self.avformat >> 16,
        }
    }

    /// `"63.2.100"` style text of one packed version number.
    pub fn text(v: u32) -> String {
        format!("{}.{}.{}", v >> 16, (v >> 8) & 0xff, v & 0xff)
    }
}

/// A codec of the loaded build (`Runtime::codecs`, the About / Settings listing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecDesc {
    pub name: String,
    pub long_name: String,
    pub kind: StreamKind,
    pub decoder: bool,
    pub encoder: bool,
}

/// The loaded FFmpeg libraries. One per process: a successful load is final (the
/// libraries are never unloaded), only a failed attempt is retried when the folder list or
/// a found file's modification time changes.
#[derive(Debug)]
pub struct Runtime {
    av: &'static sys::Av,
    /// `has_encoder` results (opening a hardware encoder costs a driver round trip).
    encoders: Mutex<HashMap<String, bool>>,
}

static LOADED: OnceLock<Arc<Runtime>> = OnceLock::new();

/// The last failed attempt, so an unchanged situation is not re-tried.
struct Attempt {
    dirs: Vec<PathBuf>,
    stamp: Option<(PathBuf, SystemTime)>,
    error: LoadError,
}

static TRIED: Mutex<Option<Attempt>> = Mutex::new(None);

impl Runtime {
    /// Load the libraries from the first folder of `dirs` holding a complete, supported
    /// set (`dirs` is usually [`search_dirs`]). The first successful load wins for the
    /// process and is returned again by every later call, whatever `dirs` says then. A
    /// failed attempt is remembered and only repeated when `dirs` or the modification
    /// time of the first library file found in them changes.
    pub fn get(dirs: &[PathBuf]) -> Result<Arc<Runtime>, LoadError> {
        if let Some(rt) = LOADED.get() {
            return Ok(Arc::clone(rt));
        }
        let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(rt) = LOADED.get() {
            return Ok(Arc::clone(rt));
        }
        let stamp = sys::loader::stamp(dirs);
        if let Some(a) = tried.as_ref()
            && a.dirs == dirs
            && a.stamp == stamp
        {
            return Err(a.error.clone());
        }
        match sys::loader::load(dirs) {
            Ok(av) => {
                let v = &av.versions;
                log::info!(
                    "soyuz-media: FFmpeg {} loaded from {} (avcodec {}{})",
                    v.majors().release().unwrap_or("?"),
                    describe_dirs(std::slice::from_ref(&av.dir)),
                    Versions::text(v.avcodec),
                    if v.gpl { ", GPL build" } else { "" }
                );
                let rt = Arc::new(Runtime { av, encoders: Mutex::new(HashMap::new()) });
                let rt = LOADED.get_or_init(|| rt);
                *tried = None;
                Ok(Arc::clone(rt))
            }
            Err(e) => {
                log::debug!("soyuz-media: {e}");
                *tried = Some(Attempt { dirs: dirs.to_vec(), stamp, error: e.clone() });
                Err(e)
            }
        }
    }

    /// The runtime if [`Runtime::get`] succeeded earlier; never loads.
    pub fn loaded() -> Option<Arc<Runtime>> {
        LOADED.get().map(Arc::clone)
    }

    pub fn versions(&self) -> &Versions {
        &self.av.versions
    }

    /// The folder the libraries were loaded from (empty for the loader's own path).
    pub fn dir(&self) -> &Path {
        &self.av.dir
    }

    pub(crate) fn av(&self) -> &'static sys::Av {
        self.av
    }

    /// `avcodec_find_decoder_by_name(name)` succeeds.
    pub fn has_decoder(&self, name: &str) -> bool {
        sys::codec::has_decoder(self.av, name)
    }

    /// The encoder exists *and* opens with a small default configuration (hardware
    /// encoders fail on machines without the driver); probed once per name and cached.
    pub fn has_encoder(&self, name: &str) -> bool {
        if let Some(v) = self.encoders.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
            return *v;
        }
        let v = sys::codec::encoder_opens(self.av, name);
        self.encoders.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), v);
        v
    }

    /// Every codec of the build (one row per codec id, with whether a decoder and an
    /// encoder exist), for the About dialog and the Settings status card.
    pub fn codecs(&self) -> Vec<CodecDesc> {
        sys::codec::codec_list(self.av)
    }
}

/// The empty entry of a [`search_dirs`] list: load the libraries by bare file name, so
/// the dynamic loader's own search applies (`LD_LIBRARY_PATH` / rpath, `PATH` on
/// Windows, `DYLD_LIBRARY_PATH` / `@rpath` on macOS).
pub const LOADER_PATH: &str = "";

/// Name of the environment variable that puts one folder in front of everything else
/// (tests and CI point it at a development runtime).
pub const DIR_ENV: &str = "SOYUZ_FFMPEG_DIR";

/// What [`search_dirs`] reads from the process (split out so tests can fix it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchEnv {
    /// `SOYUZ_FFMPEG_DIR`.
    pub env_dir: Option<PathBuf>,
    /// The folder of the running executable.
    pub exe_dir: Option<PathBuf>,
    /// Running inside a Flatpak sandbox (`/.flatpak-info` exists).
    pub flatpak: bool,
    /// `std::env::consts::OS` (`"windows"`, `"macos"`, `"linux"`, …).
    pub os: &'static str,
    /// `std::env::consts::ARCH`.
    pub arch: &'static str,
}

impl SearchEnv {
    /// The running process.
    pub fn current() -> SearchEnv {
        SearchEnv {
            env_dir: std::env::var_os(DIR_ENV).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()),
            exe_dir: std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)),
            flatpak: cfg!(target_os = "linux") && Path::new("/.flatpak-info").exists(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }
}

/// The folders to look for the libraries in, first match wins:
///
/// 1. `SOYUZ_FFMPEG_DIR` (tests, CI);
/// 2. `[media] ffmpeg_dir` (`configured`, when not empty);
/// 3. the executable's folder; macOS also `../Frameworks` (the signed bundle); Linux also
///    `../lib/soyuz` and `../lib64/soyuz` (the tarball / rpm layout);
/// 4. inside a Flatpak: the `codecs-extra` extension (the full build; the runtime's own
///    `libavcodec` is stripped of H.264 / HEVC) and then the loader's path (the sandbox's
///    `LD_LIBRARY_PATH`), both before the user-writable folders;
/// 5. `<config dir>/ffmpeg` (the in-app download, or copied there by hand);
/// 6. Linux system folders (distro packages);
/// 7. the loader's own search ([`LOADER_PATH`]).
///
/// Nothing is touched on disk; the loader opens the files.
pub fn search_dirs(configured: &str, config_dir: &Path) -> Vec<PathBuf> {
    search_dirs_in(configured, config_dir, &SearchEnv::current())
}

/// [`search_dirs`] for an explicit environment.
pub fn search_dirs_in(configured: &str, config_dir: &Path, env: &SearchEnv) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(d) = &env.env_dir {
        push(d.clone());
    }
    let configured = configured.trim();
    if !configured.is_empty() {
        push(PathBuf::from(configured));
    }
    if let Some(exe) = &env.exe_dir {
        push(exe.clone());
        match env.os {
            "macos" => push(exe.join("..").join("Frameworks")),
            "linux" => {
                push(exe.join("..").join("lib").join("soyuz"));
                push(exe.join("..").join("lib64").join("soyuz"));
            }
            _ => {}
        }
    }
    let triplet = multiarch_triplet(env.arch);
    if env.os == "linux" && env.flatpak {
        push(PathBuf::from(format!("/usr/lib/{triplet}/codecs-extra/lib")));
        push(PathBuf::from(LOADER_PATH));
    }
    push(config_dir.join("ffmpeg"));
    if env.os == "linux" {
        push(PathBuf::from("/usr/lib64"));
        push(PathBuf::from(format!("/usr/lib/{triplet}")));
        push(PathBuf::from("/usr/lib"));
        push(PathBuf::from("/usr/local/lib"));
        push(PathBuf::from("/usr/local/lib64"));
    }
    push(PathBuf::from(LOADER_PATH));
    out
}

/// Debian-style multiarch folder name of a CPU architecture.
fn multiarch_triplet(arch: &str) -> String {
    match arch {
        "x86_64" => "x86_64-linux-gnu".into(),
        "aarch64" => "aarch64-linux-gnu".into(),
        "x86" => "i386-linux-gnu".into(),
        "arm" => "arm-linux-gnueabihf".into(),
        other => format!("{other}-linux-gnu"),
    }
}

/// The five library file names of one release row on this OS, in load order (`avutil`,
/// `swresample`, `swscale`, `avcodec`, `avformat`: each depends only on earlier ones).
pub fn lib_file_names(majors: Majors) -> [String; 5] {
    lib_file_names_for(majors, std::env::consts::OS)
}

/// [`lib_file_names`] for an explicit `std::env::consts::OS` value: `avcodec-63.dll`,
/// `libavcodec.so.63`, `libavcodec.63.dylib`.
pub fn lib_file_names_for(majors: Majors, os: &str) -> [String; 5] {
    let name = |lib: &str, major: u32| match os {
        "windows" => format!("{lib}-{major}.dll"),
        "macos" | "ios" => format!("lib{lib}.{major}.dylib"),
        _ => format!("lib{lib}.so.{major}"),
    };
    [
        name("avutil", majors.avutil),
        name("swresample", majors.swresample),
        name("swscale", majors.swscale),
        name("avcodec", majors.avcodec),
        name("avformat", majors.avformat),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(os: &'static str) -> SearchEnv {
        SearchEnv { env_dir: None, exe_dir: Some(PathBuf::from("/app/bin")), flatpak: false, os, arch: "x86_64" }
    }

    #[test]
    fn search_order_linux() {
        let dirs = search_dirs_in(" /opt/ff ", Path::new("/home/u/.config/soyuz"), &env("linux"));
        let want: Vec<PathBuf> = [
            "/opt/ff",
            "/app/bin",
            "/app/bin/../lib/soyuz",
            "/app/bin/../lib64/soyuz",
            "/home/u/.config/soyuz/ffmpeg",
            "/usr/lib64",
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib",
            "/usr/local/lib",
            "/usr/local/lib64",
            LOADER_PATH,
        ]
        .map(PathBuf::from)
        .to_vec();
        assert_eq!(dirs, want);
        // The configured folder is optional; the env var goes first.
        let e = SearchEnv { env_dir: Some("/dev/rt".into()), ..env("linux") };
        let dirs = search_dirs_in("", Path::new("/c"), &e);
        assert_eq!(dirs[0], PathBuf::from("/dev/rt"));
        assert_eq!(dirs[1], PathBuf::from("/app/bin"));
        assert!(!dirs.contains(&PathBuf::from("/opt/ff")));
    }

    #[test]
    fn flatpak_prefers_codecs_extra_then_the_loader_path() {
        let e = SearchEnv { flatpak: true, arch: "aarch64", ..env("linux") };
        let dirs = search_dirs_in("", Path::new("/home/u/.var/app/x/config/soyuz"), &e);
        let codecs = dirs.iter().position(|d| d == Path::new("/usr/lib/aarch64-linux-gnu/codecs-extra/lib")).unwrap();
        let loader = dirs.iter().position(|d| d.as_os_str().is_empty()).unwrap();
        let config = dirs.iter().position(|d| d.ends_with("ffmpeg")).unwrap();
        let multiarch = dirs.iter().position(|d| d == Path::new("/usr/lib/aarch64-linux-gnu")).unwrap();
        assert!(codecs < loader && loader < config && config < multiarch, "{dirs:?}");
        assert_eq!(dirs.iter().filter(|d| d.as_os_str().is_empty()).count(), 1, "the loader path once");
        assert_eq!(dirs[0], PathBuf::from("/app/bin"), "the exe folder still comes first");
    }

    #[test]
    fn search_order_windows_and_macos() {
        let e = SearchEnv { exe_dir: Some(PathBuf::from(r"C:\Soyuz")), ..env("windows") };
        let dirs = search_dirs_in("", Path::new(r"C:\Users\u\AppData\Roaming\soyuz"), &e);
        assert_eq!(
            dirs,
            [r"C:\Soyuz", r"C:\Users\u\AppData\Roaming\soyuz\ffmpeg", LOADER_PATH].map(PathBuf::from).to_vec()
        );
        let e = SearchEnv { exe_dir: Some(PathBuf::from("/Applications/Soyuz.app/Contents/MacOS")), ..env("macos") };
        let dirs = search_dirs_in("", Path::new("/Users/u/Library/Application Support/soyuz"), &e);
        assert_eq!(dirs[0], PathBuf::from("/Applications/Soyuz.app/Contents/MacOS"));
        assert_eq!(dirs[1], PathBuf::from("/Applications/Soyuz.app/Contents/MacOS/../Frameworks"));
        assert_eq!(dirs[2], PathBuf::from("/Users/u/Library/Application Support/soyuz/ffmpeg"));
        assert_eq!(dirs[3], PathBuf::from(LOADER_PATH));
        assert_eq!(dirs.len(), 4, "no system folders off Linux: {dirs:?}");
        // No executable folder known: nothing panics, the rest stays.
        let dirs = search_dirs_in("", Path::new("/c"), &SearchEnv { exe_dir: None, ..env("windows") });
        assert_eq!(dirs, ["/c/ffmpeg", LOADER_PATH].map(PathBuf::from).to_vec());
    }

    #[test]
    fn duplicates_collapse() {
        let e = SearchEnv { env_dir: Some("/app/bin".into()), ..env("windows") };
        let dirs = search_dirs_in("/app/bin", Path::new("/c"), &e);
        assert_eq!(dirs.iter().filter(|d| **d == Path::new("/app/bin")).count(), 1, "{dirs:?}");
    }

    #[test]
    fn file_names_per_os() {
        assert_eq!(
            lib_file_names_for(Majors::V9, "windows"),
            ["avutil-61.dll", "swresample-7.dll", "swscale-10.dll", "avcodec-63.dll", "avformat-63.dll"]
        );
        assert_eq!(
            lib_file_names_for(Majors::V8, "linux"),
            ["libavutil.so.60", "libswresample.so.6", "libswscale.so.9", "libavcodec.so.62", "libavformat.so.62"]
        );
        assert_eq!(
            lib_file_names_for(Majors::V7, "macos"),
            [
                "libavutil.59.dylib",
                "libswresample.5.dylib",
                "libswscale.8.dylib",
                "libavcodec.61.dylib",
                "libavformat.61.dylib"
            ]
        );
        assert_eq!(lib_file_names(Majors::BUNDLED), lib_file_names_for(Majors::BUNDLED, std::env::consts::OS));
    }

    #[test]
    fn majors_rows() {
        assert_eq!(Majors::SUPPORTED[0], Majors::BUNDLED);
        assert_eq!(Majors::V9.release(), Some("9.0"));
        assert_eq!(Majors::V7.release(), Some("7.1"));
        assert_eq!(Majors { avcodec: 60, ..Majors::V9 }.release(), None, "mixed sets are not a row");
        let v = Versions {
            avutil: (61 << 16) | (2 << 8) | 100,
            swresample: 7 << 16,
            swscale: 10 << 16,
            avcodec: 63 << 16,
            avformat: 63 << 16,
            configuration: "--enable-shared".into(),
            gpl: false,
        };
        assert_eq!(v.majors(), Majors::V9);
        assert_eq!(Versions::text(v.avutil), "61.2.100");
        assert!(LoadError::Unsupported { found: v }.to_string().contains("avcodec 63"));
    }

    /// Without libraries (an empty temp folder; never the loader's path, which might
    /// find a system FFmpeg on a developer machine) the error lists what was searched,
    /// and an unchanged attempt is answered from memory.
    #[test]
    fn missing_runtime_lists_the_searched_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = vec![tmp.path().join("nowhere"), tmp.path().to_path_buf()];
        match Runtime::get(&dirs) {
            Err(LoadError::Missing { searched }) => {
                assert_eq!(searched, dirs);
                let text = LoadError::Missing { searched }.to_string();
                assert!(text.contains("nowhere"), "{text}");
            }
            other => panic!("{other:?}"),
        }
        assert!(Runtime::loaded().is_none());
        // The attempt is remembered (same dirs, no library file appeared).
        assert!(matches!(Runtime::get(&dirs), Err(LoadError::Missing { .. })));
        assert_eq!(
            LoadError::Missing { searched: vec![PathBuf::from(LOADER_PATH)] }.to_string(),
            "FFmpeg libraries not found (searched the loader's search path)"
        );
        assert_eq!(
            LoadError::Missing { searched: vec![] }.to_string(),
            "FFmpeg libraries not found (searched nowhere)"
        );
    }
}
