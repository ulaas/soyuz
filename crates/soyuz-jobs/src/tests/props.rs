//! Property tests on real temp directories: the batch-rename planner (random
//! chains, cycles, collisions with untouched files and injected failures), undo replay of
//! renames, and sync-plan application against an in-memory model.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proptest::prelude::*;
use soyuz_core::{ConflictResolution, ErrorAction};
use soyuz_fs::fault::{FaultKind, FaultOp, FaultPlan, install};

use super::*;
use crate::job::{SyncAction, SyncPlan};

/// Directory contents: name → bytes (no subfolders in these tests).
fn snapshot(dir: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
        .collect()
}

#[derive(Clone, Debug)]
struct RenameCase {
    /// Files that exist before the job (index i → "f{i}" with content "c{i}").
    files: usize,
    /// (source index, target name index); target names range over twice as many names, so
    /// some are free, some are other sources (chains / cycles) and some are untouched files.
    pairs: Vec<(usize, usize)>,
    /// Inject a rename failure at this occurrence (1-based), if any.
    fail_at: Option<u64>,
}

fn rename_case() -> impl Strategy<Value = RenameCase> {
    (2usize..8)
        .prop_flat_map(|n| {
            let pairs = prop::collection::vec((0..n, 0..2 * n), 1..=n);
            (Just(n), pairs, prop::option::weighted(0.35, 1u64..10))
        })
        .prop_map(|(files, raw, fail_at)| {
            // One pair per source, unique targets (the batch-rename UI guarantees both).
            let mut pairs: Vec<(usize, usize)> = Vec::new();
            for (s, t) in raw {
                if pairs.iter().all(|(ps, pt)| *ps != s && *pt != t) {
                    pairs.push((s, t));
                }
            }
            RenameCase { files, pairs, fail_at }
        })
}

/// (left files, right files, actions): `(name index, content byte)` / `(action, name index)`.
type SyncCase = (Vec<(usize, u8)>, Vec<(usize, u8)>, Vec<(u8, usize)>);

