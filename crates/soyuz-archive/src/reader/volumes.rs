//! Byte-split multi-volume archives (`name.7z.001`, `name.7z.002`, … and `name.zip.001`, …,
//! as written by 7-Zip's "split to volumes"): the parts are concatenated into one seekable
//! stream, so the zip / 7z readers work unchanged.
//!
//! Not covered: PKWARE spanned / split zip (`.z01` … `.zip`), whose central directory
//! offsets are per disk (the `zip` crate has no multi-disk support), and RAR volume sets.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::ArchiveError;
use crate::util::fs_err;

/// Most volumes looked for (`.001` … `.999`).
const MAX_PARTS: u32 = 999;

/// The volumes of `path` when it names the first part of a byte-split set (`*.001`), in
/// order (stops at the first missing number). `None` for ordinary files.
pub(crate) fn split_parts(path: &Path) -> Option<Vec<PathBuf>> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".001")?;
    if stem.is_empty() {
        return None;
    }
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut parts = vec![path.to_path_buf()];
    for n in 2..=MAX_PARTS {
        let p = dir.join(format!("{stem}.{n:03}"));
        if !p.is_file() {
            break;
        }
        parts.push(p);
    }
    Some(parts)
}

/// Seekable archive bytes: one file, or the concatenation of split volumes.
pub(crate) enum Source {
    File(BufReader<File>),
    Split(SplitReader),
}

impl Source {
    /// Open `path` (split volumes when it is a `*.001` part).
    pub fn open(path: &Path) -> Result<Source, ArchiveError> {
        match split_parts(path) {
            Some(parts) => Ok(Source::Split(SplitReader::open(&parts)?)),
            None => Ok(Source::File(super::open_file(path)?)),
        }
    }
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Source::File(f) => f.read(buf),
            Source::Split(s) => s.read(buf),
        }
    }
}

impl Seek for Source {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Source::File(f) => f.seek(pos),
            Source::Split(s) => s.seek(pos),
        }
    }
}

/// Total size of `path`, or of all its split volumes.
pub(crate) fn source_len(path: &Path) -> u64 {
    match split_parts(path) {
        Some(parts) => parts.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum(),
        None => std::fs::metadata(path).map_or(0, |m| m.len()),
    }
}

/// Read + Seek over several files laid end to end.
pub(crate) struct SplitReader {
    parts: Vec<(BufReader<File>, u64)>,
    /// Start offset of each part in the joined stream.
    starts: Vec<u64>,
    total: u64,
    pos: u64,
    /// Part the underlying file cursor is positioned for (`None` = must seek).
    cur: Option<usize>,
}

impl SplitReader {
    pub fn open(paths: &[PathBuf]) -> Result<SplitReader, ArchiveError> {
        let mut parts = Vec::with_capacity(paths.len());
        let mut starts = Vec::with_capacity(paths.len());
        let mut total = 0u64;
        for p in paths {
            let f = File::open(p).map_err(|e| fs_err("open_archive", p, &e))?;
            let len = f.metadata().map_err(|e| fs_err("open_archive", p, &e))?.len();
            starts.push(total);
            total += len;
            parts.push((BufReader::with_capacity(4 * 1024, f), len));
        }
        Ok(SplitReader { parts, starts, total, pos: 0, cur: None })
    }

    fn part_at(&self, pos: u64) -> usize {
        match self.starts.binary_search(&pos) {
            Ok(i) => {
                // Skip empty parts that start at the same offset.
                let mut i = i;
                while i + 1 < self.parts.len() && self.parts[i].1 == 0 {
                    i += 1;
                }
                i
            }
            Err(i) => i.saturating_sub(1),
        }
    }
}

impl Read for SplitReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.total {
            return Ok(0);
        }
        let i = self.part_at(self.pos);
        let within = self.pos - self.starts[i];
        if self.cur != Some(i) {
            self.parts[i].0.seek(SeekFrom::Start(within))?;
            self.cur = Some(i);
        }
        let left = self.parts[i].1 - within;
        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = self.parts[i].0.read(&mut buf[..want])?;
        if n == 0 && want > 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "volume is shorter than expected"));
        }
        self.pos += n as u64;
        if n as u64 == left {
            // Next read starts at the beginning of the next part.
            self.cur = None;
        }
        Ok(n)
    }
}

impl Seek for SplitReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.total.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        let target = target.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        if target != self.pos {
            self.pos = target;
            self.cur = None;
        }
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_parts() {
        let d = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        for (i, chunk) in data.chunks(300).enumerate() {
            std::fs::write(d.path().join(format!("a.7z.{:03}", i + 1)), chunk).unwrap();
        }
        let first = d.path().join("a.7z.001");
        let parts = split_parts(&first).unwrap();
        assert_eq!(parts.len(), 4);
        assert_eq!(source_len(&first), 1000);
        let mut r = SplitReader::open(&parts).unwrap();
        let mut all = Vec::new();
        r.read_to_end(&mut all).unwrap();
        assert_eq!(all, data);
        r.seek(SeekFrom::Start(295)).unwrap();
        let mut b = [0u8; 10];
        r.read_exact(&mut b).unwrap();
        assert_eq!(&b, &data[295..305]);
        r.seek(SeekFrom::End(-5)).unwrap();
        let mut tail = Vec::new();
        r.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, &data[995..]);
        assert!(split_parts(&d.path().join("a.7z")).is_none());
    }
}
