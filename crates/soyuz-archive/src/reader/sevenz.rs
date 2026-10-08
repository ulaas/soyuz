//! 7z reader (sevenz-rust2). Solid blocks are decoded front to back: reading one entry
//! decodes the preceding entries of its block (documented cost, PLAN §9.6).

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sevenz_rust2::{Archive, BlockDecoder, EncoderMethod, Password};
use soyuz_core::{OpControl, ProgressSink};

use super::volumes::{Source, source_len};
use super::{Listing, drain_for_test, spawn_pipe, test_failure};
use crate::extract::{Extractor, OnceData};
use crate::util::{CHUNK, EntryExtra, normalize_name, read_err};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

/// p7zip stores unix modes in the high 16 bits of the attributes, flagged by this bit.
pub(crate) const UNIX_EXTENSION: u32 = 0x8000;
pub(crate) const S_IFMT: u32 = 0o170000;
pub(crate) const S_IFLNK: u32 = 0o120000;

pub(crate) struct SevenZReader {
    path: PathBuf,
    archive: Arc<Archive>,
    password: Password,
    listing: Option<Listing>,
}

pub(crate) fn sz_err(e: sevenz_rust2::Error, encrypted: bool) -> ArchiveError {
    use sevenz_rust2::Error as E;
    match e {
        E::PasswordRequired => ArchiveError::PasswordRequired,
        E::MaybeBadPassword(_) => ArchiveError::WrongPassword,
        E::ChecksumVerificationFailed | E::NextHeaderCrcMismatch if encrypted => ArchiveError::WrongPassword,
        E::Io(io, _) | E::FileOpen(io, _) => read_err(io, None, encrypted),
        E::BadSignature(_) => ArchiveError::Corrupt("not a 7z archive".into()),
        other => ArchiveError::Corrupt(other.to_string()),
    }
}

fn block_encrypted(archive: &Archive, block: usize) -> bool {
    archive
        .blocks
        .get(block)
        .is_some_and(|b| b.coders.iter().any(|c| c.encoder_method_id() == EncoderMethod::ID_AES256_SHA256))
}

impl SevenZReader {
    pub fn open(path: &Path, password: Option<&str>) -> Result<Self, ArchiveError> {
        let mut f = Source::open(path)?;
        let pw = password.map(Password::new).unwrap_or_else(Password::empty);
        let archive = Archive::read(&mut f, &pw).map_err(|e| match e {
            sevenz_rust2::Error::PasswordRequired => ArchiveError::PasswordRequired,
            sevenz_rust2::Error::FileOpen(io, _) => crate::util::fs_err("open_archive", path, &io),
            // A password was given and the (encrypted) header did not decode.
            e if password.is_some() && !matches!(e, sevenz_rust2::Error::BadSignature(_)) => {
                log::debug!("7z header decode with password failed: {e}");
                ArchiveError::WrongPassword
            }
            e => sz_err(e, false),
        })?;
        Ok(SevenZReader { path: path.to_path_buf(), archive: Arc::new(archive), password: pw, listing: None })
    }

    fn load(&mut self) -> &mut Listing {
        if self.listing.is_none() {
            let a = &self.archive;
            let mut entries = Vec::with_capacity(a.files.len());
            let mut extras = Vec::with_capacity(a.files.len());
            for (fi, f) in a.files.iter().enumerate() {
                if f.is_anti_item {
                    continue;
                }
                let (path, raw) = normalize_name(&f.name);
                if path.is_empty() {
                    continue;
                }
                let attrs = f.windows_attributes;
                let mode = (f.has_windows_attributes && attrs & UNIX_EXTENSION != 0).then_some(attrs >> 16);
                let is_dir = f.is_directory;
                let is_symlink = !is_dir && mode.is_some_and(|m| m & S_IFMT == S_IFLNK);
                let block = a.stream_map.file_block_index.get(fi).copied().flatten();
                entries.push(ArchiveEntry {
                    index: entries.len(),
                    path,
                    is_dir,
                    size: f.size,
                    compressed: f.compressed_size,
                    modified: f.has_last_modified_date.then(|| f.last_modified_date.into()),
                    crc32: f.has_crc.then_some(f.crc as u32),
                    encrypted: block.is_some_and(|b| block_encrypted(a, b)),
                    mode,
                    is_symlink,
                    link_target: None,
                });
                extras.push(EntryExtra { raw, ordinal: fi, ..Default::default() });
            }
            self.listing = Some(Listing { entries, extras, index: None });
        }
        self.listing.as_mut().expect("listing loaded")
    }

    fn file_reader(&self) -> Result<Source, ArchiveError> {
        Source::open(&self.path)
    }
}

fn threads() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| n.get() as u32).min(8)
}

