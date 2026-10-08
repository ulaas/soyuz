//! Decompression-bomb limits and permission-bit masking.

mod common;

use std::io::Write;
use std::path::Path;

use common::*;
use soyuz_archive::{ArchiveError, ExtractLimits, ExtractOptions, open, open_entry_capped};
use soyuz_core::{ErrorAction, OpControl};
use zip::write::SimpleFileOptions;

const MIB: u64 = 1024 * 1024;

/// A zip with highly compressible (all-zero) deflated files.
fn zeros_zip(path: &Path, files: &[(&str, u64)]) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, len) in files {
        z.start_file(*name, o).unwrap();
        z.write_all(&vec![0u8; *len as usize]).unwrap();
    }
    z.finish().unwrap();
}

/// A tar.gz of all-zero files with the given mode.
fn zeros_tgz(path: &Path, files: &[(&str, u64)], mode: u32) {
    let gz = flate2::write::GzEncoder::new(std::fs::File::create(path).unwrap(), flate2::Compression::best());
    let mut b = tar::Builder::new(gz);
    for (name, len) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(*len);
        h.set_mode(mode);
        h.set_mtime(1_600_000_000);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        b.append_data(&mut h, name, std::io::Read::take(std::io::repeat(0), *len)).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap().flush().unwrap();
}

fn limit_errors(sink: &TestSink) -> usize {
    sink.errors.iter().filter(|(_, e)| matches!(e, ArchiveError::LimitExceeded(_))).count()
}

