//! Zip-slip, passwords, symlinks, conflicts, selection, strip, corruption, big listings.

mod common;

use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use common::*;
use soyuz_archive::{
    ArchiveError, ArchiveFormat, CreateExtras, ExtractOptions, create, create_with, find_entry, list_dir, open,
};
use soyuz_core::{ConflictResolution, ErrorAction, OpControl, Overwrite};
use zip::write::SimpleFileOptions;

fn write_zip(path: &Path, files: &[(&str, &[u8])], symlinks: &[(&str, &str)]) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in files {
        z.start_file(*name, o).unwrap();
        z.write_all(data).unwrap();
    }
    for (name, target) in symlinks {
        z.add_symlink(*name, *target, o).unwrap();
    }
    z.finish().unwrap();
}

/// A tar whose header names are written raw (bypassing the tar crate's path checks).
fn write_raw_tar(path: &Path, entries: &[(&str, tar::EntryType, &str, &[u8])]) {
    let mut b = tar::Builder::new(std::fs::File::create(path).unwrap());
    for (name, ty, link, data) in entries {
        let mut h = tar::Header::new_old();
        h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        if !link.is_empty() {
            h.as_old_mut().linkname[..link.len()].copy_from_slice(link.as_bytes());
        }
        h.set_entry_type(*ty);
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(1_600_000_000);
        h.set_cksum();
        b.append(&h, *data).unwrap();
    }
    b.into_inner().unwrap().flush().unwrap();
}

#[test]
fn zip_slip_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("evil.zip");
    write_zip(
        &archive,
        &[("../evil.txt", b"x"), ("a/../../evil2.txt", b"x"), ("/abs.txt", b"x"), ("ok/fine.txt", b"fine")],
        &[],
    );
    let mut r = open(&archive, None).unwrap();
    // Listing is normalized (never shows "..").
    assert!(r.entries().unwrap().iter().all(|e| !e.path.contains("..") && !e.path.starts_with('/')));
    let dest = d.path().join("dest");
    let mut sink = TestSink::default();
    let sum = r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert_eq!(sum.entries_ok, 1);
    assert_eq!(sum.entries_failed, 3);
    assert_eq!(sink.errors.len(), 3);
    assert!(sink.errors.iter().all(|(_, e)| matches!(e, ArchiveError::ZipSlip(_))), "{:?}", sink.errors);
    assert!(!d.path().join("evil.txt").exists());
    assert!(!d.path().join("evil2.txt").exists());
    assert!(!dest.join("evil.txt").exists());
    assert_eq!(std::fs::read(dest.join("ok/fine.txt")).unwrap(), b"fine");

    // Abort on the first one.
    let mut sink = TestSink { on_error: Some(ErrorAction::Abort), ..Default::default() };
    let res = r.extract(None, &d.path().join("dest2"), &ExtractOptions::default(), &OpControl::new(), &mut sink);
    assert!(matches!(res, Err(ArchiveError::ZipSlip(_))), "{res:?}");
}

#[test]
fn tar_slip_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("evil.tar");
    write_raw_tar(
        &archive,
        &[
            ("../evil.txt", tar::EntryType::Regular, "", b"x"),
            ("link", tar::EntryType::Symlink, "../../outside", b""),
            ("abs_link", tar::EntryType::Symlink, "/etc/passwd", b""),
            ("hard", tar::EntryType::Link, "../../etc/passwd", b""),
            ("good.txt", tar::EntryType::Regular, "", b"good"),
        ],
    );
    let mut r = open(&archive, None).unwrap();
    let e = r.entries().unwrap();
    assert_eq!(e.len(), 5);
    assert!(e[1].is_symlink);
    assert_eq!(e[1].link_target.as_deref(), Some("../../outside"));
    let dest = d.path().join("dest");
    let mut sink = TestSink::default();
    let sum = r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert_eq!(sum.entries_ok, 1, "{:?}", sink.errors);
    assert_eq!(sink.errors.len(), 4);
    assert!(sink.errors.iter().all(|(_, e)| matches!(e, ArchiveError::ZipSlip(_))), "{:?}", sink.errors);
    assert!(!dest.join("link").exists() && std::fs::symlink_metadata(dest.join("link")).is_err());
    assert_eq!(std::fs::read(dest.join("good.txt")).unwrap(), b"good");
}

