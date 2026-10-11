//! End-to-end tests: crawl a temp tree, query it, follow changes.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use super::*;

fn cfg(root: &Path, db: &Path) -> IndexConfig {
    IndexConfig {
        db_path: db.join("index.db"),
        roots: vec![root.to_path_buf()],
        exclude: vec!["node_modules".into(), "skip/me".into()],
        content: true,
        content_max_bytes: 4096,
        usn: false,
        throttle_ms: 0,
        watch_debounce: Duration::from_millis(50),
        ..Default::default()
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(20) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn wait_idle(ix: &Index) {
    wait_until("idle", || {
        let s = ix.status();
        s.phase == Phase::Idle && !s.ready_roots.is_empty()
    });
}

fn names(ix: &Index, q: &str) -> Vec<String> {
    let req = SearchRequest {
        expr: parse(q).unwrap(),
        include_hidden: false,
        scope: vec![],
        limit: 0,
        now: SystemTime::now(),
    };
    let mut v: Vec<String> = ix.search(&req).unwrap().into_iter().map(|e| e.name).collect();
    v.sort();
    v
}

fn tree() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::create_dir_all(r.join("docs/2026")).unwrap();
    std::fs::create_dir_all(r.join("node_modules/pkg")).unwrap();
    std::fs::create_dir_all(r.join("skip/me")).unwrap();
    std::fs::create_dir_all(r.join(".secret")).unwrap();
    std::fs::write(r.join("docs/Annual Report.pdf"), vec![0u8; 6000]).unwrap();
    std::fs::write(r.join("docs/2026/plan.md"), "the launch plan: ship the index").unwrap();
    std::fs::write(r.join("docs/notes.txt"), "remember the milk").unwrap();
    std::fs::write(r.join("node_modules/pkg/index.js"), "x").unwrap();
    std::fs::write(r.join("skip/me/hidden.txt"), "x").unwrap();
    std::fs::write(r.join(".secret/key.txt"), "x").unwrap();
    std::fs::write(r.join("photo.jpg"), vec![1u8; 100]).unwrap();
    (root, tempfile::tempdir().unwrap())
}

#[test]
fn crawls_queries_and_follows_changes() {
    let (root, db) = tree();
    let ix = Index::start(cfg(root.path(), db.path()), Arc::new(|| {}));
    wait_idle(&ix);
    let st = ix.status();
    assert_eq!(st.usn, UsnState::Off);
    assert!(ix.covers(root.path()));
    assert!(ix.covers(&root.path().join("docs")));
    assert!(!ix.covers(root.path().parent().unwrap()));

    assert_eq!(names(&ix, "report"), ["Annual Report.pdf"]);
    assert_eq!(names(&ix, "kind:image"), ["photo.jpg"]);
    assert_eq!(names(&ix, "ext:md OR ext:txt"), ["notes.txt", "plan.md"], "hidden folder skipped by default");
    assert!(names(&ix, "index.js").is_empty(), "excluded by name");
    assert!(names(&ix, "hidden.txt").is_empty(), "excluded by path");
    assert_eq!(names(&ix, "size:>5k"), ["Annual Report.pdf"]);
    assert_eq!(names(&ix, "kind:folder"), ["2026", "docs", "skip"]);
    wait_until("contents", || ix.status().content_files >= 3);
    assert_eq!(names(&ix, "content:milk"), ["notes.txt"]);
    assert_eq!(names(&ix, "content:\"launch plan\""), ["plan.md"]);
    let req = SearchRequest {
        expr: parse("key").unwrap(),
        include_hidden: true,
        scope: vec![],
        limit: 0,
        now: SystemTime::now(),
    };
    assert_eq!(ix.search(&req).unwrap().len(), 1, "hidden files are indexed, filtered at query time");
    let scoped = SearchRequest {
        expr: parse("").unwrap(),
        include_hidden: false,
        scope: vec![root.path().join("docs/2026")],
        limit: 0,
        now: SystemTime::now(),
    };
    assert_eq!(ix.search(&scoped).unwrap().len(), 1);

    // Incremental: watcher picks up create / rename / delete.
    let g0 = ix.generation();
    std::fs::write(root.path().join("docs/new budget.xlsx"), "x").unwrap();
    wait_until("create", || names(&ix, "budget") == ["new budget.xlsx"]);
    assert!(ix.generation() > g0);
    std::fs::rename(root.path().join("docs/notes.txt"), root.path().join("docs/todo.txt")).unwrap();
    wait_until("rename", || names(&ix, "notes").is_empty() && names(&ix, "todo") == ["todo.txt"]);
    std::fs::remove_dir_all(root.path().join("docs/2026")).unwrap();
    wait_until("delete", || names(&ix, "plan").is_empty());
    std::fs::create_dir_all(root.path().join("new/deep")).unwrap();
    std::fs::write(root.path().join("new/deep/inner.rs"), "fn main() {}").unwrap();
    wait_until("new subtree", || names(&ix, "inner") == ["inner.rs"]);
    wait_until("content of new file", || names(&ix, "content:main") == ["inner.rs"]);

    // Tags.
    ix.set_tags(vec![(canonical(&root.path().join("photo.jpg")), vec!["red".into()])]);
    wait_until("tags", || names(&ix, "tag:red") == ["photo.jpg"]);

    // Rebuild.
    ix.rebuild();
    wait_idle(&ix);
    assert_eq!(names(&ix, "report"), ["Annual Report.pdf"]);
}

#[test]
fn restart_keeps_the_index_and_verifies_changes() {
    let (root, db) = tree();
    {
        let ix = Index::start(cfg(root.path(), db.path()), Arc::new(|| {}));
        wait_idle(&ix);
        assert_eq!(names(&ix, "photo"), ["photo.jpg"]);
    }
    // Changes while "Soyuz is closed".
    std::fs::remove_file(root.path().join("photo.jpg")).unwrap();
    std::fs::write(root.path().join("offline.txt"), "x").unwrap();
    let ix = Index::start(cfg(root.path(), db.path()), Arc::new(|| {}));
    // Searchable right away (ready from the previous run) …
    wait_until("ready", || ix.status().ready_roots.len() == 1);
    // … and the verification crawl catches up.
    wait_idle(&ix);
    wait_until("verified", || names(&ix, "photo").is_empty() && names(&ix, "offline") == ["offline.txt"]);
}

#[test]
fn removed_roots_are_dropped_and_async_search_streams() {
    let (root, db) = tree();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("elsewhere.txt"), "x").unwrap();
    {
        let mut c = cfg(root.path(), db.path());
        c.roots.push(other.path().to_path_buf());
        let ix = Index::start(c, Arc::new(|| {}));
        wait_until("both roots", || ix.status().ready_roots.len() == 2 && ix.status().phase == Phase::Idle);
        assert_eq!(names(&ix, "elsewhere"), ["elsewhere.txt"]);
    }
    let ix = Index::start(cfg(root.path(), db.path()), Arc::new(|| {}));
    wait_idle(&ix);
    assert!(names(&ix, "elsewhere").is_empty());

    let req = SearchRequest {
        expr: parse("ext:pdf").unwrap(),
        include_hidden: false,
        scope: vec![],
        limit: 10,
        now: SystemTime::now(),
    };
    let h = ix.search_async(req, Arc::new(|| {}));
    let mut hits = 0;
    let mut done = false;
    while let Ok(ev) = h.rx.recv_timeout(Duration::from_secs(10)) {
        match ev {
            soyuz_search::SearchEvent::Hits(v) => hits += v.len(),
            soyuz_search::SearchEvent::Done(s) => {
                assert!(s.elapsed < Duration::from_secs(5));
                done = true;
                break;
            }
            _ => {}
        }
    }
    assert!(done);
    assert_eq!(hits, 1);
}

