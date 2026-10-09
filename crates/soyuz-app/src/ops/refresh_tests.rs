//! Pane refresh regression tests with real temp folders, the real lister, watcher and
//! job engine (owner report: "new files don't show up, even after a copy in Soyuz").
//! Every wait is bounded.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_jobs::{EngineConfig, JobEngine, JobEvent, JobKind, JobSpec};

use super::{
    affected_dirs, pane_listing, panes_showing, refresh_pane, relist_for_job, same_dir, start_listing, watch_pane,
};
use crate::services::Services;
use crate::services::jobs::AffectedPaths;
use crate::services::mailbox::UiMsg;
use crate::state::pane::Pane;
use crate::state::view_memory::ViewState;
use crate::state::workspace::{Tab, Workspace};

const WAIT: Duration = Duration::from_secs(10);

fn services() -> Services {
    Services::new(&egui::Context::default(), Duration::from_millis(50))
}

fn names(p: &Pane) -> Vec<String> {
    p.visible.iter().filter_map(|&i| p.entries.get(i)).map(|e| e.name.clone()).collect()
}

/// Deliver mailbox / watcher traffic to `panes` (like the frame loop does) until `done`
/// holds for them or the wait runs out; returns whether it held.
fn pump(s: &mut Services, panes: &mut [&mut Pane], done: impl Fn(&[&mut Pane]) -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    loop {
        while let Some(msg) = s.mailbox.try_recv() {
            match msg {
                UiMsg::Listing { pane, generation, chunk, done, err } => {
                    if let Some(p) = panes.iter_mut().find(|p| p.id == pane && p.generation == generation) {
                        pane_listing(p, s, chunk, done, err);
                    }
                }
                UiMsg::Patched { pane, generation, updated, removed } => {
                    if let Some(p) =
                        panes.iter_mut().find(|p| p.id == pane && p.generation == generation && !p.is_loading())
                    {
                        p.on_patch(updated, &removed);
                    }
                }
                _ => {}
            }
        }
        for (id, kind) in s.watcher.drain() {
            if let Some(p) = panes.iter_mut().find(|p| p.id == id) {
                watch_pane(p, s, kind);
            }
        }
        for p in panes.iter_mut() {
            p.refresh_visible(true, true);
        }
        if done(panes) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn listed(s: &mut Services, dir: &Path) -> Pane {
    let mut p = Pane::new(Location::Dir(dir.to_path_buf()), ViewState::default());
    start_listing(&mut p, s, false);
    assert!(pump(s, &mut [&mut p], |p| p[0].accepts_patches()), "listing of {} never completed", dir.display());
    // Let the watch register on the watcher thread before changing anything.
    std::thread::sleep(Duration::from_millis(300));
    p
}

fn shows(name: &str) -> impl Fn(&[&mut Pane]) -> bool + '_ {
    move |p| names(p[0]).iter().any(|n| n == name)
}

fn hides(name: &str) -> impl Fn(&[&mut Pane]) -> bool + '_ {
    move |p| !names(p[0]).iter().any(|n| n == name)
}

#[test]
fn watcher_changes_reach_the_visible_list() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), b"1").unwrap();
    let mut s = services();
    let mut p = listed(&mut s, d.path());
    assert_eq!(names(&p), ["a.txt"]);

    std::fs::write(d.path().join("new.txt"), b"x").unwrap();
    assert!(pump(&mut s, &mut [&mut p], shows("new.txt")), "create not shown: {:?}", names(&p));

    std::fs::write(d.path().join("new.txt"), b"longer contents").unwrap();
    let grown = |p: &[&mut Pane]| p[0].entry("new.txt").is_some_and(|e| e.size == 15);
    assert!(pump(&mut s, &mut [&mut p], grown), "modification not applied");

    std::fs::rename(d.path().join("new.txt"), d.path().join("renamed.txt")).unwrap();
    let renamed = |p: &[&mut Pane]| {
        let n = names(p[0]);
        n.contains(&"renamed.txt".to_string()) && !n.contains(&"new.txt".to_string())
    };
    assert!(pump(&mut s, &mut [&mut p], renamed), "rename not applied: {:?}", names(&p));

    std::fs::remove_file(d.path().join("renamed.txt")).unwrap();
    assert!(pump(&mut s, &mut [&mut p], hides("renamed.txt")), "delete not applied: {:?}", names(&p));
    assert_eq!(names(&p), ["a.txt"]);
}