#[test]
fn chained_symlinks_cannot_escape() {
    // `d/l1 -> ..` stays inside on its own; `l2 -> d/l1/..` is lexically `d` but on
    // disk resolves to the parent of the destination. It must be refused whether it
    // comes before or after the link it traverses.
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("chain.tar");
    write_raw_tar(
        &archive,
        &[
            ("l2", tar::EntryType::Symlink, "d/l1/..", b""),
            ("d/l1", tar::EntryType::Symlink, "..", b""),
            ("l3", tar::EntryType::Symlink, "d/l1/../..", b""),
            ("ok.txt", tar::EntryType::Regular, "", b"ok"),
        ],
    );
    let mut r = open(&archive, None).unwrap();
    let dest = d.path().join("dest");
    let mut sink = TestSink::default();
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    for name in ["l2", "l3"] {
        assert!(
            sink.errors.iter().any(|(p, e)| p == name && matches!(e, ArchiveError::ZipSlip(_))),
            "{name}: {:?}",
            sink.errors
        );
        assert!(std::fs::symlink_metadata(dest.join(name)).is_err(), "{name} must not be created");
    }
    assert_eq!(std::fs::read(dest.join("ok.txt")).unwrap(), b"ok");
}

#[cfg(unix)]
#[test]
fn never_writes_through_symlinks() {
    let d = tempfile::tempdir().unwrap();
    let outside = d.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let dest = d.path().join("dest");
    std::fs::create_dir(&dest).unwrap();
    // A pre-existing symlink in the destination pointing outside.
    std::os::unix::fs::symlink(&outside, dest.join("sneaky")).unwrap();
    let archive = d.path().join("a.zip");
    write_zip(&archive, &[("sneaky/payload.txt", b"x")], &[]);
    let mut r = open(&archive, None).unwrap();
    let mut sink = TestSink::default();
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(matches!(sink.errors[0].1, ArchiveError::ZipSlip(_)));
    assert!(!outside.join("payload.txt").exists());
}

#[test]
fn symlink_entries_list_on_every_platform() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("links.zip");
    write_zip(&archive, &[("dir/file.txt", b"data")], &[("dir/link", "file.txt")]);
    let mut r = open(&archive, None).unwrap();
    let e = r.entries().unwrap().iter().find(|e| e.path == "dir/link").unwrap().clone();
    assert!(e.is_symlink);
    assert_eq!(e.link_target.as_deref(), Some("file.txt"));
    let kids = list_dir(&mut *r, "dir").unwrap();
    assert!(matches!(kids[1].kind, soyuz_core::EntryKind::Symlink { .. }));
}

#[cfg(unix)]
#[test]
fn symlink_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let t = d.path().join("t");
    std::fs::create_dir_all(t.join("dir")).unwrap();
    std::fs::write(t.join("dir/file.txt"), b"data").unwrap();
    std::os::unix::fs::symlink("file.txt", t.join("dir/link")).unwrap();
    std::os::unix::fs::symlink("dir", t.join("dirlink")).unwrap();
    for format in [ArchiveFormat::Zip, ArchiveFormat::Tar, ArchiveFormat::TarXz, ArchiveFormat::SevenZ] {
        let archive = d.path().join(format!("s.{}", ext(format)));
        create(&archive, std::slice::from_ref(&t), &opts(format, d.path()), &OpControl::new(), &mut ()).unwrap();
        let mut r = open(&archive, None).unwrap();
        let e = r.entries().unwrap().iter().find(|e| e.path == "t/dir/link").unwrap().clone();
        assert!(e.is_symlink, "{format:?}");
        let dest = d.path().join(format!("x-{}", ext(format)));
        let mut sink = TestSink::default();
        r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
        assert!(sink.errors.is_empty(), "{format:?}: {:?}", sink.errors);
        assert_eq!(std::fs::read_link(dest.join("t/dir/link")).unwrap(), Path::new("file.txt"), "{format:?}");
        assert_eq!(std::fs::read(dest.join("t/dirlink/link")).unwrap(), b"data", "{format:?}");
    }
    // follow_symlinks stores the target content instead.
    let archive = d.path().join("follow.zip");
    let mut o = opts(ArchiveFormat::Zip, d.path());
    o.follow_symlinks = true;
    create(&archive, std::slice::from_ref(&t), &o, &OpControl::new(), &mut ()).unwrap();
    let mut r = open(&archive, None).unwrap();
    let e = r.entries().unwrap().iter().find(|e| e.path == "t/dir/link").unwrap().clone();
    assert!(!e.is_symlink && e.size == 4);
}

