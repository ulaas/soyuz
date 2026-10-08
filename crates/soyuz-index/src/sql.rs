//! Query → SQL. Every value is a bound parameter; only fixed SQL fragments are
//! concatenated.
//!
//! * Name words use the trigram FTS5 table `names` (`LIKE '%word%'`, case-insensitive)
//!   when they are at least three characters and have no LIKE wildcards; shorter words
//!   scan `files.name`.
//! * Globs are lower-cased `GLOB`s on the name, with the longest literal run of three or
//!   more characters as a trigram pre-filter.
//! * `content:` uses the contentless FTS5 table `contents` (unicode61 words, phrase match).
//! * `in:` is a range scan on the unique `path` index.

use std::path::{Path, PathBuf};

use rusqlite::types::Value;

use crate::query::{Expr, KindClass, Term};

/// Kind codes stored in `files.kind`.
pub const KIND_FILE: i64 = 0;
pub const KIND_DIR: i64 = 1;
pub const KIND_SYMLINK: i64 = 2;
pub const KIND_JUNCTION: i64 = 3;
pub const KIND_OTHER: i64 = 4;

/// Inputs that are not part of the query text.
#[derive(Clone, Debug, Default)]
pub struct SqlOptions {
    /// Unix seconds "now" and the local UTC offset (relative dates).
    pub now: i64,
    pub utc_offset: i64,
    /// Include hidden files and everything below hidden folders.
    pub include_hidden: bool,
    /// Restrict to these folders (descendants, OR-ed); empty = everything indexed.
    pub scope: Vec<PathBuf>,
    /// Row limit (0 = unlimited).
    pub limit: usize,
}

/// A generated statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Sql {
    pub text: String,
    pub params: Vec<Value>,
}

/// Columns returned by [`select`], in order.
pub const COLUMNS: &str = "f.path, f.name, f.kind, f.size, f.mtime, f.ctime, f.attrs, f.frn";

/// `SELECT … FROM files f WHERE …` for a query.
pub fn select(expr: &Expr, opts: &SqlOptions) -> Sql {
    let mut b = Builder { params: Vec::new(), opts };
    let mut clauses = Vec::new();
    if !expr.is_empty() {
        clauses.push(b.expr(expr));
    }
    if !opts.include_hidden {
        clauses.push("f.hidden = 0".to_string());
    }
    if !opts.scope.is_empty() {
        let parts: Vec<String> = opts.scope.iter().map(|p| b.under(p)).collect();
        clauses.push(format!("({})", parts.join(" OR ")));
    }
    let mut text = format!("SELECT {COLUMNS} FROM files f");
    if !clauses.is_empty() {
        text.push_str(" WHERE ");
        text.push_str(&clauses.join(" AND "));
    }
    if opts.limit > 0 {
        text.push_str(" LIMIT ?");
        b.params.push(Value::Integer(opts.limit as i64));
    }
    Sql { text, params: b.params }
}

struct Builder<'a> {
    params: Vec<Value>,
    opts: &'a SqlOptions,
}

