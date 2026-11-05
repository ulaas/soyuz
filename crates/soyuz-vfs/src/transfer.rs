//! Streaming file copies between any two providers (local↔remote, remote↔remote) with
//! progress, pause / cancel checkpoints and resume of partial transfers.
//!
//! Unless the destination writes atomically (S3 multipart), data goes to a sibling
//! `.<name>.soyuzpart` file that is renamed over the final name once complete, so a
//! half-written file never appears under the real name. On a transfer *error* the
//! partial file is kept (when the destination can resume) and the next attempt
//! continues from its size (the source is read from that offset when it supports range
//! reads, otherwise the transfer starts over). On *cancel* the partial file is removed.
//!
//! A partial belongs to one state of its source: `.<name>.soyuzpartinfo` beside it records
//! the source's size and modification time, and a later attempt resumes only when both
//! still match (no note, or no modification time to compare, means starting over). Files
//! too small to be worth the extra requests are never resumed. The byte count is checked
//! before the final rename, so a source that shrank or grew mid-copy fails the transfer.

use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::provider::{Provider, RemoteEntry, WriteOpts, stat_opt};
use crate::uri::name_of;

/// Chunk size for streaming copies.
pub const CHUNK: usize = 256 * 1024;

/// Suffix of in-progress transfer files.
pub const PART_SUFFIX: &str = ".soyuzpart";

/// Suffix of the note that says which source state a partial file belongs to.
pub const PART_INFO_SUFFIX: &str = ".soyuzpartinfo";

/// Smaller files start over after a failure: resuming them saves less than the note costs.
const RESUME_MIN_SIZE: u64 = CHUNK as u64;

/// S3 writes straight to the final key and resumes only multipart uploads (8 MiB and up).
const RESUME_MIN_SIZE_DIRECT: u64 = 8 * 1024 * 1024;

/// Callbacks from a running copy.
pub trait TransferHooks {
    /// Blocks while paused; `Err` (interrupted) when cancelled.
    fn checkpoint(&self) -> FsResult<()>;
    /// `delta` new bytes; `done` / `total` of the current file.
    fn on_bytes(&self, delta: u64, done: u64, total: u64);
}

/// No-op hooks (tests, previews).
pub struct NoHooks;

impl TransferHooks for NoHooks {
    fn checkpoint(&self) -> FsResult<()> {
        Ok(())
    }
    fn on_bytes(&self, _: u64, _: u64, _: u64) {}
}

/// One file to copy.
pub struct FileCopy<'a> {
    pub src: &'a dyn Provider,
    pub src_path: &'a str,
    pub dst: &'a dyn Provider,
    pub dst_path: &'a str,
    /// Source size (for progress and resume sanity checks).
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// An existing destination file may be replaced.
    pub replace: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyStats {
    /// Bytes actually sent in this attempt.
    pub sent: u64,
    /// Bytes that were already there from an earlier attempt.
    pub resumed_from: u64,
}

/// The temp name used for `dst_path` (`dir/.name.soyuzpart`).
pub fn part_path(dst_path: &str) -> String {
    hidden_sibling(dst_path, PART_SUFFIX)
}

/// The note beside the partial file of `dst_path` (`dir/.name.soyuzpartinfo`).
pub fn part_info_path(dst_path: &str) -> String {
    hidden_sibling(dst_path, PART_INFO_SUFFIX)
}

fn hidden_sibling(dst_path: &str, suffix: &str) -> String {
    let name = name_of(dst_path);
    let dir_len = dst_path.trim_end_matches(['/', '\\']).len() - name.len();
    format!("{}.{name}{suffix}", &dst_path[..dir_len])
}

/// What a partial file's note says: the source state it was copied from. `None` when the
/// source has no modification time to tell versions apart.
fn source_stamp(source: &RemoteEntry) -> Option<String> {
    let t = source.modified?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("soyuz-part 1\nsize={}\nmodified={}.{:09}\n", source.size, t.as_secs(), t.subsec_nanos()))
}

fn read_stamp(dst: &dyn Provider, info: &str) -> Option<String> {
    let mut text = String::new();
    dst.open_read(info, 0).ok()?.take(256).read_to_string(&mut text).ok()?;
    Some(text)
}

fn write_stamp(dst: &dyn Provider, info: &str, stamp: &str) -> FsResult<()> {
    let mut w = dst.create_write(info, &WriteOpts { resume: false, size_hint: Some(stamp.len() as u64) })?;
    if let Err(e) = w.write_all(stamp.as_bytes()) {
        w.abort(false);
        return Err(FsError::from_io("write", Some(info.as_ref()), &e));
    }
    w.commit()
}

