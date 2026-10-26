//! Crash safety: an in-memory ring-buffer logger, a panic hook that writes
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

use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;
use crate::services::jobs::{JobMeta, OpKind};

/// Log lines kept for crash reports.
pub const RING_LINES: usize = 200;
/// Default `RUST_LOG`-style filter.
pub const DEFAULT_FILTER: &str = "info,wgpu=warn,naga=warn,zbus=warn,tracing=warn,smb2=warn";

// ---------------------------------------------------------------------------
// Ring-buffer logger
// ---------------------------------------------------------------------------

static RING: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Forwards to `env_logger` and keeps the last [`RING_LINES`] enabled lines in memory.
struct RingLogger {
    inner: env_logger::Logger,
    /// `inner` writes to a log file ([`init_file_logging`]), not to stderr.
    to_file: bool,
}

impl log::Log for RingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.inner.matches(record) {
            return;
        }
        let line = format!(
            "{} {:<5} {}: {}",
            chrono::Local::now().format("%H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        );
        push_line(line);
        // In the terminal UI stderr is the screen: keep the line in the ring only (unless
        // the log goes to a file).
        if self.to_file || !tui_active() {
            self.inner.log(record);
        }
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
    let inner = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_FILTER))
        .format_timestamp_millis()
        .build();
    let max = inner.filter();
    if log::set_boxed_logger(Box::new(RingLogger { inner, to_file: false })).is_ok() {
        log::set_max_level(max);
    }
}

/// Size at which [`init_file_logging`] starts a new log file (the previous one is kept
/// as `<name>.1`).
pub const LOG_FILE_CAP: u64 = 2 * 1024 * 1024;

/// Like [`init_logging`], but the log goes to `path` instead of stderr (the terminal UI,
/// where stderr is the screen). The file is capped at `cap` bytes: past it, it is renamed
/// to `<name>.1` (replacing an older one) and a new file starts. Falls back to
/// [`init_logging`] when the file cannot be opened. Idempotent.
pub fn init_file_logging(path: &Path, cap: u64) {
    let file = match RotatingFile::open(path.to_path_buf(), cap) {
        Ok(f) => f,
        Err(_) => return init_logging(),
    };
    let inner = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_FILTER))
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(Box::new(file)))
        .write_style(env_logger::WriteStyle::Never)
        .build();
    let max = inner.filter();
    if log::set_boxed_logger(Box::new(RingLogger { inner, to_file: true })).is_ok() {
        log::set_max_level(max);
    }
}

/// An append-only log file that rotates once it would grow past `cap` bytes.
pub struct RotatingFile {
    path: PathBuf,
    file: std::fs::File,
    len: u64,
    cap: u64,
}

impl RotatingFile {
    pub fn open(path: PathBuf, cap: u64) -> std::io::Result<RotatingFile> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(RotatingFile { path, file, len, cap: cap.max(1) })
    }

    /// `<name>.1`: the previous log.
    pub fn previous_path(&self) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(".1");
        self.path.with_file_name(name)
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        let old = self.previous_path();
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&self.path, &old)?;
        self.file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        self.len = 0;
        Ok(())
    }
}

impl std::io::Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.len > 0 && self.len + buf.len() as u64 > self.cap {
            // Keep logging into the full file when it cannot be rotated.
            let _ = self.rotate();
        }
        let n = self.file.write(buf)?;
        self.len += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
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
    let _ = writeln!(s, "Version:  soyuz {} ({})", env!("CARGO_PKG_VERSION"), crate::GIT_SHA);
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
///
/// In the terminal UI ([`set_tui_hooks`]) the hook routes by thread ([`panic_route`]):
/// a panic on the TUI thread restores the terminal first, so the report path and the
/// default message print to the normal screen; a panic on any other thread (a worker
/// that catches it) writes the report but prints nothing (stderr is the alternate
/// screen) and asks the loop for a full redraw ([`take_needs_full_redraw`]).
pub fn install_panic_hook() {
    let dir = crashes_dir();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let hooks = tui_hooks();
        let route = panic_route(hooks.map(|h| h.thread), std::thread::current().id());
        if route == PanicRoute::TuiThread
            && let Some(h) = hooks
        {
            (h.restore)();
            clear_tui_hooks();
        }
        let message = panic_message(info);
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_string();
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let text = report_text(&message, &location, &thread, &bt, &recent_log());
        let written = write_report(&dir, &text);
        if route.prints() {
            match written {
                Ok(p) => eprintln!("soyuz: crash report written to {}", p.display()),
                Err(e) => eprintln!("soyuz: cannot write crash report: {e}"),
            }
            previous(info);
        } else {
            match written {
                Ok(p) => push_line(format!("soyuz: crash report written to {}", p.display())),
                Err(e) => push_line(format!("soyuz: cannot write crash report: {e}")),
            }
            NEEDS_FULL_REDRAW.store(true, Ordering::Release);
        }
    }));
}

