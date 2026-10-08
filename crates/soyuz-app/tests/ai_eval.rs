//! Task eval suite (roadmap A8): fixture folders with expected end states, run through the
//! real harness (plans approved automatically by a scripted user, jobs on a real engine
//! in a temp folder). Reports success, model turns, tokens and cost per task.
//!
//! ```sh
//! cargo test -p soyuz-app --test ai_eval -- --ignored --nocapture
//! # against a configured provider profile from config.toml:
//! SOYUZ_AI_EVAL_PROFILE=claude cargo test -p soyuz-app --test ai_eval -- --ignored --nocapture
//! ```
//!
//! Without `SOYUZ_AI_EVAL_PROFILE` every task replays a reference transcript (a known
//! good solution), which checks the fixtures, the checkers and the harness itself. The
//! dedupe task moves files to the real trash, so it only runs with `SOYUZ_TRASH_TESTS=1`.

mod ai_common;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ai_common::{Harness, Policy, fixture, tree};
use chrono::TimeZone;
use serde_json::json;
use soyuz_app::ai::context::{self, ContextInput, EntrySummary};
use soyuz_app::ai::fake::{FakeModel, FakeTurn};
use soyuz_app::ai::model::{self, ChatModel, Route};
use soyuz_app::ai::safety::Permissions;
use soyuz_core::config::AiAccess;

struct EvalTask {
    name: &'static str,
    prompt: &'static str,
    setup: fn(&Path),
    reference: fn(&Path) -> Vec<FakeTurn>,
    check: fn(&Path) -> Result<(), String>,
    needs_trash: bool,
}

fn setup_downloads(root: &Path) {
    fixture(
        root,
        &[
            ("report.pdf", "%PDF report"),
            ("invoice-2026-03.pdf", "%PDF invoice"),
            ("holiday.jpg", "jpeg bytes"),
            ("cat.png", "png bytes"),
            ("song.mp3", "mp3 bytes"),
            ("setup.zip", "zip bytes"),
            ("notes.txt", "text"),
            ("screenshot 2026-09-01.png", "png 2"),
        ],
    );
}

fn reference_downloads(root: &Path) -> Vec<FakeTurn> {
    let j = |n: &str| root.join(n);
    vec![
        FakeTurn::text("Let me look.").call("list_dir", json!({"path": root})),
        FakeTurn::text("Sorting by type.")
            .call("mkdir", json!({"path": j("Images")}))
            .call("mkdir", json!({"path": j("Documents")}))
            .call("mkdir", json!({"path": j("Other")})),
        FakeTurn::text("Moving.")
            .call("move", json!({"sources": [j("holiday.jpg"), j("cat.png"), j("screenshot 2026-09-01.png")], "dest_dir": j("Images")}))
            .call("move", json!({"sources": [j("report.pdf"), j("invoice-2026-03.pdf"), j("notes.txt")], "dest_dir": j("Documents")}))
            .call("move", json!({"sources": [j("song.mp3"), j("setup.zip")], "dest_dir": j("Other")})),
        FakeTurn::text("Done."),
    ]
}

fn check_downloads(root: &Path) -> Result<(), String> {
    let files: Vec<String> = tree(root).into_iter().filter(|p| !p.ends_with('/')).collect();
    let names: Vec<String> = files.iter().map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).collect();
    for n in ["report.pdf", "invoice-2026-03.pdf", "holiday.jpg", "cat.png", "song.mp3", "setup.zip", "notes.txt"] {
        if names.iter().filter(|x| x.as_str() == n).count() != 1 {
            return Err(format!("{n} lost or duplicated: {files:?}"));
        }
    }
    let loose = files.iter().filter(|p| !p.contains('/')).count();
    if loose > 2 {
        return Err(format!("{loose} files still loose at the top level"));
    }
    if files.iter().any(|p| !p.contains('/') && (p.ends_with(".jpg") || p.ends_with(".png"))) {
        return Err("images left at the top level".into());
    }
    Ok(())
}

