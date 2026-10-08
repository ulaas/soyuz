//! Streaming file copies between any two providers (local↔remote, remote↔remote) with
//! progress, pause / cancel checkpoints and resume of partial transfers.
//!
//! Unless the destination writes atomically (S3 multipart), data goes to a sibling
//! `.<name>.soyuzpart` file that is renamed over the final name once complete, so a
//! half-written file never appears under the real name. On a transfer *error* the
//! partial file is kept (when the destination can resume) and the next attempt
//! continues from its size (the source is read from that offset when it supports range
//! reads, otherwise the transfer starts over). On *cancel* the partial file is removed.

use std::io::{Read, Write};
use std::time::SystemTime;

use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::provider::{Provider, WriteOpts, stat_opt};
use crate::uri::name_of;

/// Chunk size for streaming copies.
pub const CHUNK: usize = 256 * 1024;

/// Suffix of in-progress transfer files.
pub const PART_SUFFIX: &str = ".soyuzpart";

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
    let name = name_of(dst_path);
    let dir_len = dst_path.trim_end_matches(['/', '\\']).len() - name.len();
    format!("{}.{name}{PART_SUFFIX}", &dst_path[..dir_len])
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
    let mut opts = WriteOpts { resume: dcaps.resume_write, size_hint: Some(fc.size) };
    let mut writer = fc.dst.create_write(&target, &opts)?;
    let mut offset = writer.resumed_at();
    if offset > 0 && (offset > fc.size || !scaps.range_read) {
        // Stale / oversized partial, or the source can't seek: start over.
        writer.abort(false);
        opts.resume = false;
        writer = fc.dst.create_write(&target, &opts)?;
        offset = 0;
    }
    hooks.on_bytes(offset, offset, fc.size);
    let mut stats = CopyStats { sent: 0, resumed_from: offset };

    let pump = (|| -> FsResult<()> {
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
        Ok(())
    })();

    if let Err(e) = pump {
        // Keep the partial for a retry unless cancelled (or nothing can resume it).
        writer.abort(!is_cancel(&e) && dcaps.resume_write);
        return Err(e);
    }
    writer.commit()?;
    if !direct {
        if let Some(existing) = stat_opt(fc.dst, fc.dst_path)? {
            if !fc.replace {
                let _ = fc.dst.delete(&target, false);
                return Err(FsError::with_message(
                    FsErrorKind::AlreadyExists,
                    "copy",
                    Some(fc.dst_path.as_ref()),
                    "the destination appeared during the transfer",
                ));
            }
            if !dcaps.rename_replaces || existing.is_dir {
                fc.dst.delete(fc.dst_path, existing.is_dir)?;
            }
        }
        fc.dst.rename(&target, fc.dst_path)?;
    }
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