/// Record that the partial file of `dst_path` belongs to the current state of the source.
/// A failed transfer does this by itself; tests use it to set up a resume.
pub fn mark_partial(src: &dyn Provider, src_path: &str, dst: &dyn Provider, dst_path: &str) -> FsResult<()> {
    let source = src.stat(src_path)?;
    let stamp = source_stamp(&source).ok_or_else(|| FsError::unsupported("stat", Some(src_path.as_ref())))?;
    write_stamp(dst, &part_info_path(dst_path), &stamp)
}

fn already_there(path: &str) -> FsError {
    FsError::with_message(
        FsErrorKind::AlreadyExists,
        "copy",
        Some(path.as_ref()),
        "the destination appeared during the transfer",
    )
}

fn short_copy(path: &str, expected: u64, got: u64) -> FsError {
    FsError::with_message(
        FsErrorKind::Io,
        "copy",
        Some(path.as_ref()),
        format!("the file changed during the transfer: expected {expected} bytes, got {got}"),
    )
}

fn is_cancel(e: &FsError) -> bool {
    e.kind == FsErrorKind::Interrupted
}

/// Copy one file. See the module docs for temp / resume / cancel behaviour.
pub fn copy_file(fc: &FileCopy<'_>, hooks: &dyn TransferHooks) -> FsResult<CopyStats> {
    let dcaps = fc.dst.capabilities();
    let scaps = fc.src.capabilities();
    let direct = dcaps.atomic_write;
    let target = if direct { fc.dst_path.to_string() } else { part_path(fc.dst_path) };
    // The temp + rename path checks this just before the rename; a direct write has no
    // later step, and a key that appeared since the caller's check must not be overwritten.
    if direct && !fc.replace && stat_opt(fc.dst, fc.dst_path)?.is_some() {
        return Err(already_there(fc.dst_path));
    }
    // The source as it is now: the caller's size may come from an older listing.
    let source = fc.src.stat(fc.src_path).ok();
    let expected = source.as_ref().map_or(fc.size, |e| e.size);
    let info = part_info_path(fc.dst_path);
    let min_size = if direct { RESUME_MIN_SIZE_DIRECT } else { RESUME_MIN_SIZE };
    let stamp = source
        .as_ref()
        .filter(|_| dcaps.resume_write && scaps.range_read && expected >= min_size)
        .and_then(source_stamp);
    let drop_stamp = || {
        if stamp.is_some() {
            let _ = fc.dst.delete(&info, false);
        }
    };
    let resume = stamp.as_deref().is_some_and(|s| read_stamp(fc.dst, &info).as_deref() == Some(s));
    // Whether a failed attempt leaves something a later attempt may continue.
    let mut resumable = resume;
    if let (false, Some(s)) = (resume, stamp.as_deref()) {
        // A partial of another source state goes before the new note can vouch for it.
        if !direct && stat_opt(fc.dst, &target).ok().flatten().is_some() {
            let _ = fc.dst.delete(&target, false);
        }
        resumable = write_stamp(fc.dst, &info, s).is_ok();
    }
    let mut opts = WriteOpts { resume, size_hint: Some(expected) };
    let mut writer = fc.dst.create_write(&target, &opts)?;
    let mut offset = writer.resumed_at();
    if offset > expected {
        // More data than the source has: start over.
        writer.abort(false);
        opts.resume = false;
        writer = fc.dst.create_write(&target, &opts)?;
        offset = 0;
    }
    hooks.on_bytes(offset, offset, fc.size);
    let mut stats = CopyStats { sent: 0, resumed_from: offset };

    let pump = (|| -> FsResult<u64> {
        let mut reader = fc.src.open_read(fc.src_path, offset)?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = offset;
        loop {
            hooks.checkpoint()?;
            let n = reader.read(&mut buf).map_err(|e| FsError::from_io("read", Some(fc.src_path.as_ref()), &e))?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n]).map_err(|e| FsError::from_io("write", Some(target.as_ref()), &e))?;
            done += n as u64;
            stats.sent += n as u64;
            hooks.on_bytes(n as u64, done, fc.size.max(done));
        }
        Ok(done)
    })();

    let done = match pump {
        Ok(done) => done,
        Err(e) => {
            // Keep the partial for a retry unless cancelled (or nothing can resume it).
            let keep = !is_cancel(&e) && resumable;
            writer.abort(keep);
            if !keep {
                drop_stamp();
            }
            return Err(e);
        }
    };
    // A source size of 0 can mean "unknown" (some servers report none); data then counts.
    if done != expected && expected != 0 {
        writer.abort(false);
        drop_stamp();
        return Err(short_copy(fc.src_path, expected, done));
    }
    writer.commit()?;
    if !direct {
        if let Ok(Some(written)) = stat_opt(fc.dst, &target)
            && written.size != done
        {
            let _ = fc.dst.delete(&target, false);
            drop_stamp();
            return Err(short_copy(fc.dst_path, done, written.size));
        }
        if let Some(existing) = stat_opt(fc.dst, fc.dst_path)? {
            if !fc.replace {
                let _ = fc.dst.delete(&target, false);
                drop_stamp();
                return Err(already_there(fc.dst_path));
            }
            if !dcaps.rename_replaces || existing.is_dir {
                fc.dst.delete(fc.dst_path, existing.is_dir)?;
            }
        }
        fc.dst.rename(&target, fc.dst_path)?;
    }
    drop_stamp();
    if let Some(m) = fc.modified
        && dcaps.set_times
    {
        let _ = fc.dst.set_times(fc.dst_path, m);
    }
    Ok(stats)
}

