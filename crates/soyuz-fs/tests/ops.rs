//! Move, delete, links, times, attributes and permissions.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use common::{try_symlink, write};
use soyuz_core::{Attributes, Control, EntryKind, FsErrorKind, OpControl};
use soyuz_fs::*;

#[test]
fn rename_modes() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    let b = d.path().join("b");
    write(&a, b"a");
    write(&b, b"b");
    let e = rename(&a, &b, RenameMode::NoReplace).unwrap_err();
    assert_eq!(e.kind, FsErrorKind::AlreadyExists);
    assert_eq!(e.path.as_deref(), Some(b.as_path()));
    rename(&a, &b, RenameMode::Replace).unwrap();
    assert_eq!(fs::read(&b).unwrap(), b"a");
    assert_eq!(rename(&a, &b, RenameMode::Replace).unwrap_err().kind, FsErrorKind::NotFound);
    // Directories rename too, and NoReplace protects existing directories.
    let d1 = d.path().join("d1");
    let d2 = d.path().join("d2");
    fs::create_dir(&d1).unwrap();
    fs::create_dir(&d2).unwrap();
    assert_eq!(rename(&d1, &d2, RenameMode::NoReplace).unwrap_err().kind, FsErrorKind::AlreadyExists);
    rename(&d1, &d.path().join("d3"), RenameMode::NoReplace).unwrap();
    let ex = rename(&b, &d2, RenameMode::Exchange);
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        // Swapping a file and a directory is allowed by RENAME_EXCHANGE / RENAME_SWAP.
        if ex.is_ok() {
            assert!(d.path().join("b").is_dir());
        }
    } else {
        assert_eq!(ex.unwrap_err().kind, FsErrorKind::Unsupported);
    }
}

#[test]
fn case_only_rename() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("readme.md");
    write(&f, b"x");
    rename_case_only(&f, "README.md").unwrap();
    let names: Vec<String> =
        fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["README.md"]);
    rename_case_only(&d.path().join("README.md"), "README.md").unwrap();
    rename_case_only(&d.path().join("README.md"), "other.md").unwrap();
    assert!(d.path().join("other.md").exists());
}

#[test]
fn cross_device_hook() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    write(&a, b"a");
    let o = MoveOptions { force_cross_device: true, ..Default::default() };
    assert_eq!(try_rename_or_crossdev(&a, &d.path().join("b"), &o).unwrap(), MoveFast::NeedsCopy);
    assert!(a.exists(), "the hook does not move anything");
    assert_eq!(try_rename_or_crossdev(&a, &d.path().join("b"), &MoveOptions::default()).unwrap(), MoveFast::Renamed);
}

#[test]
fn readonly_delete_needs_permission() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("ro.txt");
    write(&f, b"x");
    set_attributes(&f, &AttrSet { set: Attributes::READONLY, ..Default::default() }).unwrap();
    assert!(stat(&f, false).unwrap().attrs.contains(Attributes::READONLY));
    if cfg!(windows) {
        for posix in [true, false] {
            let e = delete_file(&f, &DeleteOptions { clear_readonly: false, posix_semantics: posix }).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::PermissionDenied);
            assert!(e.message.contains("read-only"), "{}", e.message);
        }
    }
    delete_file(&f, &DeleteOptions { clear_readonly: true, posix_semantics: true }).unwrap();
    assert!(!f.exists());
    let g = d.path().join("ro2.txt");
    write(&g, b"x");
    set_attributes(&g, &AttrSet { set: Attributes::READONLY, ..Default::default() }).unwrap();
    delete_file(&g, &DeleteOptions { clear_readonly: true, posix_semantics: false }).unwrap();
    assert!(!g.exists());
}

#[test]
fn delete_kinds_and_errors() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("dir");
    fs::create_dir(&dir).unwrap();
    write(&dir.join("f"), b"x");
    assert_eq!(delete_file(&dir, &DeleteOptions::default()).unwrap_err().kind, FsErrorKind::IsDirectory);
    assert_eq!(delete_empty_dir(&dir).unwrap_err().kind, FsErrorKind::NotEmpty);
    assert_eq!(delete_link(&dir).unwrap_err().kind, FsErrorKind::Unsupported);
    assert_eq!(
        delete_file(&d.path().join("missing"), &DeleteOptions::default()).unwrap_err().kind,
        FsErrorKind::NotFound
    );
    // remove_tree on a plain file just deletes it.
    let stats = remove_tree(&dir.join("f"), &DeleteOptions::default(), &OpControl::new(), &mut |_, r| {
        r.unwrap();
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.files, 1);
    delete_empty_dir(&dir).unwrap();
}

#[cfg(windows)]
#[test]
fn posix_delete_of_an_open_file() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("open.txt");
    write(&f, b"x");
    use std::os::windows::fs::OpenOptionsExt;
    let handle = fs::OpenOptions::new().read(true).share_mode(7).open(&f).unwrap();
    delete_file(&f, &DeleteOptions { clear_readonly: false, posix_semantics: true }).unwrap();
    assert!(!exists_nofollow(&f), "name gone while the handle is still open");
    drop(handle);
}

