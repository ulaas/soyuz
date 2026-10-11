//! Jobs across providers (local ↔ in-memory fake server): upload / download trees,
//! conflicts, resume after a dropped connection, cancel, moves and undo.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use soyuz_core::{ConflictResolution, ErrorAction};
use soyuz_vfs::RemoteEntry;
use soyuz_vfs::fake::FakeProvider;

use super::*;
use crate::{JobPrompt, PromptResponse, RemoteOp};

fn server(name: &str) -> Arc<FakeProvider> {
    FakeProvider::new(name).install()
}

fn uri(name: &str, path: &str) -> PathBuf {
    PathBuf::from(format!("fake://{name}{path}"))
}

fn transfer(sources: Vec<PathBuf>, dest_dir: PathBuf, mv: bool) -> JobKind {
    JobKind::Remote(RemoteOp::Transfer { sources, dest_dir, mv })
}

fn write(p: &Path, data: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 % 251) as u8).collect()
}

#[test]
fn upload_tree_then_undo() {
    let h = H::new();
    let fake = server("jobs-up");
    fake.mkdirs("/inbox");
    write(&h.p("s/a/f.txt"), b"hello");
    write(&h.p("s/a/b/g.bin"), &data(300_000));
    write(&h.p("s/top.txt"), b"top");
    let id = h.submit(transfer(vec![h.p("s/a"), h.p("s/top.txt")], uri("jobs-up", "/inbox"), false));
    let sum = h.ok(id);
    assert_eq!(sum.items_failed, 0, "{sum:?}");
    assert_eq!(fake.get("/inbox/a/f.txt").unwrap(), b"hello");
    assert_eq!(fake.get("/inbox/a/b/g.bin").unwrap(), data(300_000));
    assert_eq!(fake.get("/inbox/top.txt").unwrap(), b"top");
    let snap = h.snap(id);
    assert_eq!(snap.progress.bytes_total, 300_008);
    assert_eq!(snap.progress.bytes_done, 300_008);
    assert!(snap.label.starts_with("Upload 2 items"), "{}", snap.label);

    // Undo: the created folder (non-empty → asks, answered "delete") and file go away.
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).expect("undoable");
    let s = settle(&h, u, true);
    assert_eq!(s.items_failed, 0, "{s:?}");
    assert!(!fake.exists("/inbox/a") && !fake.exists("/inbox/top.txt"));
    assert!(fake.exists("/inbox"), "the pre-existing folder stays");
    assert!(h.p("s/a/f.txt").exists(), "sources untouched");
}

#[test]
fn download_and_undo() {
    let h = H::new();
    let fake = server("jobs-down");
    fake.put("/pub/x/readme.md", b"# hi");
    fake.put("/pub/x/data.bin", &data(100_000));
    std::fs::create_dir_all(h.p("d")).unwrap();
    let id = h.submit(transfer(vec![uri("jobs-down", "/pub/x")], h.p("d"), false));
    assert!(h.snap(id).label.starts_with("Download"));
    h.ok(id);
    assert_eq!(std::fs::read(h.p("d/x/readme.md")).unwrap(), b"# hi");
    assert_eq!(std::fs::read(h.p("d/x/data.bin")).unwrap(), data(100_000));
    let hid = h.engine.history()[0].id;
    let u = h.engine.undo(hid).unwrap();
    settle(&h, u, true);
    assert!(!h.p("d/x").exists());
    assert!(fake.exists("/pub/x/readme.md"));
}