#[test]
fn tar_hardlinks_extract() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("h.tar");
    write_raw_tar(
        &archive,
        &[
            ("a/orig.txt", tar::EntryType::Regular, "", b"same"),
            ("a/copy.txt", tar::EntryType::Link, "a/orig.txt", b""),
        ],
    );
    let mut r = open(&archive, None).unwrap();
    let dest = d.path().join("x");
    let mut sink = TestSink::default();
    let sum = r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    assert_eq!(sum.entries_ok, 2);
    assert_eq!(std::fs::read(dest.join("a/copy.txt")).unwrap(), b"same");
}

#[test]
fn zip_password() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let archive = d.path().join("p.zip");
    let mut o = opts(ArchiveFormat::Zip, d.path());
    o.password = Some("s3cret".into());
    create(&archive, std::slice::from_ref(&tree), &o, &OpControl::new(), &mut ()).unwrap();

    // Listing works without a password; data does not.
    let mut r = open(&archive, None).unwrap();
    let idx = find_entry(&mut *r, "tree/hello.txt").unwrap();
    assert!(r.entries().unwrap()[idx].encrypted);
    assert_eq!(r.open_entry(idx).err(), Some(ArchiveError::PasswordRequired));
    let mut sink = TestSink::default();
    let res = r.extract(None, &d.path().join("x0"), &ExtractOptions::default(), &OpControl::new(), &mut sink);
    assert_eq!(res, Err(ArchiveError::PasswordRequired));
    assert_eq!(r.test(&OpControl::new(), &mut ()).err(), Some(ArchiveError::PasswordRequired));

    let mut r = open(&archive, Some("wrong")).unwrap();
    assert_eq!(r.open_entry(idx).err(), Some(ArchiveError::WrongPassword));
    let res = r.extract(None, &d.path().join("x1"), &ExtractOptions::default(), &OpControl::new(), &mut sink);
    assert_eq!(res, Err(ArchiveError::WrongPassword));

    let mut r = open(&archive, Some("s3cret")).unwrap();
    let mut s = String::new();
    r.open_entry(idx).unwrap().read_to_string(&mut s).unwrap();
    assert_eq!(s, "hello world\n");
    let dest = d.path().join("x2");
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert_eq!(snapshot(&dest.join("tree")), snapshot(&tree));
    assert!(r.test(&OpControl::new(), &mut ()).unwrap().bad.is_empty());
}

#[test]
fn sevenz_password_with_encrypted_header() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let archive = d.path().join("p.7z");
    let mut o = opts(ArchiveFormat::SevenZ, d.path());
    o.password = Some("pa55".into());
    create(&archive, std::slice::from_ref(&tree), &o, &OpControl::new(), &mut ()).unwrap();
    assert_eq!(open(&archive, None).err(), Some(ArchiveError::PasswordRequired));
    assert_eq!(open(&archive, Some("nope")).err(), Some(ArchiveError::WrongPassword));
    let mut r = open(&archive, Some("pa55")).unwrap();
    let idx = find_entry(&mut *r, "tree/sub/big.bin").unwrap();
    assert!(r.entries().unwrap()[idx].encrypted);
    let mut data = Vec::new();
    r.open_entry(idx).unwrap().read_to_end(&mut data).unwrap();
    assert!(data == noise(1_500_000, 7));
    let dest = d.path().join("x");
    let mut sink = TestSink::default();
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    assert_eq!(snapshot(&dest.join("tree")), snapshot(&tree));
}

#[test]
fn sevenz_password_with_plain_header() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let archive = d.path().join("p.7z");
    let mut o = opts(ArchiveFormat::SevenZ, d.path());
    o.password = Some("pa55".into());
    let extras = CreateExtras { encrypt_7z_headers: false, ..Default::default() };
    create_with(&archive, std::slice::from_ref(&tree), &o, &extras, &OpControl::new(), &mut ()).unwrap();
    let mut r = open(&archive, None).unwrap();
    let idx = find_entry(&mut *r, "tree/hello.txt").unwrap();
    assert!(r.entries().unwrap()[idx].encrypted);
    let mut sink = TestSink::default();
    let res = r.extract(None, &d.path().join("x0"), &ExtractOptions::default(), &OpControl::new(), &mut sink);
    assert_eq!(res, Err(ArchiveError::PasswordRequired));
    let mut r = open(&archive, Some("wrong")).unwrap();
    let res = r.extract(Some(&[idx]), &d.path().join("x1"), &ExtractOptions::default(), &OpControl::new(), &mut sink);
    assert_eq!(res, Err(ArchiveError::WrongPassword), "{:?}", sink.errors);
}

