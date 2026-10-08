//! RAR reader (RAR 1.3 – RAR 7, incl. encrypted data and headers) on the pure-Rust
//! `rars` decoder. `rars` decodes members front to back and writes them into
//! caller-supplied writers, so a helper thread drives the decoder and streams each wanted
//! member through a bounded pipe (no member is buffered whole).
//!
//! Solid archives decode (and discard) the members before a wanted one; non-solid ones
//! skip them. Not supported: RAR volume sets (a member split across volumes fails to
//! read). Dictionaries are limited to [`MAX_DICTIONARY`] (memory bound).

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use rars::{ArchiveMember, ArchiveMemberDetail, ArchiveReadOptions, AttrSource, ExtractionDecision, ReadCancellation};
use soyuz_core::{OpControl, ProgressSink};

use super::{Listing, PipeMsg, PipeReader, PipeWriter, drain_for_test, test_failure};
use crate::extract::{Extractor, OnceData};
use crate::util::{CHUNK, EntryExtra, fs_err, normalize_name, read_err};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

/// Largest RAR5/7 dictionary accepted (bounds decoder memory).
pub(crate) const MAX_DICTIONARY: u64 = 1 << 30;

const S_IFMT: u64 = 0o170000;
const S_IFLNK: u64 = 0o120000;

pub(crate) fn rar_err(e: &rars::Error) -> ArchiveError {
    use rars::ErrorKind as K;
    match e.kind() {
        K::PasswordRequired => ArchiveError::PasswordRequired,
        K::BadPassword => ArchiveError::WrongPassword,
        K::Cancelled => ArchiveError::Cancelled,
        K::ResourceLimit => ArchiveError::LimitExceeded(e.to_string()),
        K::UnsupportedFormat | K::UnsupportedFeature => ArchiveError::Corrupt(format!("unsupported RAR feature: {e}")),
        _ => ArchiveError::Corrupt(e.to_string()),
    }
}

fn options<'a>(password: Option<&'a [u8]>, cancel: &'a ReadCancellation) -> ArchiveReadOptions<'a> {
    ArchiveReadOptions::with_optional_password(password)
        .with_cancellation(cancel)
        .with_rar50_dictionary_size_limit(MAX_DICTIONARY)
}

fn member_name(m: &ArchiveMember) -> String {
    let raw = m.meta.name_bytes();
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_string(),
        // Legacy (non-Unicode) names: best effort, byte → char.
        Err(_) => raw.iter().map(|&b| b as char).collect(),
    }
}

fn is_solid(m: &ArchiveMember) -> bool {
    match &m.detail {
        ArchiveMemberDetail::Rar15To40 { solid, .. } => *solid,
        ArchiveMemberDetail::Rar50Plus { compression_info, .. } => compression_info & 0x40 != 0,
        _ => true,
    }
}

fn archive_solid(a: &rars::Archive) -> bool {
    if let Some(x) = a.as_rar50() {
        x.main.is_solid()
    } else if let Some(x) = a.as_rar15_40() {
        x.main.is_solid()
    } else if let Some(x) = a.as_rar13() {
        x.main.is_solid()
    } else {
        true
    }
}

/// `(target, hard link)` of a RAR5 redirection.
fn redirection(m: &ArchiveMember) -> Option<(String, bool)> {
    match &m.detail {
        ArchiveMemberDetail::Rar50Plus { redirection: Some(r), .. } => {
            Some((String::from_utf8_lossy(&r.target_name).replace('\\', "/"), r.redirection_type >= 4))
        }
        _ => None,
    }
}

/// Messages from the decoder thread.
enum Msg {
    /// Data of member `ordinal` follows on this pipe.
    Member(usize, PipeReader),
    /// Decoding finished (or failed).
    End(Result<(), ArchiveError>),
}

