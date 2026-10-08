//! Batch rename engine (PLAN M10): a stack of rules applied to every selected name, a
//! token language for inserted text (`{n}`, `{name}`, `{ext}`, `{parent}`,
//! `{date:%Y-%m-%d}`, `{exif:…}`), validation of the result (invalid names, duplicate
//! targets, collisions with files outside the batch) and a cycle analysis of the final
//! pairs. Pure logic: no egui, no I/O except through the `exists` callback, so all of it
//! is unit-tested. Execution is a `JobKind::Rename` job, which orders the pairs and
//! breaks cycles (`a→b, b→a`) with temporary names; undo comes from the job journal.

pub mod exif;
pub mod presets;

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::format::{Item as FmtItem, StrftimeItems};
use chrono::{DateTime, Local, NaiveDateTime};
use regex::{NoExpand, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

/// Which part of the name the rules edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// The name without its extension (the extension is kept as is).
    #[default]
    Stem,
    /// Only the extension (without the dot).
    Extension,
    /// The whole name, extension included.
    Whole,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Stem, Target::Whole, Target::Extension];

    pub fn label(self) -> &'static str {
        match self {
            Target::Stem => tr!("dialogs-rename-target-stem"),
            Target::Extension => tr!("dialogs-rename-target-ext"),
            Target::Whole => tr!("dialogs-rename-target-whole"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseMode {
    #[default]
    Lower,
    Upper,
    /// `Every Word Capitalized`.
    Title,
    /// `First letter only`.
    Sentence,
    /// `sWAP cASE`.
    Invert,
}

impl CaseMode {
    pub const ALL: [CaseMode; 5] =
        [CaseMode::Lower, CaseMode::Upper, CaseMode::Title, CaseMode::Sentence, CaseMode::Invert];

    pub fn label(self) -> &'static str {
        match self {
            CaseMode::Lower => tr!("dialogs-rename-case-lower"),
            CaseMode::Upper => tr!("dialogs-rename-case-upper"),
            CaseMode::Title => tr!("dialogs-rename-case-title"),
            CaseMode::Sentence => tr!("dialogs-rename-case-sentence"),
            CaseMode::Invert => tr!("dialogs-rename-case-invert"),
        }
    }
}

/// Where inserted text goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    #[default]
    Start,
    End,
    /// Character index; negative counts from the end (`-1` = before the last char).
    Index(i32),
}

/// One rename rule. Text fields may contain tokens (see [`expand_tokens`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rule {
    /// Find / replace, literal or regex (`$1`, `${name}` refer to capture groups).
    Replace {
        find: String,
        with: String,
        #[serde(default)]
        regex: bool,
        #[serde(default)]
        case_sensitive: bool,
        #[serde(default)]
        first_only: bool,
    },
    Case {
        mode: CaseMode,
    },
    /// Insert text (prefix / suffix / at an index).
    Insert {
        text: String,
        at: Position,
    },
    /// Remove `count` characters starting at `start` (counted from the end when `from_end`).
    Remove {
        start: usize,
        count: usize,
        #[serde(default)]
        from_end: bool,
    },
    /// Insert the counter (see [`Counter`]) with a separator.
    Number {
        at: Position,
        separator: String,
    },
    /// Replace the edited part entirely with a template (`{name}_{n}`).
    Template {
        pattern: String,
    },
    /// Trim surrounding whitespace, optionally collapsing inner runs to one space.
    Trim {
        #[serde(default)]
        collapse: bool,
    },
}

impl Rule {
    pub fn title(&self) -> &'static str {
        match self {
            Rule::Replace { regex: true, .. } => tr!("dialogs-rename-rule-regex"),
            Rule::Replace { .. } => tr!("dialogs-rename-rule-replace"),
            Rule::Case { .. } => tr!("dialogs-rename-rule-case"),
            Rule::Insert { .. } => tr!("dialogs-rename-rule-insert"),
            Rule::Remove { .. } => tr!("dialogs-rename-rule-remove"),
            Rule::Number { .. } => tr!("dialogs-rename-rule-number"),
            Rule::Template { .. } => tr!("dialogs-rename-rule-template"),
            Rule::Trim { .. } => tr!("dialogs-rename-rule-trim"),
        }
    }

    /// Default instances offered by the "Add rule" menu.
    pub fn templates() -> Vec<Rule> {
        vec![
            Rule::Replace {
                find: String::new(),
                with: String::new(),
                regex: false,
                case_sensitive: false,
                first_only: false,
            },
            Rule::Replace {
                find: String::new(),
                with: String::new(),
                regex: true,
                case_sensitive: false,
                first_only: false,
            },
            Rule::Insert { text: String::new(), at: Position::Start },
            Rule::Insert { text: String::new(), at: Position::End },
            Rule::Number { at: Position::End, separator: " ".into() },
            Rule::Remove { start: 0, count: 1, from_end: false },
            Rule::Case { mode: CaseMode::Lower },
            Rule::Template { pattern: "{name}".into() },
            Rule::Trim { collapse: true },
        ]
    }
}

