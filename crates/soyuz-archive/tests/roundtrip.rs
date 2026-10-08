//! create → list → read → extract → test, for every writable format.

mod common;

use std::io::Read;
use std::path::Path;
use std::time::{Duration, SystemTime};

use common::*;
use soyuz_archive::{
    ArchiveFormat, CreateExtras, ExtractOptions, ZipMethod, create, create_with, detect, find_entry, list_dir, open,
};
use soyuz_core::{ByteProgress, EntryKind, OpControl};

fn round_trip(format: ArchiveFormat, level: u8, extras: CreateExtras) {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let old = SystemTime::now() - Duration::from_secs(86_400 * 30);
    filetime::set_file_mtime(tree.join("hello.txt"), filetime::FileTime::from_system_time(old)).unwrap();
    let archive = d.path().join(format!("out.{}", ext(format)));
    let ctl = OpControl::new();
    let mut progress = 0u64;
    let mut o = opts(format, d.path());
    o.level = level;
    create_with(&archive, std::slice::from_ref(&tree), &o, &extras, &ctl, &mut |p: ByteProgress| progress += p.delta)
        .unwrap_or_else(|e| panic!("{format:?}: {e}"));
    assert!(archive.exists());
    assert!(!d.path().join(format!("out.{}.part", ext(format))).exists());
    assert_eq!(progress, 1_500_000 + 300_000 + 12 + 7, "{format:?} progress");
    assert_eq!(detect(&archive), Some(format));

    let mut r = open(&archive, None).unwrap();
    assert_eq!(r.format(), format);
    let names: Vec<String> = r.entries().unwrap().iter().map(|e| e.path.clone()).collect();
    for want in ["tree", "tree/hello.txt", "tree/empty.bin", "tree/empty_dir", "tree/sub/big.bin"] {
        assert!(names.iter().any(|n| n == want), "{format:?}: missing {want} in {names:?}");
    }
    let big = r.entries().unwrap().iter().find(|e| e.path == "tree/sub/big.bin").unwrap().clone();
    assert_eq!(big.size, 1_500_000);
    assert!(!big.is_dir && !big.encrypted);
    let hello = r.entries().unwrap().iter().find(|e| e.path == "tree/hello.txt").unwrap().clone();
    let m = hello.modified.expect("mtime stored");
    let diff = m.duration_since(old).unwrap_or_else(|e| e.duration());
    assert!(diff <= Duration::from_secs(2), "{format:?}: mtime off by {diff:?}");

    // Directory view.
    let root = list_dir(&mut *r, "").unwrap();
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].name, "tree");
    assert_eq!(root[0].kind, EntryKind::Dir);
    let sub = list_dir(&mut *r, "tree/sub").unwrap();
    let sub_names: Vec<_> = sub.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(sub_names, ["big.bin", "deeper"]);
    assert_eq!(sub[0].size, 1_500_000);

    // Single-entry read.
    let idx = find_entry(&mut *r, "tree/sub/big.bin").unwrap();
    let mut data = Vec::new();
    r.open_entry(idx).unwrap().read_to_end(&mut data).unwrap();
    assert!(data == noise(1_500_000, 7), "{format:?}: open_entry content");
    let idx = find_entry(&mut *r, "tree/hello.txt").unwrap();
    let mut s = String::new();
    r.open_entry(idx).unwrap().read_to_string(&mut s).unwrap();
    assert_eq!(s, "hello world\n");

    // Extract everything.
    let dest = d.path().join("x");
    let mut sink = TestSink::default();
    let eo = ExtractOptions { preserve_times: true, preserve_mode: true, ..Default::default() };
    let sum = r.extract(None, &dest, &eo, &ctl, &mut sink).unwrap();
    assert!(sink.errors.is_empty(), "{format:?}: {:?}", sink.errors);
    assert_eq!(sum.entries_failed, 0);
    assert_eq!(sum.bytes, 1_500_000 + 300_000 + 12 + 7);
    assert_eq!(sink.bytes, sum.bytes);
    assert_eq!(snapshot(&dest.join("tree")), snapshot(&tree), "{format:?}: extracted tree differs");
    let m = std::fs::metadata(dest.join("tree/hello.txt")).unwrap().modified().unwrap();
    let diff = m.duration_since(old).unwrap_or_else(|e| e.duration());
    assert!(diff <= Duration::from_secs(2), "{format:?}: extracted mtime off by {diff:?}");
    assert_no_temp_files(&dest);

    // Integrity.
    let report = r.test(&ctl, &mut ()).unwrap();
    assert!(report.bad.is_empty(), "{format:?}: {:?}", report.bad);
    assert_eq!(report.entries as usize, names.len());
}

