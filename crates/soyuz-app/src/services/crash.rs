//! Crash safety (ROADMAP R3): an in-memory ring-buffer logger, a panic hook that writes
//! crash reports to `<config>/crashes/`, the jobs crash-journal slot, the startup recovery
//! offer (interrupted operations, unseen crash reports) and `--safe-mode`.
//!
//! Crash reports stay on this computer: they are plain text files (version, OS, thread,
//! panic message, backtrace, the last log lines) and are never sent anywhere.
//!
//! Startup order (`main.rs`): [`init_logging`] → [`install_panic_hook`] → [`startup`]
//! (claims the journal slot before the job engine exists, collects what to offer) → the
//! app calls [`pump`] every frame, which shows the dialogs once and carries out the
//! user's choices.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use soyuz_core::config::{self, Config};
use soyuz_jobs::journal::{self, JournalSlot, Recovered};
use soyuz_jobs::{DeleteMode, JobKind, JournalJob};

use crate::app::SoyuzApp;
use crate::services::jobs::{JobMeta, OpKind};
use crate::ui::toasts::Toast;

/// Log lines kept for crash reports.
pub const RING_LINES: usize = 200;
/// Default `RUST_LOG`-style filter.
pub const DEFAULT_FILTER: &str = "info,wgpu=warn,naga=warn";

// ---------------------------------------------------------------------------
// Ring-buffer logger
// ---------------------------------------------------------------------------

static RING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Forwards to `env_logger` and keeps the last [`RING_LINES`] enabled lines in memory.
struct RingLogger {
    inner: env_logger::Logger,
}

impl log::Log for RingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.inner.matches(record) {
            return;
        }
        push_line(format!(
            "{} {:<5} {}: {}",
            chrono::Local::now().format("%H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        ));
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

fn push_line(line: String) {
    // Never block a panicking thread on the ring (another thread may hold it).
    let Some(mut ring) = RING.try_lock_for(Duration::from_millis(50)) else { return };
    if ring.len() >= RING_LINES {
        ring.pop_front();
    }
    ring.push_back(line);
}

/// Install the logger (`RUST_LOG` overrides [`DEFAULT_FILTER`]). Idempotent.
pub fn init_logging() {
    let inner = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_FILTER)).build();
    let max = inner.filter();
    if log::set_boxed_logger(Box::new(RingLogger { inner })).is_ok() {
        log::set_max_level(max);
    }
}

/// The last log lines, oldest first.
pub fn recent_log() -> Vec<String> {
    RING.try_lock_for(Duration::from_millis(50)).map(|r| r.iter().cloned().collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Crash reports
// ---------------------------------------------------------------------------

/// `<config>/crashes`.
pub fn crashes_dir() -> PathBuf {
    config::config_dir().join("crashes")
}

const SEEN_FILE: &str = ".seen";
static REPORT_SEQ: AtomicU32 = AtomicU32::new(0);

/// Text of a crash report.
pub fn report_text(message: &str, location: &str, thread: &str, backtrace: &str, log: &[String]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Soyuz crash report");
    let _ = writeln!(s, "==================");
    let _ = writeln!(s, "This file was written on this computer and is never sent anywhere.");
    let _ = writeln!(s);
    let _ = writeln!(s, "Version:  soyuz {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "OS:       {} {} ({})", std::env::consts::OS, std::env::consts::ARCH, std::env::consts::FAMILY);
    let _ = writeln!(s, "Time:     {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z"));
    let _ = writeln!(s, "Thread:   {thread}");
    let _ = writeln!(s, "Location: {location}");
    let _ = writeln!(s, "Panic:    {message}");
    let _ = writeln!(s, "Safe mode: {}", safe_mode());
    let _ = writeln!(s);
    let _ = writeln!(s, "Backtrace");
    let _ = writeln!(s, "---------");
    let _ = writeln!(s, "{backtrace}");
    let _ = writeln!(s);
    let _ = writeln!(s, "Last {} log lines", log.len());
    let _ = writeln!(s, "-----------------");
    for l in log {
        let _ = writeln!(s, "{l}");
    }
    s
}

/// Write a report into `dir` (created when missing); returns its path.
pub fn write_report(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let name = format!(
        "crash-{}-{}-{}.txt",
        chrono::Local::now().format("%Y%m%d-%H%M%S"),
        std::process::id(),
        REPORT_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let path = dir.join(name);
    std::fs::write(&path, text)?;
    Ok(path)
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    info.payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".into())
}

/// Chain a hook that writes a crash report before the default hook runs. Panics caught
/// by a job worker (reported as a failed job) also leave a report: they are bugs too.
pub fn install_panic_hook() {
    let dir = crashes_dir();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_string();
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let text = report_text(&message, &location, &thread, &bt, &recent_log());
        match write_report(&dir, &text) {
            Ok(p) => eprintln!("soyuz: crash report written to {}", p.display()),
            Err(e) => eprintln!("soyuz: cannot write crash report: {e}"),
        }
        previous(info);
    }));
}

/// A crash report on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashReport {
    pub path: PathBuf,
    pub name: String,
    pub when: Option<SystemTime>,
    /// The `Panic:` line.
    pub summary: String,
}