// ---------------------------------------------------------------------------
// Terminal UI panic routing
// ---------------------------------------------------------------------------

/// What the terminal UI registers with the panic hook.
#[derive(Clone, Copy, Debug)]
pub struct TuiHooks {
    /// Restores the terminal (raw mode, alternate screen, ...); idempotent, panic-safe.
    pub restore: fn(),
    /// The thread that owns the terminal (runs the frame loop).
    pub thread: std::thread::ThreadId,
}

static TUI_HOOKS: Mutex<Option<TuiHooks>> = Mutex::new(None);
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);
static NEEDS_FULL_REDRAW: AtomicBool = AtomicBool::new(false);

/// Enter terminal-UI panic handling (call after the terminal is set up, on the thread
/// that owns it). Also keeps log lines off stderr while it is active.
pub fn set_tui_hooks(restore: fn(), tui_thread: std::thread::ThreadId) {
    *TUI_HOOKS.lock() = Some(TuiHooks { restore, thread: tui_thread });
    TUI_ACTIVE.store(true, Ordering::Release);
}

/// Leave terminal-UI panic handling (after the terminal was restored): back to the GUI
/// behaviour.
pub fn clear_tui_hooks() {
    TUI_ACTIVE.store(false, Ordering::Release);
    if let Some(mut h) = TUI_HOOKS.try_lock_for(Duration::from_millis(50)) {
        *h = None;
    }
}

/// Terminal-UI panic handling is on.
pub fn tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::Acquire)
}

fn tui_hooks() -> Option<TuiHooks> {
    if !tui_active() {
        return None;
    }
    // Never block a panicking thread (another may hold the lock).
    TUI_HOOKS.try_lock_for(Duration::from_millis(50)).and_then(|h| *h)
}

/// A panic on another thread wrote past the alternate screen's owner (or was logged):
/// the terminal UI should repaint everything. Reading clears it.
pub fn take_needs_full_redraw() -> bool {
    NEEDS_FULL_REDRAW.swap(false, Ordering::AcqRel)
}

/// How the panic hook treats a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanicRoute {
    /// No terminal UI: report, print, default hook (the GUI behaviour).
    Normal,
    /// On the terminal UI thread: restore the terminal, then as [`PanicRoute::Normal`].
    TuiThread,
    /// On another thread while the terminal UI runs: report quietly, request a redraw.
    TuiBackground,
}

impl PanicRoute {
    /// Whether the hook prints to stderr (and runs the default hook).
    pub fn prints(self) -> bool {
        self != PanicRoute::TuiBackground
    }
}