/// Escape `%`, `_` and `\` for `LIKE … ESCAPE '\'`.
pub fn like_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// An FTS5 phrase string (`"…"` with inner quotes doubled).
pub fn fts_phrase(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Longest run of literal (non-wildcard) characters in a glob.
fn glob_literal(g: &str) -> String {
    let mut best = String::new();
    let mut cur = String::new();
    let mut in_class = false;
    for c in g.chars() {
        match c {
            '[' => {
                in_class = true;
                cur.clear();
            }
            ']' if in_class => in_class = false,
            '*' | '?' => cur.clear(),
            _ if in_class => {}
            _ => {
                cur.push(c);
                if cur.chars().count() > best.chars().count() {
                    best = cur.clone();
                }
            }
        }
    }
    best
}

/// `lo`/`hi` bounds of the strict descendants of `dir` for a range scan on `path`.
pub fn descendant_bounds(dir: &Path) -> (String, String) {
    let sep = std::path::MAIN_SEPARATOR;
    let mut lo = dir.to_string_lossy().into_owned();
    if !lo.ends_with(['/', '\\']) {
        lo.push(sep);
    }
    let mut hi = lo.clone();
    let last = hi.pop().unwrap_or(sep);
    hi.push(char::from_u32(last as u32 + 1).unwrap_or(last));
    (lo, hi)
}

impl Builder<'_> {
    fn push(&mut self, v: impl Into<Value>) -> &'static str {
        self.params.push(v.into());
        "?"
    }

    fn under(&mut self, dir: &Path) -> String {
        let (lo, hi) = descendant_bounds(dir);
        format!("(f.path >= {} AND f.path < {})", self.push(lo), self.push(hi))
    }

    fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::And(v) if v.is_empty() => "1".into(),
            Expr::Or(v) if v.is_empty() => "0".into(),
            Expr::And(v) => format!("({})", v.iter().map(|x| self.expr(x)).collect::<Vec<_>>().join(" AND ")),
            Expr::Or(v) => format!("({})", v.iter().map(|x| self.expr(x)).collect::<Vec<_>>().join(" OR ")),
            // NULL (unknown time) counts as "doesn't match", also under NOT.
            Expr::Not(x) => format!("(NOT IFNULL({}, 0))", self.expr(x)),
            Expr::Term(t) => self.term(t),
        }
    }

    fn name_like(&mut self, needle: &str) -> String {
        let fts_ok = needle.chars().count() >= 3 && !needle.contains(['%', '_']);
        if fts_ok {
            format!("f.id IN (SELECT rowid FROM names WHERE names.name LIKE {})", self.push(format!("%{needle}%")))
        } else {
            format!("f.name LIKE {} ESCAPE '\\'", self.push(format!("%{}%", like_escape(needle))))
        }
    }

    fn term(&mut self, t: &Term) -> String {
        match t {
            Term::Name(s) => self.name_like(s),
            Term::NameGlob(g) => {
                // Parameters must be pushed in the order their `?`s appear.
                let lit = glob_literal(g);
                let pre = (lit.chars().count() >= 3 && !lit.contains(['%', '_'])).then(|| self.name_like(&lit));
                let glob = format!("lower(f.name) GLOB {}", self.push(g.to_lowercase()));
                match pre {
                    Some(pre) => format!("({pre} AND {glob})"),
                    None => glob,
                }
            }
            Term::Regex(r) => format!("regexp({}, f.name)", self.push(r.clone())),
            Term::Path(s) => {
                let norm: String =
                    s.chars().map(|c| if c == '/' || c == '\\' { std::path::MAIN_SEPARATOR } else { c }).collect();
                format!("f.path LIKE {} ESCAPE '\\'", self.push(format!("%{}%", like_escape(&norm))))
            }
            Term::In(p) => self.under(p),
            Term::Ext(list) => {
                let ph: Vec<&str> = list.iter().map(|e| self.push(e.to_lowercase())).collect();
                format!("f.ext IN ({})", ph.join(", "))
            }
            Term::Kind(KindClass::File) => format!("f.kind = {KIND_FILE}"),
            Term::Kind(KindClass::Folder) => format!("f.kind IN ({KIND_DIR}, {KIND_JUNCTION})"),
            Term::Kind(k) => {
                let ph: Vec<&str> = k.extensions().iter().map(|e| self.push(e.to_string())).collect();
                format!("(f.kind = {KIND_FILE} AND f.ext IN ({}))", ph.join(", "))
            }
            Term::Size(r) => {
                let mut parts = vec![format!("f.kind = {KIND_FILE}")];
                if let Some(lo) = r.lo {
                    parts.push(format!("f.size >= {}", self.push(lo.min(i64::MAX as u64) as i64)));
                }
                if let Some(hi) = r.hi {
                    parts.push(format!("f.size < {}", self.push(hi.min(i64::MAX as u64) as i64)));
                }
                format!("({})", parts.join(" AND "))
            }
            Term::Modified(spec) | Term::Created(spec) => {
                let col = if matches!(t, Term::Modified(_)) { "f.mtime" } else { "f.ctime" };
                let (lo, hi) = spec.bounds_at(self.opts.now, self.opts.utc_offset);
                let mut parts = vec![format!("{col} IS NOT NULL")];
                if let Some(lo) = lo {
                    parts.push(format!("{col} >= {}", self.push(lo)));
                }
                if let Some(hi) = hi {
                    parts.push(format!("{col} < {}", self.push(hi)));
                }
                format!("({})", parts.join(" AND "))
            }
            Term::Tag(tag) => {
                format!(
                    "f.path IN (SELECT path FROM tags WHERE tags LIKE {} ESCAPE '\\')",
                    self.push(format!("% {} %", like_escape(&tag.to_lowercase())))
                )
            }
            Term::Content(s) => {
                format!("f.id IN (SELECT rowid FROM contents WHERE contents MATCH {})", self.push(fts_phrase(s)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;

    fn opts() -> SqlOptions {
        SqlOptions { now: 1_000_000, utc_offset: 0, include_hidden: true, scope: vec![], limit: 0 }
    }

    fn sql(q: &str) -> Sql {
        select(&parse(q).unwrap(), &opts())
    }

    fn text(v: &Value) -> String {
        match v {
            Value::Text(s) => s.clone(),
            Value::Integer(i) => i.to_string(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn everything() {
        let s = sql("");
        assert_eq!(s.text, format!("SELECT {COLUMNS} FROM files f"));
        assert!(s.params.is_empty());
        let mut o = opts();
        o.include_hidden = false;
        o.limit = 50;
        let s = select(&parse("").unwrap(), &o);
        assert!(s.text.ends_with("WHERE f.hidden = 0 LIMIT ?"), "{}", s.text);
        assert_eq!(s.params, vec![Value::Integer(50)]);
    }

    #[test]
    fn names_use_trigram_index_when_possible() {
        let s = sql("report");
        assert!(s.text.contains("names.name LIKE ?"), "{}", s.text);
        assert_eq!(text(&s.params[0]), "%report%");
        let s = sql("ab");
        assert!(s.text.contains("f.name LIKE ? ESCAPE"), "short words scan: {}", s.text);
        let s = sql("100%_done");
        assert!(s.text.contains("f.name LIKE ? ESCAPE"));
        assert_eq!(text(&s.params[0]), "%100\\%\\_done%");
    }

    #[test]
    fn globs_get_a_literal_prefilter() {
        assert_eq!(glob_literal("*.pdf"), ".pdf");
        assert_eq!(glob_literal("IMG_????.jpg"), "IMG_");
        assert_eq!(glob_literal("a[xyz]bcd*"), "bcd");
        let s = sql("*.PDF");
        assert!(s.text.contains("names.name LIKE ?") && s.text.contains("lower(f.name) GLOB ?"), "{}", s.text);
        assert_eq!(text(&s.params[0]), "%.PDF%");
        assert_eq!(text(&s.params[1]), "*.pdf");
        let s = sql("a*");
        assert!(!s.text.contains("names.name"), "too short for trigrams: {}", s.text);
    }

    #[test]
    fn boolean_structure() {
        let s = sql("a b OR c -d");
        assert!(s.text.contains(" AND ") && s.text.contains(" OR ") && s.text.contains("NOT IFNULL("), "{}", s.text);
        let w = s.text.split(" WHERE ").nth(1).unwrap();
        assert!(w.starts_with("(f.name LIKE"), "{w}");
    }

    #[test]
    fn filters() {
        let s = sql("ext:pdf,DOCX");
        assert!(s.text.contains("f.ext IN (?, ?)"));
        assert_eq!(s.params.iter().map(text).collect::<Vec<_>>(), ["pdf", "docx"]);

        let s = sql("kind:folder");
        assert!(s.text.contains("f.kind IN (1, 3)"));
        let s = sql("kind:image");
        assert!(s.text.contains("f.kind = 0 AND f.ext IN ("));
        assert!(s.params.iter().any(|p| text(p) == "png"));

        let s = sql("size:>1k");
        assert!(s.text.contains("(f.kind = 0 AND f.size >= ?)"), "{}", s.text);
        assert_eq!(s.params, vec![Value::Integer(1025)]);
        let s = sql("size:1k..2k");
        assert!(s.text.contains("f.size >= ? AND f.size < ?"));

        let s = sql("modified:<7d");
        assert!(s.text.contains("(f.mtime IS NOT NULL AND f.mtime >= ?)"), "{}", s.text);
        assert_eq!(s.params, vec![Value::Integer(1_000_000 - 7 * 86_400)]);
        let s = sql("created:>1y");
        assert!(s.text.contains("f.ctime < ?"));

        let s = sql("tag:Red");
        assert!(s.text.contains("SELECT path FROM tags WHERE tags LIKE ?"));
        assert_eq!(text(&s.params[0]), "% red %");

        let s = sql("content:\"say \"\"hi\"\"\"");
        assert!(s.text.contains("contents MATCH ?"));

        let s = sql("path:a/b");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(text(&s.params[0]), format!("%{}%", like_escape(&format!("a{sep}b"))));

        let s = sql("regex:^x");
        assert!(s.text.contains("regexp(?, f.name)"));
    }

    #[test]
    fn scope_and_in_use_path_ranges() {
        let (lo, hi) = descendant_bounds(Path::new(r"C:\data"));
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(lo, format!("C:\\data{sep}"));
        assert!(hi > lo);
        assert!(format!("{lo}x") < hi && format!("{lo}\u{10000}") < hi);
        let (lo2, _) = descendant_bounds(Path::new("C:\\"));
        assert_eq!(lo2, "C:\\", "roots keep their single separator");

        let mut o = opts();
        o.scope = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let s = select(&parse("x").unwrap(), &o);
        assert!(s.text.contains("((f.path >= ? AND f.path < ?) OR (f.path >= ? AND f.path < ?))"), "{}", s.text);
        let s = sql("in:/data");
        assert!(s.text.contains("(f.path >= ? AND f.path < ?)"));
    }

    #[test]
    fn helpers() {
        assert_eq!(like_escape(r"a%b_c\d"), r"a\%b\_c\\d");
        assert_eq!(fts_phrase("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