/// Reports in `dir`, newest first; `unseen_only` skips those already offered.
pub fn list_reports(dir: &Path, unseen_only: bool) -> Vec<CrashReport> {
    let seen: Vec<String> = if unseen_only {
        std::fs::read_to_string(dir.join(SEEN_FILE)).unwrap_or_default().lines().map(str::to_string).collect()
    } else {
        Vec::new()
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<CrashReport> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("crash-") && name.ends_with(".txt")) || seen.contains(&name) {
                return None;
            }
            let path = e.path();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let summary = text
                .lines()
                .find_map(|l| l.strip_prefix("Panic:"))
                .map(|s| s.trim().chars().take(200).collect())
                .unwrap_or_default();
            Some(CrashReport { when: e.metadata().ok().and_then(|m| m.modified().ok()), path, name, summary })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Remember that these reports were offered (they stay on disk).
pub fn mark_seen(dir: &Path, reports: &[CrashReport]) {
    let path = dir.join(SEEN_FILE);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    // Forget names whose file is gone.
    let mut lines: Vec<String> = text.lines().filter(|n| dir.join(n).exists()).map(str::to_string).collect();
    for r in reports {
        if !lines.contains(&r.name) {
            lines.push(r.name.clone());
        }
    }
    text = lines.join("\n");
    text.push('\n');
    let _ = std::fs::write(path, text);
}

/// Delete reports; returns how many were removed.
pub fn delete_reports(reports: &[CrashReport]) -> usize {
    reports.iter().filter(|r| std::fs::remove_file(&r.path).is_ok()).count()
}

// ---------------------------------------------------------------------------
// Safe mode
// ---------------------------------------------------------------------------

static SAFE_MODE: AtomicBool = AtomicBool::new(false);

/// Running with `--safe-mode`.
pub fn safe_mode() -> bool {
    SAFE_MODE.load(Ordering::Relaxed)
}

/// The configuration used in safe mode: the defaults, with session restore, folder
/// watching, thumbnails, git status, the preview pane and the custom title bar off.
pub fn safe_config() -> Config {
    let mut c = Config::default();
    c.behaviour.restore_session = false;
    c.behaviour.auto_refresh = false;
    c.appearance.show_thumbnails = false;
    c.git.status_column = false;
    c.layout.preview_pane = false;
    c.appearance.custom_titlebar = false;
    // No user code or background automation in safe mode (F6).
    c.scripting.enabled = false;
    c.automation.enabled = false;
    c
}

/// Where the settings window saves while in safe mode (the real `config.toml` is never
/// overwritten with the safe-mode defaults).
pub fn safe_config_path() -> PathBuf {
    config::config_dir().join("safe-mode.toml")
}

/// Start another Soyuz in safe mode (plain process spawn, no shell).
pub fn restart_in_safe_mode() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).arg("--safe-mode").spawn().map(drop)
}

// ---------------------------------------------------------------------------
// Startup recovery state
// ---------------------------------------------------------------------------

static SLOT: OnceLock<JournalSlot> = OnceLock::new();