/// A rule in the stack (can be switched off without losing it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleEntry {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub rule: Rule,
}

fn yes() -> bool {
    true
}

impl RuleEntry {
    pub fn new(rule: Rule) -> Self {
        RuleEntry { enabled: true, rule }
    }
}

/// Counter used by `{n}` and [`Rule::Number`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counter {
    pub start: i64,
    pub step: i64,
    /// Minimum digits (zero padded).
    pub pad: usize,
    /// Restart the counter in every folder (search results spanning folders).
    pub per_folder: bool,
}

impl Default for Counter {
    fn default() -> Self {
        Counter { start: 1, step: 1, pad: 2, per_folder: false }
    }
}

/// Everything that defines a rename (also the content of a preset).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Spec {
    pub target: Target,
    pub counter: Counter,
    pub rules: Vec<RuleEntry>,
}

impl Spec {
    /// Whether any enabled rule references EXIF data (so the caller loads it).
    pub fn uses_exif(&self) -> bool {
        self.rules.iter().filter(|r| r.enabled).any(|r| match &r.rule {
            Rule::Replace { with, .. } => with.contains("{exif"),
            Rule::Insert { text, .. } => text.contains("{exif"),
            Rule::Template { pattern } => pattern.contains("{exif"),
            _ => false,
        })
    }
}

/// One item to rename.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub path: PathBuf,
    pub is_dir: bool,
    pub modified: Option<SystemTime>,
    /// EXIF `DateTimeOriginal` (loaded on demand, see [`Spec::uses_exif`]).
    pub exif_date: Option<NaiveDateTime>,
}

impl Item {
    pub fn new(path: PathBuf, is_dir: bool, modified: Option<SystemTime>) -> Self {
        Item { path, is_dir, modified, exif_date: None }
    }

    pub fn name(&self) -> String {
        soyuz_core::entry::display_name(&self.path)
    }
}

/// Split a name into `(stem, extension)`. Folders and dot-files without another dot
/// have no extension (`.gitignore` → `(".gitignore", None)`).
pub fn split_name(name: &str, is_dir: bool) -> (&str, Option<&str>) {
    if is_dir {
        return (name, None);
    }
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], Some(&name[i + 1..])),
        _ => (name, None),
    }
}

/// Per-item values available to tokens.
pub struct TokenCtx<'a> {
    /// Original stem / extension / parent folder name.
    pub name: &'a str,
    pub ext: &'a str,
    pub parent: &'a str,
    pub counter: i64,
    pub pad: usize,
    pub modified: Option<SystemTime>,
    pub exif: Option<NaiveDateTime>,
}

fn format_counter(n: i64, pad: usize) -> String {
    if n < 0 { format!("-{:0pad$}", -n, pad = pad) } else { format!("{n:0pad$}") }
}

/// Validate a strftime pattern (chrono panics on `to_string` of a bad one).
pub fn check_date_format(fmt: &str) -> Result<(), String> {
    if StrftimeItems::new(fmt).any(|i| matches!(i, FmtItem::Error)) {
        Err(tr!("dialogs-rename-err-date-format", format = fmt))
    } else {
        Ok(())
    }
}

fn format_naive(dt: &NaiveDateTime, fmt: &str) -> Result<String, String> {
    check_date_format(fmt)?;
    let mut s = String::new();
    write!(s, "{}", dt.format(fmt)).map_err(|_| tr!("dialogs-rename-err-date-format", format = fmt))?;
    Ok(s)
}

fn local_naive(t: SystemTime) -> NaiveDateTime {
    DateTime::<Local>::from(t).naive_local()
}

/// Default format of `{date}` / `{exif}`.
pub const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d";

