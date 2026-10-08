//! Smart search (roadmap A6): natural language → a structured query shown as editable
//! pills (`kind:image modified:2026-06..2026-08 size:>5MB`) that become a
//! `soyuz_search::SearchQuery` only when the user runs it. A local parser covers the
//! common phrasings (no model needed); a model's `search` tool call can fill the same
//! pills ([`from_search_args`]).

use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use regex::Regex;
use serde_json::Value;
use soyuz_core::{EntryKindSet, NamePatternSpec, PatternKind};
use soyuz_search::{ContentPatternSpec, SearchQuery};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PillKey {
    Kind,
    Ext,
    Name,
    Size,
    Modified,
    Content,
    In,
}

impl PillKey {
    pub const ALL: [PillKey; 7] =
        [PillKey::Kind, PillKey::Ext, PillKey::Name, PillKey::Size, PillKey::Modified, PillKey::Content, PillKey::In];

    pub fn word(self) -> &'static str {
        match self {
            PillKey::Kind => "kind",
            PillKey::Ext => "ext",
            PillKey::Name => "name",
            PillKey::Size => "size",
            PillKey::Modified => "modified",
            PillKey::Content => "content",
            PillKey::In => "in",
        }
    }

    fn parse(s: &str) -> Option<PillKey> {
        PillKey::ALL.into_iter().find(|k| k.word().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub key: PillKey,
    pub value: String,
}

impl Pill {
    pub fn new(key: PillKey, value: impl Into<String>) -> Self {
        Pill { key, value: value.into() }
    }

    pub fn text(&self) -> String {
        let v =
            if self.value.contains(char::is_whitespace) { format!("\"{}\"", self.value) } else { self.value.clone() };
        format!("{}:{v}", self.key.word())
    }
}

/// Kinds understood by `kind:` and their extensions.
pub const KINDS: &[(&str, &[&str])] = &[
    (
        "image",
        &[
            "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff", "raw", "cr2", "nef", "arw",
            "dng", "svg",
        ],
    ),
    ("video", &["mp4", "mov", "mkv", "avi", "webm", "m4v", "wmv", "flv"]),
    ("audio", &["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma"]),
    ("document", &["pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "csv"]),
    ("archive", &["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst", "tgz"]),
    (
        "code",
        &[
            "rs", "py", "js", "ts", "tsx", "go", "c", "cpp", "h", "hpp", "java", "kt", "cs", "rb", "php", "swift",
            "sh", "ps1", "toml", "json", "yaml", "yml",
        ],
    ),
    ("folder", &[]),
    ("file", &[]),
];

/// The query as pill text (`kind:image size:>5MB`).
pub fn to_text(pills: &[Pill]) -> String {
    pills.iter().map(Pill::text).collect::<Vec<_>>().join(" ")
}

/// Parse pill text back (unknown `key:` words become name terms).
pub fn parse_text(s: &str) -> Vec<Pill> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let (tok, tail) = next_token(rest);
        rest = tail.trim_start();
        if tok.is_empty() {
            continue;
        }
        match tok.split_once(':').and_then(|(k, v)| PillKey::parse(k).map(|k| (k, v))) {
            Some((k, v)) => {
                let v = v.trim_matches('"');
                if !v.is_empty() {
                    out.push(Pill::new(k, v));
                }
            }
            None => out.push(Pill::new(PillKey::Name, tok.trim_matches('"'))),
        }
    }
    out
}

/// One whitespace token; quotes keep spaces (`name:"tax report"`).
fn next_token(s: &str) -> (&str, &str) {
    let mut in_q = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_q = !in_q,
            c if c.is_whitespace() && !in_q => return (&s[..i], &s[i..]),
            _ => {}
        }
    }
    (s, "")
}

fn size_value(num: &str, unit: &str) -> Option<u64> {
    let n: f64 = num.replace(',', ".").parse().ok()?;
    let unit = unit.to_ascii_lowercase();
    let mult = match unit.trim_end_matches('s') {
        "" | "b" | "byte" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" | "meg" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" | "gig" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some((n * mult) as u64)
}

/// `>5MB`, `<100k`, `1MB..10MB` → `(min, max)`.
pub fn parse_size(s: &str) -> Option<(Option<u64>, Option<u64>)> {
    let re = Regex::new(r"(?i)^\s*([0-9]+(?:[.,][0-9]+)?)\s*([a-z]*)\s*$").ok()?;
    let one = |t: &str| -> Option<u64> {
        let c = re.captures(t)?;
        size_value(&c[1], &c[2])
    };
    let s = s.trim();
    if let Some((a, b)) = s.split_once("..") {
        return Some((
            if a.is_empty() { None } else { Some(one(a)?) },
            if b.is_empty() { None } else { Some(one(b)?) },
        ));
    }
    if let Some(v) = s.strip_prefix(">=").or_else(|| s.strip_prefix('>')) {
        return Some((Some(one(v)?), None));
    }
    if let Some(v) = s.strip_prefix("<=").or_else(|| s.strip_prefix('<')) {
        return Some((None, Some(one(v)?)));
    }
    let v = one(s)?;
    Some((Some(v), Some(v)))
}

fn date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").ok())
        .or_else(|| s.parse::<i32>().ok().and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1)))
}

