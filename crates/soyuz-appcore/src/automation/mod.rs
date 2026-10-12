//! Scripts, automation rules and the CLI companion inside the window.
//!
//! * Scripts (`scripts/*.rhai`, [`scripts`]): hot-reloaded on a thread; actions run on
//!   a short-lived thread and come back as planned jobs (approval dialog when
//!   `[scripting] approve_writes`) plus UI operations; columns and previewers are
//!   served from worker threads.
//! * Rules (`[[automation.rules]]`): [`soyuz_script::rules::worker`] watches the folders
//!   and runs the steps as jobs on the app's engine; this module tags those jobs in the
//!   activity panel, applies color tags and keeps the per-rule run log. Only one window
//!   runs them (a per-user pipe name is the lock); secondary windows never do.
//! * CLI ([`cli_server`]): `soyuz cp|mv|…` jobs submitted over the pipe.

pub mod cli_server;
pub mod menu;
pub mod scripts;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::Location;
use soyuz_core::config::{Action, Automation, AutomationRule, KeyContext, Scripting};
use soyuz_jobs::{JobId, JobKind};
use soyuz_mcp::pipe::PipeServer;
use soyuz_script::rules::PreviewItem;
use soyuz_script::rules::worker::{self, Cmd as WorkerCmd, Event, LogEntry};
use soyuz_script::{ActionCtx, PlannedJob, ScriptSet, UiOp};

use crate::core::AppCore;
use crate::host::Host;
use crate::icon::IconId;
use crate::models::toasts::Toast;
use crate::ops::{self, NavOpts};
use crate::services::jobs::{JobMeta, JobOrigin, OpKind};
use crate::services::mailbox::Wake;

/// Commands from dialogs / settings pages.
#[derive(Clone, Debug)]
pub enum AutoCmd {
    /// Submit a script action's planned jobs (after approval).
    SubmitPlanned {
        title: String,
        jobs: Vec<PlannedJob>,
    },
    /// Act on everything the rule (by name) matches right now.
    RunRule(String),
    ReloadScripts,
    /// Show the scripts folder in the focused pane (created, with an example, if missing).
    OpenScriptsFolder,
    /// Write a new script from a template and show it.
    NewScript(&'static str),
    ClearLog(String),
}

/// A UI step of a script, resolved off the UI thread.
enum UiStep {
    Go { dir: PathBuf, select: Option<String> },
    Select { dir: PathBuf, names: Vec<String> },
    Toast(String),
    CopyText(String),
    Tag(Vec<PathBuf>, String),
}

struct ActionDone {
    /// A script action finished (`false`: UI steps from another worker).
    counted: bool,
    title: String,
    result: Result<(Vec<PlannedJob>, Vec<UiStep>), soyuz_script::ScriptError>,
}

/// Live preview of a rule on the Automations page.
#[derive(Clone, Debug)]
pub struct RulePreview {
    pub rule: AutomationRule,
    pub at: Instant,
    pub loading: bool,
    pub result: Option<Result<(Vec<PreviewItem>, usize), String>>,
}

/// A finished rule preview: `(rule index, rule as previewed, matches)`.
type PreviewDone = (usize, AutomationRule, Result<(Vec<PreviewItem>, usize), String>);

pub struct AutomationState {
    started: bool,
    wake: Option<Wake>,
    loader: Option<scripts::Loader>,
    scripting_sent: Option<Scripting>,
    /// The loaded scripts (also in [`scripts::current`]).
    pub set: Option<Arc<ScriptSet>>,
    /// Runtime errors of script actions by script file (shown on the Scripts page).
    pub runtime_errors: HashMap<String, String>,
    warned_hotkeys: HashSet<String>,
    action_tx: Sender<ActionDone>,
    action_rx: Receiver<ActionDone>,
    /// Script actions currently running.
    pub running: usize,
    // --- rules ---
    worker: Option<worker::Handle>,
    lock: Option<PipeServer>,
    /// Why this window does not run the rules (another window does / disabled).
    pub rules_note: Option<String>,
    sent: Option<Automation>,
    pending: Option<(Automation, Instant)>,
    /// Per rule (by name): run log, newest last.
    pub logs: HashMap<String, VecDeque<LogEntry>>,
    /// Per rule: watching or the reason why not.
    pub status: HashMap<String, Result<(), String>>,
    pub previews: HashMap<usize, RulePreview>,
    preview_tx: Sender<PreviewDone>,
    preview_rx: Receiver<PreviewDone>,
    // --- CLI ---
    cli: Option<cli_server::CliServer>,
    cli_failed: bool,
    /// `Ok(pipe name)` / `Err(why not)`; `None` = off.
    pub cli_status: Option<Result<String, String>>,
}

impl Default for AutomationState {
    fn default() -> Self {
        let (action_tx, action_rx) = unbounded();
        let (preview_tx, preview_rx) = unbounded();
        AutomationState {
            started: false,
            wake: None,
            loader: None,
            scripting_sent: None,
            set: None,
            runtime_errors: HashMap::new(),
            warned_hotkeys: HashSet::new(),
            action_tx,
            action_rx,
            running: 0,
            worker: None,
            lock: None,
            rules_note: None,
            sent: None,
            pending: None,
            logs: HashMap::new(),
            status: HashMap::new(),
            previews: HashMap::new(),
            preview_tx,
            preview_rx,
            cli: None,
            cli_failed: false,
            cli_status: None,
        }
    }
}

/// How long the rules config must stay unchanged before the worker is reconfigured
/// (editing a rule in Settings changes the config on every keystroke).
const RULES_DEBOUNCE: Duration = Duration::from_millis(800);

/// Icon of a job's origin (activity panel).
pub fn origin_icon(o: JobOrigin) -> IconId {
    match o {
        JobOrigin::Automation => IconId::Robot,
        JobOrigin::Script => IconId::Code,
        JobOrigin::Cli => IconId::TerminalWindow,
    }
}

pub fn origin_label(o: JobOrigin) -> &'static str {
    match o {
        JobOrigin::Automation => tr!("automation-origin-rule"),
        JobOrigin::Script => tr!("automation-origin-script"),
        JobOrigin::Cli => tr!("automation-origin-cli"),
    }
}

