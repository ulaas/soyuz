//! Dispatch through `FsToolHost`: scope enforcement, read tools and dry-run plans for
//! copy / move / rename / trash. Everything happens inside a temp dir; nothing is executed.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use soyuz_ai::tools::{
    CallContext, FsToolHost, PlanAction, PlanConflict, PlanOp, Risk, Scope, ScopeError, ToolError, ToolHost,
    ToolResponse, dispatch,
};
use soyuz_core::config::AiPrivacy;
use soyuz_jobs::{DeleteMode, JobKind};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    host: FsToolHost,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let w = |p: &str, data: &[u8]| {
        let p = root.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    };
    w("src/a.txt", b"hello");
    w("src/b.txt", b"world!");
    w("src/sub/c.txt", b"0123456789");
    w("dst/a.txt", b"old");
    w("notes/readme.md", b"line1\nline2\nline3\n");
    w("secrets/.env", b"TOKEN=1");
    w("photos/img1.jpg", b"jpg1");
    w("photos/img2.jpg", b"jpg2");
    let host = FsToolHost::new(Scope::new(vec![root.clone()], &[], false));
    Fixture { _dir: dir, root, host }
}

fn ctx(max: Risk) -> CallContext {
    CallContext::new("test", max, AiPrivacy::MetadataOnly)
}

fn p(f: &Fixture, rel: &str) -> String {
    f.root.join(rel).to_string_lossy().into_owned()
}

fn plan_of(r: ToolResponse) -> soyuz_ai::tools::Plan {
    match r {
        ToolResponse::Plan(p) | ToolResponse::NeedsApproval(p) => p,
        ToolResponse::Data(d) => panic!("expected a plan, got {d}"),
    }
}

