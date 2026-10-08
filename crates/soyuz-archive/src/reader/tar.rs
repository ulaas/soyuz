//! tar / tar.gz / tar.bz2 / tar.xz / tar.zst reader (sequential stream).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use soyuz_core::{OpControl, ProgressSink};
use tar::EntryType;

use super::{Listing, drain_for_test, open_file, test_failure};
use crate::extract::{Extractor, OnceData};
use crate::util::{CHUNK, EntryExtra, normalize_name, read_err, unix_secs_to_time};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

pub(crate) struct TarReader {
    path: PathBuf,
    format: ArchiveFormat,
    listing: Option<Listing>,
}

/// Decompressed byte stream of a tar.* file.
pub(crate) fn decoder(path: &Path, format: ArchiveFormat) -> Result<Box<dyn Read + Send>, ArchiveError> {
    let f = super::open_file_with(path, CHUNK)?;
    Ok(match format {
        ArchiveFormat::TarGz => Box::new(flate2::read::MultiGzDecoder::new(f)),
        ArchiveFormat::TarBz2 => Box::new(bzip2::read::MultiBzDecoder::new(f)),
        ArchiveFormat::TarXz => Box::new(lzma_rust2::XzReader::new(f, true)),
        #[cfg(feature = "zstd")]
        ArchiveFormat::TarZst => Box::new(zstd::stream::read::Decoder::with_buffer(f).map_err(ArchiveError::from)?),
        #[cfg(not(feature = "zstd"))]
        ArchiveFormat::TarZst => Box::new(super::zst::ZstdReader::new(f)),
        _ => Box::new(f),
    })
}

enum Kind {
    File,
    Dir,
    Symlink,
    Hardlink,
}

fn kind_of(t: EntryType) -> Option<Kind> {
    match t {
        EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => Some(Kind::File),
        EntryType::Directory => Some(Kind::Dir),
        EntryType::Symlink => Some(Kind::Symlink),
        EntryType::Link => Some(Kind::Hardlink),
        // Devices, FIFOs, pax / GNU meta entries are not listed.
        _ => None,
    }
}

fn describe(e: &mut tar::Entry<'_, impl Read>, ordinal: usize) -> io::Result<Option<(ArchiveEntry, EntryExtra)>> {
    let Some(kind) = kind_of(e.header().entry_type()) else { return Ok(None) };
    let raw_name = String::from_utf8_lossy(&e.path_bytes()).into_owned();
    let (path, raw) = normalize_name(&raw_name);
    if path.is_empty() {
        return Ok(None);
    }
    let h = e.header();
    let link = e.link_name_bytes().map(|b| String::from_utf8_lossy(&b).into_owned());
    let is_dir = matches!(kind, Kind::Dir) || (matches!(kind, Kind::File) && raw_name.ends_with('/'));
    let size = if matches!(kind, Kind::File) && !is_dir { e.size() } else { 0 };
    let entry = ArchiveEntry {
        index: 0,
        path,
        is_dir,
        size,
        compressed: size,
        modified: h.mtime().ok().and_then(|s| unix_secs_to_time(s as i64)),
        crc32: None,
        encrypted: false,
        mode: h.mode().ok(),
        is_symlink: matches!(kind, Kind::Symlink),
        link_target: if matches!(kind, Kind::Symlink | Kind::Hardlink) { link } else { None },
    };
    let extra = EntryExtra {
        raw,
        hardlink: matches!(kind, Kind::Hardlink),
        sparse: h.entry_type() == EntryType::GNUSparse,
        data_pos: e.raw_file_position(),
        ordinal,
    };
    Ok(Some((entry, extra)))
}

/// [`describe`] for other readers that embed a tar stream (.deb payloads).
pub(crate) fn describe_entry(
    e: &mut tar::Entry<'_, impl Read>,
    ordinal: usize,
) -> io::Result<Option<(ArchiveEntry, EntryExtra)>> {
    describe(e, ordinal)
}