/// End (exclusive) of a date given at year / month / day precision.
fn date_end(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    let d = date(s)?;
    Some(match s.matches('-').count() {
        0 => NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)?,
        1 => {
            if d.month() == 12 {
                NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)?
            }
        }
        _ => d + Duration::days(1),
    })
}

fn to_time(d: NaiveDate) -> Option<SystemTime> {
    let dt = d.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&dt).earliest().map(SystemTime::from)
}

/// `2026-06..2026-08`, `>2026-01-01`, `2025` → `(after, before)`.
pub fn parse_modified(s: &str) -> Option<(Option<SystemTime>, Option<SystemTime>)> {
    let s = s.trim();
    if let Some((a, b)) = s.split_once("..") {
        let after = if a.is_empty() { None } else { Some(to_time(date(a)?)?) };
        let before = if b.is_empty() { None } else { Some(to_time(date_end(b)?)?) };
        return Some((after, before));
    }
    if let Some(v) = s.strip_prefix('>') {
        return Some((Some(to_time(date_end(v)?)?), None));
    }
    if let Some(v) = s.strip_prefix('<') {
        return Some((None, Some(to_time(date(v)?)?)));
    }
    Some((Some(to_time(date(s)?)?), Some(to_time(date_end(s)?)?)))
}

/// Build the search query from pills (errors name the bad pill).
pub fn to_query(pills: &[Pill], default_root: PathBuf) -> Result<SearchQuery, String> {
    let mut q = SearchQuery { max_results: 20_000, ..SearchQuery::default() };
    let mut names: Vec<String> = Vec::new();
    for p in pills {
        match p.key {
            PillKey::Kind => {
                let k = p.value.to_lowercase();
                let k = k.trim_end_matches('s');
                match KINDS.iter().find(|(n, _)| *n == k) {
                    Some(("folder", _)) => q.kinds = Some(EntryKindSet::DIR),
                    Some(("file", _)) => q.kinds = Some(EntryKindSet::FILE),
                    Some((_, exts)) => {
                        q.extensions.extend(exts.iter().map(|e| e.to_string()));
                        q.kinds = Some(EntryKindSet::FILE);
                    }
                    None => {
                        return Err(tr!("ai-smart-unknown-kind", kind = &p.value));
                    }
                }
            }
            PillKey::Ext => q.extensions.extend(
                p.value.split(',').map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()),
            ),
            PillKey::Name => names.push(p.value.clone()),
            PillKey::Size => {
                q.size = Some(parse_size(&p.value).ok_or_else(|| tr!("ai-smart-bad-size", value = &p.value))?)
            }
            PillKey::Modified => {
                q.modified = Some(parse_modified(&p.value).ok_or_else(|| tr!("ai-smart-bad-date", value = &p.value))?)
            }
            PillKey::Content => {
                q.content = Some(ContentPatternSpec { regex: false, text: p.value.clone(), case_sensitive: false })
            }
            PillKey::In => q.roots.push(super::safety::expand(std::path::Path::new(&p.value))),
        }
    }
    if !names.is_empty() {
        let joined = names.join(" ");
        let kind = if joined.contains(['*', '?']) { PatternKind::Glob } else { PatternKind::Substring };
        q.name = Some(NamePatternSpec::new(kind, &joined));
    }
    q.extensions.sort();
    q.extensions.dedup();
    if q.roots.is_empty() {
        q.roots.push(default_root);
    }
    Ok(q)
}

fn month_num(w: &str) -> Option<u32> {
    const M: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let w = w.to_lowercase();
    if w.len() < 3 {
        return None;
    }
    M.iter().position(|m| m.starts_with(&w)).map(|i| i as u32 + 1)
}

