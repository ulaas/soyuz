//! Sparse-file aware copying: only the allocated (data) ranges are copied and holes are
//! recreated by extending the destination.
//!
//! * Windows: `FSCTL_QUERY_ALLOCATED_RANGES` on the source, `FSCTL_SET_SPARSE` +
//!   `FileEndOfFileInfo` on the destination.
//! * Linux / macOS: `lseek(SEEK_DATA / SEEK_HOLE)` + `ftruncate`.

use std::fs::File;
use std::io;
use std::path::Path;

use soyuz_core::{ByteProgress, FsResult, OpControl, ProgressSink};

use crate::err;

/// Data (non-hole) ranges `(offset, length)` of `f`, which is `size` bytes long.
pub(crate) fn data_ranges(f: &File, size: u64) -> io::Result<Vec<(u64, u64)>> {
    #[cfg(windows)]
    {
        crate::sys::windows::fs::allocated_ranges(f, size)
    }
    #[cfg(unix)]
    {
        use rustix::fs::{SeekFrom, seek};
        use rustix::io::Errno;
        let mut out = Vec::new();
        let mut off = 0u64;
        while off < size {
            let data = match seek(f, SeekFrom::Data(off)) {
                Ok(d) => d,
                Err(Errno::NXIO) => break,
                Err(e) => return Err(e.into()),
            };
            let hole = seek(f, SeekFrom::Hole(data))?.min(size);
            if hole > data {
                out.push((data, hole - data));
            }
            off = hole.max(data + 1);
        }
        Ok(out)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(vec![(0, size)])
    }
}

fn read_at(f: &File, buf: &mut [u8], off: u64) -> io::Result<usize> {
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(f, buf, off)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(f, buf, off)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (f, buf, off);
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn write_all_at(f: &File, mut buf: &[u8], mut off: u64) -> io::Result<()> {
    while !buf.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_write(f, buf, off)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::write_at(f, buf, off)?;
        #[cfg(not(any(unix, windows)))]
        let n: usize = return Err(io::ErrorKind::Unsupported.into());
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

/// Copy `size` bytes of `src` to the (new, empty) `dst`, skipping holes. Progress deltas sum
/// to `size` (holes are reported as one final jump).
#[allow(clippy::too_many_arguments)]
pub(crate) fn copy_sparse(
    src: &File,
    dst: &File,
    size: u64,
    chunk: usize,
    src_path: &Path,
    dst_path: &Path,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> FsResult<()> {
    let ranges = data_ranges(src, size).map_err(err::map("copy", src_path))?;
    #[cfg(windows)]
    crate::sys::windows::fs::set_sparse(dst, true).map_err(err::map("copy", dst_path))?;
    let mut buf = vec![0u8; chunk.clamp(64 * 1024, 8 * 1024 * 1024)];
    let mut done = 0u64;
    for (start, len) in ranges {
        let mut off = start;
        let end = (start + len).min(size);
        while off < end {
            ctl.checkpoint()?;
            let want = ((end - off) as usize).min(buf.len());
            let n = read_at(src, &mut buf[..want], off).map_err(err::map("copy", src_path))?;
            if n == 0 {
                break;
            }
            write_all_at(dst, &buf[..n], off).map_err(err::map("copy", dst_path))?;
            off += n as u64;
            done += n as u64;
            sink.on_progress(ByteProgress { done, total: size, delta: n as u64 });
        }
    }
    // Recreate the trailing hole / final length.
    #[cfg(windows)]
    crate::sys::windows::fs::set_end_of_file(dst, size).map_err(err::map("copy", dst_path))?;
    #[cfg(not(windows))]
    dst.set_len(size).map_err(err::map("copy", dst_path))?;
    if done < size {
        sink.on_progress(ByteProgress { done: size, total: size, delta: size - done });
    }
    Ok(())
}

/// Heuristic: the file occupies fewer blocks than its length (unix) / has the sparse
/// attribute (Windows, decided by the caller).
#[cfg(unix)]
pub(crate) fn looks_sparse(alloc: Option<u64>, size: u64) -> bool {
    size > 0 && alloc.is_some_and(|a| a < size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_copy_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let src_p = d.path().join("sparse.bin");
        let dst_p = d.path().join("copy.bin");
        {
            let f = File::create(&src_p).unwrap();
            #[cfg(windows)]
            crate::sys::windows::fs::set_sparse(&f, true).unwrap();
            write_all_at(&f, b"head", 0).unwrap();
            write_all_at(&f, b"tail", 8 * 1024 * 1024).unwrap();
        }
        let src = File::open(&src_p).unwrap();
        let size = src.metadata().unwrap().len();
        let ranges = data_ranges(&src, size).unwrap();
        assert!(!ranges.is_empty());
        let dst = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&dst_p).unwrap();
        let mut total = 0;
        copy_sparse(&src, &dst, size, 1 << 20, &src_p, &dst_p, &OpControl::new(), &mut |p: ByteProgress| {
            total += p.delta
        })
        .unwrap();
        drop(dst);
        assert_eq!(total, size);
        let a = std::fs::read(&src_p).unwrap();
        let b = std::fs::read(&dst_p).unwrap();
        assert_eq!(a.len(), b.len());
        assert!(a == b);
    }
}
