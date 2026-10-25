//! `SOYUZ_SCRIPT`, the debug / automation hook shared by both frontends (Phase T8): a list
//! of steps separated by `;`, run one frame at a time. The core runs the steps that only
//! need [`AppCore`] and the [`Host`] and hands every other step to the frontend as a
//! [`FrontStep`].
//!
//! Core steps:
//! * `wait:<ms>` — let the UI settle;
//! * `until:<path>` — pause the script until `<path>` exists (at most 120 s); external
//!   drivers such as `scripts/uia-audit.ps1` use it to hold a state while they inspect
//!   the window;
//! * `wait_for:<check>` — pause the script until an `expect` check holds (at most 30 s),
//!   then log it like `expect` (a timeout logs `expect FAILED`);
//! * `cd:<path>` — navigate the focused pane; `select:<name>` — select one item;
//! * `theme:dark|light|system`;
//! * `config:<dotted.key>=<toml value>` — change a setting live (`config:appearance.density="touch"`);
//! * `scroll:<px>` — scroll the focused pane to an offset; `welcome:on|off`;
//! * `expect:<check>` — log `SOYUZ_SCRIPT: expect ok <check>` or `expect FAILED <check>
//!   (actual …)`; checks: `cwd=<folder name>`, `cursor=<name>`, `settings=open|closed`,
//!   `palette=open|closed`, `dialog=open|closed`, `exists=<path>`, `missing=<path>`,
//!   `lang=<tag>`, `has=<name>` (the focused pane shows it), `focus=<name>` (the
//!   frontend's focus), `screen~<text>` (the last frame shows that text). `~` instead of
//!   `=` matches a substring for any check;
//! * `toast:<kind>|<title>[|<body>[|undo]]` — show a toast for 30 s (`info`, `success`,
//!   `warning`, `error`; `undo` adds an Undo action), for toast screenshots;
//! * `panes:<label>` — log the panes of the active tab (location, status, entry and
//!   visible counts, visible names);
//! * `mark:<label>` — log a marker line (`SOYUZ_SCRIPT: mark <label>`), e.g. where the
//!   frame-budget test starts counting;
//! * `smart:<name>` — open a smart folder (`smart:` alone: the new smart folder dialog);
//! * `edit:<path>` — open a file in the built-in editor; `editor:<command>` — drive it
//!   (see `editors::devscript`);
//! * `ai:<prompt>` — the assistant (see `ai::state::script`);
//! * `ws:<verb> [arg]` — workspaces (see `models::workspaces::devscript`);
//! * `quit`;
//! * anything else is an action name (`view_grid`, `toggle_dual_pane`, …).
//!
//! Frontend steps ([`FRONT_VERBS`]): `shot:<file>`, `keys:<spec>` (see [`parse_keys`]),
//! `size:<w>x<h>`, and the GUI's `hover`, `click`, `press`, `move`, `release`, `native`,
//! `dropdown`, `settings`, `appearance`, `export_picker`, `win` (`win:<k> <step>`: one
//! step inside the GUI's secondary window `k`, run through [`Runner::run_step`]).
//!
//! The frontend loop, once per frame:
//! ```ignore
//! loop {
//!     let facts = ScriptFacts { dialog_open, settings_open, focus, screen_text };
//!     let Some(step) = runner.pump(&mut core, &host, &facts) else { break };
//!     let yields = step.yields();
//!     handle(step);
//!     if yields { break }
//! }
//! ```
//! Waits (`wait`, `until`, `wait_for`) ask for the next frame through
//! [`Host::redraw_after`], so the frontend needs no timeout of its own. `expect` and
//! `wait_for` results are kept on the runner ([`Runner::results`], [`Runner::failures`])
//! as well as logged (`SOYUZ_SCRIPT: expect ok …` / `expect FAILED …`), so in-process tests
//! can assert on them without a logger.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::{Action, Config, KeyChord};

use crate::cmd::Cmd;
use crate::core::AppCore;
use crate::host::{Host, ThemePref, WindowCmd, WindowId};

/// The environment variable the script comes from.
pub const ENV: &str = "SOYUZ_SCRIPT";