/// Remove a leftover partial file of `dst_path` (after a cancelled job).
pub fn discard_partial(dst: &dyn Provider, dst_path: &str) {
    let part = part_path(dst_path);
    if stat_opt(dst, &part).ok().flatten().is_some() {
        let _ = dst.delete(&part, false);
    }
    let info = part_info_path(dst_path);
    if stat_opt(dst, &info).ok().flatten().is_some() {
        let _ = dst.delete(&info, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeProvider;
    use crate::local::LocalProvider;
    use std::cell::Cell;

    struct Counting {
        cancel_after: Option<u64>,
        seen: Cell<u64>,
    }

    impl TransferHooks for Counting {
        fn checkpoint(&self) -> FsResult<()> {
            match self.cancel_after {
                Some(n) if self.seen.get() >= n => Err(FsError::interrupted("copy")),
                _ => Ok(()),
            }
        }
        fn on_bytes(&self, delta: u64, _: u64, _: u64) {
            self.seen.set(self.seen.get() + delta);
        }
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn part_names() {
        assert_eq!(part_path("/a/b.txt"), "/a/.b.txt.soyuzpart");
        assert_eq!(part_path(r"C:\x\y.bin"), r"C:\x\.y.bin.soyuzpart");
        assert_eq!(part_path("/f"), "/.f.soyuzpart");
        assert_eq!(part_info_path("/a/b.txt"), "/a/.b.txt.soyuzpartinfo");
    }

    #[test]
    fn local_to_fake_and_back() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("in.bin");
        let bytes = data(700_000);
        std::fs::write(&src, &bytes).unwrap();
        let fake = FakeProvider::new("t1");
        fake.mkdirs("/up");
        let local = LocalProvider;
        let s = src.to_string_lossy().into_owned();
        let fc = FileCopy {
            src: &local,
            src_path: &s,
            dst: &fake,
            dst_path: "/up/in.bin",
            size: bytes.len() as u64,
            modified: None,
            replace: false,
        };
        let st = copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(st.sent, bytes.len() as u64);
        assert_eq!(fake.get("/up/in.bin").unwrap(), bytes);
        assert!(!fake.exists("/up/.in.bin.soyuzpart"), "temp renamed away");

        let back = d.path().join("back.bin");
        let b = back.to_string_lossy().into_owned();
        let fc = FileCopy {
            src: &fake,
            src_path: "/up/in.bin",
            dst: &local,
            dst_path: &b,
            size: bytes.len() as u64,
            modified: None,
            replace: false,
        };
        copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(std::fs::read(&back).unwrap(), bytes);
    }

    #[test]
    fn upload_resumes_after_dropped_connection() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("big.bin");
        let bytes = data(1_000_000);
        std::fs::write(&src, &bytes).unwrap();
        let s = src.to_string_lossy().into_owned();
        let fake = FakeProvider::new("t2");
        let local = LocalProvider;
        let fc = FileCopy {
            src: &local,
            src_path: &s,
            dst: &fake,
            dst_path: "/big.bin",
            size: bytes.len() as u64,
            modified: None,
            replace: false,
        };
        fake.fail_after(400_000);
        let e = copy_file(&fc, &NoHooks).unwrap_err();
        assert_ne!(e.kind, FsErrorKind::Interrupted);
        let partial = fake.get("/.big.bin.soyuzpart").expect("partial kept for resume");
        assert!(!partial.is_empty() && partial.len() < bytes.len());
        assert!(!fake.exists("/big.bin"));

        let before = fake.bytes_written.load(std::sync::atomic::Ordering::Relaxed);
        let st = copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(st.resumed_from, partial.len() as u64);
        assert_eq!(st.sent, (bytes.len() - partial.len()) as u64, "only the rest is re-sent");
        let after = fake.bytes_written.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(after - before, st.sent);
        assert_eq!(fake.get("/big.bin").unwrap(), bytes);
        assert!(!fake.exists("/.big.bin.soyuzpart"));
    }

    #[test]
    fn download_resumes_with_range_read() {
        let d = tempfile::tempdir().unwrap();
        let fake = FakeProvider::new("t3");
        let bytes = data(900_000);
        fake.put("/f.bin", &bytes);
        let dst = d.path().join("f.bin");
        let ds = dst.to_string_lossy().into_owned();
        let local = LocalProvider;
        let fc = FileCopy {
            src: &fake,
            src_path: "/f.bin",
            dst: &local,
            dst_path: &ds,
            size: bytes.len() as u64,
            modified: None,
            replace: false,
        };
        fake.fail_after(300_000);
        assert!(copy_file(&fc, &NoHooks).is_err());
        let part = d.path().join(".f.bin.soyuzpart");
        let have = std::fs::metadata(&part).unwrap().len();
        assert!(have > 0);
        let st = copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(st.resumed_from, have);
        assert_eq!(std::fs::read(&dst).unwrap(), bytes);
        assert!(!part.exists());
    }

    #[test]
    fn no_range_read_restarts() {
        let fake_src = FakeProvider::new("t4s");
        let mut caps = fake_src.capabilities();
        caps.range_read = false;
        fake_src.set_capabilities(caps);
        let bytes = data(200_000);
        fake_src.put("/x", &bytes);
        let fake_dst = FakeProvider::new("t4d");
        fake_dst.put("/.x.soyuzpart", &bytes[..1000]);
        let fc = FileCopy {
            src: &fake_src,
            src_path: "/x",
            dst: &fake_dst,
            dst_path: "/x",
            size: bytes.len() as u64,
            modified: None,
            replace: false,
        };
        let st = copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(st.resumed_from, 0);
        assert_eq!(fake_dst.get("/x").unwrap(), bytes);
    }

    #[test]
    fn cancel_removes_partial() {
        let fake_src = FakeProvider::new("t5s");
        let bytes = data(2_000_000);
        fake_src.put("/x", &bytes);
        let fake_dst = FakeProvider::new("t5d");
        let fc = FileCopy {
            src: &fake_src,
            src_path: "/x",
            dst: &fake_dst,
            dst_path: "/x",
            size: bytes.len() as u64,
            modified: None,
            replace: false,
        };
        let hooks = Counting { cancel_after: Some(500_000), seen: Cell::new(0) };
        let e = copy_file(&fc, &hooks).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::Interrupted);
        assert_eq!(fake_dst.paths(), vec!["/".to_string()], "nothing left behind");
    }

    /// A failed upload of `bytes`, leaving its partial and note on `dst`.
    fn fail_partway(fc: &FileCopy<'_>, dst: &FakeProvider, at: u64) {
        dst.fail_after(at);
        assert!(copy_file(fc, &NoHooks).is_err());
        dst.fail_after(u64::MAX);
    }

    #[test]
    fn a_partial_of_an_edited_source_is_not_resumed() {
        let source = FakeProvider::new("stale-src");
        let old = data(1_000_000);
        source.put("/f", &old);
        let dest = FakeProvider::new("stale-dst");
        let size = old.len() as u64;
        let fc =
            FileCopy { src: &source, src_path: "/f", dst: &dest, dst_path: "/f", size, modified: None, replace: false };
        fail_partway(&fc, &dest, 400_000);
        assert!(dest.exists("/.f.soyuzpart") && dest.exists("/.f.soyuzpartinfo"));

        // Same size, new content, new modification time: nothing of the old partial may stay.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let new: Vec<u8> = old.iter().map(|b| b ^ 0xFF).collect();
        source.put("/f", &new);
        let st = copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(st.resumed_from, 0);
        assert_eq!(dest.get("/f").unwrap(), new);
        assert!(!dest.exists("/.f.soyuzpart") && !dest.exists("/.f.soyuzpartinfo"), "{:?}", dest.paths());
    }

    #[test]
    fn a_partial_without_a_note_is_not_resumed() {
        let source = FakeProvider::new("orphan-src");
        let bytes = data(1_000_000);
        source.put("/f", &bytes);
        let dest = FakeProvider::new("orphan-dst");
        dest.put("/.f.soyuzpart", &vec![7u8; bytes.len()]);
        let size = bytes.len() as u64;
        let fc =
            FileCopy { src: &source, src_path: "/f", dst: &dest, dst_path: "/f", size, modified: None, replace: false };
        let st = copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(st.resumed_from, 0);
        assert_eq!(dest.get("/f").unwrap(), bytes, "a full-size partial of something else is not used");
    }

    #[test]
    fn small_files_keep_no_partial() {
        let source = FakeProvider::new("small-src");
        source.put("/f", &data(100_000));
        let dest = FakeProvider::new("small-dst");
        let fc = FileCopy {
            src: &source,
            src_path: "/f",
            dst: &dest,
            dst_path: "/f",
            size: 100_000,
            modified: None,
            replace: false,
        };
        fail_partway(&fc, &dest, 40_000);
        assert_eq!(dest.paths(), vec!["/".to_string()], "nothing to resume, nothing left");
    }

    /// Reads end early, the way a source shrinking mid-copy looks from here.
    struct Shrinking(FakeProvider, u64);

    impl Provider for Shrinking {
        fn describe(&self) -> String {
            "shrinking".into()
        }
        fn capabilities(&self) -> crate::provider::Capabilities {
            self.0.capabilities()
        }
        fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
            self.0.list(path)
        }
        fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
            self.0.stat(path)
        }
        fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
            Ok(Box::new(self.0.open_read(path, offset)?.take(self.1)))
        }
        fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn crate::provider::WriteStream>> {
            self.0.create_write(path, opts)
        }
        fn mkdir(&self, path: &str) -> FsResult<()> {
            self.0.mkdir(path)
        }
        fn rename(&self, from: &str, to: &str) -> FsResult<()> {
            self.0.rename(from, to)
        }
        fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
            self.0.delete(path, is_dir)
        }
    }

    #[test]
    fn a_short_read_never_commits() {
        let inner = FakeProvider::new("short-src");
        inner.put("/f", &data(500_000));
        let source = Shrinking(inner, 200_000);
        let dest = FakeProvider::new("short-dst");
        dest.put("/f", b"old");
        let fc = FileCopy {
            src: &source,
            src_path: "/f",
            dst: &dest,
            dst_path: "/f",
            size: 500_000,
            modified: None,
            replace: true,
        };
        let e = copy_file(&fc, &NoHooks).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::Io, "{e}");
        assert!(e.to_string().contains("changed during the transfer"), "{e}");
        assert_eq!(dest.get("/f").unwrap(), b"old", "the old file stays");
        assert!(!dest.exists("/.f.soyuzpart") && !dest.exists("/.f.soyuzpartinfo"));
    }

    #[test]
    fn a_grown_source_uses_its_current_size() {
        // The caller's size is from an older listing; the source as it is now is copied.
        let source = FakeProvider::new("grown-src");
        let bytes = data(500_000);
        source.put("/f", &bytes);
        let dest = FakeProvider::new("grown-dst");
        let fc = FileCopy {
            src: &source,
            src_path: "/f",
            dst: &dest,
            dst_path: "/f",
            size: 300_000,
            modified: None,
            replace: false,
        };
        copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(dest.get("/f").unwrap(), bytes);
    }

    #[test]
    fn a_direct_write_does_not_replace_an_object_that_appeared() {
        let source = FakeProvider::new("direct-src");
        source.put("/f", b"new");
        let dest = FakeProvider::new("direct-dst");
        let mut caps = dest.capabilities();
        caps.atomic_write = true;
        dest.set_capabilities(caps);
        dest.put("/f", b"old");
        let mut fc = FileCopy {
            src: &source,
            src_path: "/f",
            dst: &dest,
            dst_path: "/f",
            size: 3,
            modified: None,
            replace: false,
        };
        assert_eq!(copy_file(&fc, &NoHooks).unwrap_err().kind, FsErrorKind::AlreadyExists);
        assert_eq!(dest.get("/f").unwrap(), b"old");
        fc.replace = true;
        copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(dest.get("/f").unwrap(), b"new");
    }

    #[test]
    fn replace_rules() {
        let a = FakeProvider::new("t6a");
        a.put("/x", b"new");
        let b = FakeProvider::new("t6b");
        b.put("/x", b"old");
        let mut fc =
            FileCopy { src: &a, src_path: "/x", dst: &b, dst_path: "/x", size: 3, modified: None, replace: false };
        assert_eq!(copy_file(&fc, &NoHooks).unwrap_err().kind, FsErrorKind::AlreadyExists);
        assert_eq!(b.get("/x").unwrap(), b"old");
        assert!(!b.exists("/.x.soyuzpart"));
        fc.replace = true;
        copy_file(&fc, &NoHooks).unwrap();
        assert_eq!(b.get("/x").unwrap(), b"new");
    }
}