#[test]
fn bad_database_path_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    let c =
        IndexConfig { db_path: file.join("sub/index.db"), roots: vec![dir.path().to_path_buf()], ..Default::default() };
    let ix = Index::start(c, Arc::new(|| {}));
    wait_until("failure", || matches!(ix.status().phase, Phase::Failed(_)));
}

#[test]
fn unwatchable_roots_are_polled() {
    let (root, db) = tree();
    let croot = canonical(root.path());
    crate::worker::FORCE_POLL.lock().push(croot.clone());
    let ix = Index::start(cfg(root.path(), db.path()), Arc::new(|| {}));
    ix.set_rescan_interval(Duration::from_millis(200));
    wait_idle(&ix);
    let st = ix.status();
    assert_eq!(st.polled_roots, std::slice::from_ref(&croot));
    assert!(st.warnings.is_empty(), "polling isn't a warning: {:?}", st.warnings);
    assert_eq!(names(&ix, "photo"), ["photo.jpg"]);

    // Changes deep down, at the root, new subtrees and deletions show up on a later pass.
    std::fs::write(root.path().join("docs/2026/later.txt"), "x").unwrap();
    std::fs::write(root.path().join("top.txt"), "x").unwrap();
    std::fs::create_dir_all(root.path().join("fresh/deep")).unwrap();
    std::fs::write(root.path().join("fresh/deep/inner.rs"), "fn main() {}").unwrap();
    std::fs::remove_file(root.path().join("docs/notes.txt")).unwrap();
    std::fs::remove_dir_all(root.path().join("node_modules")).unwrap();
    std::fs::write(root.path().join("node_modules"), "now a file, still excluded").unwrap();
    wait_until("polled changes", || {
        names(&ix, "later") == ["later.txt"]
            && names(&ix, "top") == ["top.txt"]
            && names(&ix, "inner") == ["inner.rs"]
            && names(&ix, "notes").is_empty()
    });
    std::fs::remove_dir_all(root.path().join("docs")).unwrap();
    wait_until("polled delete", || names(&ix, "report").is_empty() && names(&ix, "later").is_empty());
    assert!(names(&ix, "node_modules").is_empty());
    crate::worker::FORCE_POLL.lock().retain(|r| r != &croot);
}

