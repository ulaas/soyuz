//! Behavioural tests for the public `soyuz-fs` API. They run against whatever
//! implementation is active (M0 baseline or the raw per-OS paths), so they must keep
//! passing when bodies are replaced.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use soyuz_core::{ByteProgress, Control, EntryKind, FsErrorKind, HashAlgo, OpControl, Overwrite};
use soyuz_fs::*;

fn write(p: &Path, bytes: &[u8]) {
    fs::write(p, bytes).unwrap();
}

#[test]
fn list_and_stat() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join("a.txt"), b"hello");
    write(&d.path().join(".hidden"), b"x");
    fs::create_dir(d.path().join("sub")).unwrap();

    let mut all = read_dir(d.path(), &ListOptions::default()).unwrap();
    all.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, [".hidden", "a.txt", "sub"]);
    let a = all.iter().find(|e| e.name == "a.txt").unwrap();
    assert_eq!(a.kind, EntryKind::File);
    assert_eq!(a.size, 5);
    assert!(a.modified.is_some());
    assert_eq!(all.iter().find(|e| e.name == "sub").unwrap().kind, EntryKind::Dir);

    let visible = read_dir(d.path(), &ListOptions { include_hidden: false, ..Default::default() }).unwrap();
    // On Windows ".hidden" has no HIDDEN attribute but dot-files count as hidden everywhere.
    assert_eq!(visible.len(), 2);

    let s = stat(&d.path().join("a.txt"), true).unwrap();
    assert_eq!(s.size, 5);
    assert!(exists_nofollow(&d.path().join("a.txt")));
    assert!(!exists_nofollow(&d.path().join("missing")));
    assert_eq!(stat(&d.path().join("missing"), false).unwrap_err().kind, FsErrorKind::NotFound);
}

#[test]
fn streaming_chunks_and_stop() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..10 {
        write(&d.path().join(format!("f{i}")), b"");
    }
    let ctl = OpControl::new();
    let mut chunks = Vec::new();
    let opts = ListOptions { chunk: 3, ..Default::default() };
    let stats = read_dir_streaming(d.path(), &opts, &ctl, &mut |c| {
        chunks.push(c.len());
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.entries, 10);
    assert!(!stats.truncated);
    assert_eq!(chunks, [3, 3, 3, 1]);

    let stats = read_dir_streaming(d.path(), &opts, &ctl, &mut |_| Control::Stop).unwrap();
    assert!(stats.truncated);
    assert_eq!(stats.entries, 3);

    ctl.cancel();
    let err = read_dir_streaming(d.path(), &opts, &ctl, &mut |_| Control::Continue).unwrap_err();
    assert!(err.is_cancel());
}

fn make_tree(root: &Path) {
    fs::create_dir_all(root.join("a/b")).unwrap();
    write(&root.join("top.txt"), b"12345");
    write(&root.join("a/one.txt"), b"1");
    write(&root.join("a/b/two.txt"), b"22");
}

#[test]
fn walk_pre_and_post_order() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("t");
    make_tree(&root);
    let ctl = OpControl::new();
    let mut seen = Vec::new();
    let opts = WalkOptions { post_order: true, ..Default::default() };
    let stats = walk(&root, &opts, &ctl, &mut |item| {
        match item {
            WalkItem::Entry { entry, depth } => seen.push(format!("E{depth}:{}", entry.name)),
            WalkItem::DirDone { path, depth } => {
                seen.push(format!("D{depth}:{}", path.file_name().unwrap().to_string_lossy()))
            }
            WalkItem::Error { path, error } => panic!("{path:?}: {error}"),
        }
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.files, 3);
    assert_eq!(stats.dirs, 3);
    assert_eq!(stats.bytes, 8);
    assert_eq!(seen.first().unwrap(), "E0:t");
    assert_eq!(seen.last().unwrap(), "D0:t");
    // Every DirDone comes after all of its children.
    let pos = |s: &str| seen.iter().position(|x| x == s).unwrap();
    assert!(pos("D2:b") > pos("E3:two.txt"));
    assert!(pos("D1:a") > pos("D2:b"));

    let shallow =
        walk(&root, &WalkOptions { max_depth: Some(1), ..Default::default() }, &ctl, &mut |_| Control::Continue)
            .unwrap();
    assert_eq!(shallow.files, 1);
    assert_eq!(shallow.dirs, 2);
}

