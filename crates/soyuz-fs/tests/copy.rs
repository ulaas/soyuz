//! Copy engine: fast paths, progress, pause/cancel, metadata preservation, overwrite safety.

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use common::{pattern, try_symlink, write};
use soyuz_core::{Attributes, ByteProgress, EntryKind, FsErrorKind, HashAlgo, OpControl, Overwrite};
use soyuz_fs::*;

#[test]
fn empty_file_copies_and_reports() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("empty");
    write(&src, b"");
    let dst = d.path().join("copy");
    let ctl = OpControl::new();
    let out = copy_file(&src, &dst, &CopyOptions::default(), &ctl, &mut ()).unwrap();
    assert!(matches!(out, CopyOutcome::Copied { bytes: 0, .. }));
    assert_eq!(fs::metadata(&dst).unwrap().len(), 0);
}

#[test]
fn large_file_progress_is_monotonic_and_complete() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big.bin");
    let data = pattern(64 * 1024 * 1024);
    write(&src, &data);
    let dst = d.path().join("big.copy");
    let ctl = OpControl::new();
    let mut last = 0;
    let mut sum = 0;
    let mut calls = 0;
    let opts = CopyOptions { chunk: 4 * 1024 * 1024, ..Default::default() };
    let out = copy_file(&src, &dst, &opts, &ctl, &mut |p: ByteProgress| {
        assert!(p.done >= last, "monotonic");
        assert_eq!(p.total, 64 * 1024 * 1024);
        last = p.done;
        sum += p.delta;
        calls += 1;
    })
    .unwrap();
    assert_eq!(sum, 64 * 1024 * 1024);
    assert_eq!(last, 64 * 1024 * 1024);
    let method = match out {
        CopyOutcome::Copied { bytes, method } => {
            assert_eq!(bytes, 64 * 1024 * 1024);
            method
        }
        other => panic!("{other:?}"),
    };
    if !matches!(method, CopyMethod::Reflink | CopyMethod::BlockClone | CopyMethod::Clonefile) {
        assert!(calls >= 4, "progress at least every chunk: {calls} calls via {method:?}");
    }
    if cfg!(windows) {
        assert_eq!(method, CopyMethod::CopyFileEx);
    }
    assert!(fs::read(&dst).unwrap() == data);
}

#[test]
fn cancel_mid_copy_removes_partial_destination() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big.bin");
    write(&src, &pattern(32 * 1024 * 1024));
    let dst = d.path().join("cancelled.bin");
    let ctl = OpControl::new();
    let c2 = ctl.clone();
    let opts = CopyOptions { chunk: 1024 * 1024, ..Default::default() };
    let err = copy_file(&src, &dst, &opts, &ctl, &mut move |p: ByteProgress| {
        if p.done > 0 && p.done < p.total {
            c2.cancel();
        }
    });
    match err {
        Err(e) => {
            assert!(e.is_cancel(), "{e:?}");
            assert!(!dst.exists(), "partial destination removed");
        }
        // A reflink/clone completes in a single step and cannot be interrupted.
        Ok(CopyOutcome::Copied { method, .. }) => {
            assert!(matches!(method, CopyMethod::Reflink | CopyMethod::Clonefile | CopyMethod::BlockClone))
        }
        Ok(o) => panic!("{o:?}"),
    }
    assert!(fs::read_dir(d.path()).unwrap().count() <= 2, "no temp files left");
}