/// Palette / menu title of a script action.
pub fn action_title(id: &str) -> String {
    scripts::current()
        .and_then(|s| s.action(id).map(|(_, a)| a.title.clone()))
        .unwrap_or_else(|| tr!("automation-script-fallback-title", id = id))
}

/// Per frame.
pub fn pump(app: &mut AppCore, host: &dyn Host) {
    let st = &mut app.automation;
    if !st.started {
        st.started = true;
        let wake = app.wake.clone();
        scripts::set_wake(wake.clone());
        st.wake = Some(wake.clone());
        st.loader = Some(scripts::Loader::start(soyuz_script::scripts_dir(), app.config.scripting.clone(), wake));
        st.scripting_sent = Some(app.config.scripting.clone());
    }
    if st.scripting_sent.as_ref() != Some(&app.config.scripting) {
        st.scripting_sent = Some(app.config.scripting.clone());
        if let Some(l) = &st.loader {
            l.configure(app.config.scripting.clone());
        }
    }
    if let Some(l) = &st.loader {
        let fresh: Vec<Arc<ScriptSet>> = l.rx.try_iter().collect();
        if let Some(set) = fresh.into_iter().last() {
            scripts::set_current(set.clone());
            st.runtime_errors.clear();
            if let Some(w) = &st.worker {
                w.send(WorkerCmd::Scripts(set.clone()));
            }
            st.set = Some(set);
            // Script columns may have new titles / widths.
            for t in &mut app.ws.tabs {
                for p in &mut t.panes {
                    p.layout_sig = None;
                }
            }
        }
    }
    bind_hotkeys(app);
    let done: Vec<ActionDone> = app.automation.action_rx.try_iter().collect();
    for d in done {
        if d.counted {
            app.automation.running = app.automation.running.saturating_sub(1);
        }
        finish_action(app, d);
    }
    for (idx, rule, r) in app.automation.preview_rx.try_iter().collect::<Vec<_>>() {
        if let Some(p) = app.automation.previews.get_mut(&idx)
            && p.rule == rule
        {
            p.loading = false;
            p.result = Some(r);
            p.at = Instant::now();
        }
    }
    if app.secondary {
        app.automation.rules_note = Some(tr!("automation-secondary-note").into());
        return;
    }
    pump_rules(app, host);
    pump_cli(app, host);
}