#[test]
fn conflicts_ask_and_keep_both() {
    let h = H::new();
    let fake = server("jobs-conflict");
    fake.put("/a.txt", b"old");
    write(&h.p("a.txt"), b"new!");
    let id = h.submit(transfer(vec![h.p("a.txt")], uri("jobs-conflict", "/"), false));
    match h.prompt(id) {
        JobPrompt::Conflict(info) => {
            assert_eq!(info.kind, crate::ConflictKind::FileExists);
            assert_eq!(info.suggested_name, "a (2).txt");
            assert_eq!(info.dst.size, 3);
        }
        p => panic!("unexpected {p:?}"),
    }
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::KeepBoth, apply_to_all: false });
    h.ok(id);
    assert_eq!(fake.get("/a.txt").unwrap(), b"old");
    assert_eq!(fake.get("/a (2).txt").unwrap(), b"new!");

    // Overwrite replaces (and the entry is marked not undoable: servers have no trash).
    let mut spec = JobSpec::new(transfer(vec![h.p("a.txt")], uri("jobs-conflict", "/"), false));
    spec.options.conflict = Some(ConflictResolution::Overwrite);
    let id = h.engine.submit(spec);
    h.ok(id);
    assert_eq!(fake.get("/a.txt").unwrap(), b"new!");
    assert!(!h.engine.history()[1].undoable, "replacing on a server is irreversible");
}

#[test]
fn dropped_connection_resumes_on_retry() {
    let h = H::new();
    let fake = server("jobs-resume");
    let bytes = data(2_000_000);
    write(&h.p("big.iso"), &bytes);
    fake.fail_after(700_000);
    let mut spec = JobSpec::new(transfer(vec![h.p("big.iso")], uri("jobs-resume", "/"), false));
    spec.options.errors = Some(ErrorAction::Retry); // automatic retries
    let id = h.engine.submit(spec);
    let sum = h.ok(id);
    assert_eq!(sum.items_failed, 0);
    assert_eq!(fake.get("/big.iso").unwrap(), bytes);
    // Only the missing part was sent again.
    assert_eq!(fake.bytes_written.load(Ordering::Relaxed), bytes.len() as u64);
    let snap = h.snap(id);
    assert_eq!(snap.progress.bytes_done, bytes.len() as u64, "no double counting");
    let log = h.engine.log(id, 0);
    assert!(log.iter().any(|l| l.msg.contains("resumed at")), "{log:#?}");
}

#[test]
fn dropped_connection_asks_and_retry_resumes() {
    let h = H::new();
    let fake = server("jobs-resume-ask");
    let bytes = data(1_000_000);
    fake.put("/movie.mkv", &bytes);
    std::fs::create_dir_all(h.p("dl")).unwrap();
    fake.fail_after(400_000);
    let id = h.submit(transfer(vec![uri("jobs-resume-ask", "/movie.mkv")], h.p("dl"), false));
    match h.prompt(id) {
        JobPrompt::Error { retryable, .. } => assert!(retryable),
        p => panic!("unexpected {p:?}"),
    }
    assert!(h.p("dl/.movie.mkv.soyuzpart").exists(), "partial kept while asking");
    h.engine.respond(id, PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false });
    h.ok(id);
    assert_eq!(std::fs::read(h.p("dl/movie.mkv")).unwrap(), bytes);
    assert!(!h.p("dl/.movie.mkv.soyuzpart").exists());
}

#[test]
fn cancel_leaves_no_partial() {
    let h = H::new();
    let fake = server("jobs-cancel");
    write(&h.p("slow.bin"), &data(3_000_000));
    fake.set_delay(Some(Duration::from_millis(20)));
    let id = h.submit(transfer(vec![h.p("slow.bin")], uri("jobs-cancel", "/"), false));
    h.wait_state(id, |s| *s == JobState::Running);
    wait_for(Duration::from_secs(10), || (h.snap(id).progress.bytes_done > 0).then_some(()));
    h.engine.cancel(id);
    assert_eq!(h.done(id), JobState::Cancelled);
    fake.set_delay(None);
    assert_eq!(fake.paths(), vec!["/".to_string()], "nothing left on the server");
}