/// Cancels the decoder when dropped.
struct CancelOnDrop(ReadCancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Decode the archive on a helper thread, streaming the members whose ordinal is wanted.
fn spawn_decoder(
    archive: Arc<rars::Archive>,
    password: Option<Vec<u8>>,
    solid: bool,
    wanted: Vec<bool>,
) -> (Receiver<Msg>, CancelOnDrop) {
    let (tx, rx) = sync_channel::<Msg>(1);
    let cancel = ReadCancellation::new();
    let token = cancel.clone();
    let last = wanted.iter().rposition(|&w| w);
    let spawned = std::thread::Builder::new().name("soyuz-rar".into()).spawn(move || {
        let Some(last) = last else {
            let _ = tx.send(Msg::End(Ok(())));
            return;
        };
        let mut ordinal = 0usize;
        let mut current: Option<SyncSender<PipeMsg>> = None;
        let tx2 = tx.clone();
        let res = archive.extract_with_control(options(password.as_deref(), &token), |m| {
            // The previous member was decoded and verified.
            if let Some(t) = current.take() {
                let _ = t.send(PipeMsg::Done);
            }
            let k = ordinal;
            ordinal += 1;
            if k > last {
                return Ok(ExtractionDecision::Stop);
            }
            if m.meta.is_directory || m.meta.is_redirection {
                return Ok(ExtractionDecision::Skip);
            }
            if wanted.get(k).copied().unwrap_or(false) {
                let (dtx, drx) = sync_channel(4);
                let reader = PipeReader { rx: drx, cur: Vec::new(), pos: 0, done: false };
                if tx2.send(Msg::Member(k, reader)).is_err() {
                    return Ok(ExtractionDecision::Stop);
                }
                current = Some(dtx.clone());
                return Ok(ExtractionDecision::Extract(Box::new(PipeWriter { tx: dtx })));
            }
            Ok(if solid { ExtractionDecision::Extract(Box::new(io::sink())) } else { ExtractionDecision::Skip })
        });
        let res = res.map(drop).map_err(|e| rar_err(&e));
        if let Some(t) = current.take() {
            let _ = t.send(match &res {
                Ok(()) => PipeMsg::Done,
                Err(e) => PipeMsg::Err(e.clone()),
            });
        }
        let _ = tx.send(Msg::End(res));
    });
    if let Err(e) = spawned {
        log::error!("cannot spawn RAR decoder thread: {e}");
    }
    (rx, CancelOnDrop(cancel))
}

pub(crate) struct RarReader {
    path: PathBuf,
    archive: Arc<rars::Archive>,
    password: Option<Vec<u8>>,
    solid: bool,
    listing: Listing,
}

impl RarReader {
    pub fn open(path: &Path, password: Option<&str>) -> Result<Self, ArchiveError> {
        std::fs::metadata(path).map_err(|e| fs_err("open_archive", path, &e))?;
        let pw = password.map(|p| p.as_bytes().to_vec());
        let cancel = ReadCancellation::new();
        let archive =
            rars::ArchiveReader::read_path_with_options(path, options(pw.as_deref(), &cancel)).map_err(|e| {
                match rar_err(&e) {
                    ArchiveError::Corrupt(_) if e.kind() == rars::ErrorKind::Io => {
                        ArchiveError::Io(soyuz_core::FsError::with_message(
                            soyuz_core::FsErrorKind::Io,
                            "open_archive",
                            Some(path),
                            e.to_string(),
                        ))
                    }
                    other => other,
                }
            })?;
        let mut entries = Vec::new();
        let mut extras = Vec::new();
        // `rars` refuses to skip any file of a solid archive (archive-level flag).
        let mut solid = archive_solid(&archive);
        for (ordinal, m) in archive.members().enumerate() {
            solid |= !m.meta.is_directory && is_solid(&m);
            let (path, raw) = normalize_name(&member_name(&m));
            if path.is_empty() || m.meta.is_split_before {
                continue;
            }
            let unix = m.meta.attr_source() == AttrSource::Unix;
            let attr = m.meta.file_attr;
            let redirect = redirection(&m);
            let hardlink = redirect.as_ref().is_some_and(|(_, hard)| *hard);
            let is_symlink = (redirect.is_some() && !hardlink) || (unix && attr & S_IFMT == S_IFLNK);
            let is_dir = m.meta.is_directory;
            entries.push(ArchiveEntry {
                index: entries.len(),
                path,
                is_dir,
                size: if is_dir || redirect.is_some() { 0 } else { m.meta.unpacked_size },
                compressed: m.meta.packed_size,
                modified: m.meta.modification_time(),
                crc32: match &m.detail {
                    ArchiveMemberDetail::Rar15To40 { crc32, .. } => Some(*crc32),
                    ArchiveMemberDetail::Rar50Plus { crc32, .. } => *crc32,
                    _ => None,
                },
                encrypted: m.meta.is_encrypted,
                mode: unix.then_some((attr & 0o7777) as u32),
                is_symlink,
                link_target: redirect.map(|(t, _)| t),
            });
            extras.push(EntryExtra { raw, hardlink, ordinal, ..Default::default() });
        }
        Ok(RarReader {
            path: path.to_path_buf(),
            archive: Arc::new(archive),
            password: pw,
            solid,
            listing: Listing { entries, extras, index: None },
        })
    }

    /// Entries whose data comes from the decoder (files, and unix symlinks stored as data).
    fn streamed(&self, i: usize) -> bool {
        let e = &self.listing.entries[i];
        !(e.is_dir || self.listing.extras[i].hardlink || (e.is_symlink && e.link_target.is_some()))
    }

    fn wanted_ordinals(&self, pick: impl Fn(usize) -> bool) -> Vec<bool> {
        let n = self.listing.extras.iter().map(|x| x.ordinal + 1).max().unwrap_or(0);
        let mut w = vec![false; n];
        for i in 0..self.listing.entries.len() {
            if pick(i) && self.streamed(i) {
                w[self.listing.extras[i].ordinal] = true;
            }
        }
        w
    }

    fn by_ordinal(&self) -> std::collections::HashMap<usize, usize> {
        self.listing.extras.iter().enumerate().map(|(i, x)| (x.ordinal, i)).collect()
    }
}

/// Reader handed out by `open_entry`: keeps the decoder alive while it is read.
struct EntryStream {
    pipe: PipeReader,
    _rx: Receiver<Msg>,
    _cancel: CancelOnDrop,
}

impl Read for EntryStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.pipe.read(buf)
    }
}