#[test]
fn pause_blocks_then_resumes() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big.bin");
    let data = pattern(16 * 1024 * 1024);
    write(&src, &data);
    let dst = d.path().join("paused.bin");
    let ctl = OpControl::new();
    let c2 = ctl.clone();
    let paused_once = Arc::new(AtomicBool::new(false));
    let p2 = paused_once.clone();
    let start = Instant::now();
    let opts = CopyOptions { chunk: 1024 * 1024, reflink: Reflink::Never, ..Default::default() };
    copy_file(&src, &dst, &opts, &ctl, &mut move |p: ByteProgress| {
        if p.done > 0 && p.done < p.total && !p2.swap(true, Ordering::SeqCst) {
            c2.pause();
            let c3 = c2.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(400));
                c3.resume();
            });
        }
    })
    .unwrap();
    if paused_once.load(Ordering::SeqCst) {
        assert!(start.elapsed() >= Duration::from_millis(350), "copy waited while paused");
    }
    assert!(fs::read(&dst).unwrap() == data);
}

#[test]
fn times_and_attributes_are_preserved() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src.txt");
    write(&src, b"content");
    let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000) + Duration::from_millis(250);
    let a = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_100_000);
    let c = SystemTime::UNIX_EPOCH + Duration::from_secs(1_400_000_000);
    set_times(&src, &TimesSet { modified: Some(m), accessed: Some(a), created: Some(c) }, true).unwrap();
    #[cfg(windows)]
    set_attributes(&src, &AttrSet { set: Attributes::HIDDEN | Attributes::READONLY, ..Default::default() }).unwrap();
    #[cfg(unix)]
    set_mode(&src, 0o640).unwrap();

    let ctl = OpControl::new();
    for (i, opts) in [
        CopyOptions::default(),
        CopyOptions { throttle_bps: Some(100_000_000), ..Default::default() },
        CopyOptions { reflink: Reflink::Never, ..Default::default() },
    ]
    .into_iter()
    .enumerate()
    {
        let dst = d.path().join(format!("dst{i}.txt"));
        copy_file(&src, &dst, &opts, &ctl, &mut ()).unwrap();
        let e = stat(&dst, false).unwrap();
        assert_eq!(e.modified, Some(m), "mtime ({opts:?})");
        if cfg!(windows) {
            assert_eq!(e.created, Some(c), "creation time on Windows");
            assert!(e.attrs.contains(Attributes::HIDDEN | Attributes::READONLY), "{:?}", e.attrs);
            set_attributes(&dst, &AttrSet { clear: Attributes::READONLY, ..Default::default() }).unwrap();
        } else {
            assert_eq!(e.mode.unwrap() & 0o777, 0o640);
        }
    }
    // Without TIMES/ATTRS the copy gets fresh metadata.
    let dst = d.path().join("fresh.txt");
    let opts = CopyOptions { preserve: Preserve::empty(), ..Default::default() };
    copy_file(&src, &dst, &opts, &ctl, &mut ()).unwrap();
    let e = stat(&dst, false).unwrap();
    assert!(e.modified.unwrap() > m + Duration::from_secs(3600));
    if cfg!(windows) {
        assert!(!e.attrs.contains(Attributes::HIDDEN));
        set_attributes(&src, &AttrSet { clear: Attributes::READONLY, ..Default::default() }).unwrap();
    }
}

#[test]
fn overwrite_replaces_readonly_destination_atomically() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("new.txt");
    write(&src, b"new contents");
    let dst = d.path().join("old.txt");
    write(&dst, b"old");
    set_attributes(&dst, &AttrSet { set: Attributes::READONLY, ..Default::default() }).unwrap();
    let ctl = OpControl::new();
    let o = CopyOptions { overwrite: Overwrite::Always, ..Default::default() };
    copy_file(&src, &dst, &o, &ctl, &mut ()).unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"new contents");
    let names: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 2, "no temp file left: {names:?}");

    // A cancelled overwrite keeps the old destination.
    let big = d.path().join("big.bin");
    write(&big, &pattern(8 * 1024 * 1024));
    let keep = d.path().join("keep.bin");
    write(&keep, b"precious");
    let ctl2 = OpControl::new();
    let c2 = ctl2.clone();
    let o = CopyOptions { overwrite: Overwrite::Always, reflink: Reflink::Never, chunk: 1 << 20, ..Default::default() };
    let res = copy_file(&big, &keep, &o, &ctl2, &mut move |p: ByteProgress| {
        if p.done > 0 {
            c2.cancel();
        }
    });
    if res.is_err() {
        assert_eq!(fs::read(&keep).unwrap(), b"precious");
    }
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 4, "temp copy removed after cancel");
}