/// Route a panic on `current` given the terminal UI thread (`None` outside the TUI).
pub fn panic_route(tui_thread: Option<std::thread::ThreadId>, current: std::thread::ThreadId) -> PanicRoute {
    match tui_thread {
        None => PanicRoute::Normal,
        Some(t) if t == current => PanicRoute::TuiThread,
        Some(_) => PanicRoute::TuiBackground,
    }
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
/// watching, thumbnails, program icons, git status, the preview pane and the custom
/// title bar off.
pub fn safe_config() -> Config {
    let mut c = Config::default();
    c.behaviour.restore_session = false;
    c.behaviour.auto_refresh = false;
    c.appearance.show_thumbnails = false;
    c.appearance.program_icons = false;
    c.git.status_column = false;
    c.layout.preview_pane = false;
    c.appearance.custom_titlebar = false;
    // No user code or background automation in safe mode.
    c.scripting.enabled = false;
    c.automation.enabled = false;
    c
}

/// Where the settings window saves while in safe mode (the real `config.toml` is never
/// overwritten with the safe-mode defaults).
pub fn safe_config_path() -> PathBuf {
    config::config_dir().join("safe-mode.toml")
}

/// Start another Soyuz in safe mode (plain process spawn, no shell), with the same
/// frontend as this process (`--tui` from the terminal UI).
pub fn restart_in_safe_mode() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let args = safe_mode_args(crate::core::process_frontend(), &exe);
    std::process::Command::new(exe).args(args).spawn().map(drop)
}

/// Arguments of the safe-mode restart for `frontend`, run by executable `exe`. The
/// terminal UI needs `--tui` from the `soyuz` binary, but not from `soyuz-tui`, which is
/// the terminal UI and rejects `--tui`.
pub fn safe_mode_args(frontend: crate::core::FrontendKind, exe: &Path) -> Vec<&'static str> {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match frontend {
        crate::core::FrontendKind::Tui if stem != "soyuz-tui" => vec!["--tui", "--safe-mode"],
        _ => vec!["--safe-mode"],
    }
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
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    {
        let mut pending = PENDING.lock();
        if let Some(p) = pending.as_mut().filter(|p| !p.shown) {
            p.shown = true;
            if safe_mode() {
                app.ui.toasts.push(
                    Toast::warning(tr!("services-crash-safe-mode")).body(tr!("services-crash-safe-mode-body")).sticky(),
                );
            }
            if !p.crashes.is_empty() {
                app.frontend.push(crate::models::frontend::FrontendCmd::CrashReports(p.crashes.clone()));
            }
            if !p.recovered.is_empty() {
                app.frontend.push(crate::models::frontend::FrontendCmd::RecoveredJobs(p.recovered.clone()));
            }
        }
    }
    let actions: Vec<RecoveryAction> = std::mem::take(&mut *ACTIONS.lock());
    for a in actions {
        apply(app, host, a);
    }
}

