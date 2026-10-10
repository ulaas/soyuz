//! Trash and launch surfaces. Tests touching the real recycle bin / trash run only with
//! `SOYUZ_TRASH_TESTS=1` (and are `#[ignore]`d otherwise); they only ever trash, restore
//! and purge files created in a temp directory.

mod common;

use std::fs;

use common::write;
use soyuz_core::{Control, EntryKind, FsErrorKind, OpControl};
use soyuz_fs::trash::{self, TrashSupport};
use soyuz_fs::*;

#[test]
fn trash_support_is_reported_without_side_effects() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x");
    write(&f, b"x");
    match trash_support(&f) {
        TrashSupport::Native => {}
        TrashSupport::Unsupported(why) => assert!(!why.is_empty()),
    }
    assert!(f.exists());
    assert!(matches!(trash_support(&d.path().join("missing")), TrashSupport::Unsupported(_)));
    assert_eq!(trash::trash(&d.path().join("missing")).unwrap_err().kind, FsErrorKind::NotFound);
}

fn real_trash_enabled() -> bool {
    std::env::var("SOYUZ_TRASH_TESTS").is_ok_and(|v| v == "1")
}

#[test]
#[ignore = "touches the real trash / recycle bin; run with SOYUZ_TRASH_TESTS=1 -- --ignored"]
fn real_trash_round_trip() {
    if !real_trash_enabled() {
        eprintln!("set SOYUZ_TRASH_TESTS=1 to run");
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let name = format!("soyuz-trash-test-{}.txt", std::process::id());
    let f = d.path().join(&name);
    write(&f, b"trash me");
    if let TrashSupport::Unsupported(why) = trash_support(&f) {
        eprintln!("temp dir volume has no trash: {why}");
        return;
    }
    let bins_before = per_mount_bins(d.path());
    let item = trash::trash(&f).unwrap();
    assert!(!f.exists());
    assert_eq!(item.kind, EntryKind::File);
    assert_eq!(item.size, 8);
    let listed = trash::list().unwrap();
    let found = listed.iter().find(|i| i.id == item.id && i.trash_root == item.trash_root).expect("listed");
    assert_eq!(strip_verbatim(&found.original), strip_verbatim(&normalize(&f)));
    let back = trash::restore(found, None).unwrap();
    assert_eq!(fs::read(&back).unwrap(), b"trash me");

    // Directory: trash, then purge.
    let dir = d.path().join(format!("soyuz-trash-dir-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    write(&dir.join("inner.txt"), b"1234");
    let ditem = trash::trash(&dir).unwrap();
    assert_eq!(ditem.kind, EntryKind::Dir);
    assert_eq!(ditem.size, 4);
    trash::purge(&ditem).unwrap();
    assert!(!trash::list().unwrap().iter().any(|i| i.id == ditem.id && i.trash_root == ditem.trash_root));
    assert_eq!(trash::purge(&ditem).unwrap_err().kind, FsErrorKind::NotFound);
    // A per-mount bin this test created (`/tmp/.Trash-$uid` on a tmpfs /tmp) goes again.
    if !bins_before.contains(&item.trash_root) && item.trash_root.file_name().is_some_and(|n| n != "Trash") {
        let _ = fs::remove_dir_all(&item.trash_root);
    }
}

/// Existing per-mount bins next to the volume root of `p` (unix; empty elsewhere).
fn per_mount_bins(p: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(root) = volume_root(p) else { return Vec::new() };
    let Ok(rd) = fs::read_dir(&root) else { return Vec::new() };
    rd.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains(".Trash")).collect()
}

/// The freedesktop home trash (`$XDG_DATA_HOME/Trash`): items from a temp directory inside
/// the data directory (same filesystem as the home trash) are trashed, listed with their
/// `.trashinfo`, restored and purged; the temp directory is removed afterwards.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
#[ignore = "touches the real home trash; run with SOYUZ_TRASH_TESTS=1 -- --ignored"]
fn real_home_trash_round_trip() {
    if !real_trash_enabled() {
        eprintln!("set SOYUZ_TRASH_TESTS=1 to run");
        return;
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local").join("share"));
    let home_trash = data.join("Trash");
    let d = tempfile::Builder::new().prefix(".soyuz-trash-test-").tempdir_in(&data).unwrap();
    let f = d.path().join("home trash test ü.txt");
    write(&f, b"home");
    let item = trash::trash(&f).unwrap();
    assert_eq!(item.trash_root, home_trash, "same filesystem as the home trash");
    let info = home_trash.join("info").join(format!("{}.trashinfo", item.id));
    let text = fs::read_to_string(&info).unwrap();
    assert!(text.starts_with("[Trash Info]\nPath=/"), "absolute Path= in the home trash: {text}");
    assert!(text.contains("%C3%BC.txt\n"), "{text}");
    let listed = trash::list().unwrap();
    let found = listed.iter().find(|i| i.id == item.id && i.trash_root == item.trash_root).expect("listed");
    assert_eq!(found.original, f);
    assert_eq!(trash::restore(found, None).unwrap(), f);
    assert!(!info.exists());

    let dir = d.path().join("folder");
    fs::create_dir(&dir).unwrap();
    write(&dir.join("inner"), b"12345");
    let ditem = trash::trash(&dir).unwrap();
    let sizes = fs::read_to_string(home_trash.join("directorysizes")).unwrap();
    assert!(sizes.lines().any(|l| l.starts_with("5 ") && l.ends_with(&format!(" {}", ditem.id))), "{sizes}");
    trash::purge(&ditem).unwrap();
    assert!(!home_trash.join("files").join(&ditem.id).exists());
    let sizes = fs::read_to_string(home_trash.join("directorysizes")).unwrap_or_default();
    assert!(!sizes.lines().any(|l| l.ends_with(&format!(" {}", ditem.id))), "{sizes}");
}

#[test]
fn empty_is_cancellable() {
    // Never empties anything: the control is cancelled up front.
    let ctl = OpControl::new();
    ctl.cancel();
    let res = trash::empty(&ctl, &mut |_, _| Control::Stop);
    match res {
        Ok(()) => {}
        Err(e) => assert!(e.is_cancel() || e.kind == FsErrorKind::Unsupported, "{e:?}"),
    }
}

#[test]
fn shortcut_and_apps_queries_do_not_spawn() {
    let d = tempfile::tempdir().unwrap();
    let txt = d.path().join("a.txt");
    write(&txt, b"x");
    // Read-only registry / desktop-database queries.
    let apps = launch::apps_for(&txt);
    for a in &apps {
        assert!(!a.name.is_empty());
        assert!(!a.id.is_empty());
    }
    assert_eq!(launch::shortcut_target(&txt).unwrap(), None);
    assert_eq!(launch::open_default(&d.path().join("missing.txt")).unwrap_err().kind, FsErrorKind::NotFound);
}