/// The verbs the core hands to the frontend as [`FrontStep`]s.
pub const FRONT_VERBS: &[&str] = &[
    "shot",
    "keys",
    "size",
    "hover",
    "click",
    "press",
    "move",
    "release",
    "native",
    "dropdown",
    "settings",
    "appearance",
    "export_picker",
    "win",
];

/// How long `until:` waits for its file.
const UNTIL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long `wait_for:` waits for its check (unless [`Runner::with_wait_for_timeout`]).
const WAIT_FOR_TIMEOUT: Duration = Duration::from_secs(30);
/// How often `wait_for:` checks again.
const WAIT_FOR_POLL: Duration = Duration::from_millis(50);

/// One token of a `keys:` step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyTok {
    /// A typed character.
    Text(char),
    /// `{Ctrl+S}`, `{Enter}`, `{Shift+F3}`.
    Chord(KeyChord),
    /// `{Copy}`.
    Copy,
    /// `{Cut}`.
    Cut,
    /// `{Paste:text}`.
    Paste(String),
}

/// A `keys:` spec: plain text is typed, `{…}` is a key, `{Copy}` / `{Cut}` /
/// `{Paste:text}` are clipboard events. A `{…}` that is no key chord is dropped; a `{`
/// without a closing `}` is typed.
pub fn parse_keys(spec: &str) -> Vec<KeyTok> {
    let mut out = Vec::new();
    let mut rest = spec;
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix('{')
            && let Some(end) = body.find('}')
        {
            let tok = &body[..end];
            rest = &body[end + 1..];
            if tok == "Copy" {
                out.push(KeyTok::Copy);
            } else if tok == "Cut" {
                out.push(KeyTok::Cut);
            } else if let Some(t) = tok.strip_prefix("Paste:") {
                out.push(KeyTok::Paste(t.to_string()));
            } else if let Ok(chord) = tok.parse::<KeyChord>() {
                out.push(KeyTok::Chord(chord));
            }
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        rest = &rest[c.len_utf8()..];
        out.push(KeyTok::Text(c));
    }
    out
}

/// What the frontend knows that the core does not, computed before [`Runner::pump`] (the
/// runner lives inside the frontend's app, so it cannot borrow it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptFacts {
    /// A frontend-only dialog is open (`expect:dialog=open`; the core's own dialog stack
    /// counts too).
    pub dialog_open: bool,
    /// The settings window / editor is open (`expect:settings=open`).
    pub settings_open: bool,
    /// The frontend's keyboard focus by name (`expect:focus=<name>`; the TUI's `TuiFocus`
    /// variant name), when it has one; the core's focus area otherwise.
    pub focus: Option<String>,
    /// The last frame as text, one line per row (`expect:screen~<text>`), when the
    /// frontend keeps it.
    pub screen_text: Option<String>,
}

/// A step only the frontend can run. Steps that need the next frame [`yield`](Self::yields).
#[derive(Clone, Debug, PartialEq)]
pub enum FrontStep {
    /// `shot:<file>`: save the next frame (GUI: PNG; TUI: by extension).
    Shot(PathBuf),
    /// `keys:<spec>`: synthetic keyboard input for the next frame.
    Keys(Vec<KeyTok>),
    /// `size:<w>x<h>`: resize the window (GUI: points; TUI: cells, headless only).
    Size { w: f32, h: f32 },
    /// Any other frontend step (`hover`, `click`, `press`, `move`, `release`, `native`,
    /// `dropdown`, `settings`, `appearance`, `export_picker`), as written.
    Other { verb: String, arg: String },
}

impl FrontStep {
    /// The frontend stops running steps for this frame after this one (the GUI's old
    /// `return`s: `shot`, `keys` and the pointer steps); the others go on in the same
    /// frame.
    pub fn yields(&self) -> bool {
        match self {
            FrontStep::Shot(_) | FrontStep::Keys(_) => true,
            FrontStep::Size { .. } => false,
            FrontStep::Other { verb, .. } => matches!(verb.as_str(), "click" | "press" | "move" | "release" | "win"),
        }
    }