fn sync_case() -> impl Strategy<Value = SyncCase> {
    let side = prop::collection::vec((0usize..6, any::<u8>()), 0..6);
    let actions = prop::collection::vec((0u8..4, 0usize..6), 0..10);
    (side.clone(), side, actions)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn batch_rename_never_loses_or_clobbers(case in rename_case()) {
        let h = H::new();
        let dir = h.p("r");
        std::fs::create_dir(&dir).unwrap();
        for i in 0..case.files {
            write(&dir.join(format!("f{i}")), format!("c{i}").as_bytes());
        }
        let before = snapshot(&dir);
        let sources: Vec<usize> = case.pairs.iter().map(|(s, _)| *s).collect();
        let pairs: Vec<(PathBuf, PathBuf)> =
            case.pairs.iter().map(|(s, t)| (dir.join(format!("f{s}")), dir.join(format!("f{t}")))).collect();
        let guard = case.fail_at.map(|n| install(FaultPlan::new(&dir).fail(FaultOp::Rename, n, FaultKind::in_use())));
        let mut spec = JobSpec::new(JobKind::Rename { pairs: pairs.clone() });
        // Collisions with files outside the batch are skipped, failures are skipped.
        spec.options.conflict = Some(ConflictResolution::Skip);
        spec.options.errors = Some(ErrorAction::Skip);
        let id = h.engine.submit(spec);
        let state = h.done(id);
        let fired = guard.as_ref().map(|g| g.fired().len()).unwrap_or(0);
        drop(guard);
        let after = snapshot(&dir);

        // 1. No content lost or duplicated (a parked entry may keep its temp name).
        let mut want: Vec<&Vec<u8>> = before.values().collect();
        let mut got: Vec<&Vec<u8>> = after.values().collect();
        want.sort();
        got.sort();
        prop_assert_eq!(got, want, "contents conserved; {:?}", case);
        // 2. Files outside the batch keep their content under their own name.
        for i in (0..case.files).filter(|i| !sources.contains(i)) {
            let name = format!("f{i}");
            prop_assert_eq!(after.get(&name), before.get(&name), "untouched {} clobbered; {:?}", name, case);
        }
        // 3. Temp names only remain with a reported failure.
        let parked = after.keys().any(|k| k.starts_with(".soyuz-rename-"));
        let summary = match &state { JobState::Done(s) => s.clone(), other => panic!("{other:?}") };
        if parked {
            prop_assert!(summary.items_failed > 0, "parked entry without a failure; {:?}", case);
        }
        // 4. Without faults: every pair whose target was free or vacated by the batch is done.
        if fired == 0 {
            let targets_outside: Vec<usize> = case.pairs.iter().map(|(_, t)| *t)
                .filter(|t| *t < case.files && !sources.contains(t)).collect();
            if targets_outside.is_empty() {
                for (s, t) in &case.pairs {
                    let got = after.get(&format!("f{t}")).cloned();
                    prop_assert_eq!(got, Some(format!("c{s}").into_bytes()), "pair {}→{}; {:?}", s, t, case);
                }
                prop_assert_eq!(summary.items_failed + summary.items_skipped, 0);
                // 5. Undo replays the journal backwards: the original layout comes back.
                if let Some(hid) = h.engine.history().last().filter(|e| e.undoable).map(|e| e.id) {
                    let u = h.engine.undo(hid).unwrap();
                    h.ok(u);
                    prop_assert_eq!(snapshot(&dir), before.clone(), "undo restores; {:?}", case);
                    let r = h.engine.redo(hid).unwrap();
                    h.ok(r);
                    prop_assert_eq!(snapshot(&dir), after.clone(), "redo re-applies; {:?}", case);
                }
            }
        }
    }

    #[test]
    fn sync_plan_matches_model((left, right, actions) in sync_case()) {
        let h = H::new();
        let (l, r) = (h.p("L"), h.p("R"));
        std::fs::create_dir_all(&l).unwrap();
        std::fs::create_dir_all(&r).unwrap();
        let mut ml: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut mr: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (i, b) in &left {
            ml.insert(format!("n{i}"), vec![*b; 1 + *i * 100]);
        }
        for (i, b) in &right {
            mr.insert(format!("n{i}"), vec![b.wrapping_add(1); 7 + *i]);
        }
        for (k, v) in &ml { write(&l.join(k), v); }
        for (k, v) in &mr { write(&r.join(k), v); }
        let plan_actions: Vec<SyncAction> = actions.iter().map(|(a, i)| {
            let p = PathBuf::from(format!("n{i}"));
            match a { 0 => SyncAction::CopyLR(p), 1 => SyncAction::CopyRL(p), 2 => SyncAction::DeleteL(p), _ => SyncAction::DeleteR(p) }
        }).collect();
        // Model.
        for a in &plan_actions {
            match a {
                SyncAction::CopyLR(p) => if let Some(v) = ml.get(&p.to_string_lossy().into_owned()).cloned() { mr.insert(p.to_string_lossy().into_owned(), v); },
                SyncAction::CopyRL(p) => if let Some(v) = mr.get(&p.to_string_lossy().into_owned()).cloned() { ml.insert(p.to_string_lossy().into_owned(), v); },
                SyncAction::DeleteL(p) => { ml.remove(&p.to_string_lossy().into_owned()); }
                SyncAction::DeleteR(p) => { mr.remove(&p.to_string_lossy().into_owned()); }
                SyncAction::CopyAs { .. } => {}
            }
        }
        let mut spec = JobSpec::new(JobKind::Sync { plan: SyncPlan { left: l.clone(), right: r.clone(), actions: plan_actions } });
        spec.options.errors = Some(ErrorAction::Skip);
        let id = h.engine.submit(spec);
        settle(&h, id, true);
        prop_assert_eq!(snapshot(&l), ml);
        prop_assert_eq!(snapshot(&r), mr);
    }
}
