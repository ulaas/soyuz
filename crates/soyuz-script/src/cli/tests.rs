#[cfg(any(windows, unix))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(any(windows, unix))]
use std::sync::Arc;
#[cfg(any(windows, unix))]
use std::time::Duration;

use soyuz_jobs::JobKind;
#[cfg(any(windows, unix))]
use soyuz_jobs::{EngineConfig, JobEngine, JobId, JobSpec};

use super::client::{run_with, spec_for};
#[cfg(any(windows, unix))]
use super::proto::{self, Host};
use super::*;

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn cwd() -> PathBuf {
    std::env::temp_dir().join("cwd")
}

fn parse_ok(v: &[&str]) -> Command {
    parse(&args(v), &cwd()).unwrap().unwrap()
}

#[test]
fn parsing() {
    assert_eq!(parse(&args(&["C:\\x"]), &cwd()).unwrap(), None, "a path is not a command");
    assert_eq!(parse(&args(&[]), &cwd()).unwrap(), None);
    let c = parse_ok(&["cp", "a.txt", "../b", "dest", "--queue", "--json", "--conflict", "rename"]);
    assert_eq!(
        c.op,
        Op::Copy { sources: vec![cwd().join("a.txt"), std::env::temp_dir().join("b")], dest: cwd().join("dest") }
    );
    assert!(c.queue && c.json && !c.headless);
    assert_eq!(c.conflict, Some(Conflict::Rename));
    let c = parse_ok(&["mv", "--headless", "--conflict=skip", "a", "b"]);
    assert!(matches!(c.op, Op::Move { .. }) && c.headless);
    assert_eq!(c.conflict, Some(Conflict::Skip));
    assert_eq!(parse_ok(&["rm", "--permanent", "x"]).op, Op::Trash { paths: vec![cwd().join("x")], permanent: true });
    assert_eq!(
        parse_ok(&["trash", "x", "y"]).op,
        Op::Trash { paths: vec![cwd().join("x"), cwd().join("y")], permanent: false }
    );
    assert_eq!(parse_ok(&["ls"]).op, Op::Ls { path: cwd(), all: false });
    assert_eq!(parse_ok(&["ls", "-a", "sub"]).op, Op::Ls { path: cwd().join("sub"), all: true });
    assert_eq!(parse_ok(&["open", "."]).op, Op::Open(cwd()));
    assert_eq!(parse_ok(&["reveal", "--", "-x"]).op, Op::Reveal(cwd().join("-x")));
    assert_eq!(parse_ok(&["jobs", "--json"]).op, Op::Jobs);
    assert_eq!(parse_ok(&["cp", "--help"]).op, Op::Help);
    for bad in [
        &["cp", "a"][..],
        &["mv"],
        &["rm"],
        &["trash", "--permanent", "x"],
        &["ls", "a", "b"],
        &["open"],
        &["jobs", "x"],
        &["cp", "a", "b", "--bogus"],
        &["cp", "a", "b", "--conflict"],
        &["cp", "a", "b", "--conflict", "maybe"],
        &["rm", "x", "--conflict", "skip"],
        &["ls", "--permanent"],
    ] {
        assert!(parse(&args(bad), &cwd()).is_err(), "{bad:?}");
    }
}

#[test]
fn job_specs() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.txt");
    std::fs::write(&a, b"a").unwrap();
    let dir = d.path().join("dir");
    std::fs::create_dir(&dir).unwrap();
    let cmd = |op| Command { op, queue: true, headless: false, json: false, conflict: Some(Conflict::Overwrite) };
    let s = spec_for(&cmd(Op::Copy { sources: vec![a.clone()], dest: dir.clone() })).unwrap();
    assert!(matches!(&s.kind, JobKind::Copy { dest_dir, rename_to: None, .. } if dest_dir == &dir));
    assert_eq!(s.options.conflict, Some(soyuz_core::ConflictResolution::Overwrite));
    assert_eq!(s.options.queue, soyuz_jobs::QueueMode::Serial);
    assert!(s.options.journal, "undoable");
    // One source and a new name.
    let s = spec_for(&cmd(Op::Move { sources: vec![a.clone()], dest: dir.join("b.txt") })).unwrap();
    assert!(matches!(&s.kind, JobKind::Move { dest_dir, rename_to: Some(n), .. } if dest_dir == &dir && n == "b.txt"));
    // Several sources need an existing folder.
    let e = spec_for(&cmd(Op::Copy { sources: vec![a.clone(), a.clone()], dest: d.path().join("nope") })).unwrap_err();
    assert_eq!(e.0, exit::USAGE);
    let e = spec_for(&cmd(Op::Copy { sources: vec![d.path().join("missing")], dest: dir.clone() })).unwrap_err();
    assert_eq!(e.0, exit::NOT_FOUND);
    let e = spec_for(&cmd(Op::Copy { sources: vec![a.clone()], dest: d.path().join("x").join("y") })).unwrap_err();
    assert_eq!(e.0, exit::NOT_FOUND, "missing destination folder");
}

#[cfg(any(windows, unix))]
fn engine() -> Arc<JobEngine> {
    JobEngine::new(EngineConfig::default(), Arc::new(soyuz_fs::RealProbe), Arc::new(|| {}))
}