    /// The frontend step for `verb:arg` (`None`: not a frontend verb, or a `size` that
    /// does not parse, which is skipped).
    fn parse(verb: &str, arg: &str) -> Option<FrontStep> {
        match verb {
            "shot" => Some(FrontStep::Shot(PathBuf::from(arg.trim()))),
            "keys" => Some(FrontStep::Keys(parse_keys(arg))),
            "size" => {
                let (w, h) = arg.split_once('x')?;
                Some(FrontStep::Size { w: w.trim().parse().ok()?, h: h.trim().parse().ok()? })
            }
            v if FRONT_VERBS.contains(&v) => Some(FrontStep::Other { verb: v.to_string(), arg: arg.to_string() }),
            _ => None,
        }
    }
}

/// The result of one `expect` / `wait_for` check: the check as written, and `Err(actual)`
/// when it did not hold.
pub type CheckResult = (String, Result<(), String>);

/// A running script.
#[derive(Debug, Default)]
pub struct Runner {
    steps: VecDeque<String>,
    /// A `wait:` holds the script until then.
    resume_at: Option<Instant>,
    /// When the current `until:` step started waiting.
    until_since: Option<Instant>,
    /// When the current `wait_for:` step started waiting.
    wait_for_since: Option<Instant>,
    /// `wait_for:`'s limit (`None`: [`WAIT_FOR_TIMEOUT`]).
    wait_for_timeout: Option<Duration>,
    /// Every `expect` / `wait_for` result, in order.
    results: Vec<CheckResult>,
}

impl Runner {
    /// The script `script` (steps separated by `;`, blanks dropped).
    pub fn parse(script: &str) -> Runner {
        let steps = script.split(';').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
        Runner { steps, ..Runner::default() }
    }

    /// The script in [`ENV`], if it is set. Read by the production entries only (the GUI's
    /// `main`, `soyuz_tui::run`), never by in-process tests.
    pub fn from_env() -> Option<Runner> {
        std::env::var(ENV).ok().map(|s| Runner::parse(&s))
    }

    /// `wait_for:` gives up after `limit` instead of 30 s (tests).
    pub fn with_wait_for_timeout(mut self, limit: Duration) -> Runner {
        self.wait_for_timeout = Some(limit);
        self
    }

    /// Put `step` in front of the remaining steps (a frontend step that expands into
    /// several, like the GUI's `click`).
    pub fn push_front(&mut self, step: String) {
        self.steps.push_front(step);
    }

    /// No step is left and nothing is waiting.
    pub fn is_done(&self) -> bool {
        self.steps.is_empty() && self.resume_at.is_none() && self.until_since.is_none() && self.wait_for_since.is_none()
    }

    /// Every `expect` / `wait_for` result so far.
    pub fn results(&self) -> &[CheckResult] {
        &self.results
    }

    /// The checks that failed, as `"<check> (actual <value>)"`.
    pub fn failures(&self) -> Vec<String> {
        self.results.iter().filter_map(|(c, r)| r.as_ref().err().map(|a| format!("{c} (actual {a})"))).collect()
    }

