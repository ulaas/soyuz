//! Scripting tests: sandbox (denied operations, limits), registration, plan generation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use soyuz_core::{EntryInfo, EntryKind};
use soyuz_jobs::{DeleteMode, JobKind};

use crate::{ActionCtx, Limits, MenuContext, Preview, ScriptSet, UiOp};

fn set(src: &str) -> ScriptSet {
    ScriptSet::from_sources(&[("t.rhai", src)], Limits::default())
}

fn set_with(src: &str, limits: Limits) -> ScriptSet {
    ScriptSet::from_sources(&[("t.rhai", src)], limits)
}

fn p(s: &str) -> String {
    s.replace('\\', "/")
}

fn ctx(dir: &Path, sel: &[PathBuf]) -> ActionCtx {
    ActionCtx { dir: Some(dir.to_path_buf()), focused: sel.first().cloned(), selected: sel.to_vec(), other_dir: None }
}

#[test]
fn action_plans_jobs_and_ui_ops() {
    let s = set(r#"
        register_action(#{
            title: "Tidy up",
            hotkey: "Ctrl+Alt+T",
            run: |ctx| {
                for f in ctx.selected { move_to(f, join_path(ctx.dir, "Archive")); }
                trash(ctx.selected[0]);
                rename(ctx.focused, "renamed.txt");
                mkdir(join_path(ctx.dir, "New"));
                copy(ctx.selected, ctx.dir);
                toast("tidied " + ctx.count);
                select(ctx.selected);
                tag(ctx.selected, "blue");
                print("hello");
            }
        });
    "#);
    let sc = &s.scripts[0];
    assert!(sc.is_ok(), "{:?}", sc.error);
    assert_eq!(sc.actions[0].id, "tidy-up");
    assert_eq!(sc.actions[0].hotkey, "Ctrl+Alt+T");
    let dir = std::env::temp_dir();
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    let out = s.run_action("t/tidy-up", &ctx(&dir, &[a.clone(), b.clone()])).unwrap();
    assert_eq!(out.jobs.len(), 6, "{:#?}", out.jobs);
    assert!(matches!(&out.jobs[0].kind, JobKind::Move { sources, dest_dir, .. }
        if sources == &vec![a.clone()] && dest_dir == &dir.join("Archive")));
    assert!(matches!(&out.jobs[2].kind, JobKind::Delete { mode: DeleteMode::Trash, .. }));
    assert!(matches!(&out.jobs[3].kind, JobKind::Rename { pairs } if pairs[0].1 == dir.join("renamed.txt")));
    assert!(matches!(&out.jobs[4].kind, JobKind::Create { .. }));
    assert!(matches!(&out.jobs[5].kind, JobKind::Copy { sources, .. } if sources.len() == 2));
    assert!(out.jobs[0].summary.starts_with("Move “a.txt”"), "{}", out.jobs[0].summary);
    assert_eq!(out.ui[0], UiOp::Toast("tidied 2".into()));
    assert_eq!(out.ui[1], UiOp::Select(vec![a.clone(), b.clone()]));
    assert_eq!(out.ui[2], UiOp::Tag(vec![a, b], "blue".into()));
    assert_eq!(out.log, vec!["hello".to_string()]);
    // Nothing touched the disk.
    assert!(!dir.join("Archive").join("a.txt").exists());
}

