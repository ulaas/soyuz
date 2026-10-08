//! Hex editor I/O off the UI thread: a per-document page reader, byte search over the
//! file (with the unsaved patches applied) and the patched save (temp file + rename).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::{Receiver, Sender, unbounded};

use super::model::overlay;
use crate::services::mailbox::Wake;

/// Bytes per page read from disk.
pub const PAGE: u64 = 64 * 1024;

pub struct PageReply {
    pub generation: u64,
    pub page: u64,
    pub bytes: Result<Arc<[u8]>, String>,
}

/// Reads pages of one file on its own thread (ends when the pager is dropped).
pub struct Pager {
    tx: Sender<(u64, u64)>,
    pub rx: Receiver<PageReply>,
}

impl Pager {
    pub fn new(path: PathBuf, wake: Wake) -> Self {
        let (tx, req) = unbounded::<(u64, u64)>();
        let (reply, rx) = unbounded();
        let spawned = std::thread::Builder::new().name("soyuz-hex-pager".into()).spawn(move || {
            while let Ok((generation, page)) = req.recv() {
                // Opened per request: a held handle would block the save's rename.
                let bytes = File::open(&path)
                    .and_then(|mut f| read_at(&mut f, page * PAGE, PAGE as usize))
                    .map(Arc::from)
                    .map_err(|e| e.to_string());
                if reply.send(PageReply { generation, page, bytes }).is_err() {
                    return;
                }
                wake();
            }
        });
        if let Err(e) = spawned {
            log::error!("cannot spawn the hex pager: {e}");
        }
        Pager { tx, rx }
    }

    pub fn request(&self, generation: u64, page: u64) {
        let _ = self.tx.send((generation, page));
    }
}

fn read_at(f: &mut File, off: u64, len: usize) -> std::io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(off))?;
    let mut buf = vec![0u8; len];
    let mut n = 0;
    while n < len {
        let r = f.read(&mut buf[n..])?;
        if r == 0 {
            break;
        }
        n += r;
    }
    buf.truncate(n);
    Ok(buf)
}

/// Result of a byte search: the offset of the match (`None` = not found).
pub type SearchReply = Result<Option<u64>, String>;

/// Search `needle` in the file (with `patches` applied) starting after / before `from`,
/// wrapping around. Runs on its own thread; `cancel` stops it.
#[allow(clippy::too_many_arguments)]
pub fn spawn_search(
    path: PathBuf,
    len: u64,
    patches: BTreeMap<u64, u8>,
    needle: Vec<u8>,
    from: u64,
    forward: bool,
    cancel: Arc<AtomicBool>,
    wake: Wake,
) -> Receiver<SearchReply> {
    let (tx, rx) = unbounded();
    let _ = std::thread::Builder::new().name("soyuz-hex-search".into()).spawn(move || {
        let r = search(&path, len, &patches, &needle, from, forward, &cancel);
        let _ = tx.send(r);
        wake();
    });
    rx
}

const CHUNK: u64 = 4 * 1024 * 1024;

