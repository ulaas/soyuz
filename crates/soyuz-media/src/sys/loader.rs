//! Finding and loading the five libraries ([`load`]), the version check against
//! [`Majors::SUPPORTED`], symbol binding and the load-time self-test. Libraries are never
//! unloaded: a loaded set lives for the rest of the process even when a later step of
//! the load fails (unloading a library whose code may still be referenced by FFmpeg's
//! global state would be worse than a few megabytes of memory).

use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use libloading::Library;

use super::layout::{self, Layout};
use super::symbols::{Fns, Libs, OptionalFns};
use super::types::*;
use crate::{LOADER_PATH, LoadError, Majors, Versions, lib_file_names};

/// The loaded libraries with everything bound; `&'static` once it exists (`Box::leak`,
/// see [`load`]).
pub(crate) struct Av {
    pub f: Fns,
    pub opt: OptionalFns,
    pub layout: &'static Layout,
    pub versions: Versions,
    /// The folder the libraries came from (empty for the loader's own path).
    pub dir: PathBuf,
    _libs: Libs,
}

impl std::fmt::Debug for Av {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Av").field("versions", &self.versions).field("dir", &self.dir).finish_non_exhaustive()
    }
}

/// The modification time of the first library file that exists in `dirs` (any supported
/// row): a failed load is retried when this or the folder list changes.
pub(crate) fn stamp(dirs: &[PathBuf]) -> Option<(PathBuf, SystemTime)> {
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for row in Majors::SUPPORTED {
            let path = dir.join(&lib_file_names(row)[3]);
            if let Ok(meta) = std::fs::metadata(&path) {
                return Some((path, meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
            }
        }
    }
    None
}

/// Load one library by absolute path, or by bare file name when `dir` is the loader's
/// own path.
fn load_one(dir: &Path, name: &str) -> Result<Library, String> {
    #[cfg(windows)]
    {
        use libloading::os::windows::{
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, Library as Win,
        };
        if dir.as_os_str().is_empty() {
            // SAFETY: loading a library runs its initialisers; the FFmpeg libraries have no
            // constructors with side effects beyond their own static state.
            return unsafe { Win::new(name) }.map(Library::from).map_err(|e| e.to_string());
        }
        let path = std::path::absolute(dir.join(name)).map_err(|e| e.to_string())?;
        // SAFETY: as above; the flags make the library's own imports (`avcodec-63.dll`
        // needs `avutil-61.dll`) resolve from its folder and the system folders only, never
        // from the current directory or `PATH` (review must-fix 11).
        unsafe { Win::load_with_flags(&path, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) }
            .map(Library::from)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        use libloading::os::unix::{Library as Unix, RTLD_LOCAL, RTLD_NOW};
        let path: PathBuf = if dir.as_os_str().is_empty() { PathBuf::from(name) } else { dir.join(name) };
        // SAFETY: loading a library runs its initialisers; the FFmpeg libraries have no
        // constructors with side effects beyond their own static state. RTLD_LOCAL keeps
        // their symbols out of the global namespace (no clash with another FFmpeg copy
        // the process may carry), RTLD_NOW surfaces unresolved imports here.
        unsafe { Unix::open(Some(&path), RTLD_NOW | RTLD_LOCAL) }.map(Library::from).map_err(|e| e.to_string())
    }
}

/// Load the set of `row` from `dir`: `Ok(None)` when a file is missing (`dir` is a real
/// folder) or the first library fails to load by bare name (the loader's path).
fn load_set(dir: &Path, row: Majors) -> Result<Option<Libs>, LoadError> {
    let names = lib_file_names(row);
    let by_name = dir.as_os_str().is_empty();
    if !by_name && !names.iter().all(|n| dir.join(n).is_file()) {
        return Ok(None);
    }
    let mut loaded: Vec<Library> = Vec::with_capacity(5);
    for name in &names {
        match load_one(dir, name) {
            Ok(lib) => loaded.push(lib),
            Err(why) if by_name => {
                log::debug!("soyuz-media: {name} not on the loader's path ({why})");
                // Keep what was loaded (never unload); the set is incomplete.
                for lib in loaded {
                    std::mem::forget(lib);
                }
                return Ok(None);
            }
            Err(why) => {
                for lib in loaded {
                    std::mem::forget(lib);
                }
                return Err(LoadError::Broken { lib: dir.join(name), why });
            }
        }
    }
    let mut it = loaded.into_iter();
    let (avutil, swresample, swscale, avcodec, avformat) = (
        it.next().expect("5"),
        it.next().expect("5"),
        it.next().expect("5"),
        it.next().expect("5"),
        it.next().expect("5"),
    );
    Ok(Some(Libs { avutil, swresample, swscale, avcodec, avformat }))
}

/// Read a C string the library owns.
pub(crate) fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: FFmpeg returns pointers to NUL-terminated strings it owns (static tables or
    // context-owned storage that outlives this call); the caller only passes those.
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// Versions of a freshly loaded set (binds only the five `*_version` functions and
/// `avcodec_configuration`).
fn versions(libs: &Libs) -> Result<Versions, LoadError> {
    type VersionFn = unsafe extern "C" fn() -> u32;
    type ConfFn = unsafe extern "C" fn() -> *const c_char;
    let get = |lib: &Library, name: &'static str| -> Result<u32, LoadError> {
        // SAFETY: every `*_version` function is `unsigned f(void)` in all supported majors.
        // (libloading adds the terminating NUL to the name.)
        let f = unsafe { lib.get::<VersionFn>(name) }
            .map_err(|e| LoadError::Broken { lib: PathBuf::from(name), why: e.to_string() })?;
        // SAFETY: a pure function of the library's constants.
        Ok(unsafe { f() })
    };
    let avutil = get(&libs.avutil, "avutil_version")?;
    let swresample = get(&libs.swresample, "swresample_version")?;
    let swscale = get(&libs.swscale, "swscale_version")?;
    let avcodec = get(&libs.avcodec, "avcodec_version")?;
    let avformat = get(&libs.avformat, "avformat_version")?;
    // SAFETY: `const char *avcodec_configuration(void)` in every supported major.
    let conf = unsafe { libs.avcodec.get::<ConfFn>(b"avcodec_configuration\0") }
        .map_err(|e| LoadError::Broken { lib: PathBuf::from("avcodec_configuration"), why: e.to_string() })?;
    // SAFETY: returns a static string.
    let configuration = cstr(unsafe { conf() }).unwrap_or_default();
    let gpl = configuration.contains("--enable-gpl") || configuration.contains("--enable-nonfree");
    Ok(Versions { avutil, swresample, swscale, avcodec, avformat, configuration, gpl })
}

/// Load the first complete, supported set found in `dirs`, bind everything and run the
/// self-test. The result is leaked on purpose (`&'static`): libraries are never
/// unloaded, and every wrapper holds a plain reference.
pub(crate) fn load(dirs: &[PathBuf]) -> Result<&'static Av, LoadError> {
    for dir in dirs {
        for row in Majors::SUPPORTED {
            let Some(libs) = load_set(dir, row)? else { continue };
            let shown = if dir.as_os_str() == LOADER_PATH {
                "the loader's path".to_string()
            } else {
                dir.display().to_string()
            };
            log::debug!("soyuz-media: found FFmpeg {} ({}) in {shown}", row.release().unwrap_or("?"), row.describe());
            return bind_all(libs, dir).inspect_err(|e| log::warn!("soyuz-media: FFmpeg in {shown} refused: {e}"));
        }
    }
    Err(LoadError::Missing { searched: dirs.to_vec() })
}

