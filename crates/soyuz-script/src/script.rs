//! Loading scripts and calling into them.
//!
//! A script is one `*.rhai` file. Its top level runs once at load time (sandboxed, with
//! a deadline) and may only call `register_*`; the registered functions run later:
//! actions and rule functions on a worker thread with write planning, columns and
//! previewers on worker threads read-only, `when` conditions with no I/O at all.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rhai::{AST, Array, CallFnOptions, Dynamic, Engine, FnPtr, Map, Scope};
use soyuz_core::EntryInfo;

use crate::sandbox::{self, Limits, Mode, ScriptError};
use crate::{Outcome, Registry};

/// Largest script file loaded.
const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

/// Where a context-menu entry shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuContext {
    File,
    Dir,
    Background,
}

/// A custom action (palette, key binding, optional context-menu entry).
#[derive(Clone)]
pub struct ActionDef {
    /// Unique inside its script; the full id is `<script>/<id>`.
    pub id: String,
    pub title: String,
    pub description: String,
    /// Default chord (`"Ctrl+Alt+N"`); `[keys.bindings] "run_script:<full id>"` overrides.
    pub hotkey: String,
    /// Show in the right-click menu (when the conditions hold).
    pub menu: bool,
    /// Show in the command palette.
    pub palette: bool,
    /// Menu contexts (empty = files and folders).
    pub contexts: Vec<MenuContext>,
    /// Only for items with these extensions (lower-case, empty = any).
    pub extensions: Vec<String>,
    pub(crate) when: Option<FnPtr>,
    pub(crate) run: FnPtr,
}

impl std::fmt::Debug for ActionDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionDef").field("id", &self.id).field("title", &self.title).finish_non_exhaustive()
    }
}

impl ActionDef {
    pub fn has_condition(&self) -> bool {
        self.when.is_some()
    }
}

/// A column provider.
#[derive(Clone)]
pub struct ColumnDef {
    pub id: String,
    pub title: String,
    pub width: f32,
    /// Only computed for files with these extensions (empty = every entry).
    pub extensions: Vec<String>,
    pub(crate) value: FnPtr,
}

/// A previewer for some file types.
#[derive(Clone)]
pub struct PreviewerDef {
    pub id: String,
    pub title: String,
    pub extensions: Vec<String>,
    pub(crate) preview: FnPtr,
}

/// What a previewer returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Markdown(String),
    Text(String),
}

/// One `*.rhai` file.
#[derive(Clone)]
pub struct Script {
    /// File name (`tidy.rhai`).
    pub file: String,
    /// File stem (`tidy`): the prefix of its action ids.
    pub name: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub modified: Option<SystemTime>,
    pub error: Option<ScriptError>,
    /// Defines `fn rule(path, ctx)` (usable as an automation step).
    pub has_rule: bool,
    pub actions: Vec<ActionDef>,
    pub columns: Vec<ColumnDef>,
    pub previewers: Vec<PreviewerDef>,
    pub(crate) ast: Option<Arc<AST>>,
}

impl Script {
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }

    /// Full id of one of its actions.
    pub fn action_id(&self, a: &ActionDef) -> String {
        format!("{}/{}", self.name, a.id)
    }
}

/// Context of an action call (`ctx` in scripts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionCtx {
    pub dir: Option<PathBuf>,
    pub focused: Option<PathBuf>,
    pub selected: Vec<PathBuf>,
    pub other_dir: Option<PathBuf>,
}

fn s(p: &Path) -> Dynamic {
    soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned().into()
}

impl ActionCtx {
    fn to_map(&self) -> Map {
        let mut m = Map::new();
        let opt = |p: &Option<PathBuf>| p.as_deref().map(s).unwrap_or(Dynamic::UNIT);
        m.insert("dir".into(), opt(&self.dir));
        m.insert("focused".into(), opt(&self.focused));
        m.insert("other_dir".into(), opt(&self.other_dir));
        let sel: Array = self.selected.iter().map(|p| s(p)).collect();
        m.insert("count".into(), (sel.len() as i64).into());
        m.insert("selected".into(), sel.into());
        m
    }
}

/// All scripts of the scripts folder, compiled, sharing one sandboxed engine.
pub struct ScriptSet {
    engine: Arc<Engine>,
    pub limits: Limits,
    pub dir: PathBuf,
    pub scripts: Vec<Script>,
}

impl std::fmt::Debug for ScriptSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptSet").field("dir", &self.dir).field("scripts", &self.scripts.len()).finish()
    }
}

/// Cheap change signature of a scripts folder: `(file, modified, len)` sorted.
pub fn signature(dir: &Path) -> Vec<(String, Option<SystemTime>, u64)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<_> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.to_ascii_lowercase().ends_with(".rhai") {
                return None;
            }
            let m = e.metadata().ok()?;
            m.is_file().then(|| (name, m.modified().ok(), m.len()))
        })
        .collect();
    v.sort();
    v
}

impl ScriptSet {
    /// An empty set (scripting off).
    pub fn empty(limits: Limits) -> Self {
        ScriptSet { engine: Arc::new(sandbox::engine(&limits)), limits, dir: PathBuf::new(), scripts: Vec::new() }
    }