    /// Run the due steps: core steps here, until the first frontend step, which is
    /// returned (call again unless it [`yields`](FrontStep::yields)). `None`: nothing more
    /// this frame (done, or waiting; a wait has asked `host` for a frame).
    pub fn pump(&mut self, core: &mut AppCore, host: &dyn Host, facts: &ScriptFacts) -> Option<FrontStep> {
        loop {
            if let Some(t) = self.resume_at {
                let now = Instant::now();
                if now < t {
                    host.redraw_after(t - now);
                    return None;
                }
                self.resume_at = None;
            }
            let step = self.steps.pop_front()?;
            if self.until_since.is_none() && self.wait_for_since.is_none() {
                log::info!("SOYUZ_SCRIPT: {step}");
            }
            let (verb, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
            if FRONT_VERBS.contains(&verb) {
                if let Some(front) = FrontStep::parse(verb, arg) {
                    return Some(front);
                }
            } else {
                self.core_step(core, host, facts, &step);
            }
            host.redraw();
        }
    }

    /// Run `step` now (out of order): a core step here, or the frontend step it is,
    /// returned. The GUI's `win:<k> <step>` runs it with that window swapped in.
    pub fn run_step(
        &mut self,
        core: &mut AppCore,
        host: &dyn Host,
        facts: &ScriptFacts,
        step: &str,
    ) -> Option<FrontStep> {
        let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
        if FRONT_VERBS.contains(&verb) {
            return FrontStep::parse(verb, arg);
        }
        self.core_step(core, host, facts, step);
        host.redraw();
        None
    }

    /// One core step.
    fn core_step(&mut self, core: &mut AppCore, host: &dyn Host, facts: &ScriptFacts, step: &str) {
        let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
        match verb {
            "wait" => {
                let ms = arg.trim().parse::<u64>().unwrap_or(500);
                self.resume_at = Some(Instant::now() + Duration::from_millis(ms));
            }
            "until" => {
                let since = *self.until_since.get_or_insert_with(Instant::now);
                if !Path::new(arg.trim()).exists() && since.elapsed() < UNTIL_TIMEOUT {
                    self.steps.push_front(step.to_string());
                    self.resume_at = Some(Instant::now() + Duration::from_millis(100));
                } else {
                    self.until_since = None;
                }
            }
            "wait_for" => {
                let check = arg.trim();
                let since = *self.wait_for_since.get_or_insert_with(Instant::now);
                let result = expect(core, facts, check);
                let unknown = result.as_ref().is_err_and(|a| a.starts_with("unknown check"));
                if result.is_err() && !unknown && since.elapsed() < self.wait_for_timeout.unwrap_or(WAIT_FOR_TIMEOUT) {
                    self.steps.push_front(step.to_string());
                    self.resume_at = Some(Instant::now() + WAIT_FOR_POLL);
                } else {
                    self.wait_for_since = None;
                    self.record(check, result);
                }
            }
            "cd" => {
                let pane = core.ws.pane().id;
                core.cmds.push(Cmd::Navigate {
                    pane,
                    // Like the address bar: archives (`a.zip!/x`) and servers too.
                    location: Location::parse(arg.trim()).unwrap_or_else(|| Location::Dir(PathBuf::from(arg.trim()))),
                    select: None,
                });
            }
            "select" => {
                let pane = core.ws.pane_mut();
                if let Some(pos) = pane.visible_pos(arg.trim()) {
                    pane.move_cursor(pos, false, false);
                }
            }
            "theme" => {
                core.ui.theme = ThemePref::from_config(arg);
                host.set_theme(core.ui.theme);
            }
            "config" => match edited(&core.config, arg) {
                Ok(c) => crate::services::config_watch::apply_config(core, host, c),
                Err(e) => log::warn!("SOYUZ_SCRIPT {step}: {e}"),
            },
            "welcome" => core.ui.welcome = arg.trim() != "off",
            "mark" => log::info!("SOYUZ_SCRIPT: mark {}", arg.trim()),
            "panes" => log_panes(core, arg.trim()),
            "toast" => {
                core.ui.toasts.push(script_toast(arg));
            }
            "expect" => {
                let check = arg.trim();
                let result = expect(core, facts, check);
                self.record(check, result);
            }
            "scroll" => {
                // Absolute scroll offset of the focused pane (perf probes).
                let y = arg.trim().parse::<f32>().unwrap_or(0.0);
                core.ws.pane_mut().restore_scroll = Some(y);
            }
            "quit" => {
                core.workspaces.quitting = true;
                host.window(WindowId::Root, WindowCmd::Close);
            }
            "ws" => crate::models::workspaces::devscript(core, arg),
            "smart" => {
                // `smart:<name>` opens a smart folder; `smart:` opens the "new" dialog.
                use crate::models::smart::{SmartCmd, folders};
                if arg.trim().is_empty() {
                    core.cmds.push(Cmd::Smart(SmartCmd::Edit(None)));
                } else if let Some(index) = folders().iter().position(|f| f.name == arg.trim()) {
                    core.cmds.push(Cmd::Smart(SmartCmd::Open { index, new_tab: false }));
                }
            }
            "edit" | "editor" => crate::editors::devscript(core, host, verb, arg),
            "ai" => crate::ai::state::script(core, host, arg),
            _ => match step.parse::<Action>() {
                Ok(a) => core.cmds.push(Cmd::Action(a)),
                Err(e) => log::warn!("SOYUZ_SCRIPT: {e}"),
            },
        }
    }

    /// Log and keep one check's result.
    fn record(&mut self, check: &str, result: Result<(), String>) {
        match &result {
            Ok(()) => log::info!("SOYUZ_SCRIPT: expect ok {check}"),
            Err(actual) => log::warn!("SOYUZ_SCRIPT: expect FAILED {check} (actual {actual})"),
        }
        self.results.push((check.to_string(), result));
    }
}

/// One `expect:` check; `Err(actual value)` when it does not hold. `what=want` compares,
/// `what~want` looks for a substring (the first `=` or `~` separates).
fn expect(core: &AppCore, facts: &ScriptFacts, check: &str) -> Result<(), String> {
    let (what, sep, want) = match check.find(['=', '~']) {
        Some(i) => (&check[..i], &check[i..=i], &check[i + 1..]),
        None => (check, "=", ""),
    };
    let open = |b: bool| if b { "open" } else { "closed" }.to_string();
    let actual = match what.trim() {
        "cwd" => {
            let loc = core.ws.pane().location.display();
            let name = Path::new(&loc).file_name().map(|n| n.to_string_lossy().into_owned());
            name.unwrap_or(loc)
        }
        "cursor" => {
            let pane = core.ws.pane();
            pane.cursor_pos().and_then(|p| pane.entry_at(p)).map(|e| e.name.clone()).unwrap_or_default()
        }
        "settings" => open(facts.settings_open),
        "palette" => open(core.palette.open),
        "dialog" => open(facts.dialog_open || core.dialogs.is_open()),
        // Debug hook only: a one-off metadata probe on the UI thread is fine here.
        "exists" => {
            if Path::new(want).exists() {
                want.to_string()
            } else {
                "absent".into()
            }
        }
        "missing" => {
            if Path::new(want).exists() {
                "present".into()
            } else {
                want.to_string()
            }
        }
        "lang" => crate::i18n::current().tag().to_string(),
        // `has=<name>`: the focused pane shows an entry with that name.
        "has" => {
            let pane = core.ws.pane();
            if pane.visible_pos(want).is_some() { want.to_string() } else { "absent".into() }
        }
        "focus" => facts.focus.clone().unwrap_or_else(|| format!("{:?}", core.ui.focus)),
        // `screen~<text>`: the last frame shows it (the whole screen is too long to log).
        "screen" => {
            let Some(screen) = facts.screen_text.as_deref() else { return Err("no screen text".into()) };
            let hit = if sep == "~" { screen.contains(want) } else { screen == want };
            return if hit { Ok(()) } else { Err("not on screen".into()) };
        }
        other => return Err(format!("unknown check {other}")),
    };
    let hit = if sep == "~" { actual.contains(want) } else { actual == want };
    if hit { Ok(()) } else { Err(actual) }
}

/// `panes:<label>` step: log every pane of the active tab of each window (location, listing state,
/// entry / visible counts and the visible names) for scripted refresh checks.
fn log_panes(core: &AppCore, label: &str) {
    let secondary = core.workspaces.windows.iter().map(|w| w.ws.tab());
    for (i, p) in std::iter::once(core.ws.tab()).chain(secondary).flat_map(|t| t.panes.iter()).enumerate() {
        let names: Vec<&str> = p.visible.iter().filter_map(|&j| p.entries.get(j)).map(|e| e.name.as_str()).collect();
        log::info!(
            "SOYUZ_SCRIPT: panes {label} #{i} {} status={:?} refreshing={} entries={} visible={} [{}]",
            p.location.display(),
            p.status,
            p.refreshing,
            p.entries.len(),
            p.visible.len(),
            names.join(", ")
        );
    }
}

/// `toast:` step → a toast that stays for 30 s.
fn script_toast(arg: &str) -> crate::models::toasts::Toast {
    use crate::models::toasts::{Toast, ToastKind};
    let mut parts = arg.split('|');
    let kind = match parts.next().unwrap_or("").trim() {
        "success" => ToastKind::Success,
        "warning" => ToastKind::Warning,
        "error" => ToastKind::Error,
        _ => ToastKind::Info,
    };
    let mut t = Toast::new(kind, parts.next().unwrap_or("Toast").trim());
    t.duration = Some(30.0);
    if let Some(b) = parts.next().map(str::trim).filter(|b| !b.is_empty()) {
        t = t.body(b.replace("\\n", "\n"));
    }
    if parts.next().is_some_and(|a| a.trim() == "undo") {
        t = t.action(tr!("common-undo"), Action::Undo);
    }
    t
}

/// `config` with one dotted key set to a TOML value (`appearance.density="touch"`).
pub fn edited(config: &Config, assignment: &str) -> Result<Config, String> {
    let (key, value) = assignment.split_once('=').ok_or("expected key=value")?;
    let mut root: toml::Table =
        toml::from_str(&soyuz_core::config::to_pretty_string(config).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let value: toml::Value = toml::from_str::<toml::Table>(&format!("v = {value}"))
        .map_err(|e| e.to_string())?
        .remove("v")
        .ok_or("no value")?;
    let parts: Vec<&str> = key.trim().split('.').collect();
    let mut t = &mut root;
    for p in &parts[..parts.len() - 1] {
        t = t.get_mut(*p).and_then(toml::Value::as_table_mut).ok_or_else(|| format!("no table {p}"))?;
    }
    t.insert(parts[parts.len() - 1].to_string(), value);
    soyuz_core::config::parse_str(&toml::to_string(&root).map_err(|e| e.to_string())?)
        .map(|l| l.config)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{KeyName, Mods};

    #[test]
    fn parse_splits_steps_and_drops_blanks() {
        let r = Runner::parse(" wait:10 ;; expect:cwd=a ;");
        assert!(!r.is_done());
        assert!(Runner::parse(" ; ").is_done());
        assert!(r.results().is_empty() && r.failures().is_empty());
    }

    #[test]
    fn only_frame_bound_steps_yield() {
        assert!(FrontStep::Shot("a.txt".into()).yields());
        assert!(FrontStep::Keys(Vec::new()).yields());
        assert!(!FrontStep::Size { w: 80.0, h: 24.0 }.yields());
        assert!(FrontStep::Other { verb: "click".into(), arg: "1,2".into() }.yields());
        assert!(!FrontStep::Other { verb: "hover".into(), arg: "1,2".into() }.yields());
    }

    #[test]
    fn front_steps_parse() {
        assert_eq!(FrontStep::parse("shot", " a.png "), Some(FrontStep::Shot("a.png".into())));
        assert_eq!(FrontStep::parse("size", "100x 30"), Some(FrontStep::Size { w: 100.0, h: 30.0 }));
        assert_eq!(FrontStep::parse("size", "big"), None);
        assert_eq!(
            FrontStep::parse("settings", "close"),
            Some(FrontStep::Other { verb: "settings".into(), arg: "close".into() })
        );
        assert_eq!(FrontStep::parse("cd", "x"), None);
        // `keys:` keeps leading blanks: they are typed.
        assert_eq!(FrontStep::parse("keys", " a"), Some(FrontStep::Keys(vec![KeyTok::Text(' '), KeyTok::Text('a')])));
    }

    #[test]
    fn key_specs() {
        let chord = |mods, key| KeyTok::Chord(KeyChord::new(mods, key));
        assert_eq!(parse_keys("{Ctrl+Shift+P}"), vec![chord(Mods::CTRL | Mods::SHIFT, KeyName::P)]);
        assert_eq!(
            parse_keys("ab{Enter}"),
            vec![KeyTok::Text('a'), KeyTok::Text('b'), chord(Mods::empty(), KeyName::Enter)]
        );
        assert_eq!(parse_keys("{Paste:x y}{Copy}{Cut}"), vec![KeyTok::Paste("x y".into()), KeyTok::Copy, KeyTok::Cut]);
        assert_eq!(parse_keys("ğİ日"), vec![KeyTok::Text('ğ'), KeyTok::Text('İ'), KeyTok::Text('日')]);
        assert_eq!(
            parse_keys("{Ctrl+Comma}{Shift+Delete}"),
            vec![chord(Mods::CTRL, KeyName::Comma), chord(Mods::SHIFT, KeyName::Delete)]
        );
        // Unknown keys are dropped; a `{` without `}` is typed.
        assert_eq!(parse_keys("{Nope}{Ctrl+Nope}{}x"), vec![KeyTok::Text('x')]);
        assert_eq!(parse_keys("{a"), vec![KeyTok::Text('{'), KeyTok::Text('a')]);
    }

    #[test]
    fn dotted_config_edit() {
        let c = edited(&Config::default(), "appearance.density=\"touch\"").unwrap();
        assert_eq!(c.appearance.density, soyuz_core::config::Density::Touch);
        let c = edited(&c, "appearance.high_contrast=true").unwrap();
        assert!(c.appearance.high_contrast);
        let c = edited(&c, "tui.screen_reader=true").unwrap();
        assert!(c.tui.screen_reader);
        assert_eq!(edited(&c, "nope.x=1").unwrap_err(), "no table nope");
        assert!(edited(&c, "novalue").is_err());
    }

    /// Set in the re-run child (see [`runner_drives_a_core`]): the folder to list.
    const CHILD: &str = "SOYUZ_DEVSCRIPT_CORE_DATA";

    /// The runner against a real `AppCore` (timing, core checks, `wait_for`, `quit`). The
    /// core keeps its state in the config directory, so the test re-runs itself from a
    /// copy of this test binary next to a `portable` marker in a temporary directory
    /// (like `tests/headless_core.rs`): nothing outside it is read or written.
    #[test]
    fn runner_drives_a_core() {
        if let Some(data) = std::env::var_os(CHILD) {
            run_child(PathBuf::from(data));
            return;
        }
        let tmp = tempfile::tempdir().expect("temp dir");
        let exe = std::env::current_exe().expect("test binary");
        let copy = tmp.path().join(exe.file_name().expect("exe name"));
        std::fs::copy(&exe, &copy).expect("copy the test binary");
        std::fs::write(tmp.path().join(soyuz_core::config::PORTABLE_MARKER), b"").expect("portable marker");
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("sub")).unwrap();
        std::fs::write(data.join("a.txt"), b"a").unwrap();
        let mut cmd = std::process::Command::new(&copy);
        cmd.args(["--exact", "devscript::tests::runner_drives_a_core", "--nocapture", "--test-threads=1"])
            .env(CHILD, &data)
            .env_remove(ENV);
        // On Linux a test thread that forks while the copy is open for writing makes the
        // start fail with "Text file busy" (ETXTBSY) until that child execs: retry.
        let begin = std::time::Instant::now();
        let out = loop {
            match cmd.output() {
                Err(e)
                    if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && begin.elapsed() < std::time::Duration::from_secs(10) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20))
                }
                r => break r.expect("run the copied test binary"),
            }
        };
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "child run failed:\n{log}");
        assert!(log.contains("devscript child: ok"), "the child did not run the test:\n{log}");
    }

    fn run_child(data: PathBuf) {
        use crate::core::Startup;
        use crate::host::NullHost;
        let dir = soyuz_core::config::config_dir();
        assert!(dir.ends_with(soyuz_core::config::PORTABLE_DATA_DIR), "not in portable mode: {}", dir.display());

        let mut c = Config::default();
        c.updates.weekly_check = false;
        c.cli.server_enabled = false;
        c.search.index.enabled = false;
        let startup = Startup { open: Some(data.clone()), secondary: true, safe_mode: false, ..Default::default() };
        let host = NullHost::default();
        let mut core = AppCore::new(c, startup, host.wake(), crate::services::preview_gfx::no_upload());
        let mut facts = ScriptFacts::default();

        // Timing: a wait holds the script; front steps come out in order, and only the
        // yielding ones end a frame.
        let mut r = Runner::parse("wait:150;mark:m;shot:a.txt;size:80x24;hover:1,2;keys:{Ctrl+S}");
        let start = Instant::now();
        assert_eq!(r.pump(&mut core, &host, &facts), None, "waiting");
        assert!(!r.is_done());
        while start.elapsed() < Duration::from_millis(160) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(r.pump(&mut core, &host, &facts), Some(FrontStep::Shot("a.txt".into())));
        let mut frame = Vec::new();
        while let Some(s) = r.pump(&mut core, &host, &facts) {
            let yields = s.yields();
            frame.push(s);
            if yields {
                break;
            }
        }
        assert_eq!(
            frame,
            vec![
                FrontStep::Size { w: 80.0, h: 24.0 },
                FrontStep::Other { verb: "hover".into(), arg: "1,2".into() },
                FrontStep::Keys(vec![KeyTok::Chord(KeyChord::new(Mods::CTRL, KeyName::S))]),
            ]
        );
        assert_eq!(r.pump(&mut core, &host, &facts), None);
        assert!(r.is_done());

        // Core checks and their results.
        let name = data.file_name().unwrap().to_string_lossy().into_owned();
        facts.screen_text = Some("one\nCopying a.txt\nthree".into());
        facts.focus = Some("List".into());
        let script = format!(
            "expect:cwd={name};expect:cwd=nope;expect:screen~Copying;expect:screen~Moving;expect:focus=List;\
             expect:dialog=closed;expect:exists={a};expect:missing={a};expect:cwd~{half};expect:bogus=1",
            a = data.join("a.txt").display(),
            half = &name[..2],
        );
        let mut r = Runner::parse(&script);
        assert_eq!(r.pump(&mut core, &host, &facts), None);
        let ok: Vec<bool> = r.results().iter().map(|(_, res)| res.is_ok()).collect();
        assert_eq!(ok, [true, false, true, false, true, true, true, false, true, false], "{:?}", r.results());
        assert_eq!(
            r.failures(),
            vec![
                format!("cwd=nope (actual {name})"),
                "screen~Moving (actual not on screen)".to_string(),
                format!("missing={} (actual present)", data.join("a.txt").display()),
                "bogus=1 (actual unknown check bogus)".to_string(),
            ]
        );

        // wait_for: holds until the check holds; a timeout records a failure and goes on;
        // an unknown check fails at once.
        let mut r = Runner::parse("wait_for:screen~ready;mark:a;wait_for:dialog=open;mark:b;wait_for:nope")
            .with_wait_for_timeout(Duration::from_millis(200));
        assert_eq!(r.pump(&mut core, &host, &facts), None);
        assert!(r.results().is_empty() && !r.is_done(), "still waiting");
        facts.screen_text = Some("all ready".into());
        let start = Instant::now();
        while !r.is_done() {
            assert!(start.elapsed() < Duration::from_secs(10), "wait_for never gave up");
            assert_eq!(r.pump(&mut core, &host, &facts), None);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(start.elapsed() >= Duration::from_millis(200), "the timeout was honoured");
        assert_eq!(r.results().len(), 3);
        assert_eq!(
            r.failures(),
            vec!["dialog=open (actual closed)".to_string(), "nope (actual unknown check nope)".to_string()]
        );

        // Core steps: config, welcome, action names, quit.
        let mut r = Runner::parse("config:appearance.density=\"touch\";welcome:off;toggle_hidden;not_an_action;quit");
        let before = core.cmds.len();
        assert_eq!(r.pump(&mut core, &host, &facts), None);
        assert_eq!(core.config.appearance.density, soyuz_core::config::Density::Touch);
        assert!(!core.ui.welcome);
        assert_eq!(core.cmds.len(), before + 1, "one action queued");
        assert!(matches!(core.cmds.last(), Some(Cmd::Action(Action::ToggleHidden))));
        assert!(core.workspaces.quitting);
        assert!(host.windows.lock().unwrap().contains(&(WindowId::Root, WindowCmd::Close)));
        println!("devscript child: ok");
    }
}
