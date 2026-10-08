//! Listing, stat and walk against the raw per-OS implementations.

mod common;

use std::collections::HashMap;
use std::fs;

use common::{try_symlink, write};
use soyuz_core::{Attributes, Control, EntryKind, FsErrorKind, LinkTarget, OpControl};
use soyuz_fs::*;

#[test]
fn metadata_fields_are_filled() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("data.bin");
    write(&f, &[7u8; 3000]);
    let e = stat(&f, false).unwrap();
    assert_eq!(e.kind, EntryKind::File);
    assert_eq!(e.size, 3000);
    assert!(e.modified.is_some() && e.accessed.is_some());
    assert!(e.alloc_size.is_some());
    assert!(e.dev.is_some() && e.inode.is_some());
    assert_eq!(e.nlink, Some(1));
    assert_eq!(e.dev.unwrap(), device_of(&f).unwrap());
    if cfg!(unix) {
        assert!(e.mode.is_some());
        assert!(e.owner.is_some(), "stat resolves the owner on unix");
    } else {
        assert!(e.created.is_some());
    }
    let listed = read_dir(d.path(), &ListOptions { owners: true, ..Default::default() }).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].size, 3000);
    assert_eq!(listed[0].modified, e.modified);
    assert!(listed[0].owner.is_some(), "owners requested");
    let dir = stat(d.path(), true).unwrap();
    assert_eq!(dir.kind, EntryKind::Dir);
    assert_eq!(dir.size, 0);
}

#[test]
fn unicode_and_odd_names() {
    let d = tempfile::tempdir().unwrap();
    let names = ["日本語.txt", "Ünïcödé", "emoji 🚀 file", "spaces  inside", "semi;colon,comma", "e\u{301}cole"];
    for n in names {
        write(&d.path().join(n), n.as_bytes());
    }
    let mut got: Vec<String> =
        read_dir(d.path(), &ListOptions::default()).unwrap().into_iter().map(|e| e.name).collect();
    got.sort();
    let mut want: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(got, want);
    for n in names {
        assert_eq!(stat(&d.path().join(n), false).unwrap().size, n.len() as u64);
    }
}

#[test]
fn hidden_entries() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join(".dot"), b"");
    let plain = d.path().join("plain.txt");
    write(&plain, b"");
    let hidden_attr = d.path().join("attr-hidden.txt");
    write(&hidden_attr, b"");
    let attr_ok = set_attributes(&hidden_attr, &AttrSet { set: Attributes::HIDDEN, ..Default::default() }).is_ok();
    let visible: Vec<String> = read_dir(d.path(), &ListOptions { include_hidden: false, ..Default::default() })
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(visible.contains(&"plain.txt".to_string()));
    assert!(!visible.contains(&".dot".to_string()));
    if attr_ok {
        assert!(!visible.contains(&"attr-hidden.txt".to_string()));
        assert!(stat(&hidden_attr, false).unwrap().attrs.contains(Attributes::HIDDEN));
    }
}

#[test]
fn large_directory_streams_in_chunks() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..1500 {
        write(&d.path().join(format!("f{i:05}")), b"");
    }
    let ctl = OpControl::new();
    let mut sizes = Vec::new();
    let opts = ListOptions { chunk: 512, stat_links: false, ..Default::default() };
    let stats = read_dir_streaming(d.path(), &opts, &ctl, &mut |c| {
        sizes.push(c.len());
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.entries, 1500);
    assert_eq!(sizes, [512, 512, 476]);
    assert_eq!(read_dir(d.path(), &opts).unwrap().len(), 1500);
}

#[test]
fn listing_errors() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("file");
    write(&f, b"x");
    assert_eq!(read_dir(&d.path().join("missing"), &ListOptions::default()).unwrap_err().kind, FsErrorKind::NotFound);
    let err = read_dir(&f, &ListOptions::default()).unwrap_err();
    assert!(matches!(err.kind, FsErrorKind::NotDirectory | FsErrorKind::NotFound | FsErrorKind::Io), "{err:?}");
    assert_eq!(err.op, "read_dir");
    let empty = d.path().join("empty");
    fs::create_dir(&empty).unwrap();
    assert!(read_dir(&empty, &ListOptions::default()).unwrap().is_empty());
}

