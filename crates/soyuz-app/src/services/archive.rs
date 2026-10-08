//! Archive browsing (PLAN M7): `Location::Archive` listings on a worker thread that keeps
//! a few opened readers cached (their `dir_index` makes nested navigation cheap),
//! extracting single entries to a temp folder for "open", per-archive passwords, and a
//! hand-off queue so worker threads can submit extract jobs to the app's `JobsService`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crossbeam_channel::{Sender, unbounded};
use soyuz_archive::{ArchiveError, ArchiveFormat, ArchiveReader};
use soyuz_core::{FsError, FsErrorKind};
use soyuz_jobs::JobSpec;

use super::mailbox::{Postman, UiMsg};
use crate::state::pane::PaneId;
use crate::ui::toasts::Toast;

// ---- passwords ----------------------------------------------------------------------

fn passwords() -> &'static Mutex<HashMap<PathBuf, String>> {
    static P: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

/// Password the user typed for `archive` this session.
pub fn password_for(archive: &Path) -> Option<String> {
    passwords().lock().ok()?.get(archive).cloned()
}

pub fn set_password(archive: &Path, password: String) {
    if let Ok(mut m) = passwords().lock() {
        m.insert(archive.to_path_buf(), password);
    }
    if let Ok(mut c) = cache().lock() {
        c.retain(|k, _| k.0 != archive);
    }
}

/// Listing that needs a password: `(pane, archive, wrong_password)`. The UI polls it
/// and asks.
fn pending_password() -> &'static Mutex<Option<(PaneId, PathBuf, bool)>> {
    static P: OnceLock<Mutex<Option<(PaneId, PathBuf, bool)>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

pub fn take_password_request() -> Option<(PaneId, PathBuf, bool)> {
    pending_password().lock().ok()?.take()
}

// ---- detection ----------------------------------------------------------------------

/// Archive format by file name (no I/O), for "Enter opens the archive".
pub fn format_by_name(name: &str) -> Option<ArchiveFormat> {
    let lower = name.to_ascii_lowercase();
    let mut best: Option<(usize, ArchiveFormat)> = None;
    for f in ArchiveFormat::all() {
        for ext in f.extensions() {
            if lower.len() > ext.len() + 1
                && lower.ends_with(ext)
                && lower.as_bytes()[lower.len() - ext.len() - 1] == b'.'
                && best.is_none_or(|(n, _)| ext.len() > n)
            {
                best = Some((ext.len(), *f));
            }
        }
    }
    best.map(|(_, f)| f)
}

/// Whether Enter should browse `path` as a folder.
pub fn is_browsable(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| format_by_name(n).is_some())
}

/// Map an archive error onto the pane's error type.
pub fn to_fs_error(e: ArchiveError, archive: &Path) -> FsError {
    let (kind, msg) = match e {
        ArchiveError::Io(fe) => return fe,
        ArchiveError::PasswordRequired => (FsErrorKind::PasswordRequired, "This archive is password protected".into()),
        ArchiveError::WrongPassword => (FsErrorKind::WrongPassword, "Wrong password".into()),
        ArchiveError::Corrupt(m) => (FsErrorKind::Corrupt, format!("Corrupt archive: {m}")),
        ArchiveError::Unsupported(f) => (FsErrorKind::Unsupported, format!("{f:?} archives are not supported")),
        other => (FsErrorKind::Io, other.to_string()),
    };
    FsError::with_message(kind, "open_archive", Some(archive), msg)
}

// ---- worker -------------------------------------------------------------------------

type Key = (PathBuf, Option<SystemTime>);
type ReaderCache = Mutex<HashMap<Key, (u64, Box<dyn ArchiveReader>)>>;

fn cache() -> &'static ReaderCache {
    static C: OnceLock<ReaderCache> = OnceLock::new();
    C.get_or_init(Default::default)
}

enum Req {
    List { pane: PaneId, generation: u64, archive: PathBuf, inner: String, postman: Postman },
    Open { archive: PathBuf, inner: String, postman: Postman },
}

