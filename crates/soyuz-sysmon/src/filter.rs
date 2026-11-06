//! The process filter's one-line query language. All terms must match:
//!
//! - `word`: case-insensitive substring over name, command line, path, user, description and
//!   pid; `"quoted words"` is one term; `a|b` means either; `-word` negates.
//! - `/regex/`, `/regex/i`: a regular expression over the same fields.
//! - `field:value`: substring on one column (`user: name: cmd: path: state: pid: ppid: cgroup: arch:`).
//! - `field>N`, `<`, `>=`, `<=`, `=`: a numeric column, with binary size suffixes (`mem>100M`).
//!
//! Matching allocates nothing except for non-ASCII needles. `cpu` compares what the column
//! shows, so it depends on the scale: see [`Filter::matches_in`].

use std::fmt::Write;
use std::path::Path;

use regex::{Regex, RegexBuilder};
use soyuz_core::config::monitor::CpuScale;

use crate::frame::{ProcRow, state_id};

const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A query that cannot be parsed; shown under the filter box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterError {
    pub message: String,
    /// Byte offset of the offending term in the query.
    pub at: usize,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FilterError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextField {
    User,
    Name,
    Cmd,
    Path,
    State,
    Pid,
    Ppid,
    Cgroup,
    Arch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumField {
    Cpu,
    Mem,
    MemPercent,
    Private,
    Shared,
    Virt,
    Peak,
    Threads,
    Handles,
    Read,
    Write,
    ReadTotal,
    WriteTotal,
    Gpu,
    GpuMem,
    Pid,
    Ppid,
    Nice,
    Faults,
    CpuTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
}

#[derive(Clone, Debug)]
enum Kind {
    /// Any of the alternatives, lowercased, in any text field.
    Text(Vec<String>),
    Regex(Regex),
    Field(TextField, String),
    Num(NumField, Op, f64),
}

#[derive(Clone, Debug)]
struct Term {
    negate: bool,
    kind: Kind,
}

/// A parsed query. The default one matches everything.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    source: String,
    regex_mode: bool,
    terms: Vec<Term>,
}

impl Filter {
    /// `regex_mode`: the whole text is one regular expression (see [`Filter::regex`]).
    pub fn parse(text: &str, regex_mode: bool) -> Result<Filter, FilterError> {
        if regex_mode {
            return Filter::regex(text);
        }
        let mut terms = Vec::new();
        let mut p = Parser { text, pos: 0 };
        while let Some(term) = p.next_term()? {
            terms.push(term);
        }
        Ok(Filter { source: text.to_string(), regex_mode: false, terms })
    }

    /// The whole text as one regular expression. Case-sensitive, like `/x/`; `(?i)` turns that off.
    pub fn regex(text: &str) -> Result<Filter, FilterError> {
        if text.trim().is_empty() {
            return Ok(Filter { source: text.to_string(), regex_mode: true, terms: Vec::new() });
        }
        let re = compile(text, false, 0)?;
        Ok(Filter {
            source: text.to_string(),
            regex_mode: true,
            terms: vec![Term { negate: false, kind: Kind::Regex(re) }],
        })
    }

    /// Matches every process.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The text this was parsed from.
    pub fn text(&self) -> &str {
        &self.source
    }

    pub fn is_regex_mode(&self) -> bool {
        self.regex_mode
    }

    /// `cpu` terms compare `row.cpu_percent`, where 100 is one logical CPU. A table on another
    /// scale uses [`Filter::matches_in`].
    pub fn matches(&self, row: &ProcRow) -> bool {
        self.matches_in(row, 1, CpuScale::PerCore)
    }

    /// Does this process match, with `cpu` terms on the scale the table shows.
    pub fn matches_in(&self, row: &ProcRow, ncpu: u32, scale: CpuScale) -> bool {
        self.terms.iter().all(|t| t.matches(row, ncpu, scale) != t.negate)
    }
}

impl Term {
    fn matches(&self, row: &ProcRow, ncpu: u32, scale: CpuScale) -> bool {
        match &self.kind {
            Kind::Text(alts) => alts.iter().any(|a| any_field(row, |s| contains_ci(s, a))),
            Kind::Regex(re) => any_field(row, |s| re.is_match(s)),
            Kind::Field(field, needle) => field_matches(row, *field, needle),
            Kind::Num(field, op, value) => match number(row, *field, ncpu, scale) {
                Some(v) => match op {
                    Op::Gt => v > *value,
                    Op::Lt => v < *value,
                    Op::Ge => v >= *value,
                    Op::Le => v <= *value,
                    Op::Eq => (v - *value).abs() < f64::EPSILON * value.abs().max(1.0),
                },
                None => false,
            },
        }
    }
}

fn path_text(p: &Path) -> std::borrow::Cow<'_, str> {
    p.to_string_lossy()
}