#[test]
fn password_rejected_for_tar() {
    let d = tempfile::tempdir().unwrap();
    let mut o = opts(ArchiveFormat::TarGz, d.path());
    o.password = Some("x".into());
    let res = create(&d.path().join("a.tar.gz"), &[], &o, &OpControl::new(), &mut ());
    assert_eq!(res, Err(ArchiveError::Unsupported(ArchiveFormat::TarGz)));
}

#[test]
fn conflicts_follow_policy_and_sink() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("c.zip");
    write_zip(&archive, &[("a.txt", b"new"), ("dir/b.txt", b"new-b")], &[]);
    let dest = d.path().join("dest");
    std::fs::create_dir_all(dest.join("dir")).unwrap();
    std::fs::write(dest.join("a.txt"), b"old").unwrap();
    std::fs::write(dest.join("dir/b.txt"), b"old-b").unwrap();
    let mut r = open(&archive, None).unwrap();
    let ctl = OpControl::new();

    // Default (Never) asks the sink: Skip.
    let mut sink = TestSink::default();
    let sum = r.extract(None, &dest, &ExtractOptions::default(), &ctl, &mut sink).unwrap();
    assert_eq!((sum.entries_ok, sum.entries_skipped), (0, 2));
    assert_eq!(sink.conflicts.len(), 2);
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");

    // KeepBoth.
    let mut sink = TestSink { conflict: Some(ConflictResolution::KeepBoth), ..Default::default() };
    r.extract(None, &dest, &ExtractOptions::default(), &ctl, &mut sink).unwrap();
    assert_eq!(std::fs::read(dest.join("a (2).txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"old");

    // Rename.
    let mut sink = TestSink { conflict: Some(ConflictResolution::Rename("renamed.txt".into())), ..Default::default() };
    r.extract(Some(&[0]), &dest, &ExtractOptions::default(), &ctl, &mut sink).unwrap();
    assert_eq!(std::fs::read(dest.join("renamed.txt")).unwrap(), b"new");

    // Cancel.
    let mut sink = TestSink { conflict: Some(ConflictResolution::Cancel), ..Default::default() };
    let res = r.extract(None, &dest, &ExtractOptions::default(), &ctl, &mut sink);
    assert_eq!(res, Err(ArchiveError::Conflict(dest.join("a.txt"))));

    // Policy Always: no prompt, overwritten.
    let mut sink = TestSink::default();
    let eo = ExtractOptions { overwrite: Overwrite::Always, ..Default::default() };
    let sum = r.extract(None, &dest, &eo, &ctl, &mut sink).unwrap();
    assert!(sink.conflicts.is_empty());
    assert_eq!(sum.entries_ok, 2);
    assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"new");
    assert_eq!(std::fs::read(dest.join("dir/b.txt")).unwrap(), b"new-b");

    // IfDifferentSize: same size → skipped silently.
    let eo = ExtractOptions { overwrite: Overwrite::IfDifferentSize, ..Default::default() };
    let sum = r.extract(None, &dest, &eo, &ctl, &mut TestSink::default()).unwrap();
    assert_eq!(sum.entries_skipped, 2);
    assert_no_temp_files(&dest);
}

#[test]
fn renamed_directory_receives_children() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("c.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let o = SimpleFileOptions::default();
        z.add_directory("dir/", o).unwrap();
        z.start_file("dir/inner.txt", o).unwrap();
        z.write_all(b"in").unwrap();
        z.finish().unwrap();
    }
    let dest = d.path().join("dest");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("dir"), b"a file where the dir goes").unwrap();
    let mut r = open(&archive, None).unwrap();
    let mut sink = TestSink { conflict: Some(ConflictResolution::KeepBoth), ..Default::default() };
    r.extract(None, &dest, &ExtractOptions::default(), &OpControl::new(), &mut sink).unwrap();
    assert!(sink.errors.is_empty(), "{:?}", sink.errors);
    assert_eq!(std::fs::read(dest.join("dir (2)/inner.txt")).unwrap(), b"in");
}