#[test]
fn policies_and_errors() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("a.bin");
    write(&src, b"12345");
    let dst = d.path().join("b.bin");
    write(&dst, b"123");
    let ctl = OpControl::new();
    let o = |overwrite| CopyOptions { overwrite, ..Default::default() };
    assert_eq!(
        copy_file(&src, &dst, &o(Overwrite::Never), &ctl, &mut ()).unwrap_err().kind,
        FsErrorKind::AlreadyExists
    );
    assert!(matches!(
        copy_file(&src, &dst, &o(Overwrite::IfLarger), &ctl, &mut ()).unwrap(),
        CopyOutcome::Copied { .. }
    ));
    assert_eq!(
        copy_file(&src, &dst, &o(Overwrite::IfLarger), &ctl, &mut ()).unwrap(),
        CopyOutcome::Skipped(SkipReason::Smaller)
    );
    let sub = d.path().join("dir");
    fs::create_dir(&sub).unwrap();
    assert_eq!(copy_file(&src, &sub, &o(Overwrite::Always), &ctl, &mut ()).unwrap_err().kind, FsErrorKind::IsDirectory);
    assert_eq!(
        copy_file(&sub, &d.path().join("x"), &o(Overwrite::Always), &ctl, &mut ()).unwrap_err().kind,
        FsErrorKind::IsDirectory
    );
    assert_eq!(
        copy_file(&d.path().join("missing"), &d.path().join("x"), &CopyOptions::default(), &ctl, &mut ())
            .unwrap_err()
            .kind,
        FsErrorKind::NotFound
    );
    assert_eq!(
        copy_file(&src, &d.path().join("no").join("parent"), &CopyOptions::default(), &ctl, &mut ()).unwrap_err().kind,
        FsErrorKind::NotFound
    );
    // Verification passes on a good copy.
    let v =
        CopyOptions { verify: Some(HashAlgo::Sha256), overwrite: Overwrite::Always, fsync: true, ..Default::default() };
    copy_file(&src, &dst, &v, &ctl, &mut ()).unwrap();
}

#[test]
fn same_file_and_hardlinks() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    write(&a, b"x");
    let b = d.path().join("b");
    hardlink(&a, &b).unwrap();
    assert!(same_file(&a, &b).unwrap());
    assert_eq!(stat(&a, false).unwrap().nlink, Some(2));
    let c = d.path().join("c");
    write(&c, b"x");
    assert!(!same_file(&a, &c).unwrap());
    let ctl = OpControl::new();
    let o = CopyOptions { overwrite: Overwrite::Always, ..Default::default() };
    assert_eq!(copy_file(&a, &b, &o, &ctl, &mut ()).unwrap(), CopyOutcome::Skipped(SkipReason::SameFile));
    assert_eq!(device_of(&a).unwrap(), device_of(&c).unwrap());
}

#[test]
fn symlinks_are_recreated_not_followed() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("target.txt");
    write(&target, b"target");
    let link = d.path().join("link");
    if !try_symlink(std::path::Path::new("target.txt"), &link, LinkKind::File) {
        return;
    }
    let ctl = OpControl::new();
    let dst = d.path().join("link-copy");
    copy_file(&link, &dst, &CopyOptions::default(), &ctl, &mut ()).unwrap();
    assert!(matches!(stat(&dst, false).unwrap().kind, EntryKind::Symlink { .. }));
    assert_eq!(read_link(&dst).unwrap(), std::path::Path::new("target.txt"));
    let followed = d.path().join("followed");
    let o = CopyOptions { follow_symlinks: true, ..Default::default() };
    copy_file(&link, &followed, &o, &ctl, &mut ()).unwrap();
    assert_eq!(stat(&followed, false).unwrap().kind, EntryKind::File);
    assert_eq!(fs::read(&followed).unwrap(), b"target");
    // Overwriting a link replaces the link, never writes through it.
    let o = CopyOptions { overwrite: Overwrite::Always, ..Default::default() };
    let other = d.path().join("other.txt");
    write(&other, b"other");
    copy_file(&other, &dst, &o, &ctl, &mut ()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"target", "link target untouched");
}