/// Script hotkeys (`hotkey` in `register_action`) unless `[keys.bindings]` names the
/// action. Re-applied after the key map is rebuilt (config reload).
fn bind_hotkeys(app: &mut AppCore) {
    let Some(set) = app.automation.set.clone() else { return };
    let cmd_is_ctrl = app.config.keys.cmd_is_ctrl;
    for (id, _, a) in set.actions() {
        if a.hotkey.trim().is_empty() {
            continue;
        }
        let action = Action::RunScript(id.clone());
        if app.config.keys.bindings.contains_key(&action.to_string()) || !app.keymap.sequences_for(&action).is_empty() {
            continue;
        }
        match soyuz_core::config::parse_sequence(&a.hotkey) {
            Ok(seq) if seq.len() == 1 => {
                app.keymap.bind_stealing(KeyContext::Global, action, seq[0].normalized(cmd_is_ctrl))
            }
            Ok(seq) if !seq.is_empty() => app.keymap.bind_sequence(
                KeyContext::Global,
                action,
                seq.into_iter().map(|c| c.normalized(cmd_is_ctrl)).collect(),
            ),
            Ok(_) => {}
            Err(e) => {
                if app.automation.warned_hotkeys.insert(id.clone()) {
                    log::warn!("script action {id}: hotkey “{}”: {e}", a.hotkey);
                }
            }
        }
    }
}

/// Run the script action `id` against the focused pane (off the UI thread).
pub fn run_action(app: &mut AppCore, id: &str) {
    let Some(set) = scripts::current() else {
        app.ui.toasts.push(Toast::warning(tr!("automation-scripts-still-loading")));
        return;
    };
    let Some((_, a)) = set.action(id) else {
        app.ui.toasts.push(Toast::warning(tr!("automation-no-action", id = id)).body(tr!("automation-no-action-body")));
        return;
    };
    let title = a.title.clone();
    let pc = crate::commands::PlaceholderCtx::from_app(app);
    let actx = ActionCtx { dir: pc.dir, focused: pc.focused, selected: pc.selected, other_dir: pc.other_dir };
    let tx = app.automation.action_tx.clone();
    let wake = app.automation.wake.clone();
    let id = id.to_string();
    app.automation.running += 1;
    let spawned = std::thread::Builder::new().name("soyuz-script-action".into()).spawn(move || {
        let result = set.run_action(&id, &actx).map(|out| (out.jobs, resolve_ui(out.ui)));
        let _ = tx.send(ActionDone { counted: true, title, result });
        if let Some(w) = wake {
            w();
        }
    });
    if let Err(e) = spawned {
        app.automation.running -= 1;
        app.ui.toasts.push(Toast::error(tr!("automation-start-failed")).body(e.to_string()));
    }
}

/// Resolve UI operations on the script thread (folder or file?).
fn resolve_ui(ops: Vec<UiOp>) -> Vec<UiStep> {
    let split = |p: &std::path::Path| -> Option<(PathBuf, String)> {
        Some((p.parent()?.to_path_buf(), p.file_name()?.to_string_lossy().into_owned()))
    };
    ops.into_iter()
        .filter_map(|op| match op {
            UiOp::Navigate(p) => {
                if soyuz_fs::stat(&p, true).is_ok_and(|e| e.is_dir_like()) {
                    Some(UiStep::Go { dir: p, select: None })
                } else {
                    split(&p).map(|(dir, name)| UiStep::Go { dir, select: Some(name) })
                }
            }
            UiOp::Select(paths) => {
                let (dir, _) = split(paths.first()?)?;
                let names =
                    paths.iter().filter(|p| p.parent() == Some(dir.as_path())).filter_map(|p| split(p)).map(|(_, n)| n);
                Some(UiStep::Select { names: names.collect(), dir })
            }
            UiOp::Toast(t) => Some(UiStep::Toast(t)),
            UiOp::CopyText(t) => Some(UiStep::CopyText(t)),
            UiOp::Tag(p, t) => Some(UiStep::Tag(p, t)),
        })
        .collect()
}