struct Pending {
    recovered: Vec<Recovered>,
    crashes: Vec<CrashReport>,
    shown: bool,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// What a recovery / crash dialog asked for (carried out by [`pump`] on the UI thread).
#[derive(Debug)]
pub enum RecoveryAction {
    Resume { jobs: Vec<JournalJob>, cleanup: Vec<PathBuf>, dismiss: Vec<Recovered> },
    RollBack { jobs: Vec<JournalJob>, cleanup: Vec<PathBuf>, dismiss: Vec<Recovered> },
    Discard { cleanup: Vec<PathBuf>, dismiss: Vec<Recovered> },
    KeepReports(Vec<CrashReport>),
    DeleteReports(Vec<CrashReport>),
    OpenCrashFolder,
    RestartSafeMode,
}

static ACTIONS: Mutex<Vec<RecoveryAction>> = Mutex::new(Vec::new());

/// Queue an action from a dialog.
pub fn request(a: RecoveryAction) {
    ACTIONS.lock().push(a);
}

/// Call once in `main` before the app (and its job engine) is created: claims this
/// process's crash-journal slot and collects interrupted operations and unseen crash
/// reports to offer on the first frame.
pub fn startup(safe: bool) {
    SAFE_MODE.store(safe, Ordering::Relaxed);
    let dir = config::config_dir();
    match journal::claim_slot(&dir) {
        Ok(slot) => {
            let _ = SLOT.set(slot);
        }
        Err(e) => log::warn!("crash journal disabled: {e}"),
    }
    let recovered = journal::list_recovered(&dir);
    let crashes = list_reports(&crashes_dir(), true);
    if !recovered.is_empty() {
        let n: usize = recovered.iter().map(|r| r.jobs.len()).sum();
        log::warn!("Soyuz closed during {n} operation(s); offering recovery");
    }
    *PENDING.lock() = Some(Pending { recovered, crashes, shown: false });
}

/// The journal path for the job engine (`None` in tests / when no slot was claimed).
pub fn journal_path() -> Option<PathBuf> {
    SLOT.get().map(|s| s.path().to_path_buf())
}

/// Per frame: show the startup dialogs once, then carry out queued choices.
pub fn pump(app: &mut SoyuzApp, ctx: &egui::Context) {
    {
        let mut pending = PENDING.lock();
        if let Some(p) = pending.as_mut().filter(|p| !p.shown) {
            p.shown = true;
            if safe_mode() {
                app.ui.toasts.push(
                    Toast::warning("Safe mode")
                        .body(
                            "Default settings, no session restore, no folder watching, thumbnails or git status. \
                             Restart Soyuz normally to leave safe mode.",
                        )
                        .sticky(),
                );
            }
            if !p.crashes.is_empty() {
                app.ui.dialogs.push(crate::ui::dialogs::recovery_crash::CrashDialog::new(p.crashes.clone()));
            }
            if !p.recovered.is_empty() {
                app.ui.dialogs.push(crate::ui::dialogs::recovery::RecoveryDialog::new(ctx, p.recovered.clone()));
            }
        }
    }
    let actions: Vec<RecoveryAction> = std::mem::take(&mut *ACTIONS.lock());
    for a in actions {
        apply(app, ctx, a);
    }
}

fn meta_for(job: &JournalJob, op: OpKind) -> JobMeta {
    let roots: Vec<PathBuf> = match &job.kind {
        JobKind::Copy { sources, .. } | JobKind::Move { sources, .. } | JobKind::Compress { sources, .. } => {
            sources.clone()
        }
        k => k.touched_roots(),
    };
    let dest = match &job.kind {
        JobKind::Copy { dest_dir, .. } | JobKind::Move { dest_dir, .. } | JobKind::Extract { dest_dir, .. } => {
            Some(dest_dir.clone())
        }
        _ => None,
    };
    JobMeta::new(op, &roots, dest)
}

fn cleanup(app: &mut SoyuzApp, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let meta = JobMeta::new(OpKind::Trash, &paths, None);
    let spec = app.fileops.jobs.spec(JobKind::Delete { paths, mode: DeleteMode::Trash });
    app.fileops.jobs.submit(spec, meta);
}

fn apply(app: &mut SoyuzApp, ctx: &egui::Context, a: RecoveryAction) {
    match a {
        RecoveryAction::Resume { jobs, cleanup: c, dismiss } => {
            cleanup(app, c);
            let mut started = 0;
            for job in &jobs {
                if let Some(id) = app.fileops.jobs.engine().resume_journaled(job) {
                    app.fileops.jobs.track(id, meta_for(job, OpKind::from_brief(job.kind.brief())));
                    started += 1;
                }
            }
            journal::dismiss(&dismiss);
            let msg = if started == 0 {
                "Nothing was left to resume".to_string()
            } else {
                format!("Resuming {started} operation{}", if started == 1 { "" } else { "s" })
            };
            app.ui.toasts.push(Toast::info(msg));
        }
        RecoveryAction::RollBack { jobs, cleanup: c, dismiss } => {
            cleanup(app, c);
            let mut started = 0;
            for job in &jobs {
                if let Some(id) = app.fileops.jobs.engine().rollback_journaled(job) {
                    app.fileops.jobs.track(id, meta_for(job, OpKind::Undo));
                    started += 1;
                }
            }
            journal::dismiss(&dismiss);
            let msg = if started == 0 {
                "Nothing had been completed, so there is nothing to roll back".to_string()
            } else {
                format!("Rolling back {started} operation{}", if started == 1 { "" } else { "s" })
            };
            app.ui.toasts.push(Toast::info(msg));
        }
        RecoveryAction::Discard { cleanup: c, dismiss } => {
            cleanup(app, c);
            journal::dismiss(&dismiss);
        }
        RecoveryAction::KeepReports(r) => mark_seen(&crashes_dir(), &r),
        RecoveryAction::DeleteReports(r) => {
            let n = delete_reports(&r);
            mark_seen(&crashes_dir(), &[]);
            app.ui.toasts.push(Toast::success(format!("Deleted {n} crash report{}", if n == 1 { "" } else { "s" })));
        }
        RecoveryAction::OpenCrashFolder => {
            let dir = crashes_dir();
            let _ = soyuz_fs::create_dir_all(&dir);
            crate::ops::open_in_new_tab(app, soyuz_core::Location::Dir(dir), true);
        }
        RecoveryAction::RestartSafeMode => match restart_in_safe_mode() {
            Ok(()) => {
                app.quit.allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => {
                app.ui.toasts.push(Toast::error("Couldn't restart in safe mode").body(e.to_string()));
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_the_last_lines() {
        for i in 0..(RING_LINES + 25) {
            push_line(format!("ring-test line {i}"));
        }
        let lines = recent_log();
        assert!(lines.len() <= RING_LINES);
        assert_eq!(lines.last().unwrap(), &format!("ring-test line {}", RING_LINES + 24));
    }

    #[test]
    fn reports_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let text = report_text("boom", "src/x.rs:1:2", "main", "   0: frame", &["a".into(), "b".into()]);
        assert!(text.contains("never sent anywhere") && text.contains("Panic:    boom") && text.contains("soyuz "));
        assert!(text.contains("Last 2 log lines"));
        let p1 = write_report(d.path(), &text).unwrap();
        let p2 = write_report(d.path(), &text).unwrap();
        assert_ne!(p1, p2);
        let all = list_reports(d.path(), true);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].summary, "boom");
        mark_seen(d.path(), &all[..1]);
        assert_eq!(list_reports(d.path(), true).len(), 1);
        assert_eq!(list_reports(d.path(), false).len(), 2);
        assert_eq!(delete_reports(&all), 2);
        assert!(list_reports(d.path(), false).is_empty());
        mark_seen(d.path(), &[]);
        assert_eq!(std::fs::read_to_string(d.path().join(SEEN_FILE)).unwrap().trim(), "");
    }

    #[test]
    fn panic_hook_writes_a_report() {
        // Exercise the hook body without installing it globally (tests run in parallel).
        let d = tempfile::tempdir().unwrap();
        let r = std::panic::catch_unwind(|| panic!("deliberate test panic"));
        let msg = r.unwrap_err().downcast_ref::<&str>().map(|s| s.to_string()).unwrap();
        let p = write_report(d.path(), &report_text(&msg, "here", "t", "", &recent_log())).unwrap();
        assert!(std::fs::read_to_string(p).unwrap().contains("deliberate test panic"));
    }

    #[test]
    fn safe_config_turns_things_off() {
        let c = safe_config();
        assert!(!c.behaviour.restore_session && !c.behaviour.auto_refresh);
        assert!(!c.appearance.show_thumbnails && !c.git.status_column);
        assert_eq!(c.general, Config::default().general);
    }
}