/// The libraries stay loaded whatever happens next (see the module doc).
fn keep(libs: Libs, e: LoadError) -> LoadError {
    std::mem::forget(libs);
    e
}

fn bind_all(libs: Libs, dir: &Path) -> Result<&'static Av, LoadError> {
    let versions = match versions(&libs) {
        Ok(v) => v,
        Err(e) => return Err(keep(libs, e)),
    };
    let majors = versions.majors();
    let Some(layout) = layout::for_majors(majors) else {
        let same_release = Majors::SUPPORTED
            .iter()
            .any(|r| r.avutil == majors.avutil || r.avcodec == majors.avcodec || r.avformat == majors.avformat);
        let e = if same_release { LoadError::Mismatch(versions) } else { LoadError::Unsupported { found: versions } };
        return Err(keep(libs, e));
    };
    let f = match Fns::bind(&libs) {
        Ok(f) => f,
        Err((name, why)) => {
            let lib =
                if dir.as_os_str().is_empty() { PathBuf::from(name) } else { dir.join(&lib_file_names(majors)[3]) };
            return Err(keep(libs, LoadError::Broken { lib, why: format!("missing symbol {name}: {why}") }));
        }
    };
    let opt = OptionalFns::bind(&libs);
    let level = if log::log_enabled!(target: "soyuz_media", log::Level::Debug) {
        AV_LOG_WARNING
    } else if cfg!(debug_assertions) {
        AV_LOG_ERROR
    } else {
        AV_LOG_QUIET
    };
    // SAFETY: a plain setter of a global.
    unsafe { (f.av_log_set_level)(level) };
    // Leaked on purpose: wrappers hold `&'static Av`, and the libraries inside are never
    // unloaded (also not when the self-test fails: a failed `Av` is simply never used).
    let av: &'static Av = Box::leak(Box::new(Av { f, opt, layout, versions, dir: dir.to_path_buf(), _libs: libs }));
    super::check::self_test(av).map_err(LoadError::SelfTest)?;
    Ok(av)
}