fn worker() -> &'static Sender<Req> {
    static W: OnceLock<Sender<Req>> = OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = unbounded::<Req>();
        let _ = std::thread::Builder::new().name("soyuz-archive".into()).spawn(move || {
            for req in rx.iter() {
                match req {
                    Req::List { pane, generation, archive, inner, postman } => {
                        let msg = match with_reader(&archive, |r| soyuz_archive::list_dir(r, &inner)) {
                            Ok(chunk) => UiMsg::Listing { pane, generation, chunk, done: true, err: None },
                            Err(e) => {
                                if matches!(e, ArchiveError::PasswordRequired | ArchiveError::WrongPassword)
                                    && let Ok(mut p) = pending_password().lock()
                                {
                                    *p = Some((pane, archive.clone(), matches!(e, ArchiveError::WrongPassword)));
                                }
                                let err = to_fs_error(e, &archive);
                                UiMsg::Listing { pane, generation, chunk: Vec::new(), done: true, err: Some(err) }
                            }
                        };
                        postman.send(msg);
                    }
                    Req::Open { archive, inner, postman } => {
                        if let Err(e) = open_entry(&archive, &inner) {
                            let name = inner.rsplit('/').next().unwrap_or(&inner).to_string();
                            postman.send(UiMsg::Notice(Toast::error(format!("Couldn't open {name}")).body(e)));
                        }
                    }
                }
            }
        });
        tx
    })
}

/// Run `f` with a cached reader for `archive` (opened on first use, keyed by mtime).
fn with_reader<T>(
    archive: &Path,
    f: impl FnOnce(&mut dyn ArchiveReader) -> Result<T, ArchiveError>,
) -> Result<T, ArchiveError> {
    static TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mtime = std::fs::metadata(archive).and_then(|m| m.modified()).ok();
    let key: Key = (archive.to_path_buf(), mtime);
    let tick = TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut cache = cache().lock().map_err(|_| ArchiveError::Cancelled)?;
    if !cache.contains_key(&key) {
        let pw = password_for(archive);
        let reader = soyuz_archive::open(archive, pw.as_deref())?;
        cache.retain(|k, _| k.0 != archive);
        while cache.len() >= 4 {
            let oldest = cache.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => cache.remove(&k),
                None => break,
            };
        }
        cache.insert(key.clone(), (tick, reader));
    }
    let slot = cache.get_mut(&key).ok_or(ArchiveError::Cancelled)?;
    slot.0 = tick;
    let r = f(slot.1.as_mut());
    if matches!(r, Err(ArchiveError::PasswordRequired | ArchiveError::WrongPassword)) {
        cache.remove(&key);
    }
    r
}

/// List `inner` of `archive` for a pane; the result arrives as a normal
/// `UiMsg::Listing`, so the pane machinery (sorting, selection, views) is shared.
pub fn list(pane: PaneId, generation: u64, archive: &Path, inner: &str, postman: Postman) {
    let _ = worker().send(Req::List {
        pane,
        generation,
        archive: archive.to_path_buf(),
        inner: inner.to_string(),
        postman,
    });
}

/// Extract one entry to a temp folder and open it with the default app.
pub fn open_in_app(archive: &Path, inner: &str, postman: Postman) {
    let _ = worker().send(Req::Open { archive: archive.to_path_buf(), inner: inner.to_string(), postman });
}

/// Temp folder used for entries opened from archives.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("soyuz-archive")
}

fn open_entry(archive: &Path, inner: &str) -> Result<(), String> {
    let name = inner.rsplit('/').next().unwrap_or(inner);
    let stem = soyuz_core::entry::display_name(archive);
    let dir = temp_root().join(format!("{stem}-{:x}", hash(&archive.to_string_lossy())));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(name);
    let bytes = with_reader(archive, |r| {
        let idx = soyuz_archive::find_entry(r, inner).ok_or(ArchiveError::Corrupt("entry not found".into()))?;
        let mut rd = r.open_entry(idx)?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut rd, &mut buf)?;
        Ok(buf)
    })
    .map_err(|e| e.to_string())?;
    std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
    soyuz_fs::launch::open_default(&dest).map_err(|e| e.to_string())
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

