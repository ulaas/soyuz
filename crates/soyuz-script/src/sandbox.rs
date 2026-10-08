//! The sandboxed Rhai engine: limits, per-call state and error positions.
//!
//! Rhai itself has no file, process or network access; the only way out is the API in
//! [`crate::api`]. Read functions check [`Limits::allow_read`] and their caps; write
//! functions never touch the disk, they append [`crate::PlannedJob`]s that the host
//! submits through the jobs engine. `eval` is disabled and `import` does not exist
//! (`no_module`). Every call runs with an operation limit and a wall-clock deadline.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use rhai::{Dynamic, Engine, EvalAltResult, Position};

use crate::{Outcome, Registry};

/// Sandbox limits of one engine / call.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub max_operations: u64,
    /// Deadline of action / rule calls.
    pub timeout: Duration,
    /// Deadline of column value and previewer calls.
    pub cell_timeout: Duration,
    /// Deadline of loading (evaluating the top level of) a script.
    pub load_timeout: Duration,
    pub allow_read: bool,
    pub max_read_bytes: u64,
    pub max_list_entries: usize,
    pub max_string_bytes: usize,
    pub max_array_len: usize,
    pub max_map_len: usize,
    pub max_call_depth: usize,
    /// Planned jobs one call may produce.
    pub max_jobs: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_operations: 5_000_000,
            timeout: Duration::from_secs(3),
            cell_timeout: Duration::from_millis(250),
            load_timeout: Duration::from_secs(2),
            allow_read: true,
            max_read_bytes: 1024 * 1024,
            max_list_entries: 10_000,
            max_string_bytes: 8 * 1024 * 1024,
            max_array_len: 200_000,
            max_map_len: 20_000,
            max_call_depth: 48,
            max_jobs: 1_000,
        }
    }
}

impl Limits {
    /// From `[scripting]`.
    pub fn from_config(c: &soyuz_core::config::Scripting) -> Self {
        Limits {
            max_operations: c.max_operations.max(10_000),
            timeout: Duration::from_millis(c.timeout_ms.clamp(50, 600_000)),
            cell_timeout: Duration::from_millis(c.cell_timeout_ms.clamp(10, 60_000)),
            allow_read: c.allow_read,
            max_read_bytes: c.max_read_kib.clamp(1, 1024 * 1024) * 1024,
            max_list_entries: c.max_list_entries.clamp(10, 1_000_000),
            ..Limits::default()
        }
    }
}

/// What the running code may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Evaluating the top level: only `register_*` calls (and pure code).
    Load,
    /// Context-menu `when` conditions: no reads (they run while a menu is drawn).
    Cond,
    /// Column values and previewers: reads only.
    Pure,
    /// Actions and rule scripts: reads, planned writes, UI operations.
    Full,
}

pub(crate) struct CallState {
    pub mode: Mode,
    pub deadline: Instant,
    pub limits: Limits,
    pub out: Outcome,
    pub reg: Registry,
}

thread_local! {
    static STATE: RefCell<Option<CallState>> = const { RefCell::new(None) };
}

/// Run `f` with a fresh call state; returns `f`'s result and the state afterwards.
pub(crate) fn with_state<R>(mode: Mode, limits: &Limits, timeout: Duration, f: impl FnOnce() -> R) -> (R, CallState) {
    let state = CallState {
        mode,
        deadline: Instant::now() + timeout,
        limits: limits.clone(),
        out: Outcome::default(),
        reg: Registry::default(),
    };
    let prev = STATE.with(|s| s.borrow_mut().replace(state));
    let r = f();
    let state = STATE.with(|s| {
        let mut b = s.borrow_mut();
        let cur = b.take();
        *b = prev;
        cur
    });
    (r, state.expect("call state"))
}