#[test]
fn pause_and_resume_a_transfer() {
    let h = H::new();
    let fake = server("jobs-pause");
    let bytes = data(2_000_000);
    write(&h.p("p.bin"), &bytes);
    fake.set_delay(Some(Duration::from_millis(5)));
    let id = h.submit(transfer(vec![h.p("p.bin")], uri("jobs-pause", "/"), false));
    wait_for(Duration::from_secs(10), || (h.snap(id).progress.bytes_done > 0).then_some(()));
    h.engine.pause(id);
    h.wait_state(id, |s| *s == JobState::Paused);
    let at = h.snap(id).progress.bytes_done;
    std::thread::sleep(Duration::from_millis(100));
    assert!(h.snap(id).progress.bytes_done <= at + 256 * 1024, "at most one chunk after pausing");
    h.engine.resume(id);
    fake.set_delay(None);
    h.ok(id);
    assert_eq!(fake.get("/p.bin").unwrap(), bytes);
}

#[test]
fn moves_between_servers_and_within_one() {
    let h = H::new();
    let a = server("jobs-mv-a");
    let b = server("jobs-mv-b");
    a.put("/docs/r.txt", b"report");
    b.mkdirs("/in");
    // Cross-server move: copy + delete (not undoable).
    let id = h.submit(transfer(vec![uri("jobs-mv-a", "/docs")], uri("jobs-mv-b", "/in"), true));
    h.ok(id);
    assert_eq!(b.get("/in/docs/r.txt").unwrap(), b"report");
    assert!(!a.exists("/docs"));
    assert!(!h.engine.history()[0].undoable);

    // Same-server move: a rename, undoable.
    b.mkdirs("/archive");
    let id = h.submit(transfer(vec![uri("jobs-mv-b", "/in/docs")], uri("jobs-mv-b", "/archive"), true));
    h.ok(id);
    assert!(b.exists("/archive/docs/r.txt") && !b.exists("/in/docs"));
    let hid = h.engine.history()[1].id;
    let u = h.engine.undo(hid).unwrap();
    settle(&h, u, true);
    assert!(b.exists("/in/docs/r.txt") && !b.exists("/archive/docs"));
}

#[test]
fn move_local_to_remote_deletes_source_after_asking() {
    let h = H::new();
    let fake = server("jobs-mv-up");
    write(&h.p("m/x.txt"), b"x");
    let id = h.submit(transfer(vec![h.p("m")], uri("jobs-mv-up", "/"), true));
    // The test volume has no trash: the job asks before deleting the source.
    let s = settle(&h, id, true);
    assert_eq!(s.items_failed, 0);
    assert_eq!(fake.get("/m/x.txt").unwrap(), b"x");
    assert!(!h.p("m").exists());
}

#[test]
fn remote_mkdir_rename_delete() {
    let h = H::new();
    let fake = server("jobs-ops");
    h.ok(h.submit(JobKind::Remote(RemoteOp::Mkdir { path: uri("jobs-ops", "/new") })));
    assert!(fake.exists("/new"));
    h.ok(h.submit(JobKind::Remote(RemoteOp::Rename { from: uri("jobs-ops", "/new"), to: uri("jobs-ops", "/renamed") })));
    assert!(fake.exists("/renamed") && !fake.exists("/new"));
    // Undo the rename, then the mkdir.
    let u = h.engine.undo(h.engine.history()[1].id).unwrap();
    settle(&h, u, true);
    assert!(fake.exists("/new"));
    let u = h.engine.undo(h.engine.history()[0].id).unwrap();
    settle(&h, u, true);
    assert!(!fake.exists("/new"));

    fake.put("/t/a/b.txt", b"b");
    let id = h.submit(JobKind::Remote(RemoteOp::Delete { paths: vec![uri("jobs-ops", "/t")] }));
    h.ok(id);
    assert!(!fake.exists("/t"));
    assert!(!h.engine.history()[2].undoable, "remote deletes are not undoable");
}