#[test]
fn remove_tree_never_follows_links() {
    let d = tempfile::tempdir().unwrap();
    let outside = d.path().join("outside");
    fs::create_dir(&outside).unwrap();
    write(&outside.join("keep.txt"), b"keep");
    let tree = d.path().join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    write(&tree.join("sub").join("f.txt"), b"x");
    let mut links = 0;
    if try_symlink(&outside, &tree.join("dirlink"), LinkKind::Dir) {
        links += 1;
    }
    #[cfg(windows)]
    {
        junction(&outside, &tree.join("junction")).unwrap();
        links += 1;
    }
    let ctl = OpControl::new();
    let mut seen = Vec::new();
    let stats = remove_tree(&tree, &DeleteOptions::default(), &ctl, &mut |p, r| {
        r.unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        seen.push(p.to_path_buf());
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.links, links);
    assert_eq!(stats.files, 1);
    assert_eq!(stats.dirs, 2);
    assert_eq!(stats.failed, 0);
    assert!(!tree.exists());
    assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep", "link targets untouched");
    assert_eq!(seen.last().unwrap(), &tree, "root removed last");
}

#[test]
fn remove_tree_stops_and_cancels() {
    let d = tempfile::tempdir().unwrap();
    let tree = d.path().join("t");
    fs::create_dir(&tree).unwrap();
    for i in 0..10 {
        write(&tree.join(format!("{i}")), b"");
    }
    let ctl = OpControl::new();
    let mut n = 0;
    remove_tree(&tree, &DeleteOptions::default(), &ctl, &mut |_, _| {
        n += 1;
        if n == 3 { Control::Stop } else { Control::Continue }
    })
    .unwrap();
    assert_eq!(n, 3);
    assert_eq!(fs::read_dir(&tree).unwrap().count(), 7);
    ctl.cancel();
    assert!(
        remove_tree(&tree, &DeleteOptions::default(), &ctl, &mut |_, _| Control::Continue).unwrap_err().is_cancel()
    );
}

#[test]
fn links_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("dir with space");
    fs::create_dir(&target).unwrap();
    let rel = d.path().join("rel");
    if try_symlink(Path::new("dir with space"), &rel, LinkKind::Dir) {
        assert_eq!(read_link(&rel).unwrap(), Path::new("dir with space"));
        assert_eq!(resolve_link(&rel).unwrap(), normalize(&target));
        let abs = d.path().join("abs");
        assert!(try_symlink(&target, &abs, LinkKind::Dir));
        assert_eq!(strip_verbatim(&read_link(&abs).unwrap()), strip_verbatim(&normalize(&target)));
        assert_eq!(symlink(&target, &abs, LinkKind::Dir).unwrap_err().kind, FsErrorKind::AlreadyExists);
        delete_link(&abs).unwrap();
        delete_link(&rel).unwrap();
        assert!(target.exists());
    }
    let f = d.path().join("f");
    write(&f, b"x");
    assert_eq!(read_link(&f).unwrap_err().kind, if cfg!(windows) { FsErrorKind::Unsupported } else { FsErrorKind::Io });
    assert_eq!(hardlink(&target, &d.path().join("h")).unwrap_err().kind, FsErrorKind::IsDirectory);
    let j = junction(&target, &d.path().join("j"));
    if cfg!(windows) {
        j.unwrap();
        let jp = d.path().join("j");
        assert_eq!(strip_verbatim(&read_link(&jp).unwrap()), strip_verbatim(&normalize(&target)));
        assert_eq!(resolve_link(&jp).unwrap(), normalize(&target));
        // Copying a junction recreates a junction.
        let ctl = OpControl::new();
        copy_file(&jp, &d.path().join("j2"), &CopyOptions::default(), &ctl, &mut ()).unwrap();
        assert_eq!(stat(&d.path().join("j2"), false).unwrap().kind, EntryKind::Junction);
        delete_link(&jp).unwrap();
        assert!(target.exists());
        assert_eq!(junction(&target, &d.path().join("j2")).unwrap_err().kind, FsErrorKind::AlreadyExists);
    } else {
        assert_eq!(j.unwrap_err().kind, FsErrorKind::Unsupported);
    }
}