pub fn search(
    path: &Path,
    len: u64,
    patches: &BTreeMap<u64, u8>,
    needle: &[u8],
    from: u64,
    forward: bool,
    cancel: &AtomicBool,
) -> SearchReply {
    if needle.is_empty() || needle.len() as u64 > len {
        return Ok(None);
    }
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let n = needle.len() as u64;
    let finder = memchr::memmem::Finder::new(needle);
    // Search window [a, b) of match *starts*.
    let mut scan = |a: u64, b: u64| -> Result<Option<u64>, String> {
        if forward {
            let mut s = a;
            while s < b {
                if cancel.load(Ordering::Relaxed) {
                    return Err(tr!("editors-hex-cancelled").into());
                }
                let take = CHUNK.min(b - s) + n - 1;
                let mut buf = read_at(&mut f, s, take.min(len - s) as usize).map_err(|e| e.to_string())?;
                overlay(patches, s, &mut buf);
                if let Some(i) = finder.find(&buf) {
                    let off = s + i as u64;
                    if off < b {
                        return Ok(Some(off));
                    }
                }
                s += CHUNK;
            }
        } else {
            let mut e = b;
            while e > a {
                if cancel.load(Ordering::Relaxed) {
                    return Err(tr!("editors-hex-cancelled").into());
                }
                let s = e.saturating_sub(CHUNK).max(a);
                let take = (e - s + n - 1).min(len - s);
                let mut buf = read_at(&mut f, s, take as usize).map_err(|e| e.to_string())?;
                overlay(patches, s, &mut buf);
                if let Some(i) = memchr::memmem::rfind(&buf, needle) {
                    let off = s + i as u64;
                    if off < e {
                        return Ok(Some(off));
                    }
                }
                e = s;
            }
        }
        Ok(None)
    };
    let last_start = len - n + 1;
    if forward {
        let start = (from + 1).min(last_start);
        if let Some(o) = scan(start, last_start)? {
            return Ok(Some(o));
        }
        scan(0, start.min(last_start))
    } else {
        let end = from.min(last_start);
        if let Some(o) = scan(0, end)? {
            return Ok(Some(o));
        }
        scan(end, last_start)
    }
}

/// Write the file with `patches` applied: a temp copy in the same folder, fsync, then
/// rename over the original.
pub fn save_patched(path: &Path, patches: &BTreeMap<u64, u8>) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let tmp = path.with_file_name(format!(".{name}.soyuz-save-{}", std::process::id()));
    let res = (|| -> Result<(), String> {
        let mut src = File::open(path).map_err(|e| e.to_string())?;
        let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(|e| e.to_string())?;
        let mut off = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = src.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            overlay(patches, off, &mut buf[..n]);
            dst.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            off += n as u64;
        }
        dst.sync_all().map_err(|e| e.to_string())?;
        drop(dst);
        drop(src);
        // A page read may hold the file for a moment: retry briefly.
        let mut attempt = 0;
        loop {
            match soyuz_fs::rename(&tmp, path, soyuz_fs::RenameMode::Replace) {
                Ok(()) => return Ok(()),
                Err(e) if attempt < 10 && e.kind == soyuz_fs::FsErrorKind::PermissionDenied => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_both_ways_with_patches_and_wrap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        let mut data = vec![0u8; 300_000];
        data[10..14].copy_from_slice(b"ABCD");
        data[200_000..200_004].copy_from_slice(b"ABCD");
        std::fs::write(&path, &data).unwrap();
        let len = data.len() as u64;
        let no = AtomicBool::new(false);
        let empty = BTreeMap::new();
        assert_eq!(search(&path, len, &empty, b"ABCD", 0, true, &no), Ok(Some(10)));
        assert_eq!(search(&path, len, &empty, b"ABCD", 10, true, &no), Ok(Some(200_000)));
        assert_eq!(search(&path, len, &empty, b"ABCD", 200_000, true, &no), Ok(Some(10)), "wraps");
        assert_eq!(search(&path, len, &empty, b"ABCD", 200_000, false, &no), Ok(Some(10)));
        assert_eq!(search(&path, len, &empty, b"ABCD", 10, false, &no), Ok(Some(200_000)), "wraps back");
        // Unsaved patches are searched too.
        let mut p = BTreeMap::new();
        p.insert(50u64, b'Z');
        assert_eq!(search(&path, len, &p, b"Z", 0, true, &no), Ok(Some(50)));
        assert_eq!(search(&path, len, &empty, b"nope", 0, true, &no), Ok(None));
    }

    #[test]
    fn patched_save_replaces_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let mut p = BTreeMap::new();
        p.insert(0u64, b'J');
        p.insert(10u64, b'D');
        save_patched(&path, &p).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"Jello worlD");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp file left behind");
    }
}