fn tar_err(e: io::Error, ctl: Option<&OpControl>) -> ArchiveError {
    read_err(e, ctl, false)
}

impl TarReader {
    pub fn open(path: &Path, format: ArchiveFormat) -> Result<Self, ArchiveError> {
        // List eagerly: validates the stream (a tar has no central directory to check).
        let mut r = TarReader { path: path.to_path_buf(), format, listing: None };
        r.load()?;
        Ok(r)
    }

    fn load(&mut self) -> Result<&mut Listing, ArchiveError> {
        if self.listing.is_none() {
            let mut entries = Vec::new();
            let mut extras = Vec::new();
            let mut push = |item: Option<(ArchiveEntry, EntryExtra)>| {
                if let Some((mut e, x)) = item {
                    e.index = entries.len();
                    entries.push(e);
                    extras.push(x);
                }
            };
            if self.format == ArchiveFormat::Tar {
                let mut ar = tar::Archive::new(open_file(&self.path)?);
                for (ordinal, e) in ar.entries_with_seek().map_err(|e| tar_err(e, None))?.enumerate() {
                    let mut e = e.map_err(|e| tar_err(e, None))?;
                    push(describe(&mut e, ordinal).map_err(|e| tar_err(e, None))?);
                }
            } else {
                let mut ar = tar::Archive::new(decoder(&self.path, self.format)?);
                for (ordinal, e) in ar.entries().map_err(|e| tar_err(e, None))?.enumerate() {
                    let mut e = e.map_err(|e| tar_err(e, None))?;
                    push(describe(&mut e, ordinal).map_err(|e| tar_err(e, None))?);
                }
            }
            self.listing = Some(Listing { entries, extras, index: None });
        }
        Ok(self.listing.as_mut().expect("listing loaded"))
    }

    /// Sparse entries: stream through the archive with the tar crate (handles holes).
    fn open_sparse(&self, ordinal: usize, size: u64) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        let dec = decoder(&self.path, self.format)?;
        Ok(Box::new(super::spawn_pipe(move |w| {
            let mut ar = tar::Archive::new(dec);
            for (i, e) in ar.entries().map_err(|e| tar_err(e, None))?.enumerate() {
                let mut e = e.map_err(|e| tar_err(e, None))?;
                if i == ordinal {
                    io::copy(&mut e.by_ref().take(size), w).map_err(|e| tar_err(e, None))?;
                    return Ok(());
                }
            }
            Err(ArchiveError::Corrupt("entry missing from the stream".into()))
        })))
    }
}