fn finish_action(app: &mut AppCore, d: ActionDone) {
    let (jobs, ui) = match d.result {
        Ok(r) => r,
        Err(e) => {
            app.ui.toasts.push(Toast::error(tr!("automation-script-failed", title = &d.title)).body(e.to_string()));
            app.automation.runtime_errors.insert(d.title.clone(), e.to_string());
            return;
        }
    };
    let pane = app.ws.pane().id;
    for step in ui {
        match step {
            UiStep::Go { dir, select } => {
                ops::navigate(app, pane, Location::Dir(dir), NavOpts { select, ..NavOpts::user() })
            }
            UiStep::Select { dir, names } => {
                if app.ws.pane().location != Location::Dir(dir.clone()) {
                    ops::navigate(app, pane, Location::Dir(dir), NavOpts::user());
                }
                app.ws.pane_mut().pending_selection = names;
            }
            UiStep::Toast(t) => {
                app.ui.toasts.push(Toast::info(tr!("automation-script-toast", title = &d.title, text = t)));
            }
            UiStep::CopyText(t) => app.cmds.push(crate::cmd::Cmd::CopyText(t)),
            UiStep::Tag(paths, tag) => add_tag(app, &paths, &tag),
        }
    }
    if jobs.is_empty() {
        return;
    }
    if app.config.scripting.approve_writes {
        app.dialogs.push(crate::models::dialogs::ScriptApprovalModel::new(d.title, jobs));
    } else {
        submit_planned(app, &d.title, jobs);
    }
}

/// Add a color tag (name or 1..8) to `paths` (never removes it).
pub fn add_tag(app: &mut AppCore, paths: &[PathBuf], tag: &str) {
    let Some(t) = crate::services::tags::from_name(tag) else {
        app.ui.toasts.push(Toast::warning(tr!("automation-unknown-tag", tag = tag)));
        return;
    };
    let missing: Vec<PathBuf> =
        paths.iter().filter(|p| crate::services::tags::tags_of(p) & (1 << t) == 0).cloned().collect();
    if !missing.is_empty() {
        crate::services::tags::toggle_many(&missing, t);
    }
}

fn items_of(kind: &JobKind) -> (Vec<PathBuf>, Option<PathBuf>) {
    match kind {
        JobKind::Copy { sources, dest_dir, .. } | JobKind::Move { sources, dest_dir, .. } => {
            (sources.clone(), Some(dest_dir.clone()))
        }
        JobKind::Delete { paths, .. } => (paths.clone(), None),
        JobKind::Rename { pairs } => (pairs.iter().map(|(a, _)| a.clone()).collect(), None),
        JobKind::Create { path, .. } => (vec![path.clone()], None),
        _ => (Vec::new(), None),
    }
}

/// Submit a script's planned jobs as normal (undoable) jobs.
pub fn submit_planned(app: &mut AppCore, title: &str, jobs: Vec<PlannedJob>) {
    for j in jobs {
        let mut spec = app.fileops.jobs.spec(j.kind);
        spec.options.label = Some(format!("{} · {title}", j.summary));
        let (items, dest) = items_of(&spec.kind);
        let op = OpKind::from_brief(spec.kind.brief());
        let meta = JobMeta::new(op, &items, dest).origin(JobOrigin::Script);
        app.fileops.jobs.submit(spec, meta);
    }
}

pub fn apply(app: &mut AppCore, c: AutoCmd, _host: &dyn Host) {
    match c {
        AutoCmd::SubmitPlanned { title, jobs } => submit_planned(app, &title, jobs),
        AutoCmd::RunRule(name) => {
            if app.automation.worker.is_none() {
                app.ui.toasts.push(Toast::warning(tr!("automation-not-running")));
                return;
            }
            // Make sure the worker has the latest edits first.
            if app.automation.sent.as_ref() != Some(&app.config.automation) {
                app.automation.pending = None;
                send_rules(app);
            }
            if let Some(w) = &app.automation.worker {
                w.send(WorkerCmd::RunNow(name));
            }
        }
        AutoCmd::ReloadScripts => reload_scripts(app),
        AutoCmd::OpenScriptsFolder => open_scripts_folder(app, None),
        AutoCmd::NewScript(template) => open_scripts_folder(app, Some(template)),
        AutoCmd::ClearLog(name) => {
            app.automation.logs.remove(&name);
        }
    }
}

pub fn reload_scripts(app: &mut AppCore) {
    match &app.automation.loader {
        Some(l) => {
            l.reload();
            app.ui.toasts.push(Toast::info(tr!("automation-reloading")));
        }
        None => {
            app.ui.toasts.push(Toast::warning(tr!("automation-not-loaded")));
        }
    }
}

