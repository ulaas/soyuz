//! Editor background work: loading (sniffing text / binary / image), saving (optional
//! backup of the previous version to the trash, then an atomic write) and watching the
//! open files for external changes. Nothing here runs on the UI thread.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, Sender, select, unbounded};
use image::DynamicImage;

use super::image::ops::{ImgOp, SaveMode};
use super::text::encoding;
use crate::services::mailbox::Wake;

/// Identity of the file version on disk (size, modification time).
pub type Stamp = (u64, Option<SystemTime>);

pub fn stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()))
}

/// How a document should be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Auto,
    Text,
    Hex,
}

pub enum LoadResult {
    /// Built on the worker (splitting a big file into lines is not free).
    Text(Box<super::text::TextDoc>),
    Hex {
        len: u64,
        too_large: bool,
    },
    Image(super::image::Loaded),
    Error(String),
}

pub enum Msg {
    Loaded {
        doc: u64,
        reload: bool,
        stamp: Option<Stamp>,
        result: LoadResult,
    },
    Saved {
        doc: u64,
        result: Result<Option<Stamp>, String>,
        backup: Option<String>,
    },
    /// A watched file changed (`None`: it is gone).
    Changed {
        path: PathBuf,
        stamp: Option<Stamp>,
    },
    /// Unsaved-changes prompt answers.
    Close {
        doc: u64,
        answer: Answer,
    },
    Quit(Answer),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

/// Extensions opened in the image editor.
pub fn is_image(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "jfif" | "bmp" | "gif" | "webp" | "ico" | "tif" | "tiff")
}

pub struct Limits {
    pub max_text_bytes: u64,
    pub max_image_mp: u32,
}

pub fn spawn_load(doc: u64, path: PathBuf, want: Want, limits: Limits, reload: bool, tx: Sender<Msg>, wake: Wake) {
    let _ = std::thread::Builder::new().name("soyuz-editor-load".into()).spawn(move || {
        let stamp = stamp(&path);
        let result = load(&path, want, &limits);
        let _ = tx.send(Msg::Loaded { doc, reload, stamp, result });
        wake();
    });
}

fn load(path: &Path, want: Want, limits: &Limits) -> LoadResult {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return LoadResult::Error(e.to_string()),
    };
    if meta.is_dir() {
        return LoadResult::Error("This is a folder".into());
    }
    let len = meta.len();
    if want == Want::Hex {
        return LoadResult::Hex { len, too_large: false };
    }
    if want == Want::Auto && is_image(path) {
        match super::image::load(path, limits.max_image_mp) {
            Ok(l) => return LoadResult::Image(l),
            Err(e) => log::info!("{}: not opened as an image ({e}); using the hex editor", path.display()),
        }
        return LoadResult::Hex { len, too_large: false };
    }
    let mut head = vec![0u8; 8192.min(len as usize)];
    if let Err(e) = std::fs::File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)) {
        return LoadResult::Error(e.to_string());
    }
    let text = want == Want::Text || crate::services::preview_text::looks_like_text(&head);
    if !text {
        return LoadResult::Hex { len, too_large: false };
    }
    if len > limits.max_text_bytes {
        return LoadResult::Hex { len, too_large: true };
    }
    match std::fs::read(path) {
        Ok(bytes) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            LoadResult::Text(Box::new(super::text::TextDoc::new(&name, encoding::decode(&bytes))))
        }
        Err(e) => LoadResult::Error(e.to_string()),
    }
}

/// What to write.
pub enum SaveJob {
    Bytes(Vec<u8>),
    Patches(BTreeMap<u64, u8>),
    Image { full: Arc<DynamicImage>, ops: Vec<ImgOp>, mode: SaveMode, quality: u8 },
}

/// Largest previous version copied to the trash before an overwrite.
const MAX_BACKUP: u64 = 256 * 1024 * 1024;