#[cfg(not(windows))]
#[test]
fn journal_reads_as_unavailable_off_windows() {
    let (root, db) = tree();
    let ix = Index::start(IndexConfig { usn: true, ..cfg(root.path(), db.path()) }, Arc::new(|| {}));
    wait_idle(&ix);
    let st = ix.status();
    assert_eq!(st.usn, UsnState::Unsupported(USN_NOT_WINDOWS.into()));
    assert!(st.warnings.is_empty(), "{:?}", st.warnings);
    assert!(st.polled_roots.is_empty());
}

/// Diagnostic: what the NTFS journal access does on this machine (run with
/// `cargo test -p soyuz-index journal_probe -- --ignored --nocapture`).
#[test]
#[ignore]
fn journal_probe() {
    let tmp = std::env::temp_dir();
    let t0 = Instant::now();
    let root = soyuz_fs::volume_root(&tmp).unwrap();
    println!("volume_root {} in {:?}", root.display(), t0.elapsed());
    let t0 = Instant::now();
    match soyuz_fs::usn::UsnVolume::open(&root) {
        Ok(v) => {
            println!("open ok in {:?}", t0.elapsed());
            let t0 = Instant::now();
            println!("query {:?} in {:?}", v.query(), t0.elapsed());
            let mut buf = vec![0u8; 64 * 1024];
            let t0 = Instant::now();
            let r = v.enum_mft(0, &mut buf);
            println!("enum_mft {:?} in {:?}", r.as_ref().map(|o| o.map(|n| n)), t0.elapsed());
            if let Ok(Some(n)) = r {
                let (next, recs) = usn::parse_buffer(&buf[..n]).unwrap();
                println!("next {next}, {} records, first {:?}", recs.len(), recs.first());
            }
            let ino = soyuz_fs::stat(&tmp, false).unwrap().inode.unwrap();
            println!("path_of temp {:?}", v.path_of(ino));
        }
        Err(e) => println!("open failed in {:?}: {e} ({:?})", t0.elapsed(), e.kind),
    }
}

/// Diagnostic: crawl + content timing on a real tree (`SOYUZ_INDEX_BENCH_ROOT=<dir>
/// cargo test -p soyuz-index --release bench_tree -- --ignored --nocapture`).
#[test]
#[ignore]
fn bench_tree() {
    let Ok(root) = std::env::var("SOYUZ_INDEX_BENCH_ROOT") else { return };
    let db = tempfile::tempdir().unwrap();
    let mut c = cfg(Path::new(&root), db.path());
    c.throttle_ms = 2;
    let t0 = Instant::now();
    let ix = Index::start(c, Arc::new(|| {}));
    while ix.status().ready_roots.is_empty() && t0.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(500));
        let st = ix.status();
        println!("  {:?} {:?} scanned {} files {}", t0.elapsed(), st.phase, st.scanned, st.files);
    }
    println!("crawl: {:?} ({} files)", t0.elapsed(), ix.status().files);
    let t1 = Instant::now();
    wait_until("idle", || ix.status().phase == Phase::Idle);
    println!("contents: {:?} ({} files)", t1.elapsed(), ix.status().content_files);
    for q in ["report", "kind:image size:>5MB", "*.rs", "content:todo", "ext:md OR ext:toml -final", "a"] {
        let t = Instant::now();
        let n = names(&ix, q).len();
        println!("{q:>28}: {n:>6} hits in {:?}", t.elapsed());
    }
}