fn any_field(row: &ProcRow, mut test: impl FnMut(&str) -> bool) -> bool {
    let r = &row.raw;
    if test(&r.name)
        || r.cmdline.as_deref().is_some_and(&mut test)
        || r.exe.as_deref().is_some_and(|p| test(&path_text(p)))
        || r.user.as_deref().is_some_and(&mut test)
        || r.description.as_deref().is_some_and(&mut test)
    {
        return true;
    }
    let mut buf = Decimal::default();
    test(buf.of(r.key.pid))
}

/// A number as text without allocating.
#[derive(Default)]
struct Decimal {
    buf: [u8; 10],
    len: usize,
}

impl Decimal {
    fn of(&mut self, n: u32) -> &str {
        self.len = 0;
        let _ = write!(self, "{n}");
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl Write for Decimal {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len + s.len();
        self.buf.get_mut(self.len..end).ok_or(std::fmt::Error)?.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn field_matches(row: &ProcRow, field: TextField, needle: &str) -> bool {
    let r = &row.raw;
    let mut buf = Decimal::default();
    let hay = |s: Option<&str>| s.is_some_and(|s| contains_ci(s, needle));
    match field {
        TextField::User => hay(r.user.as_deref()),
        TextField::Name => contains_ci(&r.name, needle),
        TextField::Cmd => hay(r.cmdline.as_deref()),
        TextField::Path => r.exe.as_deref().is_some_and(|p| contains_ci(&path_text(p), needle)),
        TextField::State => contains_ci(state_id(r.state), needle),
        TextField::Pid => contains_ci(buf.of(r.key.pid), needle),
        TextField::Ppid => r.parent.is_some_and(|p| contains_ci(buf.of(p), needle)),
        TextField::Cgroup => hay(r.cgroup.as_deref()),
        TextField::Arch => hay(r.arch.as_deref()),
    }
}

fn number(row: &ProcRow, field: NumField, ncpu: u32, scale: CpuScale) -> Option<f64> {
    let r = &row.raw;
    let opt = |v: Option<u64>| v.map(|v| v as f64);
    match field {
        NumField::Cpu => Some(match scale {
            CpuScale::Machine => f64::from(row.cpu_percent) / f64::from(ncpu.max(1)),
            CpuScale::PerCore => f64::from(row.cpu_percent),
        }),
        NumField::Mem => Some(r.rss as f64),
        NumField::MemPercent => Some(f64::from(row.mem_percent)),
        NumField::Private => opt(r.private),
        NumField::Shared => opt(r.shared),
        NumField::Virt => opt(r.virt),
        NumField::Peak => opt(r.peak_rss),
        NumField::Threads => Some(f64::from(r.threads)),
        NumField::Handles => r.handles.map(f64::from),
        NumField::Read => row.read_rate,
        NumField::Write => row.write_rate,
        NumField::ReadTotal => opt(r.read_bytes),
        NumField::WriteTotal => opt(r.write_bytes),
        NumField::Gpu => r.gpu_percent.map(f64::from),
        NumField::GpuMem => opt(r.gpu_mem),
        NumField::Pid => Some(f64::from(r.key.pid)),
        NumField::Ppid => r.parent.map(f64::from),
        NumField::Nice => r.nice.map(f64::from),
        NumField::Faults => opt(r.page_faults),
        NumField::CpuTime => Some(r.cpu_user_ms.saturating_add(r.cpu_system_ms) as f64),
    }
}

/// Case-insensitive substring; `needle` is already lowercase. No allocation for ASCII needles.
pub(crate) fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.is_ascii() {
        let (h, n) = (hay.as_bytes(), needle.as_bytes());
        h.len() >= n.len() && h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
    } else {
        hay.to_lowercase().contains(needle)
    }
}

fn compile(pattern: &str, ignore_case: bool, at: usize) -> Result<Regex, FilterError> {
    RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| FilterError { message: e.to_string(), at })
}

fn text_field(name: &str) -> Option<TextField> {
    Some(match name {
        "user" => TextField::User,
        "name" => TextField::Name,
        "cmd" | "command" => TextField::Cmd,
        "path" | "exe" => TextField::Path,
        "state" => TextField::State,
        "pid" => TextField::Pid,
        "ppid" | "parent" => TextField::Ppid,
        "cgroup" => TextField::Cgroup,
        "arch" => TextField::Arch,
        _ => return None,
    })
}

fn num_field(name: &str) -> Option<NumField> {
    Some(match name {
        "cpu" => NumField::Cpu,
        "mem" | "memory" | "rss" => NumField::Mem,
        "mem%" | "memory_percent" => NumField::MemPercent,
        "private" => NumField::Private,
        "shared" => NumField::Shared,
        "virt" | "virtual" => NumField::Virt,
        "peak" | "peak_memory" => NumField::Peak,
        "threads" => NumField::Threads,
        "handles" | "fds" => NumField::Handles,
        "read" | "disk_read" => NumField::Read,
        "write" | "disk_write" => NumField::Write,
        "read_total" | "disk_read_total" => NumField::ReadTotal,
        "write_total" | "disk_write_total" => NumField::WriteTotal,
        "gpu" => NumField::Gpu,
        "gpu_mem" | "gpu_memory" => NumField::GpuMem,
        "pid" => NumField::Pid,
        "ppid" | "parent" => NumField::Ppid,
        "nice" => NumField::Nice,
        "faults" | "page_faults" => NumField::Faults,
        "cpu_time" => NumField::CpuTime,
        _ => return None,
    })
}

/// `100M`, `1.5g`, `2KiB`, `5%`, `-5`.
fn parse_number(text: &str) -> Option<f64> {
    let t = text.trim();
    let t = t.strip_suffix('%').unwrap_or(t);
    let lower = t.to_ascii_lowercase();
    let body = lower.strip_suffix("ib").or_else(|| lower.strip_suffix('b')).unwrap_or(&lower);
    let (digits, factor) = match body.as_bytes().last()? {
        b'k' => (&body[..body.len() - 1], 1024f64),
        b'm' => (&body[..body.len() - 1], 1024f64.powi(2)),
        b'g' => (&body[..body.len() - 1], 1024f64.powi(3)),
        b't' => (&body[..body.len() - 1], 1024f64.powi(4)),
        _ => (body, 1.0),
    };
    let value: f64 = digits.parse().ok()?;
    value.is_finite().then_some(value * factor)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn next_term(&mut self) -> Result<Option<Term>, FilterError> {
        loop {
            let trimmed = self.rest().trim_start();
            self.pos = self.text.len() - trimmed.len();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let mut negate = false;
            // A lone `-` is a word; `-x` negates x.
            if trimmed.starts_with('-') && trimmed[1..].chars().next().is_some_and(|c| !c.is_whitespace()) {
                negate = true;
                self.pos += 1;
            }
            if let Some(kind) = self.term_kind()? {
                return Ok(Some(Term { negate, kind }));
            }
            // `user:` or `cpu>` while still typing: no term yet.
        }
    }

    fn term_kind(&mut self) -> Result<Option<Kind>, FilterError> {
        let rest = self.rest();
        if rest.starts_with('"') {
            let word = self.quoted();
            return Ok((!word.is_empty()).then(|| Kind::Text(vec![word.to_lowercase()])));
        }
        if rest.starts_with('/')
            && let Some((pattern, ignore_case, len)) = regex_literal(rest)
        {
            let at = self.pos;
            self.pos += len;
            return compile(pattern, ignore_case, at).map(|re| Some(Kind::Regex(re)));
        }

        // A field name is letters and underscores right at the start of the word.
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..word_end];
        let name_len = word.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '%')).unwrap_or(word.len());
        let name = word[..name_len].to_ascii_lowercase();
        let after = &word[name_len..];

        if after.starts_with(':')
            && let Some(field) = text_field(&name)
        {
            self.pos += name_len + 1;
            let value = if self.rest().starts_with('"') {
                self.quoted()
            } else {
                let r = self.rest();
                let end = r.find(char::is_whitespace).unwrap_or(r.len());
                self.pos += end;
                r[..end].to_string()
            };
            return Ok((!value.is_empty()).then(|| Kind::Field(field, value.to_lowercase())));
        }

        let op = [(">=", Op::Ge), ("<=", Op::Le), (">", Op::Gt), ("<", Op::Lt), ("=", Op::Eq)]
            .into_iter()
            .find(|(s, _)| after.starts_with(s));
        if let Some((symbol, op)) = op
            && let Some(field) = num_field(&name)
        {
            let value_text = &after[symbol.len()..];
            let at = self.pos + name_len + symbol.len();
            self.pos += word_end;
            if value_text.is_empty() {
                return Ok(None);
            }
            let Some(value) = parse_number(value_text) else {
                return Err(FilterError { message: format!("\"{value_text}\" is not a number"), at });
            };
            return Ok(Some(Kind::Num(field, op, value)));
        }

        self.pos += word_end;
        let alts: Vec<String> = word.split('|').filter(|a| !a.is_empty()).map(str::to_lowercase).collect();
        Ok((!alts.is_empty()).then_some(Kind::Text(alts)))
    }

    /// Reads `"..."` (an unfinished one runs to the end) and returns its content.
    fn quoted(&mut self) -> String {
        let inner = &self.rest()[1..];
        let (word, used) = match inner.find('"') {
            Some(end) => (&inner[..end], end + 2),
            None => (inner, inner.len() + 1),
        };
        let word = word.to_string();
        self.pos += used;
        word
    }
}