impl ArchiveReader for TarReader {
    fn format(&self) -> ArchiveFormat {
        self.format
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
        Ok(&self.load()?.entries)
    }

    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(Some(self.load()?.index()))
    }

    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        self.load()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let e = listing.entries.get(index).ok_or_else(|| ArchiveError::Corrupt(format!("no entry #{index}")))?;
        let x = &listing.extras[index];
        if e.is_dir || x.hardlink {
            return Ok(Box::new(io::empty()));
        }
        if e.is_symlink {
            let t = e.link_target.clone().unwrap_or_default().into_bytes();
            return Ok(Box::new(io::Cursor::new(t)));
        }
        let size = e.size;
        if x.sparse {
            return self.open_sparse(x.ordinal, size);
        }
        if self.format == ArchiveFormat::Tar {
            let mut f: BufReader<File> = open_file(&self.path)?;
            f.seek(SeekFrom::Start(x.data_pos))?;
            return Ok(Box::new(f.take(size)));
        }
        // Compressed: decode and discard everything before the entry data.
        let mut d = decoder(&self.path, self.format)?;
        let skipped = io::copy(&mut (&mut d).take(x.data_pos), &mut io::sink()).map_err(|e| tar_err(e, None))?;
        if skipped != x.data_pos {
            return Err(ArchiveError::Corrupt("unexpected end of tar stream".into()));
        }
        Ok(Box::new(d.take(size)))
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
        self.load()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let mut ex = Extractor::new(&listing.entries, &listing.extras, selection, dest, archive_size, opts, ctl, sink)?;
        // ordinal → entry index (entries are in ordinal order).
        let mut next = 0usize;
        let mut ar = tar::Archive::new(decoder(&self.path, self.format)?);
        let last_wanted = (0..listing.entries.len()).rev().find(|&i| ex.wants(i));
        let Some(last_wanted) = last_wanted else { return Ok(ex.finish()) };
        for (ordinal, item) in ar.entries().map_err(|e| tar_err(e, Some(ctl)))?.enumerate() {
            ctl.checkpoint()?;
            let mut te = match item {
                Ok(te) => te,
                Err(e) => {
                    let idx = next.min(listing.entries.len() - 1);
                    ex.report(idx, tar_err(e, Some(ctl)))?;
                    break;
                }
            };
            if next >= listing.entries.len() || listing.extras[next].ordinal != ordinal {
                continue;
            }
            let idx = next;
            next += 1;
            if ex.wants(idx) {
                let mut data = OnceData { reader: Some(&mut te) };
                ex.handle(idx, &mut data)?;
            }
            if idx >= last_wanted {
                break;
            }
        }
        // Full extraction: decode to the end so a codec checksum failure is not silent.
        if selection.is_none() && self.format != ArchiveFormat::Tar {
            let mut rest = ar.into_inner();
            if let Err(e) = io::copy(&mut rest, &mut io::sink()) {
                let err = match tar_err(e, Some(ctl)) {
                    ArchiveError::Corrupt(m) => ArchiveError::Corrupt(format!("{m} (extracted data may be damaged)")),
                    other => other,
                };
                ex.report(last_wanted, err)?;
            }
        }
        Ok(ex.finish())
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        self.load()?;
        let listing = self.listing.as_ref().expect("listing loaded");
        let mut report = TestReport::default();
        let mut buf = vec![0u8; CHUNK];
        let mut next = 0usize;
        let mut ar = tar::Archive::new(decoder(&self.path, self.format)?);
        let entries = match ar.entries() {
            Ok(it) => it,
            Err(e) => {
                report.bad.push((String::new(), tar_err(e, Some(ctl)).to_string()));
                return Ok(report);
            }
        };
        for (ordinal, item) in entries.enumerate() {
            ctl.checkpoint()?;
            let mut te = match item {
                Ok(te) => te,
                Err(e) => {
                    let at = listing.entries.get(next).map(|e| e.path.clone()).unwrap_or_default();
                    test_failure(&mut report, &at, tar_err(e, Some(ctl)))?;
                    break;
                }
            };
            let is_listed = next < listing.entries.len() && listing.extras[next].ordinal == ordinal;
            let (path, size) = if is_listed {
                let e = &listing.entries[next];
                next += 1;
                report.entries += 1;
                (e.path.clone(), e.size)
            } else {
                (String::from_utf8_lossy(&te.path_bytes()).into_owned(), te.size())
            };
            match drain_for_test(&mut te, size, ctl, sink, &mut buf) {
                Ok(n) if is_listed && n != size => report.bad.push((path, format!("size mismatch: {n} != {size}"))),
                Ok(_) => {}
                Err(e) => {
                    test_failure(&mut report, &path, tar_err(e, Some(ctl)))?;
                    break;
                }
            }
        }
        if next < listing.entries.len() {
            report.bad.push((listing.entries[next].path.clone(), "missing from the stream".into()));
        }
        // Decode to the very end so the codec trailer (gzip CRC, xz / zstd checksums) is verified.
        let mut rest = ar.into_inner();
        if let Err(e) = io::copy(&mut rest, &mut io::sink()) {
            test_failure(&mut report, "", tar_err(e, Some(ctl)))?;
        }
        Ok(report)
    }
}