pub fn spawn_save(doc: u64, path: PathBuf, job: SaveJob, backup: bool, tx: Sender<Msg>, wake: Wake) {
    let _ = std::thread::Builder::new().name("soyuz-editor-save".into()).spawn(move || {
        let (result, note) = save(&path, job, backup);
        let _ = tx.send(Msg::Saved { doc, result: result.map(|()| stamp(&path)), backup: note });
        wake();
    });
}

/// Returns the result and a note about the backup (when it could not be made).
fn save(path: &Path, job: SaveJob, backup: bool) -> (Result<(), String>, Option<String>) {
    let job = match job {
        SaveJob::Image { full, ops, mode, quality } => {
            let img = super::image::ImageDoc::edited(&full, &ops);
            match super::image::encode(&img, mode, quality) {
                Ok(b) => SaveJob::Bytes(b),
                Err(e) => return (Err(e), None),
            }
        }
        other => other,
    };
    let note = if backup { backup_to_trash(path).err() } else { None };
    let res = match job {
        SaveJob::Bytes(b) => soyuz_fs::write_atomic(path, &b).map_err(|e| e.to_string()),
        SaveJob::Patches(p) => super::hex::io::save_patched(path, &p),
        SaveJob::Image { .. } => Ok(()),
    };
    (res, note)
}

/// Copy the current file to `<name>.bak` next to it and move that copy to the trash, so
/// the previous version can be restored from there.
pub fn backup_to_trash(path: &Path) -> Result<(), String> {
    let Ok(meta) = std::fs::metadata(path) else { return Ok(()) };
    if meta.len() > MAX_BACKUP {
        return Err("too large for a backup copy".into());
    }
    let dir = path.parent().ok_or("no parent folder")?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let bak = soyuz_core::unique_name(dir, &format!("{name}.bak"), "", &|p| p.exists());
    std::fs::copy(path, &bak).map_err(|e| e.to_string())?;
    match soyuz_fs::trash::trash(&bak) {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&bak);
            Err(e.to_string())
        }
    }
}

// ---- watcher ------------------------------------------------------------------------

enum WatchCmd {
    Add(PathBuf),
    Remove(PathBuf),
}

/// Watches the folders of open documents and reports changes of the files themselves.
pub struct DocWatcher {
    tx: Sender<WatchCmd>,
}

impl DocWatcher {
    pub fn start(out: Sender<Msg>, wake: Wake) -> Self {
        let (tx, rx) = unbounded::<WatchCmd>();
        let _ = std::thread::Builder::new().name("soyuz-editor-watch".into()).spawn(move || run(rx, out, wake));
        DocWatcher { tx }
    }

    pub fn add(&self, file: &Path) {
        let _ = self.tx.send(WatchCmd::Add(file.to_path_buf()));
    }

    pub fn remove(&self, file: &Path) {
        let _ = self.tx.send(WatchCmd::Remove(file.to_path_buf()));
    }
}