#[test]
fn directory_metadata() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    let dst = d.path().join("dst");
    fs::create_dir(&src).unwrap();
    fs::create_dir(&dst).unwrap();
    let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567_890);
    set_times(&src, &TimesSet { modified: Some(m), ..Default::default() }, true).unwrap();
    copy_dir_metadata(&src, &dst, Preserve::TIMES | Preserve::ATTRS).unwrap();
    assert_eq!(stat(&dst, false).unwrap().modified, Some(m));
}

#[test]
fn reflink_always_reports_unsupported_or_clones() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("a");
    write(&src, &pattern(100_000));
    let dst = d.path().join("b");
    let o = CopyOptions { reflink: Reflink::Always, ..Default::default() };
    match copy_file(&src, &dst, &o, &OpControl::new(), &mut ()) {
        Ok(CopyOutcome::Copied { method, .. }) => {
            assert!(matches!(method, CopyMethod::Reflink | CopyMethod::Clonefile | CopyMethod::BlockClone));
            assert_eq!(fs::read(&dst).unwrap(), pattern(100_000));
        }
        Err(e) => {
            assert_eq!(e.kind, FsErrorKind::Unsupported, "{e:?}");
            assert!(!dst.exists());
        }
        Ok(o) => panic!("{o:?}"),
    }
}

#[cfg(windows)]
#[test]
fn alternate_streams_survive_all_paths() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("s.txt");
    write(&src, b"main");
    write(&d.path().join("s.txt:Zone.Identifier"), b"[ZoneTransfer]\r\nZoneId=3\r\n");
    let ctl = OpControl::new();
    for (i, o) in [CopyOptions::default(), CopyOptions { throttle_bps: Some(50_000_000), ..Default::default() }]
        .into_iter()
        .enumerate()
    {
        let dst = d.path().join(format!("d{i}.txt"));
        copy_file(&src, &dst, &o, &ctl, &mut ()).unwrap();
        let stream = fs::read(d.path().join(format!("d{i}.txt:Zone.Identifier"))).unwrap();
        assert!(stream.starts_with(b"[ZoneTransfer]"));
    }
}