/// Expand `{n}`, `{n:3}`, `{name}`, `{ext}`, `{parent}`, `{date}`, `{date:FMT}`,
/// `{exif}`, `{exif:FMT}` (falls back to the modification time); `{{` / `}}` are
/// literal braces. Unknown tokens are an error so typos are visible in the preview.
pub fn expand_tokens(text: &str, t: &TokenCtx<'_>) -> Result<String, String> {
    if !text.contains(['{', '}']) {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let c = rest.as_bytes()[i];
        let after = &rest[i + 1..];
        if c == b'}' {
            if let Some(a) = after.strip_prefix('}') {
                out.push('}');
                rest = a;
                continue;
            }
            return Err(tr!("dialogs-rename-err-unmatched").into());
        }
        if let Some(a) = after.strip_prefix('{') {
            out.push('{');
            rest = a;
            continue;
        }
        let Some(end) = after.find('}') else { return Err(tr!("dialogs-rename-err-unclosed").into()) };
        let token = &after[..end];
        rest = &after[end + 1..];
        let (key, arg) = match token.split_once(':') {
            Some((k, a)) => (k.trim(), Some(a)),
            None => (token.trim(), None),
        };
        match key {
            "n" => {
                let pad = match arg {
                    Some(a) => {
                        a.trim().parse::<usize>().map_err(|_| tr!("dialogs-rename-err-padding", token = token))?.min(32)
                    }
                    None => t.pad,
                };
                out.push_str(&format_counter(t.counter, pad));
            }
            "name" => out.push_str(t.name),
            "ext" => out.push_str(t.ext),
            "parent" => out.push_str(t.parent),
            "date" | "exif" => {
                let fmt = arg.unwrap_or(DEFAULT_DATE_FORMAT);
                let dt = if key == "exif" {
                    t.exif.or_else(|| t.modified.map(local_naive))
                } else {
                    t.modified.map(local_naive)
                };
                match dt {
                    Some(dt) => out.push_str(&format_naive(&dt, fmt)?),
                    None => check_date_format(fmt)?,
                }
            }
            _ => return Err(tr!("dialogs-rename-err-token", token = token)),
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// Rules with their regexes compiled once per preview.
#[derive(Debug)]
pub struct Compiled<'a> {
    spec: &'a Spec,
    regexes: Vec<Option<Regex>>,
}

/// Compile the enabled rules (regex errors are reported with the rule number).
pub fn compile(spec: &Spec) -> Result<Compiled<'_>, String> {
    let mut regexes = Vec::with_capacity(spec.rules.len());
    for (i, r) in spec.rules.iter().enumerate() {
        let re = match &r.rule {
            Rule::Replace { find, regex, case_sensitive, .. } if r.enabled && !find.is_empty() => {
                let pat = if *regex { find.clone() } else { regex::escape(find) };
                let re = RegexBuilder::new(&pat)
                    .case_insensitive(!case_sensitive)
                    .build()
                    .map_err(|e| tr!("dialogs-rename-err-rule", rule = i + 1, error = first_line(&e.to_string())))?;
                Some(re)
            }
            _ => None,
        };
        regexes.push(re);
    }
    Ok(Compiled { spec, regexes })
}

fn first_line(s: &str) -> String {
    s.lines()
        .rfind(|l| l.trim_start().starts_with("error:"))
        .unwrap_or_else(|| s.lines().next().unwrap_or(s))
        .trim()
        .to_string()
}

fn char_index(s: &str, at: usize) -> usize {
    s.char_indices().nth(at).map_or(s.len(), |(i, _)| i)
}

fn insert_at(s: &str, text: &str, at: Position) -> String {
    let n = s.chars().count() as i64;
    let idx = match at {
        Position::Start => 0,
        Position::End => n,
        Position::Index(i) if i < 0 => (n + i as i64).max(0),
        Position::Index(i) => (i as i64).min(n),
    } as usize;
    let b = char_index(s, idx);
    format!("{}{}{}", &s[..b], text, &s[b..])
}

fn change_case(s: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Upper => s.to_uppercase(),
        CaseMode::Invert => s
            .chars()
            .flat_map(|c| -> Vec<char> {
                if c.is_uppercase() { c.to_lowercase().collect() } else { c.to_uppercase().collect() }
            })
            .collect(),
        CaseMode::Sentence => {
            let lower = s.to_lowercase();
            let mut out = String::with_capacity(lower.len());
            let mut done = false;
            for c in lower.chars() {
                if !done && c.is_alphabetic() {
                    out.extend(c.to_uppercase());
                    done = true;
                } else {
                    out.push(c);
                }
            }
            out
        }
        CaseMode::Title => {
            let mut out = String::with_capacity(s.len());
            let mut start = true;
            for c in s.chars() {
                if c.is_alphanumeric() || c == '\'' {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = true;
                }
            }
            out
        }
    }
}

const MASK: char = '\u{E000}';

/// Hide `${…}` capture references from the token expander (they look like tokens).
fn mask_capture_refs(s: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(s.len());
    let mut groups = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        let Some(end) = rest[i..].find('}') else { break };
        out.push_str(&rest[..i]);
        groups.push(rest[i..i + end + 1].to_string());
        out.push(MASK);
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    (out, groups)
}

fn unmask_capture_refs(s: &str, groups: &[String]) -> String {
    let mut it = groups.iter();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match (c, c == MASK) {
            (_, true) => out.push_str(it.next().map_or("", String::as_str)),
            (c, false) => out.push(c),
        }
    }
    out
}

