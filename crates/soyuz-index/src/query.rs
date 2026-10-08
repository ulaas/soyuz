//! The search query language.
//!
//! ```text
//! report kind:image size:>5MB modified:<7d tag:red ext:pdf,docx path:docs "exact phrase"
//! -draft  (foo OR bar)  in:"C:\Users\me\Projects"  content:todo  regex:^IMG_\d+
//! ```
//!
//! * Bare words and `"quoted phrases"` match the file **name** (case-insensitive
//!   substring). Words containing `*`, `?` or `[` are name globs.
//! * Terms are combined with AND; `OR` (or `|`) binds tighter than the implicit AND, so
//!   `a b OR c` means `a AND (b OR c)`. Parentheses group, a leading `-` negates a term or
//!   a group.
//! * `key:value` filters (values may be quoted): `name:`, `path:`, `in:` / `under:`,
//!   `ext:`, `kind:` / `type:`, `size:`, `modified:` / `date:`, `created:`, `tag:`,
//!   `content:` / `text:`, `regex:`. Unknown keys are plain name words (so `C:\x` or
//!   `a:b` still search names).
//!
//! Sizes: `>5MB`, `>=1k`, `<16KiB`, `=0`, `10MB..1GB`, `empty` (binary multiples).
//! Times: relative ages `<7d` (newer than 7 days), `>1y` (older than a year), `2h`,
//! `30min`, `3w`, `6mo`; dates `2026-01-31`, `2026-01`, `2026` with `<` (before), `>`
//! (after), `..` ranges; keywords `today`, `yesterday`, `week`, `month`, `year`.

use std::path::PathBuf;

/// A parsed query.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// All must match; an empty `And` matches everything.
    And(Vec<Expr>),
    Or(Vec<Expr>),
    Not(Box<Expr>),
    Term(Term),
}

/// One filter.
#[derive(Clone, Debug, PartialEq)]
pub enum Term {
    /// Name contains the text (case-insensitive).
    Name(String),
    /// Name matches the glob (case-insensitive; `*`, `?`, `[...]`).
    NameGlob(String),
    /// Name matches the regular expression (case-insensitive unless it says otherwise).
    Regex(String),
    /// Full path contains the text (case-insensitive).
    Path(String),
    /// Below this folder (not the folder itself).
    In(PathBuf),
    /// Extension is one of these (lower-case, no dot).
    Ext(Vec<String>),
    Kind(KindClass),
    /// Files only; half-open byte range.
    Size(Range),
    Modified(TimeSpec),
    Created(TimeSpec),
    /// Carries a color tag (lower-case slug, e.g. `red`).
    Tag(String),
    /// File contents contain these words (index: FTS5 phrase; live walk: substring).
    Content(String),
}

/// Kind classes for `kind:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KindClass {
    File,
    Folder,
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Code,
}

impl KindClass {
    pub const ALL: [KindClass; 8] = [
        KindClass::File,
        KindClass::Folder,
        KindClass::Image,
        KindClass::Video,
        KindClass::Audio,
        KindClass::Document,
        KindClass::Archive,
        KindClass::Code,
    ];

    pub fn parse(s: &str) -> Option<KindClass> {
        Some(match s.to_ascii_lowercase().as_str() {
            "file" | "files" => KindClass::File,
            "folder" | "folders" | "dir" | "dirs" | "directory" => KindClass::Folder,
            "image" | "images" | "picture" | "pictures" | "photo" | "photos" | "pic" => KindClass::Image,
            "video" | "videos" | "movie" | "movies" => KindClass::Video,
            "audio" | "music" | "sound" => KindClass::Audio,
            "doc" | "docs" | "document" | "documents" => KindClass::Document,
            "archive" | "archives" | "zip" | "compressed" => KindClass::Archive,
            "code" | "source" | "src" => KindClass::Code,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            KindClass::File => "file",
            KindClass::Folder => "folder",
            KindClass::Image => "image",
            KindClass::Video => "video",
            KindClass::Audio => "audio",
            KindClass::Document => "document",
            KindClass::Archive => "archive",
            KindClass::Code => "code",
        }
    }

    /// Extensions of a content class (empty for `File` / `Folder`).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            KindClass::Image => &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff", "heic", "heif", "avif", "raw",
                "cr2", "nef", "arw", "dng", "psd",
            ],
            KindClass::Video => &["mp4", "mkv", "mov", "avi", "webm", "wmv", "m4v", "mpg", "mpeg", "flv", "3gp", "ts"],
            KindClass::Audio => &["mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma", "aiff", "mid", "midi"],
            KindClass::Document => &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "txt", "md", "rtf", "epub",
                "csv", "tex",
            ],
            KindClass::Archive => {
                &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "cab", "deb", "rpm", "lz", "lzma"]
            }
            KindClass::Code => &[
                "rs", "py", "js", "mjs", "ts", "tsx", "jsx", "go", "java", "kt", "c", "h", "cc", "cpp", "hpp", "cs",
                "rb", "php", "swift", "sh", "ps1", "bat", "cmd", "toml", "json", "yaml", "yml", "html", "css", "scss",
                "sql", "lua", "zig", "dart", "vue", "svelte",
            ],
            KindClass::File | KindClass::Folder => &[],
        }
    }
}