#[test]
fn times_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("t");
    write(&f, b"x");
    let m = SystemTime::UNIX_EPOCH + Duration::new(1_111_111_111, 123_456_700);
    let a = SystemTime::UNIX_EPOCH + Duration::new(1_222_222_222, 0);
    set_times(&f, &TimesSet { modified: Some(m), accessed: Some(a), created: None }, true).unwrap();
    let e = stat(&f, false).unwrap();
    assert_eq!(e.modified, Some(m));
    assert_eq!(e.accessed, Some(a));
    // Only accessed: modified untouched.
    let a2 = SystemTime::UNIX_EPOCH + Duration::new(1_333_333_333, 0);
    set_times(&f, &TimesSet { accessed: Some(a2), ..Default::default() }, true).unwrap();
    assert_eq!(stat(&f, false).unwrap().modified, Some(m));
    if cfg!(any(windows, target_os = "macos")) {
        let c = SystemTime::UNIX_EPOCH + Duration::new(1_000_000_000, 0);
        set_times(&f, &TimesSet { created: Some(c), ..Default::default() }, true).unwrap();
        assert_eq!(stat(&f, false).unwrap().created, Some(c));
    }
    let snap = meta_snapshot(&f).unwrap();
    assert_eq!(snap.times.modified, Some(m));
    assert_eq!(
        set_times(&d.path().join("missing"), &TimesSet { modified: Some(m), ..Default::default() }, true)
            .unwrap_err()
            .kind,
        FsErrorKind::NotFound
    );
}

#[test]
fn attribute_round_trips() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a");
    write(&f, b"x");
    if cfg!(windows) {
        let all = Attributes::HIDDEN | Attributes::SYSTEM | Attributes::ARCHIVE | Attributes::NOT_CONTENT_INDEXED;
        set_attributes(&f, &AttrSet { set: all, ..Default::default() }).unwrap();
        assert!(stat(&f, false).unwrap().attrs.contains(all));
        set_attributes(&f, &AttrSet { clear: all, ..Default::default() }).unwrap();
        assert!(!stat(&f, false).unwrap().attrs.intersects(all));
        assert_eq!(
            set_attributes(&f, &AttrSet { set: Attributes::EXECUTABLE, ..Default::default() }).unwrap_err().kind,
            FsErrorKind::Unsupported
        );
        // Compression works on NTFS; other filesystems may refuse.
        if set_attributes(&f, &AttrSet { set: Attributes::COMPRESSED, ..Default::default() }).is_ok() {
            assert!(stat(&f, false).unwrap().attrs.contains(Attributes::COMPRESSED));
            set_attributes(&f, &AttrSet { clear: Attributes::COMPRESSED, ..Default::default() }).unwrap();
        }
        assert_eq!(set_mode(&f, 0o644).unwrap_err().kind, FsErrorKind::Unsupported);
    } else {
        set_attributes(&f, &AttrSet { set: Attributes::EXECUTABLE, ..Default::default() }).unwrap();
        let e = stat(&f, false).unwrap();
        assert!(e.attrs.contains(Attributes::EXECUTABLE));
        set_mode(&f, 0o600).unwrap();
        assert_eq!(stat(&f, false).unwrap().mode.unwrap() & 0o777, 0o600);
        set_attributes(&f, &AttrSet { set: Attributes::READONLY, ..Default::default() }).unwrap();
        assert_eq!(stat(&f, false).unwrap().mode.unwrap() & 0o222, 0);
        set_attributes(&f, &AttrSet { clear: Attributes::READONLY, ..Default::default() }).unwrap();
        assert_ne!(stat(&f, false).unwrap().mode.unwrap() & 0o200, 0);
        assert_eq!(
            set_attributes(&f, &AttrSet { set: Attributes::SYSTEM, ..Default::default() }).unwrap_err().kind,
            FsErrorKind::Unsupported
        );
        chown(&f, None, None).unwrap();
    }
    let snap = meta_snapshot(&f).unwrap();
    assert_eq!(snap.attrs, stat(&f, false).unwrap().attrs);
}

#[test]
fn security_view() {
    let d = tempfile::tempdir().unwrap();
    let info = security_info(d.path()).unwrap();
    assert!(!info.owner.is_empty());
    assert!(!info.entries.is_empty());
    let r = effective_access(d.path()).unwrap();
    assert!(r.contains(AccessRights::READ | AccessRights::WRITE));
    let f = d.path().join("ro");
    write(&f, b"x");
    set_attributes(&f, &AttrSet { set: Attributes::READONLY, ..Default::default() }).unwrap();
    let r = effective_access(&f).unwrap();
    // (root ignores mode bits on unix, so only Windows is asserted)
    if cfg!(windows) {
        assert!(!r.contains(AccessRights::WRITE));
    }
    set_attributes(&f, &AttrSet { clear: Attributes::READONLY, ..Default::default() }).unwrap();
}

#[test]
fn hash_progress_and_vectors() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("h");
    write(&f, &vec![b'a'; 3 * 1024 * 1024]);
    let ctl = OpControl::new();
    let mut calls = 0;
    let mut sum = 0;
    let h = hash_file(&f, soyuz_core::HashAlgo::Sha1, &ctl, &mut |p: soyuz_core::ByteProgress| {
        calls += 1;
        sum += p.delta;
    })
    .unwrap();
    assert_eq!(h.len(), 40);
    assert_eq!(sum, 3 * 1024 * 1024);
    assert!(calls >= 3);
    assert_eq!(
        hash_file(&d.path().join("missing"), soyuz_core::HashAlgo::Md5, &ctl, &mut ()).unwrap_err().kind,
        FsErrorKind::NotFound
    );
}