/// Two panes on the same folder (dual pane): both see the change.
#[test]
fn both_panes_on_one_folder_update() {
    let d = tempfile::tempdir().unwrap();
    let mut s = services();
    let mut a = listed(&mut s, d.path());
    let mut b = listed(&mut s, d.path());
    std::fs::write(d.path().join("both.txt"), b"x").unwrap();
    let ok = pump(&mut s, &mut [&mut a, &mut b], |p| p.iter().all(|p| names(p).iter().any(|n| n == "both.txt")));
    assert!(ok, "a: {:?} b: {:?}", names(&a), names(&b));
}

/// Manual refresh relists everything (even with a dead watch) and registers the watch
/// again, so later changes are patched in.
#[test]
fn refresh_relists_and_rearms_the_watch() {
    let d = tempfile::tempdir().unwrap();
    let mut s = services();
    let mut p = listed(&mut s, d.path());
    s.watcher.unwatch(p.id); // the OS stopped reporting for this folder
    std::fs::write(d.path().join("unseen.txt"), b"x").unwrap();
    refresh_pane(&mut p, &mut s);
    assert!(p.refreshing, "a refresh keeps the contents while relisting");
    assert!(pump(&mut s, &mut [&mut p], shows("unseen.txt")), "refresh did not relist: {:?}", names(&p));
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(d.path().join("later.txt"), b"x").unwrap();
    assert!(pump(&mut s, &mut [&mut p], shows("later.txt")), "watch not re-armed: {:?}", names(&p));
}

/// A refresh of a pane whose listing failed starts over (no "soft" refresh of nothing).
#[test]
fn refresh_after_a_failed_listing_lists_again() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("later");
    let mut s = services();
    let mut p = Pane::new(Location::Dir(dir.clone()), ViewState::default());
    start_listing(&mut p, &mut s, false);
    let failed = |p: &[&mut Pane]| matches!(p[0].status, crate::state::pane::ListingStatus::Failed(_));
    assert!(pump(&mut s, &mut [&mut p], failed));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    refresh_pane(&mut p, &mut s);
    assert!(pump(&mut s, &mut [&mut p], shows("f.txt")), "{:?}", p.status);
}

/// Changes reported while a listing runs are re-stat'ed once it completes.
#[test]
fn changes_during_a_listing_are_not_dropped() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), b"1").unwrap();
    let mut s = services();
    let mut p = listed(&mut s, d.path());
    // A soft relist is running; meanwhile the watcher reports a file the relist missed
    // (simulated: the relist result predates the file).
    start_listing(&mut p, &mut s, true);
    assert!(p.refreshing);
    let generation = p.generation;
    let late = d.path().join("late.txt");
    watch_pane(&mut p, &mut s, soyuz_fs::WatchKind::Changed(vec![late.clone()]));
    assert_eq!(p.deferred_paths, std::slice::from_ref(&late));
    // Drop the real relist result and deliver an older snapshot instead.
    while s.mailbox.try_recv().is_some() {}
    let stale = soyuz_fs::read_dir(d.path(), &soyuz_fs::ListOptions::default()).unwrap();
    std::fs::write(&late, b"x").unwrap();
    pane_listing(&mut p, &mut s, stale, true, None);
    assert_eq!(p.generation, generation);
    assert!(p.deferred_paths.is_empty(), "deferred paths handed to the lister");
    assert!(pump(&mut s, &mut [&mut p], shows("late.txt")), "{:?}", names(&p));
}