#[test]
fn eval_and_import_are_unavailable() {
    let s = set(r#"eval("1 + 1"); register_action(#{ title: "x", run: |c| 1 });"#);
    assert!(s.scripts[0].error.is_some(), "eval must be disabled");
    let s = set(r#"import "evil" as e; register_action(#{ title: "x", run: |c| 1 });"#);
    let e = s.scripts[0].error.clone().expect("import must fail");
    assert_eq!(e.line, Some(1));
}

#[test]
fn syntax_errors_have_positions() {
    let s = set("register_action(#{ title: \"x\",\n  run: |c| { let = 3; } });");
    let e = s.scripts[0].error.clone().unwrap();
    assert_eq!(e.line, Some(2), "{e}");
    assert!(e.col.is_some());
    assert!(e.to_string().starts_with("line 2, column"), "{e}");
}

#[test]
fn runtime_errors_point_into_the_function() {
    let s = set("register_action(#{ title: \"x\", run: |c| {\n\n   let y = 5; y.no_such_method();\n } });");
    let e = s.run_action("t/x", &ActionCtx::default()).unwrap_err();
    assert_eq!(e.line, Some(3), "{e}");
}

#[test]
fn operation_and_time_limits() {
    let src = r#"register_action(#{ title: "spin", run: |c| { loop { } } });"#;
    let limits = Limits { max_operations: 100_000, ..Limits::default() };
    let e = set_with(src, limits).run_action("t/spin", &ActionCtx::default()).unwrap_err();
    assert!(e.message.contains("too many operations"), "{e}");
    let limits = Limits { max_operations: u64::MAX / 2, timeout: Duration::from_millis(100), ..Limits::default() };
    let t = std::time::Instant::now();
    let e = set_with(src, limits).run_action("t/spin", &ActionCtx::default()).unwrap_err();
    assert!(e.message.contains("time limit"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(5));
    // The top level is limited too.
    let s = set_with("loop {}", Limits { max_operations: u64::MAX / 2, ..Limits::default() });
    assert!(s.scripts[0].error.as_ref().unwrap().message.contains("time limit"));
}

#[test]
fn memory_and_recursion_limits() {
    let limits = Limits { max_string_bytes: 1000, ..Limits::default() };
    let s = set_with(r#"register_action(#{ title: "big", run: |c| { let s = "x"; loop { s += s; } } });"#, limits);
    assert!(s.run_action("t/big", &ActionCtx::default()).is_err());
    let s = set("fn f(n) { f(n + 1) } register_action(#{ title: \"deep\", run: |c| f(0) });");
    let e = s.run_action("t/deep", &ActionCtx::default()).unwrap_err();
    assert!(e.message.to_lowercase().contains("stack") || e.message.contains("call"), "{e}");
}

#[test]
fn writes_are_denied_outside_actions() {
    // At load time.
    let s = set("trash(\"C:/x\");\nregister_action(#{ title: \"x\", run: |c| 1 });");
    let e = s.scripts[0].error.clone().unwrap();
    assert!(e.message.contains("cannot run while the script loads"), "{e}");
    assert_eq!(e.line, Some(1));
    // In a column value (read-only).
    let s = set(r#"register_column(#{ title: "Evil", value: |e| { trash(e.path); "x" } });"#);
    let e = s.column_value(0, &EntryInfo::minimal(std::env::temp_dir().join("a.txt"), EntryKind::File)).unwrap_err();
    assert!(e.message.contains("not allowed here"), "{e}");
    // `when` sees only the context: no reads.
    let s = set(r#"register_menu_item(#{ title: "M", when: |c| exists(c.dir), run: |c| 1 });"#);
    let c = ctx(&std::env::temp_dir(), &[std::env::temp_dir().join("a.txt")]);
    assert!(!s.applies("t/m", &c, &[MenuContext::File], &["txt".into()]));
    // Registering outside the top level is refused.
    let s = set(r#"register_action(#{ title: "R", run: |c| register_action(#{ title: "y", run: |c| 1 }) });"#);
    assert!(s.run_action("t/r", &ActionCtx::default()).is_err());
}

#[test]
fn paths_must_be_absolute_and_names_valid() {
    let s = set(r#"
        register_action(#{ title: "rel", run: |c| trash("relative/x.txt") });
        register_action(#{ title: "bad", run: |c| rename(c.focused, "a/b") });
    "#);
    let c = ctx(&std::env::temp_dir(), &[std::env::temp_dir().join("a.txt")]);
    assert!(s.run_action("t/rel", &c).unwrap_err().message.contains("not an absolute path"));
    assert!(s.run_action("t/bad", &c).unwrap_err().message.contains("not a valid name"));
}

#[test]
fn reads_are_capped_and_can_be_turned_off() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("big.txt");
    std::fs::write(&f, "x".repeat(10_000)).unwrap();
    std::fs::write(dir.path().join("b.log"), "y").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub").join("c.log"), "z").unwrap();
    let src = format!(
        r#"register_column(#{{ title: "Len", value: |e| read_text(e.path).len() }});
           register_column(#{{ title: "List", value: |e| list("{d}").len() + "/" + search("{d}", "*.log").len() }});"#,
        d = p(&dir.path().to_string_lossy())
    );
    let limits = Limits { max_read_bytes: 100, ..Limits::default() };
    let s = set_with(&src, limits.clone());
    let e = soyuz_fs::stat(&f, true).unwrap();
    assert_eq!(s.column_value(0, &e).unwrap().as_deref(), Some("100"));
    assert_eq!(s.column_value(1, &e).unwrap().as_deref(), Some("3/2"));
    let s = set_with(&src, Limits { allow_read: false, ..limits });
    let err = s.column_value(0, &e).unwrap_err();
    assert!(err.message.contains("allow_read"), "{err}");
}

#[test]
fn columns_previewers_and_rules() {
    let s = set(r##"
        register_column(#{ title: "Upper", extensions: ["txt"], width: 80, value: |e| e.name.to_upper() });
        register_previewer(#{ title: "Notes", extensions: ["note"], preview: |path| #{ markdown: "# " + file_name(path) } });
        register_previewer(#{ extensions: ["skip"], preview: |path| () });
        fn rule(path, ctx) { if path.ends_with(".tmp") { trash(path); } toast(ctx.rule); }
    "##);
    let sc = &s.scripts[0];
    assert!(sc.is_ok(), "{:?}", sc.error);
    assert!(sc.has_rule);
    assert_eq!(sc.columns[0].width, 80.0);
    let t = std::env::temp_dir();
    let e = EntryInfo::minimal(t.join("a.txt"), EntryKind::File);
    assert_eq!(s.column_value(0, &e).unwrap().as_deref(), Some("A.TXT"));
    let other = EntryInfo::minimal(t.join("a.rs"), EntryKind::File);
    assert_eq!(s.column_value(0, &other).unwrap(), None, "extension filter");
    assert_eq!(s.column_value(3, &e).unwrap(), None, "no such slot");
    assert_eq!(s.preview(&t.join("x.note")).unwrap(), Some(Preview::Markdown("# x.note".into())));
    assert_eq!(s.preview(&t.join("x.skip")).unwrap(), None);
    assert_eq!(s.preview(&t.join("x.other")).unwrap(), None);
    let out = s.run_rule("t", &t.join("x.tmp"), "Clean").unwrap();
    assert_eq!(out.jobs.len(), 1);
    assert_eq!(out.ui, vec![UiOp::Toast("Clean".into())]);
    assert!(s.run_rule("nope", &t.join("x.tmp"), "Clean").is_err());
}

#[test]
fn menu_conditions() {
    let s = set(r#"
        register_menu_item(#{ title: "PDF only", contexts: ["file"], extensions: ["pdf"], run: |c| 1 });
        register_menu_item(#{ title: "Two", when: |c| c.count >= 2, run: |c| 1 });
        register_action(#{ title: "Palette only", run: |c| 1 });
    "#);
    let t = std::env::temp_dir();
    let one = ctx(&t, &[t.join("a.pdf")]);
    let two = ctx(&t, &[t.join("a.pdf"), t.join("b.pdf")]);
    assert!(s.applies("t/pdf-only", &one, &[MenuContext::File], &["pdf".into()]));
    assert!(!s.applies("t/pdf-only", &one, &[MenuContext::File], &["txt".into()]));
    assert!(!s.applies("t/pdf-only", &one, &[MenuContext::Dir], &[]));
    assert!(!s.applies("t/two", &one, &[MenuContext::File], &[]));
    assert!(s.applies("t/two", &two, &[MenuContext::File], &[]));
    let (_, a) = s.action("t/palette-only").unwrap();
    assert!(!a.menu && a.palette);
    assert_eq!(s.actions().count(), 3);
}

#[test]
fn load_dir_respects_disabled_and_reports_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("good.rhai"), r#"register_action(#{ title: "Hi", run: |c| toast("hi") });"#)
        .unwrap();
    std::fs::write(dir.path().join("bad.rhai"), "let x = ;").unwrap();
    std::fs::write(dir.path().join("off.rhai"), r#"register_action(#{ title: "Off", run: |c| 1 });"#).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
    let s = ScriptSet::load_dir(dir.path(), Limits::default(), &["off.rhai".into()]);
    assert_eq!(s.scripts.len(), 3);
    let by = |n: &str| s.scripts.iter().find(|x| x.file == n).unwrap();
    assert!(by("good.rhai").is_ok());
    assert!(by("bad.rhai").error.is_some());
    assert!(!by("off.rhai").enabled);
    assert_eq!(s.actions().map(|(id, ..)| id).collect::<Vec<_>>(), vec!["good/hi".to_string()]);
    let sig = crate::script::signature(dir.path());
    assert_eq!(sig.len(), 3);
}