/// Half-open range `[lo, hi)`; `None` = unbounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Range {
    pub lo: Option<u64>,
    pub hi: Option<u64>,
}

impl Range {
    pub fn contains(&self, v: u64) -> bool {
        self.lo.is_none_or(|lo| v >= lo) && self.hi.is_none_or(|hi| v < hi)
    }
}

/// A time filter, resolved against "now" when the query runs (so saved queries stay
/// relative).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSpec {
    /// Age in seconds: `newer` = younger than `lo` … (`<7d`); see [`TimeSpec::bounds_at`].
    Age { min: Option<i64>, max: Option<i64> },
    /// Calendar span `[start, end)` of a local date (`2026-01-31`, `2026-01`, `2026`),
    /// with how the operator applies.
    Date { span: DateSpan, op: DateOp },
    /// Two dates: from the start of the first to the end of the second.
    DateRange { from: DateSpan, to: DateSpan },
    /// `today`, `yesterday`, `week` (last 7 days), `month` (30), `year` (365).
    Keyword(TimeKeyword),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOp {
    /// Within the span.
    On,
    /// Before the span starts.
    Before,
    /// Before the span ends.
    OnOrBefore,
    /// After the span ends.
    After,
    /// After the span starts.
    OnOrAfter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeKeyword {
    Today,
    Yesterday,
    Week,
    Month,
    Year,
}

/// A local calendar span: a day, a month or a year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateSpan {
    pub year: i32,
    /// 1-12 or 0 for the whole year.
    pub month: u32,
    /// 1-31 or 0 for the whole month.
    pub day: u32,
}

impl DateSpan {
    /// `[start, end)` as unix seconds for a fixed UTC offset (seconds east).
    fn bounds(self, offset: i64) -> Option<(i64, i64)> {
        use chrono::NaiveDate;
        let start = NaiveDate::from_ymd_opt(self.year, self.month.max(1), self.day.max(1))?;
        let end = if self.month == 0 {
            NaiveDate::from_ymd_opt(self.year + 1, 1, 1)?
        } else if self.day == 0 {
            let (y, m) = if self.month == 12 { (self.year + 1, 1) } else { (self.year, self.month + 1) };
            NaiveDate::from_ymd_opt(y, m, 1)?
        } else {
            start.succ_opt()?
        };
        let secs = |d: NaiveDate| d.and_hms_opt(0, 0, 0).map(|t| t.and_utc().timestamp() - offset);
        Some((secs(start)?, secs(end)?))
    }
}

/// Bounds `[lo, hi)` in unix seconds; `None` = unbounded.
pub type TimeBounds = (Option<i64>, Option<i64>);

impl TimeSpec {
    /// Resolve against `now` (unix seconds) in a timezone `offset` seconds east of UTC.
    pub fn bounds_at(&self, now: i64, offset: i64) -> TimeBounds {
        let day = 86_400;
        let local_midnight = |t: i64| {
            let local = t + offset;
            local - local.rem_euclid(day) - offset
        };
        match *self {
            // Younger than `max` → modified after now - max.
            TimeSpec::Age { min, max } => (max.map(|m| now - m), min.map(|m| now - m)),
            TimeSpec::Date { span, op } => match span.bounds(offset) {
                None => (Some(i64::MAX), None),
                Some((s, e)) => match op {
                    DateOp::On => (Some(s), Some(e)),
                    DateOp::Before => (None, Some(s)),
                    DateOp::OnOrBefore => (None, Some(e)),
                    DateOp::After => (Some(e), None),
                    DateOp::OnOrAfter => (Some(s), None),
                },
            },
            TimeSpec::DateRange { from, to } => match (from.bounds(offset), to.bounds(offset)) {
                (Some((s, _)), Some((_, e))) => (Some(s), Some(e)),
                _ => (Some(i64::MAX), None),
            },
            TimeSpec::Keyword(k) => {
                let today = local_midnight(now);
                match k {
                    TimeKeyword::Today => (Some(today), None),
                    TimeKeyword::Yesterday => (Some(today - day), Some(today)),
                    TimeKeyword::Week => (Some(now - 7 * day), None),
                    TimeKeyword::Month => (Some(now - 30 * day), None),
                    TimeKeyword::Year => (Some(now - 365 * day), None),
                }
            }
        }
    }