#[test]
fn read_tools_respect_scope_and_privacy() {
    let f = fixture();
    let c = ctx(Risk::Read);
    let r = dispatch(&f.host, "list_dir", json!({"path": p(&f, "src")}), &c).unwrap();
    let ToolResponse::Data(v) = r else { panic!() };
    let names: Vec<&str> = v["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["sub", "a.txt", "b.txt"], "folders first, natural order");

    // The blocked .env is filtered out of listings and refused directly.
    let ToolResponse::Data(v) = dispatch(&f.host, "list_dir", json!({"path": p(&f, "secrets")}), &c).unwrap() else {
        panic!()
    };
    assert_eq!(v["total"], 0);
    let e = dispatch(&f.host, "stat", json!({"path": p(&f, "secrets/.env")}), &c).unwrap_err();
    assert!(matches!(e, ToolError::Scope(ScopeError::Sensitive(_))), "{e}");

    // Outside the root.
    let outside = f.root.parent().unwrap().to_string_lossy().into_owned();
    let e = dispatch(&f.host, "list_dir", json!({"path": outside}), &c).unwrap_err();
    assert!(matches!(e, ToolError::Scope(ScopeError::OutsideRoots(_))), "{e}");

    // read_text needs content privacy.
    let e = dispatch(&f.host, "read_text", json!({"path": p(&f, "notes/readme.md")}), &c).unwrap_err();
    assert_eq!(e, ToolError::ContentNotAllowed);
    let content = CallContext::new("t", Risk::Read, AiPrivacy::Contents);
    let ToolResponse::Data(v) = dispatch(
        &f.host,
        "read_text",
        json!({"path": p(&f, "notes/readme.md"), "start_line": 2, "end_line": 3}),
        &content,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(v["text"], "line2\nline3");

    // Write tools are refused under a read-only ceiling; bad args are reported.
    let e =
        dispatch(&f.host, "copy", json!({"sources": [p(&f, "src/a.txt")], "dest_dir": p(&f, "dst")}), &c).unwrap_err();
    assert!(matches!(e, ToolError::RiskTooHigh { .. }));
    let e = dispatch(&f.host, "stat", json!({"path": 1}), &c).unwrap_err();
    assert!(matches!(e, ToolError::InvalidArgs(_)));
    // UI tools are not available in the crate-level host.
    let e = dispatch(&f.host, "get_selection", Value::Null, &c).unwrap_err();
    assert_eq!(e, ToolError::Unsupported("get_selection".into()));
}

#[test]
fn search_and_sizes() {
    let f = fixture();
    let c = ctx(Risk::Read);
    let ToolResponse::Data(v) =
        dispatch(&f.host, "search", json!({"roots": [p(&f, "")], "extensions": ["jpg"]}), &c).unwrap()
    else {
        panic!()
    };
    assert_eq!(v["count"], 2, "{v}");
    let ToolResponse::Data(v) = dispatch(&f.host, "folder_size", json!({"paths": [p(&f, "src")]}), &c).unwrap() else {
        panic!()
    };
    assert_eq!(v["sizes"][0]["bytes"], 21);
    let e = dispatch(&f.host, "search", json!({"roots": [p(&f, "")], "content": "TOKEN"}), &c).unwrap_err();
    assert_eq!(e, ToolError::ContentNotAllowed, "content search needs content privacy");
}

#[test]
fn copy_plan_with_conflicts_and_sizes() {
    let f = fixture();
    let c = ctx(Risk::Write);
    let args = json!({
        "sources": [p(&f, "src/a.txt"), p(&f, "src/sub"), p(&f, "src/missing.txt")],
        "dest_dir": p(&f, "dst"),
        "conflict": "keep_both",
        "dry_run": true,
    });
    let r = dispatch(&f.host, "copy", args, &c).unwrap();
    assert!(matches!(r, ToolResponse::Plan(_)), "dry_run returns a plain plan");
    let plan = plan_of(r);
    assert_eq!(plan.items.len(), 3);
    assert_eq!(plan.items[0].op, PlanOp::Copy);
    assert!(
        matches!(&plan.items[0].conflict, Some(PlanConflict::Exists { resolution, size: 3, .. }) if resolution == "keep_both")
    );
    assert_eq!(plan.items[1].bytes, 10);
    assert_eq!(plan.items[1].entries, 2, "folder + one file");
    assert_eq!(plan.items[2].conflict, Some(PlanConflict::Missing));
    assert!(!plan.executable, "a missing source blocks execution");
    assert_eq!(plan.total_bytes, 15);
    let job = plan.job().unwrap();
    assert!(matches!(&job.kind, JobKind::Copy { sources, .. } if sources.len() == 3));
    assert_eq!(job.options.conflict, Some(soyuz_core::ConflictResolution::KeepBoth));
    assert!(job.options.journal, "AI jobs are undoable");
    assert!(job.options.label.as_deref().unwrap().starts_with("AI: Copy 3 items"));

    // Without dry_run the same plan comes back as awaiting approval.
    let r = dispatch(&f.host, "copy", json!({"sources": [p(&f, "src/b.txt")], "dest_dir": p(&f, "dst")}), &c).unwrap();
    let ToolResponse::NeedsApproval(plan) = r else { panic!() };
    assert!(plan.executable);
    assert_eq!(plan.conflicts, 0);
    // Nothing happened on disk.
    assert!(!f.root.join("dst/b.txt").exists());
}

#[test]
fn copy_into_itself_and_sensitive_trees_are_refused() {
    let f = fixture();
    let c = ctx(Risk::Write);
    let plan = plan_of(
        dispatch(
            &f.host,
            "copy",
            json!({"sources": [p(&f, "src")], "dest_dir": p(&f, "src/sub"), "dry_run": true}),
            &c,
        )
        .unwrap(),
    );
    assert_eq!(plan.items[0].conflict, Some(PlanConflict::IntoSelf));
    assert!(!plan.executable);
    // A tree containing a blocked path cannot be moved.
    let e = dispatch(
        &f.host,
        "move",
        json!({"sources": [p(&f, "secrets")], "dest_dir": p(&f, "dst"), "dry_run": true}),
        &c,
    )
    .unwrap_err();
    assert!(matches!(e, ToolError::Scope(ScopeError::Sensitive(_))), "{e}");
    // Writing into the root itself is refused for move sources.
    let e = dispatch(&f.host, "move", json!({"sources": [p(&f, "")], "dest_dir": p(&f, "dst")}), &c).unwrap_err();
    assert!(matches!(e, ToolError::Scope(_)), "{e}");
}

#[test]
fn move_plan() {
    let f = fixture();
    let c = ctx(Risk::Write);
    let plan = plan_of(
        dispatch(
            &f.host,
            "move",
            json!({"sources": [p(&f, "photos/img1.jpg")], "dest_dir": p(&f, "photos"), "dry_run": true}),
            &c,
        )
        .unwrap(),
    );
    assert_eq!(plan.items[0].conflict, Some(PlanConflict::SameItem), "moving into its own folder is a no-op");
    let plan = plan_of(
        dispatch(&f.host, "move", json!({"sources": [p(&f, "photos/img1.jpg"), p(&f, "photos/img2.jpg")], "dest_dir": p(&f, "dst"), "dry_run": true}), &c)
            .unwrap(),
    );
    assert!(plan.executable);
    assert_eq!(plan.total_bytes, 8);
    assert!(matches!(plan.job().unwrap().kind, JobKind::Move { .. }));
    assert_eq!(plan.summary, format!("Move 2 items into {}", f.root.join("dst").display()));
}

#[test]
fn rename_and_batch_rename_plans() {
    let f = fixture();
    let c = ctx(Risk::Write);
    let plan = plan_of(
        dispatch(&f.host, "rename", json!({"path": p(&f, "src/a.txt"), "new_name": "b.txt", "dry_run": true}), &c)
            .unwrap(),
    );
    assert!(
        matches!(&plan.items[0].conflict, Some(PlanConflict::Exists { resolution, .. }) if resolution == "blocked")
    );
    assert!(!plan.executable, "renames never overwrite");
    let plan = plan_of(
        dispatch(&f.host, "rename", json!({"path": p(&f, "src/a.txt"), "new_name": "bad/name", "dry_run": true}), &c)
            .unwrap(),
    );
    assert!(matches!(plan.items[0].conflict, Some(PlanConflict::InvalidName { .. })));

    // Swap a <-> b inside one batch is allowed (targets are freed by the batch).
    let plan = plan_of(
        dispatch(
            &f.host,
            "batch_rename",
            json!({"pairs": [{"path": p(&f, "src/a.txt"), "new_name": "b.txt"}, {"path": p(&f, "src/b.txt"), "new_name": "a.txt"}], "dry_run": true}),
            &c,
        )
        .unwrap(),
    );
    assert!(plan.executable, "{:?}", plan.items);
    let PlanAction::Job(job) = &plan.action else { panic!() };
    let JobKind::Rename { pairs } = &job.kind else { panic!() };
    assert_eq!(pairs.len(), 2);

    // Rule-based: prefix + regex, duplicate targets detected.
    let plan = plan_of(
        dispatch(
            &f.host,
            "batch_rename",
            json!({"paths": [p(&f, "photos/img1.jpg"), p(&f, "photos/img2.jpg")], "rule": {"find": "img(\\d)", "replace": "photo-$1", "regex": true, "prefix": "2026-"}, "dry_run": true}),
            &c,
        )
        .unwrap(),
    );
    let dsts: Vec<String> = plan
        .items
        .iter()
        .map(|i| i.dst.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(dsts, ["2026-photo-1.jpg", "2026-photo-2.jpg"]);
    let plan = plan_of(
        dispatch(
            &f.host,
            "batch_rename",
            json!({"paths": [p(&f, "photos/img1.jpg"), p(&f, "photos/img2.jpg")], "rule": {"find": "img\\d", "replace": "same", "regex": true}, "dry_run": true}),
            &c,
        )
        .unwrap(),
    );
    assert_eq!(plan.items[1].conflict, Some(PlanConflict::DuplicateTarget));
}

#[test]
fn trash_and_delete_plans() {
    let f = fixture();
    let write_only = ctx(Risk::Write);
    let e = dispatch(&f.host, "trash", json!({"paths": [p(&f, "src")]}), &write_only).unwrap_err();
    assert!(matches!(e, ToolError::RiskTooHigh { .. }), "destructive tools need the destructive ceiling");

    let c = ctx(Risk::Destructive);
    let r = dispatch(&f.host, "trash", json!({"paths": [p(&f, "src"), p(&f, "notes/readme.md")]}), &c).unwrap();
    let ToolResponse::NeedsApproval(plan) = r else { panic!("destructive tools always need approval") };
    assert_eq!(plan.items[0].op, PlanOp::Trash);
    assert_eq!(plan.items[0].bytes, 21);
    assert_eq!(plan.items[0].entries, 5, "src, a, b, sub, c");
    assert_eq!(plan.total_bytes, 21 + 18);
    assert!(matches!(plan.job().unwrap().kind, JobKind::Delete { mode: DeleteMode::Trash, .. }));
    assert!(plan.warnings.is_empty());

    let plan = plan_of(
        dispatch(&f.host, "delete_permanent", json!({"paths": [p(&f, "dst/a.txt")], "dry_run": true}), &c).unwrap(),
    );
    assert!(matches!(plan.job().unwrap().kind, JobKind::Delete { mode: DeleteMode::Permanent, .. }));
    assert_eq!(plan.warnings, ["Permanent deletion cannot be undone."]);
    assert!(f.root.join("src/a.txt").exists() && f.root.join("dst/a.txt").exists(), "planning never deletes");

    // The model-facing view hides the job spec but keeps totals and items.
    let v = plan.to_model_value();
    assert!(v.get("action").is_none());
    assert_eq!(v["executable"], true);
    assert_eq!(v["items"][0]["op"], "delete");
}

#[test]
fn app_actions_and_mkdir() {
    let f = fixture();
    let c = ctx(Risk::Destructive);
    let plan = plan_of(dispatch(&f.host, "tag", json!({"paths": [p(&f, "src/a.txt")], "tags": ["red"]}), &c).unwrap());
    assert!(matches!(&plan.action, PlanAction::App { op, .. } if op == "tag"));
    let r = dispatch(&f.host, "undo", json!({"history_id": 7}), &c).unwrap();
    assert!(matches!(r, ToolResponse::NeedsApproval(_)), "undo needs approval");
    let r = dispatch(&f.host, "run_custom_command", json!({"name": "Open in VS Code"}), &c).unwrap();
    assert!(matches!(r, ToolResponse::NeedsApproval(_)));
    let plan = plan_of(dispatch(&f.host, "mkdir", json!({"path": p(&f, "src"), "dry_run": true}), &c).unwrap());
    assert!(!plan.executable, "existing folder");
    let plan = plan_of(dispatch(&f.host, "mkdir", json!({"path": p(&f, "new/dir"), "dry_run": true}), &c).unwrap());
    assert!(plan.executable);
    assert!(!Path::new(&p(&f, "new")).exists());
    // The crate-level host cannot execute.
    assert!(matches!(f.host.execute(&plan, &c), Err(ToolError::Unsupported(_))));
}

#[cfg(windows)]
#[test]
fn junction_escape_is_caught() {
    let f = fixture();
    let outside = f.root.parent().unwrap().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("x.txt"), b"x").unwrap();
    let link = f.root.join("link");
    soyuz_fs::junction(&outside, &link).unwrap();
    let e =
        dispatch(&f.host, "stat", json!({"path": link.join("x.txt").to_string_lossy()}), &ctx(Risk::Read)).unwrap_err();
    assert!(matches!(e, ToolError::Scope(ScopeError::OutsideRoots(_))), "{e}");
}