/// `$` in token values must not be read as a capture reference by the regex replacer.
fn escape_dollars(s: &str) -> String {
    s.replace('$', "$$")
}

impl Compiled<'_> {
    fn apply_rules(&self, mut part: String, t: &TokenCtx<'_>) -> Result<String, String> {
        for (i, entry) in self.spec.rules.iter().enumerate() {
            if !entry.enabled {
                continue;
            }
            let num = |e: String| tr!("dialogs-rename-err-rule", rule = i + 1, error = e);
            part = match &entry.rule {
                Rule::Replace { regex, first_only, with, .. } => match &self.regexes[i] {
                    None => part,
                    Some(re) => {
                        let limit = if *first_only { 1 } else { 0 };
                        if *regex {
                            // `$1` typed by the user is a capture reference; a `$` coming
                            // from a token value (a folder named `$work`) is literal.
                            let (n, e, p) = (escape_dollars(t.name), escape_dollars(t.ext), escape_dollars(t.parent));
                            let te = TokenCtx { name: &n, ext: &e, parent: &p, ..*t };
                            // `${1}` / `${name}` are capture references, not tokens.
                            let (masked, groups) = mask_capture_refs(with);
                            let with = unmask_capture_refs(&expand_tokens(&masked, &te).map_err(num)?, &groups);
                            re.replacen(&part, limit, with.as_str()).into_owned()
                        } else {
                            let with = expand_tokens(with, t).map_err(num)?;
                            re.replacen(&part, limit, NoExpand(&with)).into_owned()
                        }
                    }
                },
                Rule::Case { mode } => change_case(&part, *mode),
                Rule::Insert { text, at } => insert_at(&part, &expand_tokens(text, t).map_err(num)?, *at),
                Rule::Remove { start, count, from_end } => {
                    let n = part.chars().count();
                    let (a, b) = if *from_end {
                        let end = n.saturating_sub(*start);
                        (end.saturating_sub(*count), end)
                    } else {
                        ((*start).min(n), (start + count).min(n))
                    };
                    let (ba, bb) = (char_index(&part, a), char_index(&part, b));
                    format!("{}{}", &part[..ba], &part[bb..])
                }
                Rule::Number { at, separator } => {
                    let num = format_counter(t.counter, t.pad);
                    let text = match at {
                        Position::Start => format!("{num}{separator}"),
                        Position::End => format!("{separator}{num}"),
                        Position::Index(_) => num,
                    };
                    insert_at(&part, &text, *at)
                }
                Rule::Template { pattern } => expand_tokens(pattern, t).map_err(num)?,
                Rule::Trim { collapse } => {
                    let s = part.trim();
                    if *collapse { s.split_whitespace().collect::<Vec<_>>().join(" ") } else { s.to_string() }
                }
            };
        }
        Ok(part)
    }

    /// New names for every item, in order (`Err` = a rule failed for that item).
    pub fn apply(&self, items: &[Item]) -> Vec<Result<String, String>> {
        let c = &self.spec.counter;
        let mut per_dir: HashMap<PathBuf, i64> = HashMap::new();
        items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let idx = if c.per_folder {
                    let slot = per_dir.entry(it.path.parent().map(Path::to_path_buf).unwrap_or_default()).or_insert(0);
                    let v = *slot;
                    *slot += 1;
                    v
                } else {
                    i as i64
                };
                let name = it.name();
                let (stem, ext) = split_name(&name, it.is_dir);
                let parent = it.path.parent().map(soyuz_core::entry::display_name).unwrap_or_default();
                let t = TokenCtx {
                    name: stem,
                    ext: ext.unwrap_or(""),
                    parent: &parent,
                    counter: c.start.saturating_add(idx.saturating_mul(c.step)),
                    pad: c.pad.min(32),
                    modified: it.modified,
                    exif: it.exif_date,
                };
                let part = match self.spec.target {
                    Target::Stem => stem.to_string(),
                    Target::Extension => ext.unwrap_or("").to_string(),
                    Target::Whole => name.clone(),
                };
                let out = self.apply_rules(part, &t)?;
                Ok(match self.spec.target {
                    Target::Stem => match ext {
                        Some(e) => format!("{out}.{e}"),
                        None => out,
                    },
                    Target::Extension if out.is_empty() => stem.to_string(),
                    Target::Extension => format!("{stem}.{out}"),
                    Target::Whole => out,
                })
            })
            .collect()
    }
}

/// Result of validating one new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Same name as before (not renamed).
    Unchanged,
    Ok,
    /// Not a valid file name (reason).
    Invalid(String),
    /// Two items would get the same name.
    Duplicate,
    /// A file outside the batch already has this name.
    Exists,
    /// A rule failed (bad token / date format).
    Error(String),
}