    /// Resolve against `now` in the local timezone.
    pub fn bounds(&self, now: std::time::SystemTime) -> TimeBounds {
        let secs = unix_secs(now);
        self.bounds_at(secs, local_offset(secs))
    }
}

/// Seconds since the unix epoch (negative before it).
pub fn unix_secs(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Local UTC offset (seconds east) at `secs`.
pub fn local_offset(secs: i64) -> i64 {
    use chrono::{Local, Offset, TimeZone};
    Local.timestamp_opt(secs, 0).single().map(|d| i64::from(d.offset().fix().local_minus_utc())).unwrap_or(0)
}

/// A query that could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{msg}")]
pub struct ParseError {
    pub msg: String,
    /// Byte offset of the offending token.
    pub pos: usize,
}

fn err<T>(pos: usize, msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError { msg: msg.into(), pos })
}

// ---- lexer --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    LParen,
    RParen,
    Or,
    Minus,
    /// Bare word (`quoted` = it was a "phrase").
    Word {
        text: String,
        quoted: bool,
    },
    /// `key:value`.
    Field {
        key: String,
        value: String,
    },
}

fn lex(s: &str) -> Vec<(usize, Tok)> {
    let b: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    // Read a quoted string starting after the opening quote; returns (text, next index).
    let quoted = |mut j: usize| -> (String, usize) {
        let mut t = String::new();
        while j < b.len() && b[j].1 != '"' {
            t.push(b[j].1);
            j += 1;
        }
        (t, (j + 1).min(b.len()))
    };
    while i < b.len() {
        let (pos, c) = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                out.push((pos, Tok::LParen));
                i += 1;
            }
            ')' => {
                out.push((pos, Tok::RParen));
                i += 1;
            }
            '|' => {
                out.push((pos, Tok::Or));
                i += 1;
            }
            '-' if b.get(i + 1).is_some_and(|n| !n.1.is_whitespace()) => {
                out.push((pos, Tok::Minus));
                i += 1;
            }
            '"' => {
                let (t, j) = quoted(i + 1);
                out.push((pos, Tok::Word { text: t, quoted: true }));
                i = j;
            }
            _ => {
                // A word, possibly `key:value` / `key:"value"`.
                let mut word = String::new();
                let mut j = i;
                while j < b.len() {
                    let ch = b[j].1;
                    if ch.is_whitespace() || ch == '(' || ch == ')' {
                        break;
                    }
                    if ch == ':' && is_key(&word) {
                        // Field.
                        let key = word.to_ascii_lowercase();
                        j += 1;
                        let value = if b.get(j).is_some_and(|c| c.1 == '"') {
                            let (t, k) = quoted(j + 1);
                            j = k;
                            t
                        } else {
                            // `)` ends the value unless it closes a `(` inside it
                            // (`regex:^(a|b)`).
                            let mut v = String::new();
                            let mut depth = 0usize;
                            while j < b.len() && !b[j].1.is_whitespace() {
                                match b[j].1 {
                                    '(' => depth += 1,
                                    ')' if depth == 0 => break,
                                    ')' => depth -= 1,
                                    _ => {}
                                }
                                v.push(b[j].1);
                                j += 1;
                            }
                            v
                        };
                        out.push((pos, Tok::Field { key, value }));
                        word.clear();
                        break;
                    }
                    word.push(ch);
                    j += 1;
                }
                if !word.is_empty() {
                    if word == "OR" {
                        out.push((pos, Tok::Or));
                    } else if word != "AND" {
                        out.push((pos, Tok::Word { text: word, quoted: false }));
                    }
                }
                i = j.max(i + 1);
            }
        }
    }
    out
}

const KEYS: [&str; 17] = [
    "name", "path", "in", "under", "ext", "kind", "type", "size", "modified", "mtime", "date", "created", "ctime",
    "tag", "content", "text", "regex",
];

fn is_key(w: &str) -> bool {
    KEYS.iter().any(|k| k.eq_ignore_ascii_case(w))
}

// ---- parser -------------------------------------------------------------------------

struct Parser {
    toks: Vec<(usize, Tok)>,
    i: usize,
    len: usize,
}