#[test]
fn selection_strip_and_single_top_folder() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let archive = d.path().join("s.tar.gz");
    create(&archive, std::slice::from_ref(&tree), &opts(ArchiveFormat::TarGz, d.path()), &OpControl::new(), &mut ())
        .unwrap();
    let mut r = open(&archive, None).unwrap();
    let ctl = OpControl::new();

    // Selecting a directory extracts its subtree.
    let sub = find_entry(&mut *r, "tree/sub/deeper").unwrap();
    let dest = d.path().join("sel");
    let sum = r.extract(Some(&[sub]), &dest, &ExtractOptions::default(), &ctl, &mut TestSink::default()).unwrap();
    assert_eq!(sum.entries_ok, 3);
    assert!(dest.join("tree/sub/deeper/zeros.dat").exists());
    assert!(!dest.join("tree/hello.txt").exists());

    // strip_components.
    let dest = d.path().join("strip");
    let eo = ExtractOptions { strip_components: 2, ..Default::default() };
    r.extract(Some(&[sub]), &dest, &eo, &ctl, &mut TestSink::default()).unwrap();
    assert!(dest.join("deeper/zeros.dat").exists());

    // single_top_folder drops the redundant "tree/".
    let dest = d.path().join("smart");
    let eo = ExtractOptions { single_top_folder: true, ..Default::default() };
    r.extract(None, &dest, &eo, &ctl, &mut TestSink::default()).unwrap();
    assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello world\n");
    assert!(dest.join("sub/big.bin").exists());
}

#[test]
fn corruption_is_detected() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("c");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("data.bin"), noise(200_000, 3)).unwrap();
    for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::SevenZ] {
        let archive = d.path().join(format!("c.{}", ext(format)));
        let mut o = opts(format, d.path());
        o.level = 0;
        create(&archive, std::slice::from_ref(&src), &o, &OpControl::new(), &mut ()).unwrap();
        let mut bytes = std::fs::read(&archive).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        bytes[mid + 1] ^= 0xFF;
        std::fs::write(&archive, &bytes).unwrap();
        let bad = match open(&archive, None) {
            Ok(mut r) => match r.test(&OpControl::new(), &mut ()) {
                Ok(rep) => !rep.bad.is_empty(),
                Err(_) => true,
            },
            Err(_) => true,
        };
        assert!(bad, "{format:?}: corruption not detected");
    }
}

#[test]
fn not_an_archive() {
    let d = tempfile::tempdir().unwrap();
    for name in ["junk.zip", "junk.7z", "junk.tar.gz"] {
        let p = d.path().join(name);
        std::fs::write(&p, b"this is not an archive at all, just text").unwrap();
        assert!(open(&p, None).is_err(), "{name}");
    }
}

#[test]
fn fifty_thousand_entries_navigate_fast() {
    let d = tempfile::tempdir().unwrap();
    let archive = d.path().join("big.zip");
    {
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(&archive).unwrap()));
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for i in 0..50_000 {
            z.start_file(format!("root/d{}/e{}/f{i}.txt", i % 50, i % 7), o).unwrap();
            z.write_all(b"x").unwrap();
        }
        z.finish().unwrap();
    }
    let t = Instant::now();
    let mut r = open(&archive, None).unwrap();
    assert_eq!(r.entries().unwrap().len(), 50_000);
    let root = list_dir(&mut *r, "root").unwrap();
    assert_eq!(root.len(), 50);
    let first = t.elapsed();
    let t = Instant::now();
    let mut total = 0;
    for i in 0..1000 {
        total += list_dir(&mut *r, &format!("root/d{}/e{}", i % 50, i % 7)).unwrap().len();
        assert!(find_entry(&mut *r, &format!("root/d{}/e{}/f{i}.txt", i % 50, i % 7)).is_some());
    }
    let nav = t.elapsed();
    assert!(total > 0);
    eprintln!("50k zip: open+index {first:?}, 1000 nested list_dir {nav:?}");
    // Generous bound (debug build, slow CI): navigation must not rescan 50k entries per call.
    assert!(nav.as_secs_f64() < 5.0, "navigation too slow: {nav:?}");
}
