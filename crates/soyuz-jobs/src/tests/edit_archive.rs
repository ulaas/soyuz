//! `JobKind::EditArchive` (zip rewrite + replace, trash backup, undo).

use std::io::Read;

use soyuz_archive::{ArchiveFormat, CreateOptions, ZipEdit};
use soyuz_core::OpControl;
use soyuz_fs::TrashSupport;

use super::*;
use crate::{PromptResponse, UndoRecord};

fn make_zip(h: &H) -> PathBuf {
    write(&h.p("src/a.txt"), b"alpha");
    write(&h.p("src/sub/b.txt"), b"beta");
    let zip = h.p("z.zip");
    let opts = CreateOptions {
        format: ArchiveFormat::Zip,
        level: 6,
        password: None,
        base_dir: h.p("src"),
        store_symlinks: true,
        follow_symlinks: false,
    };
    soyuz_archive::create(&zip, &[h.p("src/a.txt"), h.p("src/sub")], &opts, &OpControl::new(), &mut ()).unwrap();
    zip
}

fn listing(zip: &Path) -> Vec<(String, Vec<u8>)> {
    let mut r = soyuz_archive::open(zip, None).unwrap();
    let n = r.entries().unwrap().len();
    let mut out = Vec::new();
    for i in 0..n {
        let e = r.entries().unwrap()[i].clone();
        if e.is_dir {
            continue;
        }
        let mut v = Vec::new();
        r.open_entry(i).unwrap().read_to_end(&mut v).unwrap();
        out.push((e.path, v));
    }
    out.sort();
    out
}

fn edits(h: &H) -> Vec<ZipEdit> {
    write(&h.p("add/c.txt"), b"gamma");
    vec![
        ZipEdit::Delete { path: "a.txt".into() },
        ZipEdit::Rename { from: "sub/b.txt".into(), to: "sub/renamed.txt".into() },
        ZipEdit::Add { sources: vec![h.p("add/c.txt")], dest_dir: "sub".into() },
    ]
}

#[test]
fn edit_without_trash_asks_and_replaces_atomically() {
    let h = H::new();
    let zip = make_zip(&h);
    let id = h.submit(JobKind::EditArchive { archive: zip.clone(), edits: edits(&h), level: None });
    assert!(matches!(h.prompt(id), JobPrompt::TrashUnsupported { .. }));
    h.engine.respond(id, PromptResponse::TrashUnsupported { permanent: true, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!(s.items_ok, 1, "{s:?}");
    assert_eq!(
        listing(&zip),
        vec![("sub/c.txt".to_string(), b"gamma".to_vec()), ("sub/renamed.txt".to_string(), b"beta".to_vec())]
    );
    assert!(!soyuz_archive::edit_temp_path(&zip).exists());
    assert!(!h.engine.history()[0].undoable, "no backup, no undo");
}

#[test]
fn edit_declined_or_failed_keeps_the_original() {
    let h = H::new();
    let zip = make_zip(&h);
    let before = std::fs::read(&zip).unwrap();
    let id = h.submit(JobKind::EditArchive { archive: zip.clone(), edits: edits(&h), level: None });
    h.prompt(id);
    h.engine.respond(id, PromptResponse::TrashUnsupported { permanent: false, apply_to_all: false });
    assert_eq!(h.ok(id).items_skipped, 1);
    assert_eq!(std::fs::read(&zip).unwrap(), before);
    assert!(!soyuz_archive::edit_temp_path(&zip).exists());
    // An edit that fails (missing entry) leaves no temp file and the archive as it was.
    let id = h.submit(JobKind::EditArchive {
        archive: zip.clone(),
        edits: vec![ZipEdit::Delete { path: "nope.txt".into() }],
        level: None,
    });
    assert!(matches!(h.prompt(id), JobPrompt::Error { .. }));
    h.engine.respond(id, PromptResponse::Error { action: soyuz_core::ErrorAction::Skip, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!(s.items_failed, 1);
    assert_eq!(std::fs::read(&zip).unwrap(), before);
    assert!(!soyuz_archive::edit_temp_path(&zip).exists());
    // Not a zip: refused.
    write(&h.p("x.tar"), &[0u8; 1024]);
    let id = h.submit(JobKind::EditArchive { archive: h.p("x.tar"), edits: Vec::new(), level: None });
    assert!(matches!(h.done(id), JobState::Failed(_)));
}

#[test]
fn edit_archive_kind_serde_is_additive() {
    let k = JobKind::EditArchive {
        archive: "/a.zip".into(),
        edits: vec![ZipEdit::Delete { path: "x".into() }],
        level: None,
    };
    let json = serde_json::to_string(&k).unwrap();
    let back: JobKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, k);
    // `level` may be missing.
    let old = r#"{"edit_archive":{"archive":"/a.zip","edits":[{"delete":{"path":"x"}}]}}"#;
    assert_eq!(serde_json::from_str::<JobKind>(old).unwrap(), k);
    assert_eq!(k.touched_roots(), vec![PathBuf::from("/a.zip")]);
    assert_eq!(k.brief(), crate::JobKindBrief::EditArchive);
}

/// Real trash backup + undo (touches the real Recycle Bin: opt-in).
#[test]
fn edit_with_trash_backup_and_undo() {
    if std::env::var_os("SOYUZ_TRASH_TESTS").is_none() {
        return;
    }
    let h = H::new();
    let zip = make_zip(&h);
    if soyuz_fs::trash::trash_support(&zip) != TrashSupport::Native {
        return;
    }
    *h.probe.trash.lock() = TrashSupport::Native;
    let original = listing(&zip);
    let id = h.submit(JobKind::EditArchive { archive: zip.clone(), edits: edits(&h), level: None });
    h.ok(id);
    assert_ne!(listing(&zip), original);
    let rec = &h.engine.history()[0].records;
    assert!(matches!(rec.as_slice(), [UndoRecord::Replaced { .. }]), "{rec:?}");
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).expect("undoable");
    h.ok(u);
    assert_eq!(listing(&zip), original);
    eprintln!("edit_with_trash_backup_and_undo: trash path exercised");
}