/// Example scripts offered on the Scripts page: `(file name, source)`; titles from
/// [`template_title`].
pub const TEMPLATES: &[(&str, &str)] = &[
    ("example.rhai", include_str!("templates/example.rhai")),
    ("tidy-downloads.rhai", include_str!("templates/tidy.rhai")),
];

/// Button title of a template in [`TEMPLATES`].
pub fn template_title(file: &str) -> &'static str {
    match file {
        "example.rhai" => tr!("automation-template-example"),
        _ => tr!("automation-template-tidy"),
    }
}

fn open_scripts_folder(app: &mut AppCore, template: Option<&'static str>) {
    let dir = soyuz_script::scripts_dir();
    let pane = app.ws.pane().id;
    let postman = app.services.mailbox.postman();
    let tx = app.automation.action_tx.clone();
    let wake = app.automation.wake.clone();
    let file = template.and_then(|t| TEMPLATES.iter().find(|(f, _)| *f == t)).map(|(f, src)| (*f, *src));
    let target = dir.clone();
    // Create the folder (and the template file) off the UI thread, then show it.
    let spawned = std::thread::Builder::new().name("soyuz-scripts-folder".into()).spawn(move || {
        let mut select = None;
        let res = soyuz_fs::create_dir_all(&target).map_err(|e| e.to_string()).and_then(|_| {
            let Some((name, src)) = file else { return Ok(()) };
            let mut path = target.join(name);
            let mut n = 2;
            while soyuz_fs::exists_nofollow(&path) {
                path = target.join(format!("{}-{n}.rhai", name.trim_end_matches(".rhai")));
                n += 1;
            }
            select = path.file_name().map(|n| n.to_string_lossy().into_owned());
            soyuz_fs::write_atomic(&path, src.as_bytes()).map_err(|e| e.to_string())
        });
        match res {
            Ok(()) => {
                let step = UiStep::Go { dir: target, select };
                let _ =
                    tx.send(ActionDone { counted: false, title: String::new(), result: Ok((Vec::new(), vec![step])) });
                if let Some(w) = wake {
                    w();
                }
            }
            Err(e) => {
                postman.send(crate::services::mailbox::UiMsg::Notice(
                    Toast::error(tr!("automation-folder-failed")).body(e),
                ));
            }
        }
    });
    if spawned.is_err() {
        ops::navigate(app, pane, Location::Dir(dir), NavOpts::user());
    }
    app.frontend.push(crate::models::frontend::FrontendCmd::CloseSettings);
}

// --- rules ---

fn send_rules(app: &mut AppCore) {
    let a = app.config.automation.clone();
    if let Some(w) = &app.automation.worker {
        w.send(WorkerCmd::Configure { automation: a.clone(), copy: crate::services::jobs::copy_options(&app.config) });
    }
    app.automation.sent = Some(a);
}

fn pump_rules(app: &mut AppCore, host: &dyn Host) {
    let st = &mut app.automation;
    let want = app.config.automation.enabled && !app.config.automation.rules.is_empty();
    if want && st.lock.is_none() && st.rules_note.is_none() {
        // One window per user runs the rules: the first to bind this pipe name.
        match PipeServer::serve(&soyuz_mcp::pipe::user_pipe_name("soyuz-automation"), |_| {}) {
            Ok(l) => st.lock = Some(l),
            Err(e) => {
                log::info!("automations: another Soyuz window runs the rules ({e})");
                st.rules_note = Some(tr!("automation-another-window").into());
            }
        }
    }
    if st.lock.is_some() && st.worker.is_none() {
        let wake = st.wake.clone().unwrap_or_else(|| app.wake.clone());
        let debounce = Duration::from_millis(app.config.behaviour.watch_debounce_ms.max(50));
        let w = worker::spawn(app.fileops.jobs.engine_arc(), wake, debounce);
        if let Some(set) = &st.set {
            w.send(WorkerCmd::Scripts(set.clone()));
        }
        st.worker = Some(w);
        st.sent = None;
    }
    if st.worker.is_some() && st.sent.as_ref() != Some(&app.config.automation) {
        let now = Instant::now();
        match &st.pending {
            Some((a, at)) if a == &app.config.automation => {
                if now.duration_since(*at) >= RULES_DEBOUNCE {
                    st.pending = None;
                    send_rules(app);
                } else {
                    host.redraw_after(RULES_DEBOUNCE);
                }
            }
            _ => {
                st.pending = Some((app.config.automation.clone(), now));
                host.redraw_after(RULES_DEBOUNCE);
            }
        }
    }
    let events: Vec<Event> = app.automation.worker.as_ref().map(|w| w.events.try_iter().collect()).unwrap_or_default();
    for e in events {
        match e {
            Event::Submitted { job, rule: _, items } => track(app, job, &items, JobOrigin::Automation),
            Event::Log { rule, entry } => {
                let cap = app.config.automation.log_entries.clamp(10, 10_000);
                let log = app.automation.logs.entry(rule).or_default();
                log.push_back(entry);
                while log.len() > cap {
                    log.pop_front();
                }
            }
            Event::Tag { paths, tag } => add_tag(app, &paths, &tag),
            Event::Toast(t) => {
                app.ui.toasts.push(Toast::info(t));
            }
            Event::Status(s) => {
                app.automation.status = s.into_iter().collect();
            }
        }
    }
}

