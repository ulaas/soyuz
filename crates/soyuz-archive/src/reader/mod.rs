//! Archive readers (zip, tar.*, 7z, RAR, ISO / UDF, CAB, deb, rpm).

mod cab;
mod iso;
mod pkg;
mod random;
#[cfg(feature = "rar")]
mod rar;
mod seq;
mod sevenz;
mod tar;
mod udf;
pub(crate) mod volumes;
mod zip;
pub(crate) mod zst;

pub(crate) use sevenz::{S_IFLNK, UNIX_EXTENSION, sz_err};
pub(crate) use zip::zip_err;

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use soyuz_core::{ByteProgress, OpControl, ProgressSink};

use crate::util::{EntryExtra, fs_err};
use crate::{ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, TestReport};

/// Open `path` as `format`.
pub(crate) fn open(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    let password = password.filter(|p| !p.is_empty());
    match format {
        ArchiveFormat::Zip => Ok(Box::new(zip::ZipReader::open(path, password)?)),
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => Ok(Box::new(tar::TarReader::open(path, format)?)),
        ArchiveFormat::SevenZ => Ok(Box::new(sevenz::SevenZReader::open(path, password)?)),
        #[cfg(feature = "rar")]
        ArchiveFormat::Rar => Ok(Box::new(rar::RarReader::open(path, password)?)),
        ArchiveFormat::Iso => Ok(Box::new(iso::IsoReader::open(path)?)),
        ArchiveFormat::Cab => Ok(Box::new(cab::CabReader::open(path)?)),
        ArchiveFormat::Deb | ArchiveFormat::Rpm => Ok(Box::new(pkg::PkgReader::open(path, format)?)),
    }
}

/// Small buffer: `BufReader::seek` discards the whole buffer, and zip / 7z / plain tar
/// listing seeks once per entry. Large reads bypass the buffer anyway.
pub(crate) fn open_file(path: &Path) -> Result<BufReader<File>, ArchiveError> {
    open_file_with(path, 4 * 1024)
}

/// Large buffer for purely sequential reads (compressed streams).
pub(crate) fn open_file_with(path: &Path, capacity: usize) -> Result<BufReader<File>, ArchiveError> {
    let f = File::open(path).map_err(|e| fs_err("open_archive", path, &e))?;
    Ok(BufReader::with_capacity(capacity, f))
}

/// Cached listing shared by every reader.
#[derive(Default)]
pub(crate) struct Listing {
    pub entries: Vec<ArchiveEntry>,
    pub extras: Vec<EntryExtra>,
    pub index: Option<DirIndex>,
}

impl Listing {
    pub fn index(&mut self) -> &DirIndex {
        self.index.get_or_insert_with(|| DirIndex::build(&self.entries))
    }
}

/// Drain `r` for an integrity test, reporting progress. Returns bytes read.
pub(crate) fn drain_for_test(
    r: &mut dyn Read,
    total: u64,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
    buf: &mut [u8],
) -> Result<u64, io::Error> {
    let mut done = 0u64;
    loop {
        if ctl.checkpoint().is_err() {
            return Err(crate::util::cancelled_io());
        }
        let n = match r.read(buf) {
            Ok(0) => return Ok(done),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted && !ctl.is_cancelled() => continue,
            Err(e) => return Err(e),
        };
        done += n as u64;
        sink.on_progress(ByteProgress { done, total, delta: n as u64 });
    }
}

enum PipeMsg {
    Data(Vec<u8>),
    Err(ArchiveError),
    Done,
}

/// Write half of [`spawn_pipe`].
pub(crate) struct PipeWriter {
    tx: std::sync::mpsc::SyncSender<PipeMsg>,
}

impl io::Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.tx
            .send(PipeMsg::Data(buf.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "reader dropped"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Read half of [`spawn_pipe`]: streams what the producer thread writes (bounded memory).
pub(crate) struct PipeReader {
    rx: std::sync::mpsc::Receiver<PipeMsg>,
    cur: Vec<u8>,
    pos: usize,
    done: bool,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.cur.len() {
            if self.done {
                return Ok(0);
            }
            match self.rx.recv() {
                Ok(PipeMsg::Data(d)) => {
                    self.cur = d;
                    self.pos = 0;
                }
                Ok(PipeMsg::Done) => self.done = true,
                Ok(PipeMsg::Err(e)) => {
                    self.done = true;
                    return Err(io::Error::other(e));
                }
                Err(_) => {
                    self.done = true;
                    return Err(io::Error::other(ArchiveError::Corrupt("decoder thread stopped".into())));
                }
            }
        }
        let n = buf.len().min(self.cur.len() - self.pos);
        buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Run `produce` on a helper thread and stream its output. Used where a library only
/// offers callback-style decoding (7z blocks, sparse tar entries); dropping the reader
/// makes the producer's next write fail, which stops the thread.
pub(crate) fn spawn_pipe<F>(produce: F) -> PipeReader
where
    F: FnOnce(&mut PipeWriter) -> Result<(), ArchiveError> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(4);
    std::thread::Builder::new()
        .name("soyuz-archive-pipe".into())
        .spawn(move || {
            let mut w = PipeWriter { tx };
            let msg = match produce(&mut w) {
                Ok(()) => PipeMsg::Done,
                Err(e) => PipeMsg::Err(e),
            };
            let _ = w.tx.send(msg);
        })
        .map(drop)
        .unwrap_or_else(|e| log::error!("cannot spawn archive pipe thread: {e}"));
    PipeReader { rx, cur: Vec::new(), pos: 0, done: false }
}

/// Record a per-entry test failure; password problems and cancellation abort the test.
pub(crate) fn test_failure(report: &mut TestReport, path: &str, err: ArchiveError) -> Result<(), ArchiveError> {
    match err {
        ArchiveError::Cancelled | ArchiveError::PasswordRequired | ArchiveError::WrongPassword => Err(err),
        e => {
            report.bad.push((path.to_string(), e.to_string()));
            Ok(())
        }
    }
}