#[test]
fn headless_copy_and_json() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.txt");
    std::fs::write(&a, b"hello").unwrap();
    let dest = d.path().join("dest");
    std::fs::create_dir(&dest).unwrap();
    let cmd = Command {
        op: Op::Copy { sources: vec![a.clone()], dest: dest.clone() },
        queue: false,
        headless: true,
        json: false,
        conflict: None,
    };
    let mut out = Vec::new();
    assert_eq!(run_with(&cmd, None, None, &mut out, false), exit::OK);
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("Done: Copy “a.txt”"), "{text}");
    assert!(dest.join("a.txt").exists());
    // Again: the conflict is skipped headless (never overwrites), still exit 0.
    let mut out = Vec::new();
    let json = Command { json: true, ..cmd.clone() };
    assert_eq!(run_with(&json, None, None, &mut out, false), exit::OK);
    let text = String::from_utf8_lossy(&out);
    let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["event"], "finished");
    assert_eq!(last["job"]["summary"]["items_skipped"], 1, "{text}");
    // ls lists it.
    let mut out = Vec::new();
    let ls = Command { op: Op::Ls { path: dest.clone(), all: false }, ..cmd.clone() };
    assert_eq!(run_with(&ls, None, None, &mut out, false), exit::OK);
    assert!(String::from_utf8_lossy(&out).contains("a.txt"));
    let missing = Command { op: Op::Ls { path: d.path().join("nope"), all: false }, ..cmd };
    assert_eq!(run_with(&missing, None, None, &mut Vec::new(), false), exit::NOT_FOUND);
}

/// A stand-in for the window: submits straight to an engine, records open requests.
#[cfg(any(windows, unix))]
struct TestHost {
    engine: Arc<JobEngine>,
    opened: parking_lot::Mutex<Vec<(PathBuf, bool)>>,
}

#[cfg(any(windows, unix))]
impl Host for TestHost {
    fn engine(&self) -> Arc<JobEngine> {
        self.engine.clone()
    }

    fn submit(&self, mut spec: JobSpec) -> Result<JobId, String> {
        spec.options.label = Some("CLI job".into());
        Ok(self.engine.submit(spec))
    }

    fn open(&self, path: &Path, reveal: bool) -> Result<(), String> {
        self.opened.lock().push((path.to_path_buf(), reveal));
        Ok(())
    }
}

#[cfg(any(windows, unix))]
#[test]
fn pipe_round_trip() {
    #[cfg(windows)]
    let name = format!(r"\\.\pipe\soyuz-cli-test-{}-{}", std::process::id(), line!());
    // A temp directory instead of the user's runtime dir (`serve` creates `s/` as 0700).
    #[cfg(unix)]
    let sock_dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    let name = sock_dir.path().join("s/soyuz-cli-test.sock").to_string_lossy().into_owned();
    let host = Arc::new(TestHost { engine: engine(), opened: Default::default() });
    let h = host.clone();
    let _server = soyuz_mcp::pipe::PipeServer::serve(&name, move |s| proto::serve_connection(h.as_ref(), s)).unwrap();
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a.bin");
    std::fs::write(&a, vec![7u8; 300_000]).unwrap();
    let dest = d.path().join("dest");
    std::fs::create_dir(&dest).unwrap();
    let cmd = Command {
        op: Op::Move { sources: vec![a.clone()], dest: dest.clone() },
        queue: false,
        headless: false,
        json: true,
        conflict: None,
    };
    let mut out = Vec::new();
    assert_eq!(run_with(&cmd, Some(&name), None, &mut out, false), exit::OK);
    let text = String::from_utf8_lossy(&out).into_owned();
    let events: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(events.first().unwrap()["event"], "submitted", "{text}");
    assert_eq!(events.last().unwrap()["event"], "finished");
    assert_eq!(events.last().unwrap()["job"]["label"], "CLI job", "ran in the window's engine");
    assert!(dest.join("a.bin").exists() && !a.exists());
    // --queue returns after the submission.
    std::fs::write(&a, b"again").unwrap();
    let q =
        Command { op: Op::Copy { sources: vec![a.clone()], dest: d.path().join("c.bin") }, queue: true, ..cmd.clone() };
    let mut out = Vec::new();
    assert_eq!(run_with(&q, Some(&name), None, &mut out, false), exit::OK);
    let text = String::from_utf8_lossy(&out).into_owned();
    assert_eq!(text.lines().count(), 1, "{text}");
    // open / reveal / jobs go to the window too.
    let o = Command { op: Op::Reveal(a.clone()), ..cmd.clone() };
    assert_eq!(run_with(&o, Some(&name), None, &mut Vec::new(), false), exit::OK);
    assert_eq!(host.opened.lock().as_slice(), &[(a.clone(), true)]);
    let j = Command { op: Op::Jobs, json: false, ..cmd.clone() };
    let mut out = Vec::new();
    assert_eq!(run_with(&j, Some(&name), None, &mut out, false), exit::OK);
    assert!(String::from_utf8_lossy(&out).contains("CLI job"), "history lists the jobs");
    // No server under this name: falls back to headless.
    let other = format!("{name}-none");
    std::fs::write(&a, b"third").unwrap();
    let c = Command {
        op: Op::Copy { sources: vec![a.clone()], dest: d.path().join("d.bin") },
        json: false,
        queue: false,
        ..cmd
    };
    let mut out = Vec::new();
    assert_eq!(run_with(&c, Some(&other), None, &mut out, false), exit::OK);
    assert!(d.path().join("d.bin").exists());
    host.engine.shutdown(Duration::from_secs(2));
}