/// Register a job submitted outside the UI thread (rules, CLI) with the activity panel.
fn track(app: &mut AppCore, job: JobId, items: &[PathBuf], origin: JobOrigin) {
    let op = app.fileops.jobs.engine().snapshot(job).map(|s| OpKind::from_brief(s.kind)).unwrap_or(OpKind::Other);
    app.fileops.jobs.track(job, JobMeta::new(op, items, None).origin(origin));
}

/// The rule preview for the Automations page; refreshed when the rule changes or
/// every few seconds while shown.
pub fn rule_preview(app: &mut AppCore, idx: usize, rule: &AutomationRule) -> Option<RulePreview> {
    let st = &mut app.automation;
    let stale = match st.previews.get(&idx) {
        None => true,
        Some(p) => !p.loading && (p.rule != *rule || p.at.elapsed() > Duration::from_secs(5)),
    };
    if stale {
        let keep = st.previews.get(&idx).filter(|p| p.rule == *rule).and_then(|p| p.result.clone());
        st.previews.insert(idx, RulePreview { rule: rule.clone(), at: Instant::now(), loading: true, result: keep });
        let tx = st.preview_tx.clone();
        let wake = st.wake.clone();
        let r = rule.clone();
        let _ = std::thread::Builder::new().name("soyuz-rule-preview".into()).spawn(move || {
            let res = soyuz_script::rules::preview(&r, 200);
            let _ = tx.send((idx, r, res));
            if let Some(w) = wake {
                w();
            }
        });
    }
    st.previews.get(&idx).cloned()
}

// --- CLI ---

fn pump_cli(app: &mut AppCore, host: &dyn Host) {
    let st = &mut app.automation;
    let want = app.config.cli.server_enabled;
    if !want {
        st.cli = None;
        st.cli_failed = false;
        st.cli_status = None;
    } else if st.cli.is_none() && !st.cli_failed {
        let wake = st.wake.clone().unwrap_or_else(|| app.wake.clone());
        match cli_server::CliServer::start(app.fileops.jobs.engine_arc(), wake) {
            Ok(s) => {
                st.cli_status = Some(Ok(s.name.clone()));
                st.cli = Some(s);
            }
            Err(e) => {
                st.cli_failed = true;
                st.cli_status = Some(Err(tr!("automation-cli-unavailable", error = e.to_string())));
            }
        }
    }
    let reqs: Vec<cli_server::UiReq> =
        app.automation.cli.as_ref().map(|c| c.rx.try_iter().collect()).unwrap_or_default();
    for r in reqs {
        match r {
            cli_server::UiReq::Submit(spec, reply) => {
                let mut full = app.fileops.jobs.spec(spec.kind.clone());
                full.options.conflict = spec.options.conflict.clone();
                full.options.queue = spec.options.queue;
                let label = spec.options.label.clone().unwrap_or_else(|| default_label(&full.kind));
                full.options.label = Some(format!("{label} · CLI"));
                let (items, dest) = items_of(&full.kind);
                let meta = JobMeta::new(OpKind::from_brief(full.kind.brief()), &items, dest).origin(JobOrigin::Cli);
                let id = app.fileops.jobs.submit(full, meta);
                let _ = reply.send(Ok(id));
            }
            cli_server::UiReq::Open { path, reveal, reply } => {
                let pane = app.ws.pane().id;
                // The client checked that the path exists; a folder opens in a new tab, a
                // file launches (open) or is selected in its folder (reveal).
                if reveal {
                    if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
                        let name = name.to_string_lossy().into_owned();
                        ops::navigate(
                            app,
                            pane,
                            Location::Dir(dir.to_path_buf()),
                            NavOpts { select: Some(name), ..NavOpts::user() },
                        );
                    }
                } else {
                    open_path(app, path);
                }
                host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Minimized(false));
                host.window(crate::host::WindowId::Current, crate::host::WindowCmd::Focus);
                let _ = reply.send(Ok(()));
            }
        }
    }
}