#[test]
fn zip_deflate() {
    round_trip(ArchiveFormat::Zip, 6, CreateExtras::default());
}

#[test]
#[cfg(feature = "zstd")]
fn zip_zstd() {
    round_trip(ArchiveFormat::Zip, 9, CreateExtras { zip_method: ZipMethod::Zstd, ..Default::default() });
}

#[test]
fn zip_store() {
    round_trip(ArchiveFormat::Zip, 0, CreateExtras::default());
    round_trip(ArchiveFormat::Zip, 5, CreateExtras { zip_method: ZipMethod::Store, ..Default::default() });
}

#[test]
fn tar_plain() {
    round_trip(ArchiveFormat::Tar, 0, CreateExtras::default());
}

#[test]
fn tar_gz() {
    round_trip(ArchiveFormat::TarGz, 6, CreateExtras::default());
}

#[test]
fn tar_bz2() {
    round_trip(ArchiveFormat::TarBz2, 1, CreateExtras::default());
}

#[test]
fn tar_xz() {
    round_trip(ArchiveFormat::TarXz, 1, CreateExtras::default());
}

#[test]
#[cfg(feature = "zstd")]
fn tar_zst() {
    round_trip(ArchiveFormat::TarZst, 3, CreateExtras::default());
}

#[test]
fn sevenz_lzma2() {
    round_trip(ArchiveFormat::SevenZ, 3, CreateExtras::default());
}

#[test]
fn sevenz_copy() {
    round_trip(ArchiveFormat::SevenZ, 0, CreateExtras::default());
}

#[test]
fn create_cancel_removes_part() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::SevenZ] {
        let archive = d.path().join(format!("c.{}", ext(format)));
        let ctl = OpControl::new();
        let c2 = ctl.clone();
        let mut sink = move |p: ByteProgress| {
            if p.done > 100_000 {
                c2.cancel();
            }
        };
        let res = create(&archive, std::slice::from_ref(&tree), &opts(format, d.path()), &ctl, &mut sink);
        assert_eq!(res, Err(soyuz_archive::ArchiveError::Cancelled), "{format:?}");
        assert!(!archive.exists(), "{format:?}");
        assert!(!d.path().join(format!("c.{}.part", ext(format))).exists(), "{format:?}");
    }
}

#[test]
fn extract_cancel_leaves_no_partial_file() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarBz2, ArchiveFormat::SevenZ] {
        let archive = d.path().join(format!("c.{}", ext(format)));
        let ctl = OpControl::new();
        create(&archive, std::slice::from_ref(&tree), &opts(format, d.path()), &ctl, &mut ()).unwrap();
        let mut r = open(&archive, None).unwrap();
        let dest = d.path().join(format!("x-{}", ext(format)));
        let mut sink = TestSink { cancel_after: Some((ctl.clone(), 300_000)), ..Default::default() };
        let res = r.extract(None, &dest, &ExtractOptions::default(), &ctl, &mut sink);
        assert_eq!(res, Err(soyuz_archive::ArchiveError::Cancelled), "{format:?}");
        assert!(!dest.join("tree/sub/big.bin").exists(), "{format:?}: partial file kept");
        assert_no_temp_files(&dest);
    }
}