fn photo_dates() -> Vec<(&'static str, &'static str)> {
    vec![("IMG_0001.jpg", "2026-06-14"), ("IMG_0002.jpg", "2026-07-02"), ("DSC9921.jpg", "2025-12-24")]
}

fn setup_photos(root: &Path) {
    for (name, date) in photo_dates() {
        fixture(root, &[(name, "jpeg")]);
        let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap().and_hms_opt(12, 0, 0).unwrap();
        let t: SystemTime = chrono::Local.from_local_datetime(&d).earliest().map(SystemTime::from).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(root.join(name)).unwrap();
        f.set_modified(t).unwrap();
    }
}

fn reference_photos(root: &Path) -> Vec<FakeTurn> {
    let pairs: Vec<serde_json::Value> =
        photo_dates().iter().map(|(n, d)| json!({"path": root.join(n), "new_name": format!("{d} {n}")})).collect();
    vec![
        FakeTurn::text("Checking dates.").call("list_dir", json!({"path": root})),
        FakeTurn::text("Renaming by date.").call("batch_rename", json!({"pairs": pairs})),
        FakeTurn::text("Done."),
    ]
}

fn check_photos(root: &Path) -> Result<(), String> {
    let files = tree(root);
    if files.len() != photo_dates().len() {
        return Err(format!("expected {} files: {files:?}", photo_dates().len()));
    }
    for (orig, date) in photo_dates() {
        let stem = orig.trim_end_matches(".jpg");
        if !files.iter().any(|f| f.starts_with(date) && f.contains(stem.trim_start_matches("IMG_"))) {
            return Err(format!("{orig} not renamed to start with {date}: {files:?}"));
        }
    }
    Ok(())
}

fn setup_dupes(root: &Path) {
    fixture(
        root,
        &[
            ("a.jpg", "AAAA content one"),
            ("a copy.jpg", "AAAA content one"),
            ("b.jpg", "BBBB content two"),
            ("b (1).jpg", "BBBB content two"),
            ("c.jpg", "CCCC unique"),
        ],
    );
}

fn reference_dupes(root: &Path) -> Vec<FakeTurn> {
    vec![
        FakeTurn::text("Looking for duplicates.").call("find_duplicates", json!({"roots": [root]})),
        FakeTurn::text("Trashing the copies.")
            .call("trash", json!({"paths": [root.join("a copy.jpg"), root.join("b (1).jpg")]})),
        FakeTurn::text("Done."),
    ]
}

fn check_dupes(root: &Path) -> Result<(), String> {
    let mut by: HashMap<Vec<u8>, usize> = HashMap::new();
    for f in tree(root).iter().filter(|p| !p.ends_with('/')) {
        *by.entry(std::fs::read(root.join(f)).unwrap()).or_default() += 1;
    }
    if by.len() != 3 || by.values().any(|n| *n != 1) {
        return Err(format!("expected exactly one copy of 3 contents: {:?}", tree(root)));
    }
    Ok(())
}

fn tasks() -> Vec<EvalTask> {
    vec![
        EvalTask {
            name: "organize downloads",
            prompt: "Organize this downloads folder into a few folders by type. Keep every file.",
            setup: setup_downloads,
            reference: reference_downloads,
            check: check_downloads,
            needs_trash: false,
        },
        EvalTask {
            name: "rename photos by date",
            prompt: "Rename these photos so each name starts with its modification date as YYYY-MM-DD followed by a space and the old name.",
            setup: setup_photos,
            reference: reference_photos,
            check: check_photos,
            needs_trash: false,
        },
        EvalTask {
            name: "dedupe photos",
            prompt: "Find duplicate photos here and move the extra copies to the trash, keeping one of each.",
            setup: setup_dupes,
            reference: reference_dupes,
            check: check_dupes,
            needs_trash: true,
        },
    ]
}