#[test]
fn unknown_server_fails_cleanly() {
    let h = H::new();
    write(&h.p("f"), b"1");
    let mut spec = JobSpec::new(transfer(vec![h.p("f")], uri("jobs-nobody-here", "/"), false));
    spec.options.errors = Some(ErrorAction::Skip);
    let s = h.ok(h.engine.submit(spec));
    assert_eq!(s.items_failed, 1);
}

/// Names a hostile client can plant on a share (SMB maps `\` to U+F026 and back; SFTP
/// allows it literally), plus plain traversal names a broken server could report.
const HOSTILE: [&str; 4] = ["..\\..\\Other\\victim.txt", "..", "../../Other/victim.txt", "x\\..\\..\\..\\Other"];

#[test]
fn hostile_names_never_escape_a_remote_delete() {
    let h = H::new();
    let fake = server("jobs-hostile-del");
    fake.put("/share/dir/a.txt", b"a");
    fake.put("/Other/victim.txt", b"v");
    fake.put("/share/victim.txt", b"v");
    for n in HOSTILE {
        fake.inject_unchecked("/share/dir", RemoteEntry::file(n, 1, None));
    }
    fake.inject_unchecked("/share/dir", RemoteEntry::dir("..\\..\\Other", None));
    let mut spec =
        JobSpec::new(JobKind::Remote(RemoteOp::Delete { paths: vec![uri("jobs-hostile-del", "/share/dir")] }));
    spec.options.errors = Some(ErrorAction::Skip);
    let s = h.ok(h.engine.submit(spec));
    assert_eq!(s.items_failed, 1, "{s:?}");
    assert_eq!(fake.get("/Other/victim.txt").as_deref(), Some(&b"v"[..]));
    assert!(fake.exists("/Other"));
    assert!(fake.exists("/share/victim.txt"));
}

#[test]
fn hostile_names_never_escape_a_download() {
    let h = H::new();
    let fake = server("jobs-hostile-down");
    fake.put("/share/dir/a.txt", b"a");
    for n in HOSTILE {
        fake.inject_unchecked("/share/dir", RemoteEntry::file(n, 1, None));
    }
    std::fs::create_dir_all(h.p("in/deep")).unwrap();
    let mut spec = JobSpec::new(transfer(vec![uri("jobs-hostile-down", "/share/dir")], h.p("in/deep"), false));
    spec.options.errors = Some(ErrorAction::Skip);
    let s = h.ok(h.engine.submit(spec));
    assert_eq!(s.items_failed, HOSTILE.len() as u64, "{s:?}");
    assert_eq!(std::fs::read(h.p("in/deep/dir/a.txt")).unwrap(), b"a");
    let names: Vec<_> = std::fs::read_dir(h.p("in/deep/dir")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, ["a.txt"]);
    assert!(!h.p("Other").exists() && !h.p("in/Other").exists());
}

#[test]
fn hostile_names_never_escape_an_upload_between_servers() {
    let h = H::new();
    let src = server("jobs-hostile-src");
    let dst = server("jobs-hostile-dst");
    src.put("/dir/a.txt", b"a");
    for n in HOSTILE {
        src.inject_unchecked("/dir", RemoteEntry::file(n, 1, None));
    }
    dst.mkdirs("/share/in");
    dst.put("/Other/victim.txt", b"v");
    let mut spec =
        JobSpec::new(transfer(vec![uri("jobs-hostile-src", "/dir")], uri("jobs-hostile-dst", "/share/in"), false));
    spec.options.errors = Some(ErrorAction::Skip);
    let s = h.ok(h.engine.submit(spec));
    assert_eq!(s.items_failed, HOSTILE.len() as u64, "{s:?}");
    assert_eq!(dst.get("/share/in/dir/a.txt").as_deref(), Some(&b"a"[..]));
    assert_eq!(dst.get("/Other/victim.txt").as_deref(), Some(&b"v"[..]));
    let mut paths = dst.paths();
    paths.retain(|p| !p.starts_with("/share/in/dir") && !p.starts_with("/Other"));
    assert_eq!(paths, ["/", "/share", "/share/in"]);
}