#[test]
fn total_ratio_limit_pauses_and_can_abort_or_continue() {
    let d = tempfile::tempdir().unwrap();
    let zip = d.path().join("bomb.zip");
    zeros_zip(&zip, &[("a.bin", 3 * MIB), ("b.bin", 3 * MIB)]);
    let size = std::fs::metadata(&zip).unwrap().len();
    assert!(size < 64 * 1024, "the bomb compresses well ({size} bytes)");
    // ratio 10 of a few KiB, floor 4 MiB: the second file crosses the limit.
    let limits = ExtractLimits { max_ratio: 10, min_total_bytes: 4 * MIB, ..ExtractLimits::default() };
    assert_eq!(limits.total_cap(size), Some(4 * MIB));
    let eo = ExtractOptions { limits: limits.clone(), ..Default::default() };

    // Abort: extraction stops before b.bin, nothing half-written is left behind.
    let out = d.path().join("abort");
    let mut sink = TestSink { on_error: Some(ErrorAction::Abort), ..Default::default() };
    let r = open(&zip, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut sink);
    assert!(matches!(r, Err(ArchiveError::LimitExceeded(_))), "{r:?}");
    assert_eq!(limit_errors(&sink), 1);
    assert!(out.join("a.bin").is_file());
    assert!(!out.join("b.bin").exists());
    assert_no_temp_files(&out);

    // Continue: the limit is lifted once and everything is extracted.
    let out = d.path().join("continue");
    let mut sink = TestSink { on_error: Some(ErrorAction::Skip), ..Default::default() };
    let s = open(&zip, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut sink).unwrap();
    assert_eq!(limit_errors(&sink), 1, "asked exactly once");
    assert_eq!(s.entries_ok, 2);
    assert_eq!(std::fs::metadata(out.join("b.bin")).unwrap().len(), 3 * MIB);

    // Default limits (1 GiB floor) don't bother a small archive.
    let out = d.path().join("default");
    let mut sink = TestSink::default();
    open(&zip, None).unwrap().extract(None, &out, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert_eq!(limit_errors(&sink), 0);
}

#[test]
fn single_entry_limit_trips_on_tar_gz() {
    let d = tempfile::tempdir().unwrap();
    let tgz = d.path().join("bomb.tar.gz");
    zeros_tgz(&tgz, &[("small.bin", 1000), ("huge.bin", 5 * MIB)], 0o644);
    let limits = ExtractLimits { max_entry_bytes: MIB, ..ExtractLimits::default() };
    let eo = ExtractOptions { limits, ..Default::default() };
    let out = d.path().join("out");
    let mut sink = TestSink { on_error: Some(ErrorAction::Abort), ..Default::default() };
    let r = open(&tgz, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut sink);
    assert!(matches!(r, Err(ArchiveError::LimitExceeded(_))), "{r:?}");
    assert_eq!(sink.errors[0].0, "huge.bin");
    assert!(out.join("small.bin").is_file());
    assert!(!out.join("huge.bin").exists());
    assert_no_temp_files(&out);
}

#[test]
fn entry_count_limit_is_asked_once_up_front() {
    let d = tempfile::tempdir().unwrap();
    let tgz = d.path().join("many.tar.gz");
    let names: Vec<String> = (0..50).map(|i| format!("f{i:02}.txt")).collect();
    let files: Vec<(&str, u64)> = names.iter().map(|n| (n.as_str(), 10)).collect();
    zeros_tgz(&tgz, &files, 0o644);
    let eo = ExtractOptions { limits: ExtractLimits { max_entries: 10, ..Default::default() }, ..Default::default() };

    let out = d.path().join("abort");
    let mut sink = TestSink { on_error: Some(ErrorAction::Abort), ..Default::default() };
    let r = open(&tgz, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut sink);
    assert!(matches!(r, Err(ArchiveError::LimitExceeded(_))), "{r:?}");
    assert!(snapshot(&out).is_empty(), "nothing extracted");

    let out = d.path().join("continue");
    let mut sink = TestSink { on_error: Some(ErrorAction::SkipAll), ..Default::default() };
    let s = open(&tgz, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut sink).unwrap();
    assert_eq!(limit_errors(&sink), 1);
    assert_eq!(s.entries_ok, 50);
}

#[test]
fn open_entry_is_hard_capped() {
    let d = tempfile::tempdir().unwrap();
    let zip = d.path().join("bomb.zip");
    zeros_zip(&zip, &[("big.bin", 2 * MIB), ("ok.bin", 100)]);
    let mut r = open(&zip, None).unwrap();
    let big = soyuz_archive::find_entry(r.as_mut(), "big.bin").unwrap();
    let ok = soyuz_archive::find_entry(r.as_mut(), "ok.bin").unwrap();
    assert!(matches!(open_entry_capped(r.as_mut(), big, MIB), Err(ArchiveError::LimitExceeded(_))));
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut open_entry_capped(r.as_mut(), ok, 100).unwrap(), &mut data).unwrap();
    assert_eq!(data.len(), 100, "an entry of exactly the cap is fine");
    let limits = ExtractLimits { max_ratio: 2, min_total_bytes: MIB / 2, max_entry_bytes: 0, ..Default::default() };
    assert_eq!(limits.read_cap(1000), Some(MIB / 2));
    assert_eq!(ExtractLimits::unlimited().read_cap(1000), None);
}

#[cfg(unix)]
#[test]
fn setuid_bits_are_stripped_by_default() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let tgz = d.path().join("suid.tar.gz");
    zeros_tgz(&tgz, &[("tool", 10)], 0o4755);
    let mode = |out: &Path| std::fs::metadata(out.join("tool")).unwrap().permissions().mode() & 0o7777;

    let out = d.path().join("default");
    let eo = ExtractOptions { preserve_mode: true, ..Default::default() };
    open(&tgz, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut TestSink::default()).unwrap();
    assert_eq!(mode(&out), 0o755);

    let out = d.path().join("kept");
    let eo = ExtractOptions { preserve_mode: true, preserve_special_bits: true, ..Default::default() };
    open(&tgz, None).unwrap().extract(None, &out, &eo, &OpControl::new(), &mut TestSink::default()).unwrap();
    assert_eq!(mode(&out), 0o4755);
}