/// Parse a query. An empty or whitespace-only query is `And([])` (matches everything).
pub fn parse(s: &str) -> Result<Expr, ParseError> {
    let toks = lex(s);
    let mut p = Parser { toks, i: 0, len: s.len() };
    let e = p.and_list(false)?;
    if let Some((pos, _)) = p.toks.get(p.i) {
        return err(*pos, "unmatched “)”");
    }
    Ok(simplify(e))
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.1)
    }

    fn pos(&self) -> usize {
        self.toks.get(self.i).map(|t| t.0).unwrap_or(self.len)
    }

    fn and_list(&mut self, nested: bool) -> Result<Expr, ParseError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => break,
                Some(Tok::RParen) if nested => break,
                Some(Tok::RParen) => return err(self.pos(), "unmatched “)”"),
                Some(Tok::Or) => {
                    // A stray OR (leading / doubled) is ignored.
                    self.i += 1;
                }
                _ => items.push(self.or_expr()?),
            }
        }
        Ok(Expr::And(items))
    }

    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut alts = vec![self.unary()?];
        while matches!(self.peek(), Some(Tok::Or)) {
            self.i += 1;
            match self.peek() {
                None | Some(Tok::RParen) => break, // trailing OR
                _ => alts.push(self.unary()?),
            }
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap_or(Expr::And(Vec::new())) } else { Expr::Or(alts) })
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        let pos = self.pos();
        let Some((_, tok)) = self.toks.get(self.i).cloned() else {
            return err(pos, "expected a search term");
        };
        self.i += 1;
        match tok {
            Tok::Minus => {
                if self.peek().is_none() {
                    return err(pos, "nothing to exclude after “-”");
                }
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            Tok::LParen => {
                let inner = self.and_list(true)?;
                if !matches!(self.peek(), Some(Tok::RParen)) {
                    return err(pos, "missing “)”");
                }
                self.i += 1;
                Ok(inner)
            }
            Tok::RParen => err(pos, "unmatched “)”"),
            Tok::Or => err(pos, "OR needs a term on both sides"),
            Tok::Word { text, quoted } => Ok(Expr::Term(word_term(&text, quoted))),
            Tok::Field { key, value } => field(&key, &value, pos),
        }
    }
}

