//! Shared persistence helpers for the small per-user stores next to `config.toml`
//! (tags, recent items, palette memory, rename presets, saved searches, sync profiles).
//!
//! Two guarantees (REVIEW-2 #7 / #8):
//!
//! * **A file that exists but does not parse is never overwritten.** [`load`] copies it
//!   to `<file>.bak` (or `.bak.1` … when that exists), logs, and marks the path as
//!   *blocked* for the rest of the session: every later [`save_sync`] / [`save_async`]
//!   to it is refused, so the user's data survives until they fix or delete the file.
//! * **Saves are written in order by one worker** ([`save_async`], latest-wins per
//!   path), and [`flush`] blocks until everything queued is on disk — `App::save` calls
//!   it so debounced changes are not lost on exit.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Paths whose existing file failed to parse this session (never written to).
fn blocked() -> &'static Mutex<HashSet<PathBuf>> {
    static B: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    B.get_or_init(Default::default)
}

/// Whether saving to `path` is refused this session (its file was unreadable).
pub fn is_blocked(path: &Path) -> bool {
    blocked().lock().map(|b| b.contains(path)).unwrap_or(false)
}

/// Result of [`load`].
#[derive(Debug, PartialEq)]
pub enum Loaded<T> {
    /// No file (or unreadable as text for an I/O reason other than "not found"): start empty.
    Missing,
    Ok(T),
    /// The file exists but does not parse. It was backed up to `backup` (when possible)
    /// and saving to it is blocked for this session.
    Corrupt {
        error: String,
        backup: Option<PathBuf>,
    },
}

impl<T: Default> Loaded<T> {
    /// The value, or the default for a missing / corrupt file.
    pub fn value(self) -> T {
        match self {
            Loaded::Ok(v) => v,
            _ => T::default(),
        }
    }
}

/// Read and parse `path`. A parse failure backs the file up and blocks saving (see the
/// module docs). A read error other than "not found" also blocks saving: we could not
/// see what is there, so we must not replace it.
pub fn load<T>(path: &Path, parse: impl FnOnce(&str) -> Result<T, String>) -> Loaded<T> {
    let text = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Missing,
        Err(e) => {
            log::warn!("{}: {e}; changes to it will not be saved this session", path.display());
            block(path);
            return Loaded::Corrupt { error: e.to_string(), backup: None };
        }
    };
    let parsed = match std::str::from_utf8(&text) {
        Ok(t) => parse(t),
        Err(e) => Err(format!("not UTF-8: {e}")),
    };
    match parsed {
        Ok(v) => Loaded::Ok(v),
        Err(error) => {
            let backup = backup_path(path).and_then(|b| match copy_new(path, &b) {
                Ok(()) => Some(b),
                Err(e) => {
                    log::warn!("cannot back up {} to {}: {e}", path.display(), b.display());
                    None
                }
            });
            match &backup {
                Some(b) => log::warn!(
                    "{}: {error}; kept a copy at {} and will not overwrite it this session",
                    path.display(),
                    b.display()
                ),
                None => log::warn!("{}: {error}; will not overwrite it this session", path.display()),
            }
            block(path);
            Loaded::Corrupt { error, backup }
        }
    }
}

/// [`load`] for a TOML file deserialized with serde.
pub fn load_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Loaded<T> {
    load(path, |t| toml::from_str(t).map_err(|e| e.to_string()))
}

fn block(path: &Path) {
    if let Ok(mut b) = blocked().lock() {
        b.insert(path.to_path_buf());
    }
}

/// First free `<file>.bak`, `<file>.bak.1` … `<file>.bak.99`.
fn backup_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    (0..100)
        .map(|i| {
            if i == 0 {
                path.with_file_name(format!("{name}.bak"))
            } else {
                path.with_file_name(format!("{name}.bak.{i}"))
            }
        })
        .find(|p| std::fs::symlink_metadata(p).is_err())
}

/// Copy without ever replacing an existing file.
fn copy_new(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    std::io::copy(&mut src, &mut dst)?;
    dst.sync_all()
}

fn write_now(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if is_blocked(path) {
        return Err(tr!("services-store-blocked", path = path.display().to_string()));
    }
    if let Some(dir) = path.parent() {
        let _ = soyuz_fs::create_dir_all(dir);
    }
    soyuz_fs::write_atomic(path, bytes).map_err(|e| e.to_string())
}

/// Write `bytes` to `path` atomically, now, on this thread (refused when blocked).
/// Any queued async save to the same path is dropped first so it cannot land later.
pub fn save_sync(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let w = writer();
    if let Ok(mut st) = w.state.lock() {
        st.queue.retain(|(p, _)| p != path);
        // An in-flight write (possibly to this path) must finish before ours.
        let deadline = Instant::now() + Duration::from_secs(5);
        while st.busy && Instant::now() < deadline {
            st = match w.cv.wait_timeout(st, Duration::from_millis(100)) {
                Ok((s, _)) => s,
                Err(_) => break,
            };
        }
    }
    write_now(path, bytes)
}