impl ArchiveReader for SevenZReader {
    fn format(&self) -> ArchiveFormat {
        ArchiveFormat::SevenZ
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
        Ok(&self.load().entries)
    }

    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(Some(self.load().index()))
    }

    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        let listing = self.load();
        let e = listing.entries.get(index).ok_or_else(|| ArchiveError::Corrupt(format!("no entry #{index}")))?;
        let (encrypted, is_dir, size, target) = (e.encrypted, e.is_dir, e.size, listing.extras[index].ordinal);
        let archive = self.archive.clone();
        let Some(block) = archive.stream_map.file_block_index.get(target).copied().flatten() else {
            return Ok(Box::new(io::empty()));
        };
        if is_dir {
            return Ok(Box::new(io::empty()));
        }
        if encrypted && self.password.is_empty() {
            return Err(ArchiveError::PasswordRequired);
        }
        let mut file = self.file_reader()?;
        let password = self.password.clone();
        Ok(Box::new(spawn_pipe(move |w| {
            let first = archive.stream_map.block_first_file_index[block];
            let mut fi = first;
            let mut found = None;
            BlockDecoder::new(1, block, &archive, &password, &mut file)
                .for_each_entries(&mut |_, r: &mut dyn Read| {
                    let this = fi;
                    fi += 1;
                    if this == target {
                        found = Some(io::copy(r, w)?);
                        return Ok(false);
                    }
                    io::copy(r, &mut io::sink())?;
                    Ok(true)
                })
                .map_err(|e| sz_err(e, encrypted))?;
            match found {
                Some(n) if n == size => Ok(()),
                // Decoders can end early on garbage (wrong password) without an error.
                Some(_) if encrypted => Err(ArchiveError::WrongPassword),
                Some(n) => Err(ArchiveError::Corrupt(format!("got {n} bytes, expected {size}"))),
                None => Err(ArchiveError::Corrupt("entry missing from its block".into())),
            }
        })))
    }

    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError> {
        let archive_size = source_len(&self.path);
        self.load();
        let mut file = self.file_reader()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let archive = &*self.archive;
        let mut ex = Extractor::new(&listing.entries, &listing.extras, selection, dest, archive_size, opts, ctl, sink)?;
        if ex.wants_encrypted().is_some() && self.password.is_empty() {
            return Err(ArchiveError::PasswordRequired);
        }
        let mut idx_of: Vec<Option<usize>> = vec![None; archive.files.len()];
        for (i, x) in listing.extras.iter().enumerate() {
            idx_of[x.ordinal] = Some(i);
        }
        let wanted_file = |fi: usize, ex: &Extractor<'_>| idx_of[fi].is_some_and(|i| ex.wants(i));
        for block in 0..archive.blocks.len() {
            ctl.checkpoint()?;
            let first = archive.stream_map.block_first_file_index[block];
            let dec = BlockDecoder::new(threads(), block, archive, &self.password, &mut file);
            let count = dec.entry_count();
            if !(first..first + count).any(|fi| wanted_file(fi, &ex)) {
                continue;
            }
            let encrypted = block_encrypted(archive, block);
            let mut fatal: Option<ArchiveError> = None;
            let mut fi = first;
            let res = dec.for_each_entries(&mut |_, r: &mut dyn Read| {
                let this = fi;
                fi += 1;
                if let Some(i) = idx_of[this].filter(|&i| ex.wants(i)) {
                    let mut data = OnceData { reader: Some(&mut *r) };
                    if let Err(e) = ex.handle(i, &mut data) {
                        fatal = Some(e);
                        return Ok(false);
                    }
                }
                // Keep the block stream aligned for the next entry.
                io::copy(r, &mut io::sink())?;
                Ok(true)
            });
            if let Some(e) = fatal {
                return Err(e);
            }
            if let Err(e) = res {
                let err = sz_err(e, encrypted);
                let at = (fi.saturating_sub(1)..first + count)
                    .find_map(|f| idx_of.get(f).copied().flatten().filter(|&i| ex.wants(i)));
                match at {
                    Some(i) => ex.report(i, err)?,
                    None => return Err(err),
                }
            }
        }
        // Entries without data (directories, empty files).
        for (fi, block) in archive.stream_map.file_block_index.iter().enumerate() {
            if block.is_none()
                && let Some(i) = idx_of[fi].filter(|&i| ex.wants(i))
            {
                let mut empty: &[u8] = &[];
                let mut data = OnceData { reader: Some(&mut empty) };
                ex.handle(i, &mut data)?;
            }
        }
        Ok(ex.finish())
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        self.load();
        let mut file = self.file_reader()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let archive = &*self.archive;
        let mut report = TestReport { entries: listing.entries.len() as u64, bad: Vec::new() };
        if listing.entries.iter().any(|e| e.encrypted) && self.password.is_empty() {
            return Err(ArchiveError::PasswordRequired);
        }
        let name_of = |fi: usize| archive.files.get(fi).map(|f| f.name.clone()).unwrap_or_default();
        let mut buf = vec![0u8; CHUNK];
        for block in 0..archive.blocks.len() {
            ctl.checkpoint()?;
            let first = archive.stream_map.block_first_file_index[block];
            let encrypted = block_encrypted(archive, block);
            let mut fi = first;
            let mut fatal: Option<ArchiveError> = None;
            let res = BlockDecoder::new(threads(), block, archive, &self.password, &mut file).for_each_entries(
                &mut |entry, r: &mut dyn Read| {
                    fi += 1;
                    match drain_for_test(r, entry.size, ctl, sink, &mut buf) {
                        Ok(n) if n != entry.size && encrypted => {
                            fatal = Some(ArchiveError::WrongPassword);
                            Ok(false)
                        }
                        Ok(n) if n != entry.size => {
                            report.bad.push((entry.name.clone(), format!("size mismatch: {n} != {}", entry.size)));
                            Ok(true)
                        }
                        Ok(_) => Ok(true),
                        Err(e) => {
                            if let Err(f) = test_failure(&mut report, &entry.name, read_err(e, Some(ctl), encrypted)) {
                                fatal = Some(f);
                            }
                            Ok(false)
                        }
                    }
                },
            );
            if let Some(e) = fatal {
                return Err(e);
            }
            if let Err(e) = res {
                test_failure(&mut report, &name_of(fi.saturating_sub(1).max(first)), sz_err(e, encrypted))?;
            }
        }
        Ok(report)
    }
}