fn is_glob(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

fn word_term(text: &str, quoted: bool) -> Term {
    if !quoted && is_glob(text) { Term::NameGlob(text.to_string()) } else { Term::Name(text.to_string()) }
}

fn field(key: &str, value: &str, pos: usize) -> Result<Expr, ParseError> {
    let v = value.trim();
    let term = match key {
        "name" => {
            if v.is_empty() {
                return err(pos, "name: needs a value");
            }
            word_term(v, false)
        }
        "path" => {
            if v.is_empty() {
                return err(pos, "path: needs a value");
            }
            Term::Path(v.to_string())
        }
        "in" | "under" => {
            if v.is_empty() {
                return err(pos, "in: needs a folder");
            }
            Term::In(expand_home(v))
        }
        "ext" => {
            let exts: Vec<String> = v
                .split([',', ';', '|'])
                .map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
                .collect();
            if exts.is_empty() {
                return err(pos, "ext: needs an extension, e.g. ext:pdf");
            }
            Term::Ext(exts)
        }
        "kind" | "type" => match KindClass::parse(v) {
            Some(k) => Term::Kind(k),
            None => {
                let names: Vec<&str> = KindClass::ALL.iter().map(|k| k.name()).collect();
                return err(pos, format!("unknown kind “{v}” (use {})", names.join(", ")));
            }
        },
        "size" => Term::Size(parse_size_filter(v).ok_or_else(|| ParseError {
            msg: format!("can't read the size “{v}” (e.g. size:>5MB, size:1k..2M)"),
            pos,
        })?),
        "modified" | "mtime" | "date" | "created" | "ctime" => {
            let spec = parse_time_filter(v).ok_or_else(|| ParseError {
                msg: format!("can't read the date “{v}” (e.g. {key}:<7d, {key}:>2026-01-01, {key}:today)"),
                pos,
            })?;
            if key == "created" || key == "ctime" { Term::Created(spec) } else { Term::Modified(spec) }
        }
        "tag" => {
            if v.is_empty() {
                return err(pos, "tag: needs a color, e.g. tag:red");
            }
            Term::Tag(v.to_lowercase())
        }
        "content" | "text" => {
            if v.is_empty() {
                return err(pos, "content: needs text");
            }
            Term::Content(v.to_string())
        }
        "regex" => {
            if let Err(e) = regex::RegexBuilder::new(v).case_insensitive(true).build() {
                return err(pos, format!("invalid regex: {e}"));
            }
            Term::Regex(v.to_string())
        }
        _ => Term::Name(format!("{key}:{value}")),
    };
    Ok(Expr::Term(term))
}

fn expand_home(s: &str) -> PathBuf {
    if s == "~" {
        return soyuz_fs::home();
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        return soyuz_fs::home().join(rest);
    }
    PathBuf::from(s)
}

/// Flatten single-element / nested groups.
fn simplify(e: Expr) -> Expr {
    match e {
        Expr::And(items) => {
            let mut out = Vec::new();
            for i in items.into_iter().map(simplify) {
                match i {
                    Expr::And(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            if out.len() == 1 { out.pop().unwrap_or(Expr::And(Vec::new())) } else { Expr::And(out) }
        }
        Expr::Or(items) => {
            let mut out = Vec::new();
            for i in items.into_iter().map(simplify) {
                match i {
                    Expr::Or(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            if out.len() == 1 { out.pop().unwrap_or(Expr::And(Vec::new())) } else { Expr::Or(out) }
        }
        Expr::Not(inner) => match simplify(*inner) {
            Expr::Not(x) => *x,
            other => Expr::Not(Box::new(other)),
        },
        t => t,
    }
}

/// Parse `10`, `512k`, `1.5 MB`, `2GiB` (binary multiples) into bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let t = s.trim().to_ascii_lowercase().replace(' ', "");
    if t.is_empty() {
        return None;
    }
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim_end_matches("ib").trim_end_matches('b') {
        "" => 1.0,
        "k" => 1024.0,
        "m" => 1024f64.powi(2),
        "g" => 1024f64.powi(3),
        "t" => 1024f64.powi(4),
        _ => return None,
    };
    (n >= 0.0 && n.is_finite()).then(|| (n * mult).round() as u64)
}

/// `>5MB`, `>=5MB`, `<5MB`, `<=5MB`, `=5MB`, `5MB`, `1k..2M`, `empty`.
pub fn parse_size_filter(v: &str) -> Option<Range> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("empty") {
        return Some(Range { lo: Some(0), hi: Some(1) });
    }
    if let Some((a, b)) = v.split_once("..") {
        let lo = if a.trim().is_empty() { None } else { Some(parse_size(a)?) };
        let hi = if b.trim().is_empty() { None } else { Some(parse_size(b)?.saturating_add(1)) };
        if let (Some(l), Some(h)) = (lo, hi)
            && l >= h
        {
            return None;
        }
        return Some(Range { lo, hi });
    }
    let (op, rest) = split_op(v);
    let n = parse_size(rest)?;
    Some(match op {
        ">" => Range { lo: Some(n.saturating_add(1)), hi: None },
        ">=" => Range { lo: Some(n), hi: None },
        "<" => Range { lo: None, hi: Some(n) },
        "<=" => Range { lo: None, hi: Some(n.saturating_add(1)) },
        _ => Range { lo: Some(n), hi: Some(n.saturating_add(1)) },
    })
}

fn split_op(v: &str) -> (&str, &str) {
    for op in [">=", "<=", ">", "<", "="] {
        if let Some(rest) = v.strip_prefix(op) {
            return (op, rest.trim());
        }
    }
    ("", v)
}

/// A duration like `7d`, `2h`, `30min`, `3w`, `6mo`, `1y` in seconds.
pub fn parse_duration(s: &str) -> Option<i64> {
    let t = s.trim().to_ascii_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok()?;
    let unit_secs: f64 = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
        "d" | "day" | "days" => 86_400.0,
        "w" | "wk" | "week" | "weeks" => 7.0 * 86_400.0,
        "mo" | "mon" | "month" | "months" => 30.0 * 86_400.0,
        "y" | "yr" | "year" | "years" => 365.0 * 86_400.0,
        _ => return None,
    };
    (n >= 0.0 && n.is_finite()).then(|| (n * unit_secs).round() as i64)
}

/// `2026-01-31`, `2026-01`, `2026` (also with `/` or `.`).
pub fn parse_date(s: &str) -> Option<DateSpan> {
    let parts: Vec<&str> = s.trim().split(['-', '/', '.']).collect();
    let num = |p: &str| p.parse::<u32>().ok();
    let span = match parts.as_slice() {
        [y] if y.len() == 4 => DateSpan { year: y.parse().ok()?, month: 0, day: 0 },
        [y, m] if y.len() == 4 => DateSpan { year: y.parse().ok()?, month: num(m)?, day: 0 },
        [y, m, d] if y.len() == 4 => DateSpan { year: y.parse().ok()?, month: num(m)?, day: num(d)? },
        _ => return None,
    };
    if span.month > 12 || span.day > 31 || (span.month == 0 && span.day != 0) {
        return None;
    }
    // Validate the calendar date (e.g. 2026-02-30).
    span.bounds(0)?;
    Some(span)
}

pub fn parse_time_filter(v: &str) -> Option<TimeSpec> {
    let v = v.trim();
    let kw = match v.to_ascii_lowercase().as_str() {
        "today" => Some(TimeKeyword::Today),
        "yesterday" => Some(TimeKeyword::Yesterday),
        "week" | "thisweek" | "lastweek" => Some(TimeKeyword::Week),
        "month" | "thismonth" | "lastmonth" => Some(TimeKeyword::Month),
        "year" | "thisyear" | "lastyear" => Some(TimeKeyword::Year),
        _ => None,
    };
    if let Some(k) = kw {
        return Some(TimeSpec::Keyword(k));
    }
    if let Some((a, b)) = v.split_once("..") {
        if let (Some(from), Some(to)) = (parse_date(a), parse_date(b)) {
            return Some(TimeSpec::DateRange { from, to });
        }
        // Age range: `1d..7d` = between 1 and 7 days old.
        let min = if a.trim().is_empty() { None } else { Some(parse_duration(a)?) };
        let max = if b.trim().is_empty() { None } else { Some(parse_duration(b)?) };
        return Some(TimeSpec::Age { min, max });
    }
    let (op, rest) = split_op(v);
    if let Some(span) = parse_date(rest) {
        let op = match op {
            "<" => DateOp::Before,
            "<=" => DateOp::OnOrBefore,
            ">" => DateOp::After,
            ">=" => DateOp::OnOrAfter,
            _ => DateOp::On,
        };
        return Some(TimeSpec::Date { span, op });
    }
    let d = parse_duration(rest)?;
    Some(match op {
        ">" | ">=" => TimeSpec::Age { min: Some(d), max: None },
        _ => TimeSpec::Age { min: None, max: Some(d) },
    })
}

impl Expr {
    /// Every term, depth-first.
    pub fn terms(&self) -> Vec<&Term> {
        let mut out = Vec::new();
        fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a Term>) {
            match e {
                Expr::And(v) | Expr::Or(v) => v.iter().for_each(|x| walk(x, out)),
                Expr::Not(x) => walk(x, out),
                Expr::Term(t) => out.push(t),
            }
        }
        walk(self, &mut out);
        out
    }

    /// Terms that must hold for every match (top-level AND, not negated).
    pub fn required_terms(&self) -> Vec<&Term> {
        match self {
            Expr::Term(t) => vec![t],
            Expr::And(v) => v
                .iter()
                .filter_map(|e| match e {
                    Expr::Term(t) => Some(t),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `self AND other` (flattened).
    pub fn and(self, other: Expr) -> Expr {
        simplify(Expr::And(vec![self, other]))
    }

    /// Whether the query filters anything at all.
    pub fn is_empty(&self) -> bool {
        matches!(self, Expr::And(v) if v.is_empty())
    }

    /// Whether any term needs file contents.
    pub fn uses_content(&self) -> bool {
        self.terms().iter().any(|t| matches!(t, Term::Content(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(term: Term) -> Expr {
        Expr::Term(term)
    }
    fn name(s: &str) -> Expr {
        t(Term::Name(s.into()))
    }

    #[test]
    fn empty_and_plain_words() {
        assert_eq!(parse("").unwrap(), Expr::And(vec![]));
        assert_eq!(parse("   ").unwrap(), Expr::And(vec![]));
        assert_eq!(parse("report").unwrap(), name("report"));
        assert_eq!(parse("annual report").unwrap(), Expr::And(vec![name("annual"), name("report")]));
        assert_eq!(parse("\"annual report\"").unwrap(), name("annual report"));
        assert_eq!(parse("a AND b").unwrap(), Expr::And(vec![name("a"), name("b")]));
    }

    #[test]
    fn globs_and_names() {
        assert_eq!(parse("*.rs").unwrap(), t(Term::NameGlob("*.rs".into())));
        assert_eq!(parse("IMG_????.jpg").unwrap(), t(Term::NameGlob("IMG_????.jpg".into())));
        assert_eq!(parse("\"*.rs\"").unwrap(), name("*.rs"), "quoted text is literal");
        assert_eq!(parse("name:foo*").unwrap(), t(Term::NameGlob("foo*".into())));
        assert_eq!(parse("name:\"two words\"").unwrap(), name("two words"));
    }

    #[test]
    fn negation_or_and_groups() {
        assert_eq!(parse("-draft").unwrap(), Expr::Not(Box::new(name("draft"))));
        assert_eq!(parse("a OR b").unwrap(), Expr::Or(vec![name("a"), name("b")]));
        assert_eq!(parse("a | b | c").unwrap(), Expr::Or(vec![name("a"), name("b"), name("c")]));
        assert_eq!(
            parse("x a OR b y").unwrap(),
            Expr::And(vec![name("x"), Expr::Or(vec![name("a"), name("b")]), name("y")])
        );
        assert_eq!(parse("(a b) OR c").unwrap(), Expr::Or(vec![Expr::And(vec![name("a"), name("b")]), name("c")]));
        assert_eq!(
            parse("-(a OR b) c").unwrap(),
            Expr::And(vec![Expr::Not(Box::new(Expr::Or(vec![name("a"), name("b")]))), name("c")])
        );
        assert_eq!(parse("--a").unwrap(), name("a"), "double negation cancels");
        assert_eq!(parse("a-b").unwrap(), name("a-b"), "hyphen inside a word");
        assert_eq!(parse("-ext:tmp").unwrap(), Expr::Not(Box::new(t(Term::Ext(vec!["tmp".into()])))));
        assert_eq!(parse("or").unwrap(), name("or"), "lower-case or is a word");
        assert_eq!(parse("OR a").unwrap(), name("a"), "stray OR ignored");
        assert_eq!(parse("a OR").unwrap(), name("a"), "trailing OR ignored");
        assert_eq!(parse("((a))").unwrap(), name("a"));
    }

    #[test]
    fn parse_errors() {
        assert!(parse("(a b").is_err());
        assert!(parse("a)").is_err());
        assert!(parse("size:>lots").is_err());
        assert!(parse("size:").is_err());
        assert!(parse("kind:spaceship").unwrap_err().msg.contains("unknown kind"));
        assert!(parse("modified:sometime").is_err());
        assert!(parse("regex:(").is_err());
        assert!(parse("ext:").is_err());
        assert!(parse("tag:").is_err());
        assert_eq!(parse("-").unwrap(), name("-"), "a lone dash is a word");
        let e = parse("good size:huge").unwrap_err();
        assert_eq!(e.pos, 5, "position points at the bad token");
    }

    #[test]
    fn fields() {
        assert_eq!(parse("kind:image").unwrap(), t(Term::Kind(KindClass::Image)));
        assert_eq!(parse("type:folder").unwrap(), t(Term::Kind(KindClass::Folder)));
        assert_eq!(
            parse("ext:PDF,.docx;*.txt").unwrap(),
            t(Term::Ext(vec!["pdf".into(), "docx".into(), "txt".into()]))
        );
        assert_eq!(parse("tag:Red").unwrap(), t(Term::Tag("red".into())));
        assert_eq!(parse("path:docs").unwrap(), t(Term::Path("docs".into())));
        assert_eq!(parse("path:\"my docs\"").unwrap(), t(Term::Path("my docs".into())));
        assert_eq!(parse(r#"in:"C:\Program Files""#).unwrap(), t(Term::In(PathBuf::from(r"C:\Program Files"))));
        assert_eq!(parse("content:todo").unwrap(), t(Term::Content("todo".into())));
        assert_eq!(parse("text:\"fix me\"").unwrap(), t(Term::Content("fix me".into())));
        assert_eq!(parse("regex:^a\\d+$").unwrap(), t(Term::Regex("^a\\d+$".into())));
        assert_eq!(parse("(regex:^(a|b) x)").unwrap(), Expr::And(vec![t(Term::Regex("^(a|b)".into())), name("x")]));
        assert_eq!(parse("KIND:Video").unwrap(), t(Term::Kind(KindClass::Video)), "keys are case-insensitive");
        // Unknown keys and drive letters stay name words.
        assert_eq!(parse("foo:bar").unwrap(), name("foo:bar"));
        assert_eq!(parse(r"C:\x").unwrap(), name(r"C:\x"));
        assert!(matches!(parse("in:~").unwrap(), Expr::Term(Term::In(p)) if p == soyuz_fs::home()));
    }

    #[test]
    fn sizes() {
        let r = |s: &str| parse_size_filter(s).unwrap();
        let mb = 1024 * 1024;
        assert_eq!(r(">5MB"), Range { lo: Some(5 * mb + 1), hi: None });
        assert_eq!(r(">=5MB"), Range { lo: Some(5 * mb), hi: None });
        assert_eq!(r("<1k"), Range { lo: None, hi: Some(1024) });
        assert_eq!(r("<=1k"), Range { lo: None, hi: Some(1025) });
        assert_eq!(r("100"), Range { lo: Some(100), hi: Some(101) });
        assert_eq!(r("=0"), Range { lo: Some(0), hi: Some(1) });
        assert_eq!(r("empty"), Range { lo: Some(0), hi: Some(1) });
        assert_eq!(r("1k..2k"), Range { lo: Some(1024), hi: Some(2049) });
        assert_eq!(r("1G.."), Range { lo: Some(1 << 30), hi: None });
        assert_eq!(r("1.5 MiB"), Range { lo: Some(1_572_864), hi: Some(1_572_865) });
        assert!(parse_size_filter("2k..1k").is_none());
        assert!(parse_size_filter("5 parsecs").is_none());
        assert!(r(">5MB").contains(6 * mb) && !r(">5MB").contains(5 * mb));
    }

    #[test]
    fn times() {
        let day = 86_400;
        let now = 1_000 * day + 12 * 3600; // noon UTC
        let b = |s: &str| parse_time_filter(s).unwrap().bounds_at(now, 0);
        assert_eq!(b("<7d"), (Some(now - 7 * day), None));
        assert_eq!(b("7d"), (Some(now - 7 * day), None));
        assert_eq!(b(">1y"), (None, Some(now - 365 * day)));
        assert_eq!(b("2h"), (Some(now - 7200), None));
        assert_eq!(b("30min"), (Some(now - 1800), None));
        assert_eq!(b("1d..7d"), (Some(now - 7 * day), Some(now - day)));
        assert_eq!(b("today"), (Some(1_000 * day), None));
        assert_eq!(b("yesterday"), (Some(999 * day), Some(1_000 * day)));
        assert_eq!(b("week"), (Some(now - 7 * day), None));
        // Dates (UTC offset 0): 1970-01-02 is day 1.
        assert_eq!(b("1970-01-02"), (Some(day), Some(2 * day)));
        assert_eq!(b("<1970-01-02"), (None, Some(day)));
        assert_eq!(b("<=1970-01-02"), (None, Some(2 * day)));
        assert_eq!(b(">1970-01-02"), (Some(2 * day), None));
        assert_eq!(b(">=1970-01-02"), (Some(day), None));
        assert_eq!(b("1970-02"), (Some(31 * day), Some(59 * day)));
        assert_eq!(b("1970"), (Some(0), Some(365 * day)));
        assert_eq!(b("1970-01-01..1970-01-03"), (Some(0), Some(3 * day)));
        // A local timezone shifts calendar bounds, not ages.
        let east = parse_time_filter("today").unwrap().bounds_at(now, 3 * 3600);
        assert_eq!(east, (Some(1_000 * day - 3 * 3600), None));
        assert_eq!(parse_time_filter("1970/12/31").unwrap().bounds_at(now, 0).0, Some(364 * day));
        assert!(parse_time_filter("2026-02-30").is_none());
        assert!(parse_time_filter("2026-13").is_none());
        assert!(parse_time_filter("7 parsecs").is_none());
        // Month and year rollover.
        assert_eq!(
            parse_date("2025-12").unwrap().bounds(0).unwrap().1,
            parse_date("2026-01-01").unwrap().bounds(0).unwrap().0
        );
    }

    #[test]
    fn full_example() {
        let q = parse(r#"kind:image size:>5MB modified:<7d tag:red ext:pdf path:docs "exact phrase" -exclude a OR b"#)
            .unwrap();
        let Expr::And(items) = &q else { panic!("{q:?}") };
        assert_eq!(items.len(), 9);
        assert!(matches!(items[0], Expr::Term(Term::Kind(KindClass::Image))));
        assert!(matches!(items[1], Expr::Term(Term::Size(_))));
        assert!(matches!(items[2], Expr::Term(Term::Modified(TimeSpec::Age { .. }))));
        assert_eq!(items[6], name("exact phrase"));
        assert_eq!(items[7], Expr::Not(Box::new(name("exclude"))));
        assert_eq!(items[8], Expr::Or(vec![name("a"), name("b")]));
        assert_eq!(q.required_terms().len(), 7);
        assert!(!q.uses_content());
        assert!(parse("content:x").unwrap().uses_content());
    }

    #[test]
    fn unicode_and_odd_input() {
        assert_eq!(parse("çalışma").unwrap(), name("çalışma"));
        assert_eq!(parse("\"unterminated").unwrap(), name("unterminated"));
        assert_eq!(parse("path:\"").unwrap_err().msg, "path: needs a value");
        assert!(parse("ext:pdf)").is_err());
        // Fuzz-ish: nothing panics.
        for s in ["(", ")", "-(", "\"", "a:\"", "size:..", "modified:..", "((((", "OR OR", "|", "-\"\"", "in:\"\""] {
            let _ = parse(s);
        }
    }
}