#[test]
fn symlink_kinds_and_targets() {
    let d = tempfile::tempdir().unwrap();
    let target_dir = d.path().join("tdir");
    fs::create_dir(&target_dir).unwrap();
    write(&target_dir.join("inner.txt"), b"abc");
    let target_file = d.path().join("tfile.txt");
    write(&target_file, b"12345");
    let ldir = d.path().join("ldir");
    let lfile = d.path().join("lfile");
    let ldangling = d.path().join("ldangling");
    if !try_symlink(&target_dir, &ldir, LinkKind::Dir) {
        return;
    }
    assert!(try_symlink(&target_file, &lfile, LinkKind::File));
    assert!(try_symlink(&d.path().join("nowhere"), &ldangling, LinkKind::File));

    let by_name: HashMap<String, soyuz_core::EntryInfo> =
        read_dir(d.path(), &ListOptions::default()).unwrap().into_iter().map(|e| (e.name.clone(), e)).collect();
    assert_eq!(by_name["ldir"].kind, EntryKind::Symlink { target: LinkTarget::Dir });
    assert_eq!(by_name["lfile"].kind, EntryKind::Symlink { target: LinkTarget::File });
    assert_eq!(by_name["ldangling"].kind, EntryKind::Symlink { target: LinkTarget::Missing });
    assert!(by_name["ldir"].is_dir_like());

    // follow_symlinks reports the targets' metadata.
    let followed: HashMap<String, soyuz_core::EntryInfo> =
        read_dir(d.path(), &ListOptions { follow_symlinks: true, ..Default::default() })
            .unwrap()
            .into_iter()
            .map(|e| (e.name.clone(), e))
            .collect();
    assert_eq!(followed["ldir"].kind, EntryKind::Dir);
    assert_eq!(followed["lfile"].kind, EntryKind::File);
    assert_eq!(followed["lfile"].size, 5);
    assert!(matches!(followed["ldangling"].kind, EntryKind::Symlink { .. }));

    assert_eq!(stat(&lfile, true).unwrap().size, 5);
    assert!(matches!(stat(&ldangling, true).unwrap().kind, EntryKind::Symlink { .. }), "dangling link reports itself");
    assert!(exists_nofollow(&ldangling));

    // walk never descends into the linked dir unless asked.
    let ctl = OpControl::new();
    let mut names = Vec::new();
    walk(d.path(), &WalkOptions::default(), &ctl, &mut |i| {
        if let WalkItem::Entry { entry, .. } = i {
            names.push(entry.name);
        }
        Control::Continue
    })
    .unwrap();
    assert_eq!(names.iter().filter(|n| *n == "inner.txt").count(), 1, "{names:?}");
    let mut followed_names = Vec::new();
    walk(d.path(), &WalkOptions { follow_symlinks: true, ..Default::default() }, &ctl, &mut |i| {
        if let WalkItem::Entry { entry, .. } = i {
            followed_names.push(entry.name);
        }
        Control::Continue
    })
    .unwrap();
    assert_eq!(followed_names.iter().filter(|n| *n == "inner.txt").count(), 2, "{followed_names:?}");
}

#[test]
fn symlink_loops_are_detected() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    fs::create_dir(&a).unwrap();
    if !try_symlink(d.path(), &a.join("back"), LinkKind::Dir) {
        return;
    }
    let ctl = OpControl::new();
    let mut loops = 0;
    let stats = walk(d.path(), &WalkOptions { follow_symlinks: true, ..Default::default() }, &ctl, &mut |i| {
        if let WalkItem::Error { error, .. } = i {
            assert_eq!(error.kind, FsErrorKind::Loop);
            loops += 1;
        }
        Control::Continue
    })
    .unwrap();
    assert!(loops >= 1);
    assert!(stats.errors >= 1);
}

#[test]
fn walk_same_device_and_cancel() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("x/y/z")).unwrap();
    write(&d.path().join("x/y/z/f"), b"1");
    let ctl = OpControl::new();
    let s = walk(d.path(), &WalkOptions { same_device: true, ..Default::default() }, &ctl, &mut |_| Control::Continue)
        .unwrap();
    assert_eq!(s.files, 1);
    assert_eq!(s.dirs, 4);
    ctl.cancel();
    assert!(walk(d.path(), &WalkOptions::default(), &ctl, &mut |_| Control::Continue).unwrap_err().is_cancel());
}

#[cfg(windows)]
#[test]
fn long_paths_beyond_max_path() {
    let d = tempfile::tempdir().unwrap();
    let mut p = d.path().to_path_buf();
    while p.as_os_str().len() < 400 {
        p.push("a_rather_long_directory_name_component");
    }
    create_dir_all(&p).unwrap();
    let f = p.join("deep file.txt");
    create_file(&f, true).unwrap();
    write_atomic(&f, b"deep").unwrap();
    assert!(f.as_os_str().len() > 260);
    let e = stat(&f, false).unwrap();
    assert_eq!(e.size, 4);
    let listed = read_dir(&p, &ListOptions::default()).unwrap();
    assert_eq!(listed.len(), 1);
    let ctl = OpControl::new();
    let copy = p.join("copy.txt");
    copy_file(&f, &copy, &CopyOptions::default(), &ctl, &mut ()).unwrap();
    assert_eq!(
        hash_file(&copy, soyuz_core::HashAlgo::Crc32, &ctl, &mut ()).unwrap(),
        hash_file(&f, soyuz_core::HashAlgo::Crc32, &ctl, &mut ()).unwrap()
    );
    rename(&copy, &p.join("renamed.txt"), RenameMode::NoReplace).unwrap();
    set_times(
        &f,
        &TimesSet {
            modified: Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let top = d.path().join("a_rather_long_directory_name_component");
    let stats = remove_tree(&top, &DeleteOptions::default(), &ctl, &mut |p, r| {
        r.unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        Control::Continue
    })
    .unwrap();
    assert_eq!(stats.files, 2);
    assert!(!top.exists());
}

#[cfg(windows)]
#[test]
fn junctions_and_reparse_tags() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("target");
    fs::create_dir(&target).unwrap();
    write(&target.join("t.txt"), b"t");
    let j = d.path().join("junction");
    junction(&target, &j).unwrap();
    let e = stat(&j, false).unwrap();
    assert_eq!(e.kind, EntryKind::Junction);
    assert_eq!(e.reparse_tag, Some(0xA000_0003));
    assert!(e.attrs.contains(Attributes::REPARSE));
    assert!(e.is_dir_like());
    let listed = read_dir(d.path(), &ListOptions::default()).unwrap();
    assert_eq!(listed.iter().find(|x| x.name == "junction").unwrap().kind, EntryKind::Junction);
    assert_eq!(read_dir(&j, &ListOptions::default()).unwrap().len(), 1, "junction lists its target");
    // walk does not descend through junctions.
    let ctl = OpControl::new();
    let s = walk(d.path(), &WalkOptions::default(), &ctl, &mut |_| Control::Continue).unwrap();
    assert_eq!((s.files, s.links), (1, 1));
}
