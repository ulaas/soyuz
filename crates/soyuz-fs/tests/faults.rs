//! Fault-injection tests of the copy / move / delete internals (feature `fault-inject`;
//! enabled for `cargo test --workspace` through soyuz-jobs' dev-dependency).
#![cfg(feature = "fault-inject")]

use std::path::Path;
use std::time::{Duration, Instant};

use soyuz_core::{FsErrorKind, OpControl, Overwrite};
use soyuz_fs::fault::{FaultKind, FaultOp, FaultPlan, install};
use soyuz_fs::{CopyOptions, MoveFast, MoveOptions, RenameMode};

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn big(path: &Path, len: usize) {
    let data: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(path, data).unwrap();
}

/// Small chunks so a write fault lands mid-file on every copy path.
fn opts() -> CopyOptions {
    CopyOptions { chunk: 64 * 1024, throttle_bps: Some(1 << 40), ..CopyOptions::default() }
}

#[test]
fn enospc_mid_copy_leaves_nothing_behind() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src.bin");
    big(&src, 1 << 20);
    let out = d.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let g = install(FaultPlan::new(&out).fail(FaultOp::Write, 3, FaultKind::no_space()));
    let e = soyuz_fs::copy_file(&src, &out.join("dst.bin"), &opts(), &OpControl::new(), &mut ()).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NoSpace, "{e}");
    assert_eq!(g.fired().len(), 1);
    assert!(names(&out).is_empty(), "no partial, no temp: {:?}", names(&out));
    assert_eq!(std::fs::metadata(&src).unwrap().len(), 1 << 20, "source intact");
}

#[test]
fn enospc_on_default_copy_path_too() {
    // Without throttling Windows uses CopyFileExW; the fault fires inside its callback.
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src.bin");
    big(&src, 3 << 20);
    let out = d.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let _g = install(FaultPlan::new(&out).fail_from(FaultOp::Write, 1, FaultKind::no_space()));
    let o = CopyOptions { chunk: 1 << 20, ..CopyOptions::default() };
    let e = soyuz_fs::copy_file(&src, &out.join("dst.bin"), &o, &OpControl::new(), &mut ()).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NoSpace, "{e}");
    assert!(names(&out).is_empty(), "{:?}", names(&out));
}

#[test]
fn failed_overwrite_keeps_the_old_file() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("new.bin");
    big(&src, 512 * 1024);
    let dst = d.path().join("old.bin");
    std::fs::write(&dst, b"precious").unwrap();
    let _g = install(FaultPlan::new(d.path()).on_name(FaultOp::Write, "old.bin", FaultKind::io_error()));
    let o = CopyOptions { overwrite: Overwrite::Always, ..opts() };
    assert!(soyuz_fs::copy_file(&src, &dst, &o, &OpControl::new(), &mut ()).is_err());
    assert_eq!(std::fs::read(&dst).unwrap(), b"precious");
    assert_eq!(names(d.path()), vec!["new.bin", "old.bin"]);
}

#[test]
fn copy_is_staged_under_a_temp_name() {
    // A failure of the final rename (the temp → destination step) cleans the temp up and
    // never leaves a file under the final name.
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("a.txt");
    std::fs::write(&src, b"data").unwrap();
    let out = d.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let g = install(FaultPlan::new(&out).fail(FaultOp::Rename, 1, FaultKind::access_denied()));
    let e = soyuz_fs::copy_file(&src, &out.join("a.txt"), &opts(), &OpControl::new(), &mut ()).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::PermissionDenied, "{e}");
    let fired = g.fired();
    assert!(fired[0].1.to_string_lossy().contains(soyuz_fs::copy::PART_MARKER), "{fired:?}");
    assert!(names(&out).is_empty());
    drop(g);
    soyuz_fs::copy_file(&src, &out.join("a.txt"), &opts(), &OpControl::new(), &mut ()).unwrap();
    assert_eq!(names(&out), vec!["a.txt"]);
}

#[test]
fn vanished_source_is_not_found() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("gone.txt");
    std::fs::write(&src, b"x").unwrap();
    let _g = install(FaultPlan::new(d.path()).fail(FaultOp::Open, 1, FaultKind::Vanish));
    let e = soyuz_fs::copy_file(&src, &d.path().join("dst.txt"), &opts(), &OpControl::new(), &mut ()).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::NotFound);
    assert!(names(d.path()).is_empty());
}

#[test]
fn exdev_rename_falls_back_to_copy() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    std::fs::write(&a, b"x").unwrap();
    let _g = install(FaultPlan::new(d.path()).fail(FaultOp::Rename, 1, FaultKind::cross_device()));
    let e = soyuz_fs::rename(&a, &d.path().join("b"), RenameMode::NoReplace).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::CrossDevice);
    // The next rename attempt is reported as "needs a copy" by the move helper only when it
    // is cross-device; here the fault has fired once, so it succeeds.
    let o = MoveOptions::default();
    assert_eq!(soyuz_fs::try_rename_or_crossdev(&a, &d.path().join("b"), &o).unwrap(), MoveFast::Renamed);
    let _g2 = install(FaultPlan::new(d.path()).fail(FaultOp::Rename, 1, FaultKind::cross_device()));
    assert_eq!(soyuz_fs::try_rename_or_crossdev(&d.path().join("b"), &a, &o).unwrap(), MoveFast::NeedsCopy);
}

#[test]
fn delete_failure_and_mkdir_failure() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("f");
    std::fs::write(&f, b"x").unwrap();
    let g = install(FaultPlan::new(d.path()).fail(FaultOp::Delete, 1, FaultKind::in_use()).fail(
        FaultOp::Create,
        1,
        FaultKind::access_denied(),
    ));
    let o = soyuz_fs::DeleteOptions::default();
    assert_eq!(soyuz_fs::delete_file(&f, &o).unwrap_err().kind, FsErrorKind::InUse);
    assert!(f.exists());
    soyuz_fs::delete_file(&f, &o).unwrap();
    assert!(soyuz_fs::create_dir(&d.path().join("x")).is_err());
    soyuz_fs::create_dir(&d.path().join("x")).unwrap();
    assert_eq!(g.fired().len(), 2);
}

#[test]
fn slow_io_is_slow() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("s");
    big(&src, 256 * 1024);
    let _g = install(FaultPlan::new(d.path()).slow(FaultOp::Write, Duration::from_millis(30)));
    let t = Instant::now();
    soyuz_fs::copy_file(&src, &d.path().join("t"), &opts(), &OpControl::new(), &mut ()).unwrap();
    assert!(t.elapsed() >= Duration::from_millis(90), "4 chunks × 30 ms");
    assert_eq!(std::fs::read(d.path().join("t")).unwrap(), std::fs::read(&src).unwrap());
}