impl Status {
    pub fn is_problem(&self) -> bool {
        !matches!(self, Status::Unchanged | Status::Ok)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub new_name: String,
    pub status: Status,
}

/// Comparison key of a path (lower-cased on case-insensitive filesystems).
fn key(p: &Path, case_insensitive: bool) -> String {
    let p = soyuz_fs::strip_verbatim(p);
    let s = p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    if case_insensitive { s.to_lowercase() } else { s }
}

/// Validate new names: invalid characters / reserved names, duplicates inside the
/// batch, and collisions with existing entries that are not themselves renamed away.
pub fn validate(
    items: &[Item],
    names: &[Result<String, String>],
    exists: &dyn Fn(&Path) -> bool,
    windows_rules: bool,
    case_insensitive: bool,
) -> Vec<Row> {
    let targets: Vec<Option<PathBuf>> =
        items.iter().zip(names).map(|(it, n)| n.as_ref().ok().map(|n| it.path.with_file_name(n))).collect();
    // Every name that will exist after the rename (unchanged items keep theirs).
    let mut count: HashMap<String, usize> = HashMap::new();
    for t in targets.iter().flatten() {
        *count.entry(key(t, case_insensitive)).or_default() += 1;
    }
    let sources: HashSet<String> = items.iter().map(|it| key(&it.path, case_insensitive)).collect();
    items
        .iter()
        .zip(names)
        .zip(&targets)
        .map(|((it, n), t)| {
            let new_name = n.clone().unwrap_or_default();
            let status = match (n, t) {
                (Err(e), _) => Status::Error(e.clone()),
                (Ok(name), Some(t)) => {
                    let old = it.name();
                    let tk = key(t, case_insensitive);
                    if *name == old {
                        Status::Unchanged
                    } else if let Err(e) = soyuz_core::validate_name_with(name, windows_rules) {
                        Status::Invalid(e.to_string())
                    } else if count.get(&tk).copied().unwrap_or(0) > 1 {
                        Status::Duplicate
                    } else if tk != key(&it.path, case_insensitive) && !sources.contains(&tk) && exists(t) {
                        Status::Exists
                    } else {
                        Status::Ok
                    }
                }
                (Ok(_), None) => Status::Error(tr!("dialogs-rename-err-no-target").into()),
            };
            Row { new_name, status }
        })
        .collect()
}

/// `(from, to)` pairs of the rows that will actually be renamed.
pub fn pairs(items: &[Item], rows: &[Row]) -> Vec<(PathBuf, PathBuf)> {
    items
        .iter()
        .zip(rows)
        .filter(|(_, r)| r.status == Status::Ok)
        .map(|(it, r)| (it.path.clone(), it.path.with_file_name(&r.new_name)))
        .collect()
}

/// Number of rename cycles in `pairs` (`a→b, b→a` is one cycle; `a→b, b→c` none).
/// The jobs engine breaks each cycle by parking one member under a temporary name.
pub fn count_cycles(pairs: &[(PathBuf, PathBuf)], case_insensitive: bool) -> usize {
    let next: HashMap<String, String> =
        pairs.iter().map(|(a, b)| (key(a, case_insensitive), key(b, case_insensitive))).collect();
    // 0 = unseen, 1 = on the current walk, 2 = finished.
    let mut state: HashMap<&str, u8> = HashMap::new();
    let mut cycles = 0;
    for start in next.keys() {
        if state.get(start.as_str()).copied().unwrap_or(0) != 0 {
            continue;
        }
        let mut walk: Vec<&str> = Vec::new();
        let mut cur = Some(start.as_str());
        while let Some(node) = cur {
            match state.get(node).copied().unwrap_or(0) {
                1 => {
                    cycles += 1;
                    break;
                }
                2 => break,
                _ => {}
            }
            state.insert(node, 1);
            walk.push(node);
            cur = next.get(node).map(String::as_str).filter(|n| next.contains_key(*n) && *n != node);
        }
        for n in walk {
            state.insert(n, 2);
        }
    }
    cycles
}

/// Parse the "edit in text editor" buffer: one name per line, exactly `n` lines
/// (a trailing newline is fine).
pub fn parse_editor(text: &str, n: usize) -> Result<Vec<String>, String> {
    let mut lines: Vec<String> = text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect();
    while lines.len() > n && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.len() != n {
        return Err(tr!("dialogs-rename-err-lines", lines = lines.len(), count = n));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(p: &str) -> Item {
        Item::new(PathBuf::from(p), false, None)
    }

    fn spec(rules: Vec<Rule>) -> Spec {
        Spec { rules: rules.into_iter().map(RuleEntry::new).collect(), ..Spec::default() }
    }

    fn run(s: &Spec, items: &[Item]) -> Vec<String> {
        compile(s).unwrap().apply(items).into_iter().map(|r| r.unwrap()).collect()
    }

    fn base() -> Vec<Item> {
        vec![item("/d/IMG_0001.JPG"), item("/d/IMG_0002.JPG"), item("/d/notes.txt")]
    }

    #[test]
    fn split_names() {
        assert_eq!(split_name("a.tar.gz", false), ("a.tar", Some("gz")));
        assert_eq!(split_name(".gitignore", false), (".gitignore", None));
        assert_eq!(split_name("noext", false), ("noext", None));
        assert_eq!(split_name("dir.d", true), ("dir.d", None));
        assert_eq!(split_name("trailing.", false), ("trailing.", None));
    }

    #[test]
    fn literal_replace_case_insensitive_and_first_only() {
        let s = spec(vec![Rule::Replace {
            find: "img".into(),
            with: "Photo".into(),
            regex: false,
            case_sensitive: false,
            first_only: false,
        }]);
        assert_eq!(run(&s, &base())[0], "Photo_0001.JPG");
        let s = spec(vec![Rule::Replace {
            find: "a".into(),
            with: "$1".into(),
            regex: false,
            case_sensitive: true,
            first_only: true,
        }]);
        assert_eq!(run(&s, &[item("/d/aaa.txt")])[0], "$1aa.txt", "literal replacement never expands");
    }

    #[test]
    fn regex_with_capture_groups() {
        let s = spec(vec![Rule::Replace {
            find: r"IMG_(\d+)".into(),
            with: "${1}_shot".into(),
            regex: true,
            case_sensitive: true,
            first_only: false,
        }]);
        assert_eq!(run(&s, &base())[..2], ["0001_shot.JPG".to_string(), "0002_shot.JPG".to_string()]);
        let bad = spec(vec![Rule::Replace {
            find: "(".into(),
            with: String::new(),
            regex: true,
            case_sensitive: true,
            first_only: false,
        }]);
        assert!(compile(&bad).unwrap_err().starts_with("rule 1"));
    }

    #[test]
    fn regex_token_values_are_literal() {
        let s = spec(vec![Rule::Replace {
            find: "(x)".into(),
            with: "{parent}$1".into(),
            regex: true,
            case_sensitive: true,
            first_only: false,
        }]);
        assert_eq!(run(&s, &[item("/$1/x.txt")])[0], "$1x.txt");
    }

    #[test]
    fn case_modes() {
        let it = [item("/d/hello wORLD-foo.TXT")];
        let c = |m| run(&spec(vec![Rule::Case { mode: m }]), &it)[0].clone();
        assert_eq!(c(CaseMode::Lower), "hello world-foo.TXT");
        assert_eq!(c(CaseMode::Upper), "HELLO WORLD-FOO.TXT");
        assert_eq!(c(CaseMode::Title), "Hello World-Foo.TXT");
        assert_eq!(c(CaseMode::Sentence), "Hello world-foo.TXT");
        assert_eq!(c(CaseMode::Invert), "HELLO World-FOO.TXT");
        let mut s = spec(vec![Rule::Case { mode: CaseMode::Lower }]);
        s.target = Target::Extension;
        assert_eq!(run(&s, &it)[0], "hello wORLD-foo.txt");
        s.target = Target::Whole;
        assert_eq!(run(&s, &it)[0], "hello world-foo.txt");
    }

    #[test]
    fn insert_prefix_suffix_index() {
        let it = [item("/d/abc.txt")];
        let r = |at| run(&spec(vec![Rule::Insert { text: "-".into(), at }]), &it)[0].clone();
        assert_eq!(r(Position::Start), "-abc.txt");
        assert_eq!(r(Position::End), "abc-.txt");
        assert_eq!(r(Position::Index(1)), "a-bc.txt");
        assert_eq!(r(Position::Index(-1)), "ab-c.txt");
        assert_eq!(r(Position::Index(99)), "abc-.txt");
        assert_eq!(r(Position::Index(-99)), "-abc.txt");
    }

    #[test]
    fn remove_range() {
        let it = [item("/d/héllo.txt")];
        let r = |start, count, from_end| run(&spec(vec![Rule::Remove { start, count, from_end }]), &it)[0].clone();
        assert_eq!(r(0, 1, false), "éllo.txt");
        assert_eq!(r(1, 2, false), "hlo.txt");
        assert_eq!(r(0, 2, true), "hél.txt");
        assert_eq!(r(1, 1, true), "hélo.txt");
        assert_eq!(r(10, 3, false), "héllo.txt");
    }

    #[test]
    fn numbering_counter() {
        let mut s = spec(vec![Rule::Number { at: Position::Start, separator: "_".into() }]);
        s.counter = Counter { start: 8, step: 2, pad: 3, per_folder: false };
        assert_eq!(run(&s, &base()), ["008_IMG_0001.JPG", "010_IMG_0002.JPG", "012_notes.txt"]);
        let mut s = spec(vec![Rule::Template { pattern: "{n:1}-{name}".into() }]);
        s.counter = Counter { start: 1, step: 1, pad: 5, per_folder: true };
        let items = [item("/a/x.txt"), item("/b/y.txt"), item("/a/z.txt")];
        assert_eq!(run(&s, &items), ["1-x.txt", "1-y.txt", "2-z.txt"]);
        let mut s = spec(vec![Rule::Template { pattern: "{n}".into() }]);
        s.counter = Counter { start: -1, step: 1, pad: 2, per_folder: false };
        assert_eq!(run(&s, &[item("/a/q"), item("/a/r")]), ["-01", "00"]);
    }

    #[test]
    fn tokens() {
        let t = TokenCtx {
            name: "pic",
            ext: "jpg",
            parent: "Holiday",
            counter: 7,
            pad: 3,
            modified: None,
            exif: NaiveDateTime::parse_from_str("2024-05-06 07:08:09", "%Y-%m-%d %H:%M:%S").ok(),
        };
        assert_eq!(expand_tokens("{parent}-{name}.{ext} #{n}", &t).unwrap(), "Holiday-pic.jpg #007");
        assert_eq!(expand_tokens("{n:1}{{x}}", &t).unwrap(), "7{x}");
        assert_eq!(expand_tokens("{exif}", &t).unwrap(), "2024-05-06");
        assert_eq!(expand_tokens("{exif:%Y%m%d_%H%M}", &t).unwrap(), "20240506_0708");
        assert_eq!(expand_tokens("{date}", &t).unwrap(), "", "no mtime → empty");
        assert!(expand_tokens("{date:%Q}", &t).is_err(), "bad format detected");
        assert!(expand_tokens("{nope}", &t).unwrap_err().contains("unknown token"));
        assert!(expand_tokens("{name", &t).is_err());
        assert!(expand_tokens("a}b", &t).is_err());
    }

    #[test]
    fn date_token_from_mtime() {
        let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let it = Item::new(PathBuf::from("/d/a.txt"), false, Some(mtime));
        let s = spec(vec![Rule::Template { pattern: "{date:%Y}_{name}".into() }]);
        let year = DateTime::<Local>::from(mtime).format("%Y").to_string();
        assert_eq!(run(&s, &[it]), [format!("{year}_a.txt")]);
        let mut it2 = item("/d/b.jpg");
        it2.exif_date = NaiveDateTime::parse_from_str("2001-02-03 04:05:06", "%Y-%m-%d %H:%M:%S").ok();
        let s = spec(vec![Rule::Template { pattern: "{exif}".into() }]);
        assert!(s.uses_exif());
        assert_eq!(run(&s, &[it2]), ["2001-02-03.jpg"]);
    }

    #[test]
    fn trim_and_rule_toggle() {
        let mut s = spec(vec![Rule::Trim { collapse: true }, Rule::Case { mode: CaseMode::Upper }]);
        s.rules[1].enabled = false;
        assert_eq!(run(&s, &[item("/d/  a   b .txt")]), ["a b.txt"]);
    }

    #[test]
    fn extension_target_empty_removes_extension() {
        let mut s = spec(vec![Rule::Template { pattern: String::new() }]);
        s.target = Target::Extension;
        assert_eq!(run(&s, &[item("/d/a.txt")]), ["a"]);
        s.rules = vec![RuleEntry::new(Rule::Template { pattern: "md".into() })];
        assert_eq!(run(&s, &[item("/d/a.txt"), item("/d/noext")]), ["a.md", "noext.md"]);
    }

    #[test]
    fn rule_errors_are_per_item() {
        let s = spec(vec![Rule::Template { pattern: "{bogus}".into() }]);
        let out = compile(&s).unwrap().apply(&[item("/d/a")]);
        assert!(out[0].as_ref().unwrap_err().starts_with("rule 1: unknown token"));
    }

    #[test]
    fn validation() {
        let items = vec![item("/d/a.txt"), item("/d/b.txt"), item("/d/c.txt"), item("/d/d.txt"), item("/d/e.txt")];
        let names: Vec<Result<String, String>> = vec![
            Ok("a.txt".into()),     // unchanged
            Ok("x.txt".into()),     // duplicate with the next
            Ok("x.txt".into()),     // duplicate
            Ok("taken.txt".into()), // exists outside the batch
            Ok("bad/name".into()),  // invalid
        ];
        let exists = |p: &Path| p.ends_with("taken.txt");
        let rows = validate(&items, &names, &exists, false, false);
        assert_eq!(rows[0].status, Status::Unchanged);
        assert_eq!(rows[1].status, Status::Duplicate);
        assert_eq!(rows[2].status, Status::Duplicate);
        assert_eq!(rows[3].status, Status::Exists);
        assert!(matches!(rows[4].status, Status::Invalid(_)));
        assert!(pairs(&items, &rows).is_empty());
        let rows = validate(&items[..1], &[Ok("CON.txt".into())], &|_| false, true, true);
        assert!(matches!(rows[0].status, Status::Invalid(_)), "windows reserved name");
        let rows = validate(&items[..1], &[Err("rule 1: x".into())], &|_| false, true, true);
        assert!(matches!(rows[0].status, Status::Error(_)));
    }

    #[test]
    fn validation_swaps_and_case_only() {
        // a→b and b→a: both targets exist on disk, but they are vacated by the batch.
        let items = vec![item("/d/a"), item("/d/b")];
        let names = vec![Ok("b".to_string()), Ok("a".to_string())];
        let rows = validate(&items, &names, &|_| true, false, false);
        assert!(rows.iter().all(|r| r.status == Status::Ok), "{rows:?}");
        let p = pairs(&items, &rows);
        assert_eq!(p.len(), 2);
        assert_eq!(count_cycles(&p, false), 1);
        // Case-only change on a case-insensitive fs: the "existing" file is itself.
        let rows = validate(&items[..1], &[Ok("A".to_string())], &|_| true, false, true);
        assert_eq!(rows[0].status, Status::Ok);
        // An unchanged item keeps its name: renaming another onto it is a duplicate.
        let rows = validate(&items, &[Ok("a".into()), Ok("A".into())], &|_| true, false, true);
        assert_eq!(rows[1].status, Status::Duplicate);
    }

    #[test]
    fn cycles_plan() {
        let p = |v: &[(&str, &str)]| -> Vec<(PathBuf, PathBuf)> {
            v.iter().map(|(a, b)| (PathBuf::from(a), PathBuf::from(b))).collect()
        };
        assert_eq!(count_cycles(&p(&[("/a", "/b"), ("/b", "/c")]), false), 0, "a chain is ordered, no cycle");
        assert_eq!(count_cycles(&p(&[("/a", "/b"), ("/b", "/a")]), false), 1);
        assert_eq!(count_cycles(&p(&[("/a", "/b"), ("/b", "/c"), ("/c", "/a"), ("/x", "/y"), ("/y", "/x")]), false), 2);
        assert_eq!(count_cycles(&p(&[("/q", "/a"), ("/a", "/b"), ("/b", "/a")]), false), 1, "tail into a cycle");
        assert_eq!(count_cycles(&p(&[("/A", "/b"), ("/B", "/a")]), true), 1, "case-insensitive keys");
        assert_eq!(count_cycles(&p(&[("/A", "/b"), ("/B", "/a")]), false), 0);
        // A large rotation (5k files, every name shifted by one) is a single cycle.
        let n = 5000;
        let big: Vec<(PathBuf, PathBuf)> = (0..n)
            .map(|i| (PathBuf::from(format!("/d/f{i}")), PathBuf::from(format!("/d/f{}", (i + 1) % n))))
            .collect();
        assert_eq!(count_cycles(&big, false), 1);
    }

    #[test]
    fn editor_mode() {
        assert_eq!(parse_editor("a\nb\n", 2).unwrap(), ["a", "b"]);
        assert_eq!(parse_editor("a\r\nb", 2).unwrap(), ["a", "b"]);
        assert_eq!(parse_editor("a\n", 2).unwrap(), ["a", ""]);
        assert!(parse_editor("a", 2).is_err());
        assert!(parse_editor("a\nb\nc", 2).is_err());
        assert_eq!(parse_editor("\n", 1).unwrap(), [""], "an empty line is an (invalid) empty name");
    }

    #[test]
    fn spec_round_trips_through_toml() {
        let s = Spec {
            target: Target::Whole,
            counter: Counter { start: 3, step: 5, pad: 4, per_folder: true },
            rules: vec![
                RuleEntry::new(Rule::Replace {
                    find: "a".into(),
                    with: "b".into(),
                    regex: true,
                    case_sensitive: true,
                    first_only: true,
                }),
                RuleEntry { enabled: false, rule: Rule::Insert { text: "x".into(), at: Position::Index(-2) } },
                RuleEntry::new(Rule::Case { mode: CaseMode::Title }),
            ],
        };
        let text = toml::to_string(&s).unwrap();
        let back: Spec = toml::from_str(&text).unwrap();
        assert_eq!(back, s);
    }
}