#[test]
fn copy_basic_and_policies() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src.bin");
    let dst = d.path().join("dst.bin");
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    write(&src, &data);
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    set_times(&src, &TimesSet { modified: Some(old), ..Default::default() }, true).unwrap();

    let ctl = OpControl::new();
    let mut total = 0;
    let out = copy_file(&src, &dst, &CopyOptions::default(), &ctl, &mut |p: ByteProgress| total += p.delta).unwrap();
    assert!(matches!(out, CopyOutcome::Copied { bytes: 100_000, .. }));
    assert_eq!(total, 100_000);
    assert_eq!(fs::read(&dst).unwrap(), data);
    let mtime = fs::metadata(&dst).unwrap().modified().unwrap();
    assert_eq!(mtime, old, "mtime preserved");

    // Never → AlreadyExists; IfNewer → skipped (same mtime); Always → copied.
    let err = copy_file(&src, &dst, &CopyOptions::default(), &ctl, &mut ()).unwrap_err();
    assert_eq!(err.kind, FsErrorKind::AlreadyExists);
    let o = CopyOptions { overwrite: Overwrite::IfNewer, ..Default::default() };
    assert_eq!(copy_file(&src, &dst, &o, &ctl, &mut ()).unwrap(), CopyOutcome::Skipped(SkipReason::Older));
    let o = CopyOptions { overwrite: Overwrite::IfDifferentSize, ..Default::default() };
    assert_eq!(copy_file(&src, &dst, &o, &ctl, &mut ()).unwrap(), CopyOutcome::Skipped(SkipReason::SameSize));
    let o = CopyOptions { overwrite: Overwrite::Always, verify: Some(HashAlgo::Blake3), ..Default::default() };
    assert!(matches!(copy_file(&src, &dst, &o, &ctl, &mut ()).unwrap(), CopyOutcome::Copied { .. }));
    assert_eq!(copy_file(&src, &src, &o, &ctl, &mut ()).unwrap(), CopyOutcome::Skipped(SkipReason::SameFile));
}

#[test]
fn throttled_copy_reports_progress_and_cancels() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big.bin");
    write(&src, &vec![7u8; 400_000]);
    let ctl = OpControl::new();

    // Throttled to 1 MB/s: completes in ~0.4 s with several monotonic progress callbacks.
    let dst = d.path().join("ok.bin");
    let o = CopyOptions { throttle_bps: Some(1_000_000), ..Default::default() };
    let mut last = 0;
    let mut calls = 0;
    copy_file(&src, &dst, &o, &ctl, &mut |p: ByteProgress| {
        assert!(p.done >= last);
        last = p.done;
        calls += 1;
    })
    .unwrap();
    assert_eq!(last, 400_000);
    assert!(calls > 3, "{calls}");

    // Cancel mid-copy: Err(Interrupted) and no partial destination.
    let dst2 = d.path().join("cancel.bin");
    let o = CopyOptions { throttle_bps: Some(200_000), ..Default::default() };
    let ctl2 = OpControl::new();
    let c = ctl2.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        c.cancel();
    });
    let err = copy_file(&src, &dst2, &o, &ctl2, &mut ()).unwrap_err();
    t.join().unwrap();
    assert!(err.is_cancel());
    assert!(!dst2.exists());
}

#[test]
fn rename_and_move() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.txt");
    let b = d.path().join("b.txt");
    write(&a, b"a");
    write(&b, b"b");
    assert_eq!(rename(&a, &b, RenameMode::NoReplace).unwrap_err().kind, FsErrorKind::AlreadyExists);
    rename(&a, &d.path().join("c.txt"), RenameMode::NoReplace).unwrap();
    rename(&d.path().join("c.txt"), &b, RenameMode::Replace).unwrap();
    assert_eq!(fs::read(&b).unwrap(), b"a");

    rename_case_only(&b, "B.txt").unwrap();
    let names: Vec<String> =
        fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["B.txt"]);

    let forced = MoveOptions { force_cross_device: true, ..Default::default() };
    let b2 = d.path().join("B.txt");
    assert_eq!(try_rename_or_crossdev(&b2, &d.path().join("x"), &forced).unwrap(), MoveFast::NeedsCopy);
    assert_eq!(try_rename_or_crossdev(&b2, &d.path().join("x"), &MoveOptions::default()).unwrap(), MoveFast::Renamed);
}