/// After a job: the panes showing its destination (and source folders) are relisted,
/// without any help from the watcher.
#[test]
fn job_completion_relists_panes_showing_affected_folders() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    let dst = d.path().join("dst");
    let other = d.path().join("other");
    for p in [&src, &dst, &other] {
        std::fs::create_dir(p).unwrap();
    }
    std::fs::write(src.join("report.txt"), b"data").unwrap();
    std::fs::write(src.join("move.txt"), b"data").unwrap();

    let mut s = services();
    let mut pd = listed(&mut s, &dst);
    let mut ps = listed(&mut s, &src);
    let mut po = listed(&mut s, &other);
    // No watcher at all: only the job-completion refresh can update these panes.
    for p in [&pd, &ps, &po] {
        s.watcher.unwatch(p.id);
    }
    let mut ws = Workspace::new(Tab::new(Pane::new(Location::Computer, ViewState::default())));
    let (pd_id, ps_id, po_id) = (pd.id, ps.id, po.id);
    ws.tabs.push(Tab::new(std::mem::replace(&mut pd, Pane::new(Location::Computer, ViewState::default()))));
    ws.tabs.push(Tab::new(std::mem::replace(&mut ps, Pane::new(Location::Computer, ViewState::default()))));
    ws.tabs.push(Tab::new(std::mem::replace(&mut po, Pane::new(Location::Computer, ViewState::default()))));

    let engine = JobEngine::new(EngineConfig::default(), Arc::new(soyuz_fs::RealProbe), Arc::new(|| {}));
    let mut affected = AffectedPaths::default();
    let run = |engine: &JobEngine, affected: &mut AffectedPaths, kind: JobKind| {
        let id = engine.submit(JobSpec::new(kind.clone()));
        affected.submitted(id, &kind);
        let deadline = Instant::now() + WAIT;
        loop {
            let ev = engine.events().recv_timeout(Duration::from_millis(100));
            if let Ok(e) = &ev {
                affected.event(e);
                if matches!(e, JobEvent::Finished(j, _) if *j == id) {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "job did not finish");
        }
        affected.take()
    };

    // Copy src/report.txt → dst: only the destination (and source) panes are relisted.
    let changed = run(
        &engine,
        &mut affected,
        JobKind::Copy { sources: vec![src.join("report.txt")], dest_dir: dst.clone(), rename_to: None },
    );
    let dirs = affected_dirs(&changed);
    let ids = panes_showing(&ws, &dirs);
    assert!(ids.contains(&pd_id) && ids.contains(&ps_id), "{dirs:?}");
    assert!(!ids.contains(&po_id));
    for t in &mut ws.tabs {
        for p in &mut t.panes {
            if ids.contains(&p.id) {
                relist_for_job(p, &mut s);
            }
        }
    }
    let (first, rest) = ws.tabs.split_at_mut(2);
    let pd = &mut first[1].panes[0];
    let ps = &mut rest[0].panes[0];
    let ok = pump(&mut s, &mut [&mut *pd, &mut *ps], |p| {
        names(p[0]).contains(&"report.txt".to_string()) && p[1].accepts_patches()
    });
    assert!(ok, "copy not shown in the destination pane: {:?}", names(pd));

    // Move src/move.txt → dst: it leaves the source pane and shows in the destination.
    let changed = run(
        &engine,
        &mut affected,
        JobKind::Move { sources: vec![src.join("move.txt")], dest_dir: dst.clone(), rename_to: None },
    );
    let dirs = affected_dirs(&changed);
    for p in [&mut *pd, &mut *ps] {
        assert!(dirs.iter().any(|d| same_dir(d, p.location.as_fs_path().unwrap())));
        relist_for_job(p, &mut s);
    }
    let ok = pump(&mut s, &mut [&mut *pd, &mut *ps], |p| {
        names(p[0]).contains(&"move.txt".to_string()) && !names(p[1]).contains(&"move.txt".to_string())
    });
    assert!(
        ok,
        "move not reflected: dst {:?} src {:?} on disk {:?}",
        names(pd),
        names(ps),
        std::fs::read_dir(&dst).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>()
    );
    engine.shutdown(Duration::from_secs(2));
}

/// A job finishing while the pane is still loading relists it right after.
#[test]
fn job_finishing_during_a_listing_relists_afterwards() {
    let d = tempfile::tempdir().unwrap();
    let mut s = services();
    let mut p = Pane::new(Location::Dir(d.path().to_path_buf()), ViewState::default());
    start_listing(&mut p, &mut s, false);
    assert!(p.is_loading());
    relist_for_job(&mut p, &mut s);
    assert!(p.relist_pending, "no restart while loading");
    std::fs::write(d.path().join("copied.txt"), b"x").unwrap();
    assert!(pump(&mut s, &mut [&mut p], shows("copied.txt")), "{:?}", names(&p));
    assert!(!p.relist_pending);
}

#[test]
fn affected_dirs_cover_paths_and_parents() {
    let dirs = affected_dirs(&[PathBuf::from("/data/dst"), PathBuf::from("/data/src/a.txt"), PathBuf::from("")]);
    assert_eq!(
        dirs,
        [
            PathBuf::from("/data/dst"),
            PathBuf::from("/data"),
            PathBuf::from("/data/src/a.txt"),
            PathBuf::from("/data/src")
        ]
    );
    assert!(same_dir(Path::new("/data/dst/"), Path::new("/data/dst")));
    if cfg!(windows) {
        assert!(same_dir(Path::new(r"C:\Data\Dst"), Path::new(r"c:\data\dst\")));
    }
    assert!(!same_dir(Path::new("/data/dst"), Path::new("/data/dst2")));
}
