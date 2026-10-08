//! Shared plumbing for readers whose entry data lives in a few sequential (compressed)
//! streams: CAB folders, the payload of a .deb / .rpm. Each entry records its stream in
//! `EntryExtra::ordinal` and its byte offset inside the decoded stream in
//! `EntryExtra::data_pos`.
//!
//! Opening one entry decodes its stream up to the entry (documented cost). Extraction and
//! testing walk every stream once, front to back.

use std::io::{self, Read};

use soyuz_core::{OpControl, ProgressSink};

use super::{Listing, drain_for_test, test_failure};
use crate::extract::{Extractor, OnceData};
use crate::util::{CHUNK, read_err};
use crate::{ArchiveError, ExtractOptions, ExtractSink, ExtractSummary, TestReport};

/// `EntryExtra::ordinal` of an entry whose data cannot be read (e.g. a CAB file that
/// continues in another cabinet).
pub(crate) const NO_STREAM: usize = usize::MAX;

/// Opens decoded stream `stream` from its start.
pub(crate) trait StreamSource: Send {
    fn open_stream(&self, stream: usize) -> Result<Box<dyn Read + Send>, ArchiveError>;
}

/// Discard exactly `n` bytes of `r`.
fn skip(r: &mut dyn Read, n: u64) -> Result<(), ArchiveError> {
    let got = io::copy(&mut r.take(n), &mut io::sink()).map_err(|e| read_err(e, None, false))?;
    if got != n {
        return Err(ArchiveError::Corrupt("unexpected end of the archive stream".into()));
    }
    Ok(())
}

/// Entries that carry data (not directories, links with a known target or hard links).
fn has_data(listing: &Listing, i: usize) -> bool {
    let e = &listing.entries[i];
    !(e.is_dir || listing.extras[i].hardlink || (e.is_symlink && e.link_target.is_some()))
}

/// [`crate::ArchiveReader::open_entry`]: decode the entry's stream up to its data.
pub(crate) fn open_entry(
    src: &dyn StreamSource,
    listing: &Listing,
    index: usize,
) -> Result<Box<dyn Read + Send>, ArchiveError> {
    let e = listing.entries.get(index).ok_or_else(|| ArchiveError::Corrupt(format!("no entry #{index}")))?;
    if e.is_dir || listing.extras[index].hardlink {
        return Ok(Box::new(io::empty()));
    }
    if e.is_symlink
        && let Some(t) = &e.link_target
    {
        return Ok(Box::new(io::Cursor::new(t.clone().into_bytes())));
    }
    let x = &listing.extras[index];
    let mut s = src.open_stream(x.ordinal)?;
    skip(&mut s, x.data_pos)?;
    Ok(Box::new(s.take(e.size)))
}

/// Processing order: data entries by (stream, offset), then everything else (hard links
/// last, so their targets exist).
fn order(listing: &Listing, wanted: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..listing.entries.len()).filter(|&i| wanted(i)).collect();
    idx.sort_by_key(|&i| {
        let x = &listing.extras[i];
        let data = has_data(listing, i);
        (x.hardlink, !data, if data { (x.ordinal, x.data_pos) } else { (0, 0) }, i)
    });
    idx
}

/// [`crate::ArchiveReader::extract`] over sequential streams.
#[allow(clippy::too_many_arguments)]
pub(crate) fn extract(
    src: &dyn StreamSource,
    listing: &Listing,
    archive_size: u64,
    selection: Option<&[usize]>,
    dest: &std::path::Path,
    opts: &ExtractOptions,
    ctl: &OpControl,
    sink: &mut dyn ExtractSink,
) -> Result<ExtractSummary, ArchiveError> {
    let mut ex = Extractor::new(&listing.entries, &listing.extras, selection, dest, archive_size, opts, ctl, sink)?;
    let todo = order(listing, |i| ex.wants(i));
    // (stream id, decoded position, reader)
    let mut cur: Option<(usize, u64, Box<dyn Read + Send>)> = None;
    for i in todo {
        ctl.checkpoint()?;
        let e = &listing.entries[i];
        if !has_data(listing, i) {
            let mut empty = io::empty();
            ex.handle(i, &mut OnceData { reader: Some(&mut empty) })?;
            continue;
        }
        let x = &listing.extras[i];
        let reuse = matches!(&cur, Some((s, pos, _)) if *s == x.ordinal && *pos <= x.data_pos);
        if !reuse {
            cur = None;
            match src.open_stream(x.ordinal) {
                Ok(r) => cur = Some((x.ordinal, 0, r)),
                Err(err) => {
                    ex.report(i, err)?;
                    continue;
                }
            }
        }
        let (_, pos, r) = cur.as_mut().expect("stream open");
        if let Err(err) = skip(r, x.data_pos - *pos) {
            cur = None;
            ex.report(i, err)?;
            continue;
        }
        *pos = x.data_pos;
        let mut take = r.take(e.size);
        ex.handle(i, &mut OnceData { reader: Some(&mut take) })?;
        let consumed = e.size - take.limit();
        *pos += consumed;
    }
    Ok(ex.finish())
}

/// [`crate::ArchiveReader::test`]: decode every stream to its end, checking each entry.
pub(crate) fn test(
    src: &dyn StreamSource,
    listing: &Listing,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<TestReport, ArchiveError> {
    let mut report = TestReport { entries: listing.entries.len() as u64, ..Default::default() };
    let todo = order(listing, |i| has_data(listing, i) && listing.extras[i].ordinal != NO_STREAM);
    for i in 0..listing.entries.len() {
        if listing.extras[i].ordinal == NO_STREAM && has_data(listing, i) {
            report.bad.push((listing.entries[i].path.clone(), "data is in another cabinet".into()));
        }
    }
    let mut buf = vec![0u8; CHUNK];
    let mut k = 0;
    while k < todo.len() {
        let stream = listing.extras[todo[k]].ordinal;
        let mut r = match src.open_stream(stream) {
            Ok(r) => r,
            Err(err) => {
                test_failure(&mut report, &listing.entries[todo[k]].path, err)?;
                while k < todo.len() && listing.extras[todo[k]].ordinal == stream {
                    k += 1;
                }
                continue;
            }
        };
        let mut pos = 0u64;
        let mut broken = false;
        while k < todo.len() && listing.extras[todo[k]].ordinal == stream {
            let i = todo[k];
            k += 1;
            ctl.checkpoint()?;
            if broken {
                continue;
            }
            let (e, x) = (&listing.entries[i], &listing.extras[i]);
            if x.data_pos < pos {
                // Overlapping entries (same data twice): checked through their first copy.
                continue;
            }
            let res = skip(&mut r, x.data_pos - pos).and_then(|()| {
                drain_for_test(&mut (&mut r).take(e.size), e.size, ctl, sink, &mut buf)
                    .map_err(|err| read_err(err, Some(ctl), false))
            });
            match res {
                Ok(n) => {
                    pos = x.data_pos + n;
                    if n != e.size {
                        report.bad.push((e.path.clone(), format!("size mismatch: {n} != {}", e.size)));
                        broken = true;
                    }
                }
                Err(err) => {
                    test_failure(&mut report, &e.path, err)?;
                    broken = true;
                }
            }
        }
        // Decode to the end so trailing damage / codec checksums are noticed.
        if !broken && let Err(e) = io::copy(&mut r, &mut io::sink()) {
            test_failure(&mut report, "", read_err(e, Some(ctl), false))?;
        }
    }
    Ok(report)
}