#[cfg(windows)]
#[test]
fn sparse_files_stay_sparse() {
    use std::io::{Seek, SeekFrom, Write};
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("sparse.img");
    {
        let mut f = fs::File::create(&src).unwrap();
        drop(f);
        set_attributes(&src, &AttrSet { set: Attributes::SPARSE, ..Default::default() }).unwrap();
        f = fs::OpenOptions::new().write(true).open(&src).unwrap();
        f.write_all(b"start").unwrap();
        f.seek(SeekFrom::Start(64 * 1024 * 1024)).unwrap();
        f.write_all(b"end").unwrap();
    }
    let s = stat(&src, false).unwrap();
    assert!(s.attrs.contains(Attributes::SPARSE));
    let dst = d.path().join("copy.img");
    let mut total = 0;
    let o = CopyOptions { preserve: Preserve::TIMES | Preserve::ATTRS | Preserve::SPARSE, ..Default::default() };
    copy_file(&src, &dst, &o, &OpControl::new(), &mut |p: ByteProgress| total += p.delta).unwrap();
    assert_eq!(total, s.size);
    let e = stat(&dst, false).unwrap();
    assert!(e.attrs.contains(Attributes::SPARSE));
    assert_eq!(e.size, s.size);
    assert!(e.alloc_size.unwrap() < 4 * 1024 * 1024, "holes preserved: {:?}", e.alloc_size);
    let ctl = OpControl::new();
    assert_eq!(
        hash_file(&src, HashAlgo::Blake3, &ctl, &mut ()).unwrap(),
        hash_file(&dst, HashAlgo::Blake3, &ctl, &mut ()).unwrap()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn xattrs_are_copied_when_supported() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("x");
    write(&src, b"x");
    if ::xattr::set(&src, "user.soyuz", b"value").is_err() {
        eprintln!("filesystem without user xattrs; skipping");
        return;
    }
    let dst = d.path().join("y");
    copy_file(&src, &dst, &CopyOptions::default(), &OpControl::new(), &mut ()).unwrap();
    assert_eq!(::xattr::get(&dst, "user.soyuz").unwrap().as_deref(), Some(&b"value"[..]));
}

/// Unix names are bytes: the staged temp name is built from the raw name (non-UTF-8 names
/// survive) and its length is counted in bytes against NAME_MAX (255).
#[cfg(unix)]
#[test]
fn unix_names_are_bytes() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    write(&src, b"payload");
    let raw = std::ffi::OsString::from_vec(b"caf\xe9-\xff.bin".to_vec());
    let dst = d.path().join(&raw);
    copy_file(&src, &dst, &CopyOptions::default(), &OpControl::new(), &mut ()).unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"payload");
    let names: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(names.iter().any(|n| n.as_bytes() == raw.as_bytes()), "{names:?}");
    assert_eq!(names.len(), 2, "no temp left behind: {names:?}");
    // 120 two-byte characters = 240 bytes: fits NAME_MAX, but a temp sibling would not.
    let long: String = "é".repeat(120);
    let dst = d.path().join(&long);
    copy_file(&src, &dst, &CopyOptions::default(), &OpControl::new(), &mut ()).unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"payload");
}

/// Linux inode flags travel with `Preserve::ATTRS` (when the filesystem has them).
#[cfg(target_os = "linux")]
#[test]
fn inode_flags_are_preserved() {
    use soyuz_fs::attrs::{InodeFlags, inode_flags, set_inode_flags};
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    write(&src, b"flags");
    if let Err(e) = set_inode_flags(&src, InodeFlags::NODUMP | InodeFlags::NOATIME, InodeFlags::empty()) {
        eprintln!("no inode flags on the temp filesystem: {e}");
        return;
    }
    let dst = d.path().join("dst");
    copy_file(&src, &dst, &CopyOptions::default(), &OpControl::new(), &mut ()).unwrap();
    assert!(inode_flags(&dst).unwrap().contains(InodeFlags::NODUMP | InodeFlags::NOATIME));
    let plain = d.path().join("plain");
    let o = CopyOptions { preserve: Preserve::TIMES, ..CopyOptions::default() };
    copy_file(&src, &plain, &o, &OpControl::new(), &mut ()).unwrap();
    assert!(!inode_flags(&plain).unwrap().intersects(InodeFlags::NODUMP | InodeFlags::NOATIME));
}

/// Overwriting a file whose name leaves no room for the usual temp suffix still goes
/// through a (hash-shortened) temp sibling instead of failing with ENAMETOOLONG.
#[test]
fn replacing_a_file_with_a_long_name() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    write(&src, b"new contents");
    let dst = d.path().join(format!("{}.txt", "n".repeat(236)));
    write(&dst, b"old");
    let o = CopyOptions { overwrite: Overwrite::Always, ..CopyOptions::default() };
    copy_file(&src, &dst, &o, &OpControl::new(), &mut ()).unwrap();
    assert_eq!(fs::read(&dst).unwrap(), b"new contents");
    let names: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 2, "no temp left behind: {names:?}");
}
