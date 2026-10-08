//! Shared plumbing for random-access readers (ISO / UDF, CAB, RAR single entries): each
//! entry can be opened on its own, so extraction and testing just open every wanted
//! entry in archive order.

use std::io::{self, Read};

use soyuz_core::{OpControl, ProgressSink};

use super::{Listing, drain_for_test, test_failure};
use crate::extract::{EntryData, Extractor};
use crate::util::{CHUNK, read_err};
use crate::{ArchiveEntry, ArchiveError, ExtractOptions, ExtractSink, ExtractSummary, TestReport};

/// Opens the data of entry `idx` (never called for directories or symlinks with a known
/// target).
pub(crate) trait EntrySource: Send {
    fn open_index(&mut self, idx: usize, entry: &ArchiveEntry) -> Result<Box<dyn Read + Send + '_>, ArchiveError>;
}

struct SrcData<'a> {
    src: &'a mut dyn EntrySource,
    idx: usize,
    entry: &'a ArchiveEntry,
}

impl EntryData for SrcData<'_> {
    fn open(&mut self) -> Result<Box<dyn Read + '_>, ArchiveError> {
        Ok(self.src.open_index(self.idx, self.entry)?)
    }
    fn can_reopen(&self) -> bool {
        true
    }
}

/// [`crate::ArchiveReader::open_entry`] for a random-access source.
pub(crate) fn open_entry<'a>(
    src: &'a mut dyn EntrySource,
    listing: &'a Listing,
    index: usize,
) -> Result<Box<dyn Read + Send + 'a>, ArchiveError> {
    let e = listing.entries.get(index).ok_or_else(|| ArchiveError::Corrupt(format!("no entry #{index}")))?;
    if e.is_dir || listing.extras[index].hardlink {
        return Ok(Box::new(io::empty()));
    }
    if e.is_symlink
        && let Some(t) = &e.link_target
    {
        return Ok(Box::new(io::Cursor::new(t.clone().into_bytes())));
    }
    src.open_index(index, e)
}

/// [`crate::ArchiveReader::extract`] for a random-access source.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract(
    src: &mut dyn EntrySource,
    listing: &Listing,
    archive_size: u64,
    selection: Option<&[usize]>,
    dest: &std::path::Path,
    opts: &ExtractOptions,
    ctl: &OpControl,
    sink: &mut dyn ExtractSink,
) -> Result<ExtractSummary, ArchiveError> {
    let mut ex = Extractor::new(&listing.entries, &listing.extras, selection, dest, archive_size, opts, ctl, sink)?;
    for (i, e) in listing.entries.iter().enumerate() {
        if !ex.wants(i) {
            continue;
        }
        let mut data = SrcData { src: &mut *src, idx: i, entry: e };
        ex.handle(i, &mut data)?;
    }
    Ok(ex.finish())
}

/// [`crate::ArchiveReader::test`] for a random-access source: reads every file entry and
/// checks its length.
pub(crate) fn test(
    src: &mut dyn EntrySource,
    listing: &Listing,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<TestReport, ArchiveError> {
    let mut report = TestReport::default();
    let mut buf = vec![0u8; CHUNK];
    for (i, e) in listing.entries.iter().enumerate() {
        ctl.checkpoint()?;
        report.entries += 1;
        if e.is_dir || e.is_symlink || listing.extras[i].hardlink {
            continue;
        }
        let res = src.open_index(i, e).and_then(|mut r| {
            drain_for_test(&mut r, e.size, ctl, sink, &mut buf).map_err(|err| read_err(err, Some(ctl), e.encrypted))
        });
        match res {
            Ok(n) if n != e.size => report.bad.push((e.path.clone(), format!("size mismatch: {n} != {}", e.size))),
            Ok(_) => {}
            Err(err) => test_failure(&mut report, &e.path, err)?,
        }
    }
    Ok(report)
}