    /// The same set, remembering `dir` as its folder.
    pub fn with_dir(mut self, dir: &Path) -> Self {
        self.dir = dir.to_path_buf();
        self
    }

    /// Load every `*.rhai` in `dir` (missing folder = no scripts). `disabled` holds file
    /// names that are listed but not run.
    pub fn load_dir(dir: &Path, limits: Limits, disabled: &[String]) -> Self {
        let engine = Arc::new(sandbox::engine(&limits));
        let mut scripts = Vec::new();
        for (file, modified, len) in signature(dir) {
            let path = dir.join(&file);
            let enabled = !disabled.iter().any(|d| d.eq_ignore_ascii_case(&file));
            let text = if len > MAX_SCRIPT_BYTES {
                Err(ScriptError::msg(format!("the script is larger than {} KiB", MAX_SCRIPT_BYTES / 1024)))
            } else {
                std::fs::read_to_string(&path).map_err(|e| ScriptError::msg(format!("cannot read: {e}")))
            };
            let mut sc = match text {
                Ok(t) if enabled => compile(&engine, &limits, &file, &t),
                Ok(_) => Script::blank(&file),
                Err(e) => Script { error: Some(e), ..Script::blank(&file) },
            };
            sc.path = path;
            sc.enabled = enabled;
            sc.modified = modified;
            scripts.push(sc);
        }
        ScriptSet { engine, limits, dir: dir.to_path_buf(), scripts }
    }

    /// Compile one script from source (tests, "check" in settings).
    pub fn from_sources(sources: &[(&str, &str)], limits: Limits) -> Self {
        let engine = Arc::new(sandbox::engine(&limits));
        let scripts = sources.iter().map(|(file, text)| compile(&engine, &limits, file, text)).collect();
        ScriptSet { engine, limits, dir: PathBuf::new(), scripts }
    }

    /// Enabled scripts that loaded.
    pub fn active(&self) -> impl Iterator<Item = &Script> {
        self.scripts.iter().filter(|s| s.enabled && s.ast.is_some())
    }

    /// `(full id, script, action)` of every action.
    pub fn actions(&self) -> impl Iterator<Item = (String, &Script, &ActionDef)> {
        self.active().flat_map(|s| s.actions.iter().map(move |a| (s.action_id(a), s, a)))
    }

    pub fn action(&self, full_id: &str) -> Option<(&Script, &ActionDef)> {
        let (script, id) = full_id.split_once('/')?;
        let sc = self.active().find(|s| s.name.eq_ignore_ascii_case(script))?;
        sc.actions.iter().find(|a| a.id == id).map(|a| (sc, a))
    }

    /// Column providers in slot order (`Column::Script1..4`).
    pub fn columns(&self) -> Vec<(&Script, &ColumnDef)> {
        self.active().flat_map(|s| s.columns.iter().map(move |c| (s, c))).collect()
    }

    pub fn previewer_for(&self, ext: &str) -> Option<(&Script, &PreviewerDef)> {
        let ext = ext.to_ascii_lowercase();
        self.active().flat_map(|s| s.previewers.iter().map(move |p| (s, p))).find(|(_, p)| p.extensions.contains(&ext))
    }

    /// Scripts defining `fn rule(path, ctx)`.
    pub fn rule_script(&self, name: &str) -> Option<&Script> {
        let name = name.trim_end_matches(".rhai");
        self.active().find(|s| s.name.eq_ignore_ascii_case(name) && s.has_rule)
    }

    fn call(&self, sc: &Script, f: &FnPtr, mode: Mode, timeout: Duration, args: Vec<Dynamic>) -> CallResult {
        let ast = sc.ast.clone().ok_or_else(|| ScriptError::msg("the script did not load"))?;
        let (r, st) = sandbox::with_state(mode, &self.limits, timeout, || f.call::<Dynamic>(&self.engine, &ast, args));
        match r {
            Ok(v) => Ok((v, st.out)),
            Err(e) => Err(ScriptError::from_eval(&e)),
        }
    }

    /// Run an action: returns the planned jobs and UI operations.
    pub fn run_action(&self, full_id: &str, ctx: &ActionCtx) -> Result<Outcome, ScriptError> {
        let (sc, a) = self.action(full_id).ok_or_else(|| ScriptError::msg(format!("no script action “{full_id}”")))?;
        let (_, out) = self.call(sc, &a.run, Mode::Full, self.limits.timeout, vec![ctx.to_map().into()])?;
        Ok(out)
    }

    /// Whether the action's menu entry shows for `ctx` (`kinds` = what is selected,
    /// `exts` = extensions of the selected files). Conditions that fail count as false.
    pub fn applies(&self, full_id: &str, ctx: &ActionCtx, kinds: &[MenuContext], exts: &[String]) -> bool {
        let Some((sc, a)) = self.action(full_id) else { return false };
        let contexts: &[MenuContext] =
            if a.contexts.is_empty() { &[MenuContext::File, MenuContext::Dir] } else { &a.contexts };
        if !kinds.iter().any(|k| contexts.contains(k)) {
            return false;
        }
        if !a.extensions.is_empty()
            && (exts.is_empty() || !exts.iter().all(|e| a.extensions.contains(&e.to_ascii_lowercase())))
        {
            return false;
        }
        let Some(when) = &a.when else { return true };
        match self.call(sc, when, Mode::Cond, Duration::from_millis(20), vec![ctx.to_map().into()]) {
            Ok((v, _)) => v.as_bool().unwrap_or(false),
            Err(e) => {
                log::debug!("{full_id}: `when` failed: {e}");
                false
            }
        }
    }