#[test]
fn delete_and_remove_tree() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("t");
    make_tree(&root);
    let ro = root.join("a/readonly.txt");
    write(&ro, b"r");
    set_attributes(&ro, &AttrSet { set: soyuz_core::Attributes::READONLY, ..Default::default() }).unwrap();
    let ctl = OpControl::new();
    let mut items = 0;
    let stats = remove_tree(&root, &DeleteOptions { clear_readonly: true, ..Default::default() }, &ctl, &mut |_, r| {
        r.unwrap();
        items += 1;
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.files, 4);
    assert_eq!(stats.dirs, 3);
    assert_eq!(stats.failed, 0);
    assert_eq!(items, 7);
    assert!(!root.exists());

    let f = d.path().join("f");
    create_file(&f, true).unwrap();
    assert_eq!(create_file(&f, true).unwrap_err().kind, FsErrorKind::AlreadyExists);
    delete_file(&f, &DeleteOptions::default()).unwrap();
    create_dir(&d.path().join("e")).unwrap();
    delete_empty_dir(&d.path().join("e")).unwrap();
}

#[test]
fn secure_wipe_removes_file() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("secret");
    write(&f, &[1u8; 5000]);
    let mut done = 0;
    secure_wipe(&f, 2, &OpControl::new(), &mut |p: ByteProgress| done = p.done).unwrap();
    assert_eq!(done, 10_000);
    assert!(!f.exists());
}

#[test]
fn hashes_known_vectors() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("abc");
    write(&f, b"abc");
    let ctl = OpControl::new();
    let h = |a| hash_file(&f, a, &ctl, &mut ()).unwrap();
    assert_eq!(h(HashAlgo::Sha256), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(h(HashAlgo::Sha1), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(h(HashAlgo::Md5), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(h(HashAlgo::Crc32), "352441c2");
    assert_eq!(h(HashAlgo::Blake3), "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85");
}

#[test]
fn atomic_write_and_meta() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("session.toml");
    write_atomic(&f, b"one").unwrap();
    write_atomic(&f, b"two").unwrap();
    assert_eq!(fs::read(&f).unwrap(), b"two");
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1, "no temp files left");
    let snap = meta_snapshot(&f).unwrap();
    assert!(snap.times.modified.is_some());
}

#[test]
fn volumes_and_roots() {
    assert!(!volumes().is_empty());
    let d = tempfile::tempdir().unwrap();
    let root = volume_root(d.path()).unwrap();
    assert!(d.path().starts_with(&root) || normalize(d.path()).starts_with(&root), "{root:?}");
    assert!(device_of(d.path()).is_ok());
    let probe = RealProbe;
    assert_eq!(probe.device_of(d.path()).unwrap(), device_of(d.path()).unwrap());
}

#[test]
fn unsupported_baseline_surfaces() {
    let d = tempfile::tempdir().unwrap();
    // These may become supported later; they must never panic.
    let _ = junction(d.path(), &d.path().join("j"));
    let _ = security_info(d.path());
    let _ = effective_access(d.path());
    let _ = trash_support(d.path());
    if cfg!(windows) {
        assert_eq!(set_mode(d.path(), 0o755).unwrap_err().kind, FsErrorKind::Unsupported);
        assert_eq!(chown(d.path(), None, None).unwrap_err().kind, FsErrorKind::Unsupported);
    }
}

#[cfg(unix)]
#[test]
fn symlinks_unix() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = d.path().join("link");
    symlink(Path::new("target"), &link, LinkKind::Dir).unwrap();
    let e = stat(&link, false).unwrap();
    assert_eq!(e.kind, EntryKind::Symlink { target: soyuz_core::LinkTarget::Dir });
    assert_eq!(read_link(&link).unwrap(), Path::new("target"));
    assert_eq!(resolve_link(&link).unwrap(), normalize(&target));
    delete_link(&link).unwrap();
    assert!(target.exists());
}

#[test]
fn watcher_reports_changes() {
    let d = tempfile::tempdir().unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let w2 = wakes.clone();
    let mut w = Watcher::new(
        tx,
        Duration::from_millis(50),
        Arc::new(move || {
            w2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }),
    )
    .unwrap();
    let id = w.watch(d.path(), false).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    write(&d.path().join("new.txt"), b"x");
    let ev = rx.recv_timeout(Duration::from_secs(5)).expect("watch event");
    assert_eq!(ev.id, id);
    assert!(matches!(ev.kind, WatchKind::Changed(_) | WatchKind::Rescan), "{ev:?}");
    // `wake` runs right after the send; give it a moment.
    let start = std::time::Instant::now();
    while wakes.load(std::sync::atomic::Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(wakes.load(std::sync::atomic::Ordering::SeqCst) >= 1);
    w.unwatch(id);
}