fn run(rx: Receiver<WatchCmd>, out: Sender<Msg>, wake: Wake) {
    let (ev_tx, ev_rx) = unbounded();
    let noop: Wake = Arc::new(|| {});
    let mut watcher = match soyuz_fs::Watcher::new(ev_tx, Duration::from_millis(300), noop) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("editor file watching unavailable: {e}");
            while rx.recv().is_ok() {}
            return;
        }
    };
    let mut files: HashMap<PathBuf, (usize, Option<Stamp>)> = HashMap::new();
    let mut dirs: HashMap<PathBuf, soyuz_fs::WatchId> = HashMap::new();
    let mut by_id: HashMap<soyuz_fs::WatchId, PathBuf> = HashMap::new();
    loop {
        select! {
            recv(rx) -> cmd => {
                let Ok(cmd) = cmd else { return };
                match cmd {
                    WatchCmd::Add(f) => {
                        let e = files.entry(f.clone()).or_insert((0, None));
                        e.0 += 1;
                        e.1 = stamp(&f);
                        if let Some(dir) = f.parent().map(Path::to_path_buf)
                            && !dirs.contains_key(&dir)
                            && let Ok(id) = watcher.watch(&dir, false)
                        {
                            dirs.insert(dir.clone(), id);
                            by_id.insert(id, dir);
                        }
                    }
                    WatchCmd::Remove(f) => {
                        if let Some(e) = files.get_mut(&f) {
                            e.0 -= 1;
                            if e.0 == 0 {
                                files.remove(&f);
                            }
                        }
                        let dir = f.parent().map(Path::to_path_buf);
                        if let Some(dir) = dir
                            && !files.keys().any(|k| k.parent() == Some(dir.as_path()))
                            && let Some(id) = dirs.remove(&dir)
                        {
                            by_id.remove(&id);
                            watcher.unwatch(id);
                        }
                    }
                }
            }
            recv(ev_rx) -> ev => {
                let Ok(ev) = ev else { return };
                let Some(dir) = by_id.get(&ev.id).cloned() else { continue };
                let touched: HashSet<PathBuf> = match &ev.kind {
                    soyuz_fs::WatchKind::Changed(paths) => paths.iter().cloned().collect(),
                    _ => files.keys().filter(|f| f.parent() == Some(dir.as_path())).cloned().collect(),
                };
                let mut any = false;
                for (f, (_, last)) in files.iter_mut() {
                    if f.parent() != Some(dir.as_path()) || !touched.iter().any(|t| same_path(t, f)) {
                        continue;
                    }
                    let now = stamp(f);
                    if now != *last {
                        *last = now;
                        any |= out.send(Msg::Changed { path: f.clone(), stamp: now }).is_ok();
                    }
                }
                if any {
                    wake();
                }
            }
        }
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits { max_text_bytes: 1024, max_image_mp: 10 }
    }

    #[test]
    fn load_sniffs_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("a.txt");
        std::fs::write(&t, "hi\r\n").unwrap();
        assert!(matches!(load(&t, Want::Auto, &limits()), LoadResult::Text(d) if d.buf.text() == "hi\n"));
        let b = dir.path().join("a.bin");
        std::fs::write(&b, [0u8, 1, 2, 0, 0]).unwrap();
        assert!(matches!(load(&b, Want::Auto, &limits()), LoadResult::Hex { len: 5, too_large: false }));
        assert!(matches!(load(&b, Want::Text, &limits()), LoadResult::Text(_)));
        assert!(matches!(load(&t, Want::Hex, &limits()), LoadResult::Hex { .. }));
        let big = dir.path().join("big.txt");
        std::fs::write(&big, "x".repeat(2000)).unwrap();
        assert!(matches!(load(&big, Want::Auto, &limits()), LoadResult::Hex { too_large: true, .. }));
        assert!(matches!(load(dir.path(), Want::Auto, &limits()), LoadResult::Error(_)));
        let png = dir.path().join("p.png");
        image::RgbaImage::new(3, 2).save(&png).unwrap();
        assert!(matches!(load(&png, Want::Auto, &limits()), LoadResult::Image(l) if l.full.width() == 3));
        let bad = dir.path().join("bad.png");
        std::fs::write(&bad, b"not an image").unwrap();
        assert!(matches!(load(&bad, Want::Auto, &limits()), LoadResult::Hex { .. }));
    }

    #[test]
    fn save_writes_atomically_without_backup() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("a.txt");
        std::fs::write(&t, "old").unwrap();
        let (r, note) = save(&t, SaveJob::Bytes(b"new".to_vec()), false);
        assert!(r.is_ok() && note.is_none());
        assert_eq!(std::fs::read(&t).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn image_save_rotates_png() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("p.png");
        image::RgbaImage::new(3, 2).save(&png).unwrap();
        let LoadResult::Image(l) = load(&png, Want::Auto, &limits()) else { panic!() };
        let job = SaveJob::Image {
            full: l.full,
            ops: vec![ImgOp::RotateCw],
            mode: super::super::image::ops::save_mode(l.format),
            quality: 90,
        };
        let (r, _) = save(&png, job, false);
        r.unwrap();
        let back = image::open(&png).unwrap();
        assert_eq!((back.width(), back.height()), (2, 3));
    }
}