impl ArchiveReader for RarReader {
    fn format(&self) -> ArchiveFormat {
        ArchiveFormat::Rar
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
        Ok(&self.listing.entries)
    }

    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(Some(self.listing.index()))
    }

    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        let e = self.listing.entries.get(index).ok_or_else(|| ArchiveError::Corrupt(format!("no entry #{index}")))?;
        if e.is_dir || self.listing.extras[index].hardlink {
            return Ok(Box::new(io::empty()));
        }
        if let Some(t) = e.link_target.as_ref().filter(|_| e.is_symlink) {
            return Ok(Box::new(io::Cursor::new(t.clone().into_bytes())));
        }
        if e.encrypted && self.password.is_none() {
            return Err(ArchiveError::PasswordRequired);
        }
        let wanted = self.wanted_ordinals(|i| i == index);
        let (rx, cancel) = spawn_decoder(self.archive.clone(), self.password.clone(), self.solid, wanted);
        match rx.recv() {
            Ok(Msg::Member(_, pipe)) => Ok(Box::new(EntryStream { pipe, _rx: rx, _cancel: cancel })),
            Ok(Msg::End(Err(e))) => Err(e),
            Ok(Msg::End(Ok(()))) | Err(_) => Err(ArchiveError::Corrupt("entry missing from the RAR stream".into())),
        }
    }

    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError> {
        let archive_size = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        let listing = &self.listing;
        let mut ex = Extractor::new(&listing.entries, &listing.extras, selection, dest, archive_size, opts, ctl, sink)?;
        if ex.wants_encrypted().is_some() && self.password.is_none() {
            return Err(ArchiveError::PasswordRequired);
        }
        // Directories and links with a known target need no data.
        for i in 0..listing.entries.len() {
            if ex.wants(i) && !self.streamed(i) && !listing.extras[i].hardlink {
                ex.handle(i, &mut OnceData { reader: Some(&mut io::empty()) })?;
            }
        }
        let wanted = self.wanted_ordinals(|i| ex.wants(i));
        let mut pending: Vec<usize> = (0..listing.entries.len()).filter(|&i| ex.wants(i) && self.streamed(i)).collect();
        if !pending.is_empty() {
            let by_ordinal = self.by_ordinal();
            let (rx, _cancel) = spawn_decoder(self.archive.clone(), self.password.clone(), self.solid, wanted);
            loop {
                ctl.checkpoint()?;
                match rx.recv() {
                    Ok(Msg::Member(ordinal, mut pipe)) => {
                        let Some(&i) = by_ordinal.get(&ordinal) else { continue };
                        pending.retain(|&p| p != i);
                        ex.handle(i, &mut OnceData { reader: Some(&mut pipe) })?;
                        // Let the decoder finish (and verify) the member even if it was skipped.
                        let _ = io::copy(&mut pipe, &mut io::sink());
                    }
                    Ok(Msg::End(Ok(()))) => break,
                    Ok(Msg::End(Err(ArchiveError::Cancelled))) => return Err(ArchiveError::Cancelled),
                    Ok(Msg::End(Err(e))) => {
                        // Members never delivered failed with the archive-level error.
                        if let Some(&first) = pending.first() {
                            ex.report(first, e)?;
                        }
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
        // Hard links last: their targets exist now.
        for i in 0..listing.entries.len() {
            if ex.wants(i) && listing.extras[i].hardlink {
                ex.handle(i, &mut OnceData { reader: Some(&mut io::empty()) })?;
            }
        }
        Ok(ex.finish())
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        let mut report = TestReport { entries: self.listing.entries.len() as u64, bad: Vec::new() };
        if self.listing.entries.iter().any(|e| e.encrypted) && self.password.is_none() {
            return Err(ArchiveError::PasswordRequired);
        }
        let wanted = self.wanted_ordinals(|_| true);
        let by_ordinal = self.by_ordinal();
        let (rx, _cancel) = spawn_decoder(self.archive.clone(), self.password.clone(), self.solid, wanted);
        let mut buf = vec![0u8; CHUNK];
        let mut last_path = String::new();
        loop {
            ctl.checkpoint()?;
            match rx.recv() {
                Ok(Msg::Member(ordinal, mut pipe)) => {
                    let Some(&i) = by_ordinal.get(&ordinal) else { continue };
                    let e = &self.listing.entries[i];
                    last_path = e.path.clone();
                    match drain_for_test(&mut pipe, e.size, ctl, sink, &mut buf) {
                        Ok(n) if n != e.size => {
                            report.bad.push((e.path.clone(), format!("size mismatch: {n} != {}", e.size)))
                        }
                        Ok(_) => {}
                        Err(err) => test_failure(&mut report, &e.path, read_err(err, Some(ctl), e.encrypted))?,
                    }
                }
                Ok(Msg::End(Ok(()))) | Err(_) => break,
                Ok(Msg::End(Err(e))) => {
                    if !report.bad.iter().any(|(p, _)| *p == last_path) {
                        test_failure(&mut report, &last_path, e)?;
                    }
                    break;
                }
            }
        }
        Ok(report)
    }
}