// ---- jobs hand-off -----------------------------------------------------------------

/// Jobs submitted from worker threads, drained into the app's `JobsService` each frame.
fn pending() -> &'static Mutex<Vec<JobSpec>> {
    static P: OnceLock<Mutex<Vec<JobSpec>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn ctx_slot() -> &'static OnceLock<egui::Context> {
    static C: OnceLock<egui::Context> = OnceLock::new();
    &C
}

/// Remember the egui context so worker threads can wake the UI after queueing a job.
pub fn remember_ctx(ctx: &egui::Context) {
    let _ = ctx_slot().set(ctx.clone());
}

/// Queue a job from a worker thread; the UI submits it on its next frame.
pub fn submit_job_detached(spec: JobSpec) {
    if let Ok(mut q) = pending().lock() {
        q.push(spec);
    }
    if let Some(ctx) = ctx_slot().get() {
        ctx.request_repaint();
    }
}

/// Take the jobs queued by [`submit_job_detached`].
pub fn take_pending_jobs() -> Vec<JobSpec> {
    pending().lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_by_name() {
        assert_eq!(format_by_name("a.zip"), Some(ArchiveFormat::Zip));
        assert_eq!(format_by_name("A.TAR.GZ"), Some(ArchiveFormat::TarGz));
        assert_eq!(format_by_name("x.tgz"), Some(ArchiveFormat::TarGz));
        assert_eq!(format_by_name("x.tar"), Some(ArchiveFormat::Tar));
        assert_eq!(format_by_name("x.7z"), Some(ArchiveFormat::SevenZ));
        assert_eq!(format_by_name("zip"), None);
        assert_eq!(format_by_name("notes.txt"), None);
        assert!(is_browsable(Path::new("C:/d/b.tar.xz")));
    }

    #[test]
    fn errors_map_to_fs_kinds() {
        let p = Path::new("a.zip");
        assert_eq!(to_fs_error(ArchiveError::PasswordRequired, p).kind, FsErrorKind::PasswordRequired);
        assert_eq!(to_fs_error(ArchiveError::Corrupt("x".into()), p).kind, FsErrorKind::Corrupt);
    }

    #[test]
    fn lists_a_zip_through_the_mailbox() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("inner")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("inner/b.txt"), "world").unwrap();
        let zip = dir.path().join("t.zip");
        let opts = soyuz_archive::CreateOptions {
            format: ArchiveFormat::Zip,
            level: 6,
            password: None,
            base_dir: src.clone(),
            store_symlinks: false,
            follow_symlinks: false,
        };
        let ctl = soyuz_core::OpControl::new();
        soyuz_archive::create(&zip, &[src.join("a.txt"), src.join("inner")], &opts, &ctl, &mut ()).unwrap();

        let mb = super::super::mailbox::Mailbox::new(std::sync::Arc::new(|| {}));
        let pane = PaneId(9_999);
        list(pane, 3, &zip, "", mb.postman());
        list(pane, 4, &zip, "inner", mb.postman());
        let mut got = Vec::new();
        let start = std::time::Instant::now();
        while got.len() < 2 && start.elapsed() < std::time::Duration::from_secs(10) {
            if let Some(UiMsg::Listing { generation, chunk, err, .. }) = mb.try_recv() {
                assert!(err.is_none(), "{err:?}");
                let mut names: Vec<String> = chunk.iter().map(|e| e.name.clone()).collect();
                names.sort();
                got.push((generation, names));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got[0], (3, vec!["a.txt".to_string(), "inner".to_string()]));
        assert_eq!(got[1], (4, vec!["b.txt".to_string()]));
        // Entries read for previews come through the same cache-less path.
        let (bytes, truncated) = super::super::preview::read_archive_entry(&zip, "inner/b.txt", 1024).unwrap();
        assert_eq!((bytes.as_slice(), truncated), (&b"world"[..], false));
    }
}