/// `soyuz open <path>`: folders open in a new tab, files launch (decided off-thread).
fn open_path(app: &mut AppCore, path: PathBuf) {
    let postman = app.services.mailbox.postman();
    let _ = std::thread::Builder::new().name("soyuz-cli-open".into()).spawn(move || {
        if soyuz_fs::stat(&path, true).is_ok_and(|e| e.is_dir_like()) {
            postman.send(crate::services::mailbox::UiMsg::OpenDir(path));
        } else if let Err(e) = soyuz_fs::launch::open_default(&path) {
            postman.send(crate::services::mailbox::UiMsg::Notice(
                Toast::error(tr!("automation-open-failed", name = soyuz_core::entry::display_name(&path)))
                    .body(e.to_string()),
            ));
        }
    });
}

fn default_label(kind: &JobKind) -> String {
    let (items, dest) = items_of(kind);
    let count = items.len();
    let name = match items.as_slice() {
        [one] => soyuz_core::entry::display_name(one).to_string(),
        _ => String::new(),
    };
    match (kind, dest) {
        (JobKind::Copy { .. }, Some(d)) => {
            tr!("automation-cli-copy", count = count, name = name, dest = soyuz_core::entry::display_name(&d))
        }
        (JobKind::Move { .. }, Some(d)) => {
            tr!("automation-cli-move", count = count, name = name, dest = soyuz_core::entry::display_name(&d))
        }
        (JobKind::Delete { mode: soyuz_jobs::DeleteMode::Trash, .. }, _) => {
            tr!("automation-cli-trash", count = count, name = name)
        }
        (JobKind::Delete { .. }, _) => tr!("automation-cli-delete", count = count, name = name),
        _ => tr!("automation-cli-items", count = count, name = name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_load_and_run() {
        let srcs: Vec<(&str, &str)> = TEMPLATES.to_vec();
        let set = ScriptSet::from_sources(&srcs, soyuz_script::Limits::default());
        for s in &set.scripts {
            assert!(s.is_ok(), "{}: {:?}", s.file, s.error);
        }
        assert_eq!(set.actions().count(), 2);
        assert_eq!(set.columns().len(), 1);
        assert!(set.rule_script("tidy-downloads").is_some());
        let d = tempfile::tempdir().unwrap();
        let csv = d.path().join("t.csv");
        std::fs::write(&csv, "a,b\n1,2\n").unwrap();
        let Ok(Some(soyuz_script::Preview::Markdown(md))) = set.preview(&csv) else { panic!("csv preview") };
        assert!(md.contains("a   b"), "{md}");
        assert!(md.contains("1   2"), "{md}");
        let e = soyuz_fs::stat(&csv, true).unwrap();
        assert_eq!(set.column_value(0, &e).unwrap().as_deref(), Some("3"));
        let a = d.path().join("a.txt");
        let ctx = ActionCtx {
            dir: Some(d.path().to_path_buf()),
            focused: Some(a.clone()),
            selected: vec![a.clone(), csv.clone()],
            other_dir: None,
        };
        let out = set.run_action("example/copy-selected-names", &ctx).unwrap();
        assert_eq!(out.ui[0], UiOp::CopyText("a.txt\nt.csv".into()));
        let out = set.run_action("example/move-into-a-new-folder", &ctx).unwrap();
        assert_eq!(out.jobs.len(), 2, "mkdir + move");
        let img = d.path().join("x.png");
        let out = set.run_rule("tidy-downloads", &img, "r").unwrap();
        assert_eq!(out.jobs.len(), 2, "mkdir Images + move");
        assert_eq!(origin_icon(JobOrigin::Cli), IconId::TerminalWindow);
    }
}