/// Local natural-language parser (no model).
pub fn parse_nl(text: &str, today: NaiveDate) -> Vec<Pill> {
    let mut pills = Vec::new();
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '.').filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);

    // Kinds.
    let kind_words: &[(&[&str], &str)] = &[
        (&["photo", "photos", "image", "images", "picture", "pictures", "pics", "screenshots", "screenshot"], "image"),
        (&["video", "videos", "movie", "movies", "clips"], "video"),
        (&["music", "song", "songs", "audio", "podcasts"], "audio"),
        (&["document", "documents", "docs", "spreadsheets", "presentations"], "document"),
        (&["archive", "archives", "zips", "compressed"], "archive"),
        (&["code", "source", "scripts"], "code"),
        (&["folder", "folders", "directories"], "folder"),
    ];
    for (ws, k) in kind_words {
        if ws.iter().any(|w| has(w)) {
            pills.push(Pill::new(PillKey::Kind, *k));
            break;
        }
    }
    // Explicit extensions ("pdfs", ".psd files", "jpg").
    let known: Vec<&str> = KINDS.iter().flat_map(|(_, e)| e.iter().copied()).collect();
    let mut exts = Vec::new();
    for w in &words {
        let w = w.trim_start_matches('.');
        let base = w.strip_suffix('s').filter(|b| known.contains(b)).unwrap_or(w);
        let listed = known.contains(&base) && !["doc", "docs", "code", "c", "h", "md"].contains(&base);
        let dotted = w.len() > 1 && lower.contains(&format!(".{w}"));
        if (listed || dotted) && !exts.contains(&base.to_string()) {
            exts.push(base.to_string());
        }
    }
    if !exts.is_empty() {
        pills.retain(|p| p.key != PillKey::Kind);
        pills.push(Pill::new(PillKey::Ext, exts.join(",")));
    }
    // Size.
    let size_re = Regex::new(
        r"(?i)(larger|bigger|greater|more|over|above|>|at least|smaller|less|under|below|<|at most)\s*(?:than)?\s*([0-9]+(?:[.,][0-9]+)?)\s*(kb|kib|mb|mib|gb|gib|tb|k|m|g|bytes?)\b",
    )
    .expect("size regex");
    if let Some(c) = size_re.captures(&lower) {
        let up = matches!(&c[1], "larger" | "bigger" | "greater" | "more" | "over" | "above" | ">" | "at least");
        let unit = c[3].to_uppercase().replace("IB", "B");
        let unit = if unit.len() == 1 && unit != "B" { format!("{unit}B") } else { unit };
        pills.push(Pill::new(PillKey::Size, format!("{}{}{}", if up { ">" } else { "<" }, &c[2], unit)));
    } else if has("huge") || has("large") || has("big") {
        pills.push(Pill::new(PillKey::Size, ">100MB"));
    }
    // Dates.
    let fmt = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let month_range = |y: i32, m1: u32, m2: u32| format!("{y}-{m1:02}..{y}-{m2:02}");
    let year = today.year();
    let modified = if lower.contains("today") {
        Some(fmt(today))
    } else if lower.contains("yesterday") {
        Some(fmt(today - Duration::days(1)))
    } else if lower.contains("last week") || lower.contains("past week") || lower.contains("this week") {
        Some(format!("{}..", fmt(today - Duration::days(7))))
    } else if lower.contains("last month") || lower.contains("past month") {
        Some(format!("{}..", fmt(today - Duration::days(31))))
    } else if lower.contains("this month") {
        Some(format!("{year}-{:02}..", today.month()))
    } else if lower.contains("last year") {
        Some((year - 1).to_string())
    } else if lower.contains("this year") {
        Some(format!("{year}-01-01.."))
    } else if let Some(season) = ["spring", "summer", "autumn", "fall", "winter"].iter().find(|s| has(s)) {
        let last = lower.contains("last ");
        let (m1, m2) = match *season {
            "spring" => (3, 5),
            "summer" => (6, 8),
            "autumn" | "fall" => (9, 11),
            _ => (12, 12),
        };
        // "last summer" = the most recent one that has ended.
        let y = if last && today.month() <= m2 { year - 1 } else { year };
        Some(if *season == "winter" { format!("{}-12..{}-02", y - 1, y) } else { month_range(y, m1, m2) })
    } else if let Some(y) = words.iter().filter_map(|w| w.parse::<i32>().ok()).find(|y| (1990..=2100).contains(y)) {
        match words.iter().find_map(|w| month_num(w)) {
            Some(m) => Some(format!("{y}-{m:02}")),
            None => Some(y.to_string()),
        }
    } else {
        words.iter().find_map(|w| month_num(w).filter(|_| w.len() >= 4)).map(|m| {
            let y = if m > today.month() { year - 1 } else { year };
            format!("{y}-{m:02}")
        })
    };
    if let Some(m) = modified {
        let older = lower.contains("older than") || lower.contains("before");
        pills.push(Pill::new(PillKey::Modified, if older { format!("..{}", m.trim_end_matches("..")) } else { m }));
    }
    // Quoted text, "named x", "containing x".
    let quoted = Regex::new(r#""([^"]+)""#).expect("quote regex");
    let named = Regex::new(r#"(?i)\b(?:named|called|with name|name contains)\s+([^\s,]+)"#).expect("named regex");
    let containing =
        Regex::new(r#"(?i)\b(?:containing|mentioning|that contain|which contain|with text)\s+"?([^",]+)"?"#)
            .expect("content regex");
    if let Some(c) = containing.captures(text) {
        pills.push(Pill::new(PillKey::Content, c[1].trim()));
    } else if let Some(c) = named.captures(text) {
        pills.push(Pill::new(PillKey::Name, c[1].trim_matches('"')));
    } else if let Some(c) = quoted.captures(text) {
        pills.push(Pill::new(PillKey::Name, &c[1]));
    }
    pills
}

/// Pills from a model's `search` tool arguments.
pub fn from_search_args(args: &Value) -> Vec<Pill> {
    let mut p = Vec::new();
    if let Some(n) = args.get("name").and_then(Value::as_str) {
        p.push(Pill::new(PillKey::Name, n));
    }
    if let Some(e) = args.get("extensions").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        p.push(Pill::new(PillKey::Ext, e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",")));
    }
    let mb =
        |b: u64| if b >= 1024 * 1024 { format!("{}MB", b / (1024 * 1024)) } else { format!("{}KB", b.div_ceil(1024)) };
    match (args.get("min_size").and_then(Value::as_u64), args.get("max_size").and_then(Value::as_u64)) {
        (Some(a), Some(b)) => p.push(Pill::new(PillKey::Size, format!("{}..{}", mb(a), mb(b)))),
        (Some(a), None) => p.push(Pill::new(PillKey::Size, format!(">{}", mb(a)))),
        (None, Some(b)) => p.push(Pill::new(PillKey::Size, format!("<{}", mb(b)))),
        _ => {}
    }
    let day = |k: &str| args.get(k).and_then(Value::as_str).map(|s| s.get(..10).unwrap_or(s).to_string());
    match (day("modified_after"), day("modified_before")) {
        (Some(a), Some(b)) => p.push(Pill::new(PillKey::Modified, format!("{a}..{b}"))),
        (Some(a), None) => p.push(Pill::new(PillKey::Modified, format!("{a}.."))),
        (None, Some(b)) => p.push(Pill::new(PillKey::Modified, format!("..{b}"))),
        _ => {}
    }
    if let Some(c) = args.get("content").and_then(Value::as_str) {
        p.push(Pill::new(PillKey::Content, c));
    }
    if args.get("kind").and_then(Value::as_str) == Some("dir") {
        p.push(Pill::new(PillKey::Kind, "folder"));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    }

    #[test]
    fn natural_language_to_pills() {
        let p = parse_nl("photos from last summer larger than 5 MB", today());
        assert_eq!(to_text(&p), "kind:image size:>5MB modified:2026-06..2026-08");
        let p = parse_nl("pdfs mentioning \"invoice 42\" from 2025", today());
        assert_eq!(to_text(&p), "ext:pdf modified:2025 content:\"invoice 42\"");
        let p = parse_nl("videos smaller than 200mb modified last week", today());
        assert_eq!(to_text(&p), "kind:video size:<200MB modified:2026-10-01..");
    }

    #[test]
    fn pills_round_trip_and_build_query() {
        let text = "kind:image size:>5MB modified:2026-06..2026-08 name:\"tax report\" ext:psd";
        let pills = parse_text(text);
        assert_eq!(pills.len(), 5);
        assert_eq!(to_text(&pills), text);
        let q = to_query(&pills, PathBuf::from("/root")).unwrap();
        assert!(q.extensions.contains(&"jpg".to_string()) && q.extensions.contains(&"psd".to_string()));
        assert_eq!(q.size, Some((Some(5 * 1024 * 1024), None)));
        let (a, b) = q.modified.unwrap();
        assert!(a.unwrap() < b.unwrap());
        assert_eq!(q.roots, vec![PathBuf::from("/root")]);
        assert!(to_query(&[Pill::new(PillKey::Size, "big")], PathBuf::new()).is_err());
        assert!(to_query(&[Pill::new(PillKey::Kind, "spaceship")], PathBuf::new()).is_err());
        assert_eq!(parse_size("1MB..10MB"), Some((Some(1 << 20), Some(10 << 20))));
    }

    #[test]
    fn model_args_to_pills() {
        let p = from_search_args(&serde_json::json!({
            "roots": ["/x"], "extensions": ["jpg", "png"], "min_size": 5_242_880, "modified_after": "2026-06-01T00:00:00Z"
        }));
        assert_eq!(to_text(&p), "ext:jpg,png size:>5MB modified:2026-06-01..");
    }
}