fn meta_for(job: &JournalJob, op: OpKind) -> JobMeta {
    // A resumed media job keeps its tool and output name ("Trimming “a.mp4” to “a.cut.mp4”").
    if let (OpKind::Media(_), Some(meta)) = (op, crate::models::media_tools::job_meta(&job.kind)) {
        return meta;
    }
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

fn cleanup(app: &mut AppCore, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let meta = JobMeta::new(OpKind::Trash, &paths, None);
    let spec = app.fileops.jobs.spec(JobKind::Delete { paths, mode: DeleteMode::Trash });
    app.fileops.jobs.submit(spec, meta);
}

fn apply(app: &mut AppCore, host: &dyn Host, a: RecoveryAction) {
    match a {
        RecoveryAction::Resume { jobs, cleanup: c, dismiss } => {
            cleanup(app, c);
            let mut started = 0;
            for job in &jobs {
                if let Some(id) = app.fileops.jobs.engine().resume_journaled(job) {
                    app.fileops.jobs.track(id, meta_for(job, OpKind::of_kind(&job.kind)));
                    started += 1;
                }
            }
            journal::dismiss(&dismiss);
            let msg = if started == 0 {
                tr!("services-crash-nothing-to-resume").to_string()
            } else {
                tr!("services-crash-resuming", count = started)
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
                tr!("services-crash-nothing-to-roll-back").to_string()
            } else {
                tr!("services-crash-rolling-back", count = started)
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
            app.ui.toasts.push(Toast::success(tr!("services-crash-deleted", count = n)));
        }
        RecoveryAction::OpenCrashFolder => {
            let dir = crashes_dir();
            let _ = soyuz_fs::create_dir_all(&dir);
            crate::ops::open_in_new_tab(app, soyuz_core::Location::Dir(dir), true);
        }
        RecoveryAction::RestartSafeMode => match restart_in_safe_mode() {
            Ok(()) => {
                app.quit.allowed = true;
                host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Close);
            }
            Err(e) => {
                app.ui.toasts.push(Toast::error(tr!("services-crash-restart-failed")).body(e.to_string()));
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_rotates_at_the_cap() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("soyuz-tui.log");
        let mut f = RotatingFile::open(path.clone(), 100).unwrap();
        f.write_all(&[b'a'; 60]).unwrap();
        f.write_all(&[b'b'; 30]).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 90);
        f.write_all(&[b'c'; 30]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), vec![b'c'; 30]);
        assert_eq!(std::fs::metadata(f.previous_path()).unwrap().len(), 90);
        assert!(f.previous_path().ends_with("soyuz-tui.log.1"));
        // Reopening appends.
        drop(f);
        let mut f = RotatingFile::open(path.clone(), 100).unwrap();
        f.write_all(b"d").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 31);
    }

    #[test]
    fn safe_mode_restart_keeps_the_frontend() {
        use crate::core::FrontendKind;
        let gui = Path::new("/opt/soyuz/soyuz");
        let gui_exe = Path::new("/opt/Soyuz/soyuz.exe");
        let tui = Path::new("/usr/bin/soyuz-tui");
        let tui_exe = Path::new("/opt/Soyuz/Soyuz-TUI.exe");
        for exe in [gui, gui_exe] {
            assert_eq!(safe_mode_args(FrontendKind::Gui, exe), vec!["--safe-mode"]);
            assert_eq!(safe_mode_args(FrontendKind::Tui, exe), vec!["--tui", "--safe-mode"]);
        }
        // `soyuz-tui` is the terminal UI already and rejects `--tui` (`soyuz-tui/src/launch.rs`).
        for exe in [tui, tui_exe] {
            assert_eq!(safe_mode_args(FrontendKind::Tui, exe), vec!["--safe-mode"]);
            assert_eq!(safe_mode_args(FrontendKind::Gui, exe), vec!["--safe-mode"]);
        }
    }

    #[test]
    fn resumed_media_jobs_keep_their_tool_in_the_title() {
        let kind = JobKind::Media {
            src: "/v/clip.mp4".into(),
            dest: "/v/clip.cut.mp4".into(),
            op: soyuz_media::MediaOp::Trim { start: 1.0, end: 2.0, stream_copy: true },
        };
        let job = JournalJob {
            id: 3,
            label: "Trim".into(),
            kind,
            options: soyuz_jobs::JobOptions::default(),
            started: SystemTime::UNIX_EPOCH,
            records: Vec::new(),
            stamps: Vec::new(),
            items_done: 0,
            items_total: 1,
            bytes_done: 0,
            bytes_total: 0,
        };
        let meta = meta_for(&job, OpKind::of_kind(&job.kind));
        assert_eq!(meta.op, OpKind::Media(crate::models::media_tools::MediaTool::Trim));
        let title = crate::ops::fileops::names::job_title(&meta, true, None, "Resume: Trim");
        assert_eq!(title, "Trimming \u{201c}clip.mp4\u{201d} to \u{201c}clip.cut.mp4\u{201d}");
        // A roll back stays an undo.
        assert_eq!(meta_for(&job, OpKind::Undo).op, OpKind::Undo);
    }

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
    fn panic_routing_by_thread() {
        let me = std::thread::current().id();
        let other = std::thread::spawn(|| std::thread::current().id()).join().unwrap();
        assert_eq!(panic_route(None, me), PanicRoute::Normal);
        assert_eq!(panic_route(Some(me), me), PanicRoute::TuiThread);
        assert_eq!(panic_route(Some(me), other), PanicRoute::TuiBackground);
        assert!(PanicRoute::Normal.prints() && PanicRoute::TuiThread.prints());
        assert!(!PanicRoute::TuiBackground.prints());
    }

    #[test]
    fn safe_config_turns_things_off() {
        let c = safe_config();
        assert!(!c.behaviour.restore_session && !c.behaviour.auto_refresh);
        assert!(!c.appearance.show_thumbnails && !c.appearance.program_icons && !c.git.status_column);
        assert_eq!(c.general, Config::default().general);
    }
}
