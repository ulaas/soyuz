//! Editing inside a zip (`rewrite_zip`).

mod common;

use std::io::Read;

use common::*;
use soyuz_archive::{ArchiveError, ArchiveFormat, ZipEdit, can_edit, create, edit_temp_path, open, rewrite_zip};
use soyuz_core::OpControl;

fn read(r: &mut dyn soyuz_archive::ArchiveReader, path: &str) -> Vec<u8> {
    let idx = r.entries().unwrap().iter().position(|e| e.path == path).unwrap_or_else(|| panic!("no {path}"));
    let mut v = Vec::new();
    r.open_entry(idx).unwrap().read_to_end(&mut v).unwrap();
    v
}

#[test]
fn add_delete_rename() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let zip = d.path().join("a.zip");
    create(&zip, std::slice::from_ref(&tree), &opts(ArchiveFormat::Zip, d.path()), &OpControl::new(), &mut ()).unwrap();
    assert!(can_edit(&zip));
    let before_big = {
        let mut r = open(&zip, None).unwrap();
        r.entries().unwrap().iter().find(|e| e.path == "tree/sub/big.bin").unwrap().compressed
    };
    std::fs::write(d.path().join("new.txt"), b"brand new").unwrap();
    std::fs::create_dir_all(d.path().join("newdir/inner")).unwrap();
    std::fs::write(d.path().join("newdir/inner/x.txt"), b"x").unwrap();
    std::fs::write(d.path().join("hello.txt"), b"replaced hello").unwrap();
    let out = edit_temp_path(&zip);
    assert!(out.file_name().unwrap().to_string_lossy().starts_with(".a.zip"));
    let edits = vec![
        ZipEdit::Delete { path: "tree/empty.bin".into() },
        ZipEdit::Rename { from: "tree/sub".into(), to: "tree/moved".into() },
        ZipEdit::Add {
            sources: vec![d.path().join("new.txt"), d.path().join("newdir")],
            dest_dir: "tree/moved".into(),
        },
        ZipEdit::Add { sources: vec![d.path().join("hello.txt")], dest_dir: "tree".into() },
    ];
    let mut progress = 0u64;
    let sum =
        rewrite_zip(&zip, &out, &edits, 6, &OpControl::new(), &mut |p: soyuz_core::ByteProgress| progress += p.delta)
            .unwrap();
    assert!(progress > 0);
    assert_eq!(sum.removed, 2, "{sum:?}"); // empty.bin + the replaced hello.txt
    assert_eq!(sum.added, 5, "{sum:?}"); // new.txt, newdir, newdir/inner, x.txt, hello.txt
    assert!(sum.renamed >= 3, "{sum:?}");
    let mut r = soyuz_archive::open_with_format(&out, ArchiveFormat::Zip, None).unwrap();
    let names: Vec<String> = r.entries().unwrap().iter().map(|e| e.path.clone()).collect();
    assert!(!names.iter().any(|n| n.starts_with("tree/sub")), "{names:?}");
    assert!(!names.contains(&"tree/empty.bin".to_string()));
    assert_eq!(read(r.as_mut(), "tree/moved/big.bin"), noise(1_500_000, 7));
    assert_eq!(read(r.as_mut(), "tree/moved/new.txt"), b"brand new");
    assert_eq!(read(r.as_mut(), "tree/moved/newdir/inner/x.txt"), b"x");
    assert_eq!(read(r.as_mut(), "tree/hello.txt"), b"replaced hello");
    assert_eq!(names.iter().filter(|n| *n == "tree/hello.txt").count(), 1);
    // Copied raw: the compressed size is unchanged.
    let big = r.entries().unwrap().iter().find(|e| e.path == "tree/moved/big.bin").unwrap().clone();
    assert_eq!(big.compressed, before_big);
    let rep = r.test(&OpControl::new(), &mut ()).unwrap();
    assert!(rep.bad.is_empty(), "{:?}", rep.bad);
    // The original is untouched.
    let mut orig = open(&zip, None).unwrap();
    assert!(orig.entries().unwrap().iter().any(|e| e.path == "tree/empty.bin"));
}

#[test]
fn errors_leave_no_output() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let zip = d.path().join("a.zip");
    create(&zip, std::slice::from_ref(&tree), &opts(ArchiveFormat::Zip, d.path()), &OpControl::new(), &mut ()).unwrap();
    let out = edit_temp_path(&zip);
    let ctl = OpControl::new();
    let clash = rewrite_zip(
        &zip,
        &out,
        &[ZipEdit::Rename { from: "tree/hello.txt".into(), to: "tree/empty.bin".into() }],
        6,
        &ctl,
        &mut (),
    );
    assert!(matches!(clash, Err(ArchiveError::Conflict(_))), "{clash:?}");
    assert!(!out.exists());
    let missing = rewrite_zip(&zip, &out, &[ZipEdit::Delete { path: "nope".into() }], 6, &ctl, &mut ());
    assert!(matches!(missing, Err(ArchiveError::Io(ref e)) if e.kind == soyuz_core::FsErrorKind::NotFound));
    let escape =
        rewrite_zip(&zip, &out, &[ZipEdit::Rename { from: "tree".into(), to: "../x".into() }], 6, &ctl, &mut ());
    assert!(escape.is_err());
    let into_self =
        rewrite_zip(&zip, &out, &[ZipEdit::Rename { from: "tree".into(), to: "tree/sub/x".into() }], 6, &ctl, &mut ());
    assert!(into_self.is_err());
    ctl.cancel();
    let cancelled = rewrite_zip(&zip, &out, &[ZipEdit::Delete { path: "tree/hello.txt".into() }], 6, &ctl, &mut ());
    assert_eq!(cancelled.err(), Some(ArchiveError::Cancelled));
    assert!(!out.exists());
}

#[test]
fn encrypted_entries_stay_encrypted() {
    let d = tempfile::tempdir().unwrap();
    let tree = make_tree(d.path());
    let zip = d.path().join("s.zip");
    let mut o = opts(ArchiveFormat::Zip, d.path());
    o.password = Some("pw".into());
    create(&zip, std::slice::from_ref(&tree), &o, &OpControl::new(), &mut ()).unwrap();
    let out = edit_temp_path(&zip);
    rewrite_zip(
        &zip,
        &out,
        &[ZipEdit::Rename { from: "tree/hello.txt".into(), to: "hi.txt".into() }],
        6,
        &OpControl::new(),
        &mut (),
    )
    .unwrap();
    let mut r = soyuz_archive::open_with_format(&out, ArchiveFormat::Zip, Some("pw")).unwrap();
    let e = r.entries().unwrap().iter().find(|e| e.path == "hi.txt").unwrap().clone();
    assert!(e.encrypted);
    assert_eq!(read(r.as_mut(), "hi.txt"), b"hello world\n");
    let mut r = soyuz_archive::open_with_format(&out, ArchiveFormat::Zip, None).unwrap();
    let idx = r.entries().unwrap().iter().position(|e| e.path == "hi.txt").unwrap();
    assert_eq!(r.open_entry(idx).err(), Some(ArchiveError::PasswordRequired));
}