/// `/.../` or `/.../i` closed at a word end; the first such slash closes it. Anything else
/// (`/usr/bin`, `/tmp`) is an ordinary word. Returns the pattern, the flag and the length.
fn regex_literal(text: &str) -> Option<(&str, bool, usize)> {
    let mut escaped = false;
    for (i, c) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '/' {
            let tail = &text[i + 1..];
            let ends = |s: &str| s.is_empty() || s.starts_with(char::is_whitespace);
            if ends(tail) {
                return Some((&text[1..i], false, i + 1));
            }
            if let Some(t) = tail.strip_prefix('i')
                && ends(t)
            {
                return Some((&text[1..i], true, i + 2));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::row;
    use std::sync::Arc;

    fn firefox() -> ProcRow {
        let mut r = row(1234, Some(1), "firefox.exe", 12.0, 200 * 1024 * 1024);
        r.raw.cmdline = Some(Arc::from("firefox.exe --new-tab https://Example.org"));
        r.raw.exe = Some(Arc::from(Path::new("C:/Program Files/Mozilla/firefox.exe")));
        r.raw.user = Some(Arc::from("Alice"));
        r.raw.description = Some(Arc::from("Web Browser"));
        r.raw.threads = 40;
        r.raw.arch = Some(Arc::from("x64"));
        r.raw.gpu_percent = Some(3.0);
        r.read_rate = Some(2.0 * 1024.0 * 1024.0);
        r
    }

    fn check(q: &str, expected: bool) {
        let f = Filter::parse(q, false).unwrap_or_else(|e| panic!("{q}: {e}"));
        assert_eq!(f.matches(&firefox()), expected, "query {q:?}");
    }

    #[test]
    fn empty_matches_everything() {
        check("", true);
        check("   ", true);
        assert!(Filter::default().matches(&firefox()));
        assert!(Filter::parse("", false).unwrap().is_empty());
    }

    #[test]
    fn plain_words_search_every_text_field() {
        check("FIREFOX", true);
        check("example.org", true);
        check("mozilla", true);
        check("alice", true);
        check("browser", true);
        check("1234", true);
        check("chrome", false);
    }

    #[test]
    fn all_terms_must_match() {
        check("firefox alice", true);
        check("firefox bob", false);
    }

    #[test]
    fn quotes_negation_and_alternatives() {
        check("\"web browser\"", true);
        check("\"web  browser\"", false);
        check("-firefox", false);
        check("-chrome", true);
        check("-\"web browser\"", false);
        check("chrome|firefox", true);
        check("chrome|edge", false);
        check("-chrome|edge", true);
        check("\"a|firefox\"", false);
        check("\"unfinished web", false);
        check("-", true); // a lone dash is a word, and "--new-tab" has one
    }

    #[test]
    fn regex_terms() {
        check("/fire.ox/", true);
        check("/FIRE.OX/", false);
        check("/FIRE.OX/i", true);
        check("-/fire.ox/", false);
        check("/web browser/i firefox", true);
        check(r"/a\/b/", false);
    }

    #[test]
    fn slashes_that_are_not_a_regex_are_words() {
        check("/usr/bin", false);
        check("program", true);
        check("files/mozilla", true);
    }

    #[test]
    fn regex_errors_do_not_panic_and_say_where() {
        let e = Filter::parse("firefox /a(/", false).unwrap_err();
        assert_eq!(e.at, 8);
        assert!(!e.message.is_empty());
        assert!(Filter::regex("a(").is_err());
        assert!(Filter::regex(r"\w{1000}{1000}").is_err(), "the size limit stops huge expressions");
    }

    #[test]
    fn whole_query_regex_mode() {
        let f = Filter::parse("fire.*tab", true).unwrap();
        assert!(f.matches(&firefox()));
        assert!(f.is_regex_mode());
        assert!(!Filter::regex("^chrome").unwrap().matches(&firefox()));
        assert!(Filter::regex("").unwrap().is_empty());
    }

    #[test]
    fn field_terms() {
        check("user:ali", true);
        check("user:bob", false);
        check("name:fox", true);
        check("cmd:new-tab", true);
        check("path:mozilla", true);
        check("state:sleep", true);
        check("pid:23", true);
        check("ppid:1", true);
        check("arch:x64", true);
        check("cgroup:x", false);
        check("-user:bob", true);
        check("user:\"ali\"", true);
        check("user:", true);
        check("user: firefox", true);
        check("foo:bar", false);
    }

    #[test]
    fn numeric_terms_and_suffixes() {
        check("cpu>5", true);
        check("cpu>50", false);
        check("mem>100M", true);
        check("mem>100m", true);
        check("mem<1g", true);
        check("mem>=200M", true);
        check("mem<=200MiB", true);
        check("mem=200mb", true);
        check("threads>=40", true);
        check("threads<40", false);
        check("read>1M", true);
        check("gpu>0", true);
        check("pid=1234", true);
        check("nice>0", false);
        check("-nice>0", true);
        check("cpu>", true);
    }

    #[test]
    fn bad_numbers_are_errors() {
        let e = Filter::parse("cpu>abc", false).unwrap_err();
        assert_eq!(e.at, 4);
        assert!(e.message.contains("abc"));
        assert_eq!(Filter::parse("x -cpu>abc", false).unwrap_err().at, 7);
    }

    #[test]
    fn unknown_field_is_a_word() {
        check("foo=1", false);
        check("https://example.org", true);
    }

    #[test]
    fn missing_values_never_match_a_comparison() {
        let mut r = firefox();
        r.raw.gpu_percent = None;
        assert!(!Filter::parse("gpu<5", false).unwrap().matches(&r));
    }

    #[test]
    fn cpu_follows_the_table_scale() {
        let f = Filter::parse("cpu>5", false).unwrap();
        let r = firefox(); // 12 % of one CPU
        assert!(f.matches(&r));
        assert!(f.matches_in(&r, 1, CpuScale::Machine));
        assert!(!f.matches_in(&r, 8, CpuScale::Machine), "1.5 % of an eight-CPU machine");
        assert!(f.matches_in(&r, 8, CpuScale::PerCore));
    }

    #[test]
    fn non_ascii_needle() {
        let mut r = firefox();
        r.raw.name = Arc::from("TÜRKÇE");
        assert!(Filter::parse("türkçe", false).unwrap().matches(&r));
    }

    #[test]
    fn keeps_its_source_text() {
        let f = Filter::parse("cpu>5 x", false).unwrap();
        assert_eq!(f.text(), "cpu>5 x");
        assert!(!f.is_regex_mode());
    }
}