#[test]
fn root_checks_tell_gone_from_unavailable() {
    use crate::worker::{RootCheck, check_root};
    let d = tempfile::tempdir().unwrap();
    assert_eq!(check_root(d.path()), RootCheck::Present);
    assert_eq!(check_root(&d.path().join("deleted")), RootCheck::Gone);
    std::fs::write(d.path().join("file"), "x").unwrap();
    assert_eq!(check_root(&d.path().join("file")), RootCheck::Gone);
    // The parent (mount point / drive) is missing too: unplugged, not deleted.
    assert!(matches!(check_root(&d.path().join("mnt/disk/root")), RootCheck::Unavailable(_)));
}

/// A polled root whose volume goes away (its parent folder vanishes, like an unmounted
/// disk) keeps its index; when it's back, a full rescan picks up what changed.
#[test]
fn unplugged_roots_keep_their_index_and_are_rescanned() {
    let base = tempfile::tempdir().unwrap();
    let mnt = base.path().join("mnt");
    let root = mnt.join("root");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/kept.txt"), "x").unwrap();
    let db = tempfile::tempdir().unwrap();
    let croot = canonical(&root);
    crate::worker::FORCE_POLL.lock().push(croot.clone());
    let ix = Index::start(cfg(&root, db.path()), Arc::new(|| {}));
    ix.set_rescan_interval(Duration::from_millis(100));
    wait_idle(&ix);
    assert_eq!(names(&ix, "kept"), ["kept.txt"]);

    let away = base.path().join("away");
    std::fs::rename(&mnt, &away).unwrap();
    wait_until("unavailable", || ix.status().unavailable_roots == std::slice::from_ref(&croot));
    // Several passes later the rows are still there.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(names(&ix, "kept"), ["kept.txt"]);
    assert_eq!(ix.status().ready_roots, std::slice::from_ref(&croot));

    std::fs::write(away.join("root/docs/new.txt"), "x").unwrap();
    std::fs::rename(&away, &mnt).unwrap();
    wait_until("rescanned", || ix.status().unavailable_roots.is_empty() && names(&ix, "new") == ["new.txt"]);
    assert_eq!(names(&ix, "kept"), ["kept.txt"]);
    // Really deleted (parent still there): dropped.
    std::fs::remove_dir_all(&root).unwrap();
    wait_until("dropped", || names(&ix, "kept").is_empty());
    crate::worker::FORCE_POLL.lock().retain(|r| r != &croot);
}

/// A polled root that can't be reached (permission denied on its parent: `stat` fails
/// with EACCES, not NotFound) keeps its index across quick passes and full verifies.
#[cfg(unix)]
#[test]
fn unreadable_roots_keep_their_index() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let parent = base.path().join("p");
    let root = parent.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("kept.txt"), "x").unwrap();
    let db = tempfile::tempdir().unwrap();
    let croot = canonical(&root);
    crate::worker::FORCE_POLL.lock().push(croot.clone());
    let ix = Index::start(cfg(&root, db.path()), Arc::new(|| {}));
    ix.set_rescan_interval(Duration::from_millis(50));
    wait_idle(&ix);
    assert_eq!(names(&ix, "kept"), ["kept.txt"]);
    let mode = |m| std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o000);
    if std::fs::metadata(&root).is_ok() {
        mode(0o755); // Running as root: permissions don't apply.
        crate::worker::FORCE_POLL.lock().retain(|r| r != &croot);
        return;
    }
    wait_until("unavailable", || ix.status().unavailable_roots == std::slice::from_ref(&croot));
    // Long enough for quick passes and a full verification (every 12th pass).
    std::thread::sleep(Duration::from_millis(1000));
    let kept = names(&ix, "kept");
    mode(0o755);
    assert_eq!(kept, ["kept.txt"], "an unreachable root must not be wiped");
    std::fs::write(root.join("new.txt"), "x").unwrap();
    wait_until("back", || ix.status().unavailable_roots.is_empty() && names(&ix, "new") == ["new.txt"]);
    crate::worker::FORCE_POLL.lock().retain(|r| r != &croot);
}