fn context_for(root: &Path) -> String {
    let listing: Vec<EntrySummary> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| {
            let m = e.metadata().unwrap();
            EntrySummary {
                path: e.path(),
                name: e.file_name().to_string_lossy().into_owned(),
                dir: m.is_dir(),
                size: m.len(),
                modified: m.modified().ok(),
            }
        })
        .collect();
    let input = ContextInput { location: Some(root.to_path_buf()), listing, ..Default::default() };
    context::build(&input, &Permissions::new(&[], AiAccess::Metadata), 80, false)
}

/// The configured model for `SOYUZ_AI_EVAL_PROFILE`, if set.
fn real_model() -> Option<(String, Arc<dyn ChatModel>, soyuz_core::config::Ai)> {
    let profile = std::env::var("SOYUZ_AI_EVAL_PROFILE").ok().filter(|p| !p.is_empty())?;
    let cfg = soyuz_core::config::load_or_default(&soyuz_core::config::default_path()).expect("config").config;
    let mut ai = cfg.ai.clone();
    let p = ai.profile(&profile).unwrap_or_else(|| panic!("no profile “{profile}” in config.toml")).clone();
    ai.tasks.strong = soyuz_core::config::AiTaskModel { profile: p.name.clone(), model: p.model.clone() };
    let m = model::build(&ai, Route::Strong, &HashMap::new()).expect("build model");
    Some((profile, m, ai))
}

#[test]
#[ignore = "task eval: cargo test -p soyuz-app --test ai_eval -- --ignored"]
fn task_eval_suite() {
    let real = real_model();
    let label =
        real.as_ref().map(|(p, m, _)| format!("{p} / {}", m.model())).unwrap_or_else(|| "reference transcripts".into());
    let trash_ok = std::env::var("SOYUZ_TRASH_TESTS").is_ok_and(|v| v == "1");
    let mut report: BTreeMap<&str, (bool, String, usize, u64, Option<f64>)> = BTreeMap::new();
    for t in tasks() {
        if t.needs_trash && !trash_ok {
            println!("skip  {:<24} (uses the real trash; set SOYUZ_TRASH_TESTS=1)", t.name);
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let root: PathBuf = dir.path().join("fixture");
        (t.setup)(&root);
        let mut h = Harness::new(&root);
        h.limits.max_steps = 16;
        h.limits.wall_clock = Duration::from_secs(300);
        let model: Arc<dyn ChatModel> = match &real {
            Some((_, m, ai)) => {
                h.budget = Some(soyuz_app::ai::agent::Budget {
                    ledger_path: None,
                    prices: soyuz_ai::PriceTable::new(&ai.prices),
                    cap_usd: 0.0,
                    warn_percent: 80,
                });
                m.clone()
            }
            None => Arc::new(FakeModel::new((t.reference)(&root))),
        };
        let (_, out) = h.chat_with_context(model, t.prompt, context_for(&root), Policy::ApproveAll);
        let result = (t.check)(&root);
        let ok = result.is_ok() && out.finish == Some(soyuz_app::ai::agent::Finish::Done);
        let why = match (&result, &out.finish) {
            (Err(e), _) => e.clone(),
            (Ok(()), Some(f)) => f.describe(),
            (Ok(()), None) => "no finish".into(),
        };
        report.insert(t.name, (ok, why, out.model_turns, out.usage.total(), out.cost));
    }
    println!("\nAI task eval — {label}");
    println!("{:<24} {:<6} {:>6} {:>9} {:>9}  notes", "task", "ok", "turns", "tokens", "cost");
    let mut passed = 0;
    for (name, (ok, why, turns, tokens, cost)) in &report {
        passed += usize::from(*ok);
        println!(
            "{name:<24} {:<6} {turns:>6} {tokens:>9} {:>9}  {why}",
            if *ok { "pass" } else { "FAIL" },
            cost.map(|c| format!("${c:.4}")).unwrap_or_else(|| "-".into())
        );
    }
    println!("success rate: {passed}/{}", report.len());
    if real.is_none() {
        assert_eq!(passed, report.len(), "reference transcripts must solve every task");
    }
}