#[test]
fn large_entry_streams() {
    // 24 MiB: well beyond any internal buffer; checks the streaming paths end to end.
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big");
    std::fs::create_dir(&src).unwrap();
    let data = noise(24 << 20, 99);
    std::fs::write(src.join("blob.bin"), &data).unwrap();
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::SevenZ] {
        let archive = d.path().join(format!("l.{}", ext(format)));
        let ctl = OpControl::new();
        let mut o = opts(format, d.path());
        o.level = 1;
        create(&archive, std::slice::from_ref(&src), &o, &ctl, &mut ()).unwrap();
        let mut r = open(&archive, None).unwrap();
        let idx = find_entry(&mut *r, "big/blob.bin").unwrap();
        let mut rd = r.open_entry(idx).unwrap();
        let mut buf = vec![0u8; 1 << 20];
        let mut off = 0usize;
        loop {
            let n = rd.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            assert!(buf[..n] == data[off..off + n], "{format:?}: mismatch at {off}");
            off += n;
        }
        assert_eq!(off, data.len());
    }
}

#[test]
fn empty_input_list_makes_valid_archive() {
    let d = tempfile::tempdir().unwrap();
    for format in [ArchiveFormat::Zip, ArchiveFormat::Tar, ArchiveFormat::SevenZ] {
        let archive = d.path().join(format!("e.{}", ext(format)));
        create(&archive, &[], &opts(format, d.path()), &OpControl::new(), &mut ()).unwrap();
        let mut r = open(&archive, None).unwrap();
        assert!(r.entries().unwrap().is_empty(), "{format:?}");
    }
}

#[test]
fn inputs_outside_base_use_file_name() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let archive = d.path().join("n.zip");
    let mut o = opts(ArchiveFormat::Zip, Path::new("/somewhere/else"));
    o.level = 1;
    create(&archive, &[tree.join("hello.txt"), tree.join("sub")], &o, &OpControl::new(), &mut ()).unwrap();
    let mut r = open(&archive, None).unwrap();
    let mut names: Vec<_> = r.entries().unwrap().iter().map(|e| e.path.clone()).collect();
    names.sort();
    assert_eq!(names[0], "hello.txt");
    assert!(names.contains(&"sub/deeper/zeros.dat".to_string()));
}

#[test]
fn tar_zst_read_pure_rust() {
    // Build a .tar with our writer, compress it with ruzstd, then browse / extract / test it.
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let tar_path = d.path().join("plain.tar");
    create(&tar_path, std::slice::from_ref(&tree), &opts(ArchiveFormat::Tar, d.path()), &OpControl::new(), &mut ())
        .unwrap();
    let raw = std::fs::read(&tar_path).unwrap();
    let zst = ruzstd::encoding::compress_to_vec(&raw[..], ruzstd::encoding::CompressionLevel::Fastest);
    let archive = d.path().join("x.tar.zst");
    std::fs::write(&archive, zst).unwrap();
    assert_eq!(detect(&archive), Some(ArchiveFormat::TarZst));
    let mut r = open(&archive, None).unwrap();
    let idx = find_entry(&mut *r, "tree/sub/big.bin").unwrap();
    let mut data = Vec::new();
    r.open_entry(idx).unwrap().read_to_end(&mut data).unwrap();
    assert!(data == noise(1_500_000, 7));
    let dest = d.path().join("x");
    let mut sink = TestSink::default();
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    assert_eq!(snapshot(&dest.join("tree")), snapshot(&tree));
    assert!(r.test(&OpControl::new(), &mut ()).unwrap().bad.is_empty());
}

#[cfg(not(feature = "zstd"))]
#[test]
fn zstd_writing_needs_feature() {
    let d = tempfile::tempdir().unwrap();
    let ctl = OpControl::new();
    assert!(!ArchiveFormat::TarZst.can_write());
    let res = create(&d.path().join("a.tar.zst"), &[], &opts(ArchiveFormat::TarZst, d.path()), &ctl, &mut ());
    assert_eq!(res, Err(soyuz_archive::ArchiveError::Unsupported(ArchiveFormat::TarZst)));
    let extras = CreateExtras { zip_method: ZipMethod::Zstd, ..Default::default() };
    let zip_opts = opts(ArchiveFormat::Zip, d.path());
    let res = create_with(&d.path().join("a.zip"), &[], &zip_opts, &extras, &ctl, &mut ());
    assert_eq!(res, Err(soyuz_archive::ArchiveError::Unsupported(ArchiveFormat::Zip)));
    assert!(!d.path().join("a.zip").exists() && !d.path().join("a.zip.part").exists());
}