    /// Value of column slot `slot` for `e` (`Ok(None)`: no provider / not applicable).
    pub fn column_value(&self, slot: usize, e: &EntryInfo) -> Result<Option<String>, ScriptError> {
        let cols = self.columns();
        let Some((sc, c)) = cols.get(slot) else { return Ok(None) };
        if !c.extensions.is_empty() && !e.extension().is_some_and(|x| c.extensions.contains(&x.to_ascii_lowercase())) {
            return Ok(None);
        }
        let arg = Dynamic::from_map(crate::api::entry_map(e));
        let (v, _) = self.call(sc, &c.value, Mode::Pure, self.limits.cell_timeout, vec![arg])?;
        Ok(if v.is_unit() { None } else { Some(v.to_string().chars().take(500).collect()) })
    }

    /// Run the previewer registered for `path`'s extension (`Ok(None)`: none, or it
    /// returned `()` to fall back to the built-in preview).
    pub fn preview(&self, path: &Path) -> Result<Option<Preview>, ScriptError> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(ext) = crate::api::name_ext(&name) else { return Ok(None) };
        let Some((sc, p)) = self.previewer_for(ext) else { return Ok(None) };
        let arg = s(path);
        let timeout = self.limits.cell_timeout.max(Duration::from_millis(1500));
        let (v, _) = self.call(sc, &p.preview, Mode::Pure, timeout, vec![arg])?;
        Ok(preview_of(v))
    }

    /// Run `fn rule(path, ctx)` of `script` for an automation step.
    pub fn run_rule(&self, script: &str, path: &Path, rule_name: &str) -> Result<Outcome, ScriptError> {
        let sc = self
            .rule_script(script)
            .ok_or_else(|| ScriptError::msg(format!("no script “{script}” with a `fn rule(path, ctx)`")))?;
        let ast = sc.ast.clone().ok_or_else(|| ScriptError::msg("the script did not load"))?;
        let mut ctx = Map::new();
        ctx.insert("rule".into(), rule_name.to_string().into());
        let (r, st) = sandbox::with_state(Mode::Full, &self.limits, self.limits.timeout, || {
            let mut scope = Scope::new();
            let opts = CallFnOptions::new().eval_ast(false).rewind_scope(true);
            self.engine.call_fn_with_options::<Dynamic>(
                opts,
                &mut scope,
                &ast,
                "rule",
                (s(path), Dynamic::from_map(ctx)),
            )
        });
        let _ = r.map_err(|e| ScriptError::from_eval(&e))?;
        Ok(st.out)
    }
}

type CallResult = Result<(Dynamic, Outcome), ScriptError>;

fn preview_of(v: Dynamic) -> Option<Preview> {
    if v.is_unit() {
        return None;
    }
    if v.is_map() {
        let m = v.cast::<Map>();
        if let Some(md) = m.get("markdown") {
            return Some(Preview::Markdown(md.to_string()));
        }
        if let Some(t) = m.get("text") {
            return Some(Preview::Text(t.to_string()));
        }
        return None;
    }
    Some(Preview::Markdown(v.to_string()))
}

impl Script {
    fn blank(file: &str) -> Self {
        Script {
            file: file.to_string(),
            name: file.strip_suffix(".rhai").unwrap_or(file).to_string(),
            path: PathBuf::from(file),
            enabled: true,
            modified: None,
            error: None,
            has_rule: false,
            actions: Vec::new(),
            columns: Vec::new(),
            previewers: Vec::new(),
            ast: None,
        }
    }
}

fn compile(engine: &Engine, limits: &Limits, file: &str, text: &str) -> Script {
    let mut sc = Script::blank(file);
    let ast = match engine.compile(text) {
        Ok(a) => a,
        Err(e) => {
            sc.error = Some(ScriptError::from_parse(&e));
            return sc;
        }
    };
    let (r, st) = sandbox::with_state(Mode::Load, limits, limits.load_timeout, || engine.run_ast(&ast));
    if let Err(e) = r {
        sc.error = Some(ScriptError::from_eval(&e));
        return sc;
    }
    let Registry { actions, columns, previewers } = st.reg;
    sc.has_rule = ast.iter_functions().any(|f| f.name == "rule" && f.params.len() == 2);
    if actions.is_empty() && columns.is_empty() && previewers.is_empty() && !sc.has_rule {
        sc.error = Some(ScriptError::msg(
            "the script registers nothing (call register_action / register_column / register_previewer, or define fn rule(path, ctx))",
        ));
    }
    sc.actions = actions;
    sc.columns = columns;
    sc.previewers = previewers;
    sc.ast = Some(Arc::new(ast));
    sc
}
