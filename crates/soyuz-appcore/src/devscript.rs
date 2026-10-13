//! `SOYUZ_SCRIPT`, the debug / automation hook shared by both frontends (Phase T8): a list
//! of steps separated by `;`, run one frame at a time. The core runs the steps that only
//! need [`AppCore`] and the [`Host`] (`wait`, `until`, `wait_for`, `cd`, `select`, `config`,
//! `scroll`, `welcome`, `mark`, `panes`, `toast`, `expect`, `smart`, `edit` / `editor`, `ai`,
//! `ws`, `theme`, `quit`, action names) and hands every other step to the frontend as a
//! [`FrontStep`] (`shot`, `keys`, `size`, and the GUI's pointer / window steps).
//!
//! The frontend loop, once per frame:
//! ```ignore
//! let facts = ScriptFacts { dialog_open, settings_open, focus, screen_text };
//! while let Some(step) = runner.pump(&mut core, &host, &facts) {
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
//!
//! Wave 0 skeleton: the types and signatures; the bodies come with Phase T8 P1.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use soyuz_core::config::{Config, KeyChord};

use crate::core::AppCore;
use crate::host::Host;

/// The environment variable the script comes from.
pub const ENV: &str = "SOYUZ_SCRIPT";

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
/// `{Paste:text}` are clipboard events.
pub fn parse_keys(spec: &str) -> Vec<KeyTok> {
    let _ = spec;
    Vec::new()
}

/// What the frontend knows that the core does not, computed before [`Runner::pump`] (the
/// runner lives inside the frontend's app, so it cannot borrow it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptFacts {
    /// A dialog is open (`expect:dialog=open`).
    pub dialog_open: bool,
    /// The settings window / editor is open (`expect:settings=open`).
    pub settings_open: bool,
    /// The frontend's keyboard focus by name (`expect:focus=<name>`; the TUI's `TuiFocus`
    /// variant name), when it has one.
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
    /// `dropdown`, `settings`, `appearance`, `export_picker`, …), as written.
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
            FrontStep::Other { verb, .. } => matches!(verb.as_str(), "click" | "press" | "move" | "release"),
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
        let _ = (core, host, facts);
        None
    }
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
    fn dotted_config_edit() {
        let c = edited(&Config::default(), "appearance.density=\"touch\"").unwrap();
        assert_eq!(c.appearance.density, soyuz_core::config::Density::Touch);
        let c = edited(&c, "tui.screen_reader=true").unwrap();
        assert!(c.tui.screen_reader);
        assert!(edited(&c, "nope.x=1").is_err());
    }
}