/// Access the current call state (`None` outside a call).
pub(crate) fn state<R>(f: impl FnOnce(&mut CallState) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

pub(crate) type ApiResult<T> = Result<T, Box<EvalAltResult>>;

/// A runtime error raised by an API function.
pub(crate) fn fail<T>(msg: impl Into<String>) -> ApiResult<T> {
    Err(Box::new(EvalAltResult::ErrorRuntime(Dynamic::from(msg.into()), Position::NONE)))
}

/// Require at least `mode` for the current call.
pub(crate) fn require(what: &str, need: Mode) -> ApiResult<()> {
    let mode = state(|s| s.mode).unwrap_or(Mode::Load);
    let ok = match need {
        Mode::Load => mode == Mode::Load,
        Mode::Cond => mode != Mode::Load,
        Mode::Pure => matches!(mode, Mode::Pure | Mode::Full),
        Mode::Full => mode == Mode::Full,
    };
    if ok {
        return Ok(());
    }
    match (need, mode) {
        (Mode::Load, _) => fail(format!("{what}() only works at the top level of a script")),
        (_, Mode::Load) => fail(format!("{what}() cannot run while the script loads; call it from a function")),
        (_, Mode::Cond) => fail(format!("{what}() is not allowed in a `when` condition (it only sees the context)")),
        _ => fail(format!("{what}() is not allowed here (columns, previewers and conditions are read-only)")),
    }
}

/// Build a sandboxed engine with the Soyuz API.
pub fn engine(limits: &Limits) -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(limits.max_operations);
    e.set_max_call_levels(limits.max_call_depth);
    e.set_max_expr_depths(64, 48);
    e.set_max_string_size(limits.max_string_bytes);
    e.set_max_array_size(limits.max_array_len);
    e.set_max_map_size(limits.max_map_len);
    e.set_max_variables(10_000);
    e.set_max_functions(10_000);
    e.disable_symbol("eval");
    e.on_progress(|ops| {
        if ops % 512 != 0 {
            return None;
        }
        let late = state(|s| Instant::now() > s.deadline).unwrap_or(false);
        late.then(|| Dynamic::from("time limit"))
    });
    e.on_print(|s| {
        let line: String = s.chars().take(2000).collect();
        log::info!("script: {line}");
        state(|st| {
            if st.out.log.len() < 1000 {
                st.out.log.push(line)
            }
        });
    });
    e.on_debug(|s, _src, pos| {
        let line: String = s.chars().take(2000).collect();
        log::debug!("script debug {pos}: {line}");
        state(|st| {
            if st.out.log.len() < 1000 {
                st.out.log.push(line)
            }
        });
    });
    crate::api::register(&mut e);
    e
}

/// A script error with its position (1-based line / column).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub message: String,
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.col) {
            (Some(l), Some(c)) => write!(f, "line {l}, column {c}: {}", self.message),
            (Some(l), None) => write!(f, "line {l}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

impl ScriptError {
    pub fn msg(m: impl Into<String>) -> Self {
        ScriptError { message: m.into(), line: None, col: None }
    }

    fn at(message: String, pos: Position) -> Self {
        ScriptError { message, line: pos.line(), col: pos.position() }
    }

    pub(crate) fn from_parse(e: &rhai::ParseError) -> Self {
        ScriptError::at(e.err_type().to_string(), e.position())
    }

    /// Runtime error: the innermost error (inside nested function calls) with its position.
    pub(crate) fn from_eval(e: &EvalAltResult) -> Self {
        let mut cur = e;
        let mut outer_pos = e.position();
        while let EvalAltResult::ErrorInFunctionCall(_, _, inner, pos) = cur {
            if !pos.is_none() {
                outer_pos = *pos;
            }
            cur = inner;
        }
        let message = match cur {
            EvalAltResult::ErrorTerminated(..) => "the script ran longer than its time limit".to_string(),
            EvalAltResult::ErrorTooManyOperations(..) => "the script ran too many operations".to_string(),
            EvalAltResult::ErrorRuntime(v, _) => v.to_string(),
            other => {
                let s = other.to_string();
                // Rhai appends " (line x, position y)"; we report the position separately.
                match s.rfind(" (line ") {
                    Some(i) => s[..i].to_string(),
                    None => s,
                }
            }
        };
        let pos = if cur.position().is_none() { outer_pos } else { cur.position() };
        ScriptError::at(message, pos)
    }
}