// ---- the save worker ---------------------------------------------------------------

#[derive(Default)]
struct WriterState {
    queue: VecDeque<(PathBuf, Vec<u8>)>,
    busy: bool,
}

struct Writer {
    state: Mutex<WriterState>,
    cv: Condvar,
}

fn writer() -> &'static Writer {
    static W: OnceLock<&'static Writer> = OnceLock::new();
    W.get_or_init(|| {
        let w: &'static Writer = Box::leak(Box::new(Writer { state: Mutex::default(), cv: Condvar::new() }));
        let spawned = std::thread::Builder::new().name("soyuz-store-save".into()).spawn(move || {
            loop {
                let job = {
                    let Ok(mut st) = w.state.lock() else { return };
                    while st.queue.is_empty() {
                        st = match w.cv.wait(st) {
                            Ok(s) => s,
                            Err(_) => return,
                        };
                    }
                    st.busy = true;
                    st.queue.pop_front()
                };
                if let Some((path, bytes)) = job
                    && let Err(e) = write_now(&path, &bytes)
                {
                    log::warn!("cannot save {}: {e}", path.display());
                }
                if let Ok(mut st) = w.state.lock() {
                    st.busy = false;
                }
                w.cv.notify_all();
            }
        });
        if spawned.is_err() {
            log::warn!("cannot spawn the store save worker; saves will run inline");
        }
        w
    })
}

/// Queue an atomic write of `bytes` to `path` on the single save worker. Writes happen
/// in order; a newer queued write to the same path replaces an older one (latest wins).
pub fn save_async(path: PathBuf, bytes: Vec<u8>) {
    if is_blocked(&path) {
        log::warn!("not saving {}: it could not be read at startup", path.display());
        return;
    }
    let w = writer();
    let Ok(mut st) = w.state.lock() else {
        let _ = write_now(&path, &bytes);
        return;
    };
    if let Some(slot) = st.queue.iter_mut().find(|(p, _)| *p == path) {
        slot.1 = bytes;
    } else {
        st.queue.push_back((path, bytes));
    }
    drop(st);
    w.cv.notify_all();
}

/// Block until every queued save is written (bounded by `timeout`, so a dead disk
/// cannot hang shutdown). Returns `false` on timeout.
pub fn flush(timeout: Duration) -> bool {
    let w = writer();
    let deadline = Instant::now() + timeout;
    let Ok(mut st) = w.state.lock() else { return false };
    while !st.queue.is_empty() || st.busy {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        st = match w.cv.wait_timeout(st, left) {
            Ok((s, _)) => s,
            Err(_) => return false,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, serde::Deserialize)]
    struct Doc {
        #[serde(default)]
        n: u32,
    }

    #[test]
    fn corrupt_file_is_backed_up_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.toml");
        std::fs::write(&path, "n = \"not a number\"\n").unwrap();
        // An older backup must not be clobbered.
        std::fs::write(dir.path().join("store.toml.bak"), "old").unwrap();
        let got = load_toml::<Doc>(&path);
        let Loaded::Corrupt { backup: Some(b), .. } = got else { panic!("{got:?}") };
        assert_eq!(b, dir.path().join("store.toml.bak.1"));
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "n = \"not a number\"\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("store.toml.bak")).unwrap(), "old");
        assert!(is_blocked(&path));
        assert!(save_sync(&path, b"n = 1\n").is_err());
        save_async(path.clone(), b"n = 2\n".to_vec());
        assert!(flush(Duration::from_secs(5)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "n = \"not a number\"\n", "user file untouched");
    }

    #[test]
    fn missing_and_valid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.toml");
        assert_eq!(load_toml::<Doc>(&path), Loaded::Missing);
        std::fs::write(&path, "n = 7\n").unwrap();
        assert_eq!(load_toml::<Doc>(&path), Loaded::Ok(Doc { n: 7 }));
        assert!(!is_blocked(&path));
    }

    #[test]
    fn async_saves_land_in_order_and_flush_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("seq.toml");
        for i in 0..50 {
            save_async(path.clone(), format!("n = {i}\n").into_bytes());
        }
        assert!(flush(Duration::from_secs(10)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "n = 49\n");
        // A sync save after queued async ones wins.
        save_async(path.clone(), b"n = 100\n".to_vec());
        save_sync(&path, b"n = 200\n").unwrap();
        assert!(flush(Duration::from_secs(10)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "n = 200\n");
    }
}
