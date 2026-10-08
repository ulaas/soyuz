//! Query evaluation without the index: a [`Matcher`] checks entries from a live
//! `soyuz-search` walk, and [`to_search_query`] narrows the walk with the filters the
//! walker understands, so only the remaining terms are checked app-side.

use std::path::{Component, Path};
use std::time::SystemTime;

use soyuz_core::{EntryInfo, EntryKind, EntryKindSet, NamePatternSpec, PatternKind};
use soyuz_search::{ContentPatternSpec, SearchQuery};

use crate::query::{Expr, KindClass, Term, TimeBounds, local_offset, unix_secs};

/// Answers `tag:` terms (path, lower-case tag slug).
pub type TagLookup<'a> = &'a dyn Fn(&Path, &str) -> bool;

enum Compiled {
    And(Vec<Compiled>),
    Or(Vec<Compiled>),
    Not(Box<Compiled>),
    Name(String),
    Glob(globset::GlobMatcher),
    Regex(regex::Regex),
    Path(String),
    In(std::path::PathBuf),
    Ext(Vec<String>),
    Kind(KindClass),
    Size(crate::query::Range),
    Modified(TimeBounds),
    Created(TimeBounds),
    Tag(String),
    /// Content terms are checked by the walker (see [`to_search_query`]); at this level
    /// they are assumed to hold.
    Content,
}

/// A compiled query for checking entries one by one. `Send + Sync`.
pub struct Matcher {
    root: Compiled,
}

impl std::fmt::Debug for Matcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Matcher")
    }
}

impl Matcher {
    /// Compile `expr`; relative dates are resolved against `now`.
    pub fn new(expr: &Expr, now: SystemTime) -> Result<Matcher, String> {
        let secs = unix_secs(now);
        let offset = local_offset(secs);
        Ok(Matcher { root: compile(expr, secs, offset)? })
    }

    /// Whether `e` matches. `tags` answers `tag:` terms.
    pub fn matches(&self, e: &EntryInfo, tags: TagLookup<'_>) -> bool {
        eval(&self.root, e, tags)
    }
}

fn compile(e: &Expr, now: i64, offset: i64) -> Result<Compiled, String> {
    Ok(match e {
        Expr::And(v) => Compiled::And(v.iter().map(|x| compile(x, now, offset)).collect::<Result<_, _>>()?),
        Expr::Or(v) => Compiled::Or(v.iter().map(|x| compile(x, now, offset)).collect::<Result<_, _>>()?),
        Expr::Not(x) => Compiled::Not(Box::new(compile(x, now, offset)?)),
        Expr::Term(t) => match t {
            Term::Name(s) => Compiled::Name(s.to_lowercase()),
            Term::NameGlob(g) => Compiled::Glob(
                globset::GlobBuilder::new(g)
                    .case_insensitive(true)
                    .literal_separator(false)
                    .backslash_escape(false)
                    .build()
                    .map_err(|e| format!("invalid glob: {e}"))?
                    .compile_matcher(),
            ),
            Term::Regex(r) => Compiled::Regex(
                regex::RegexBuilder::new(r)
                    .case_insensitive(true)
                    .build()
                    .map_err(|e| format!("invalid regex: {e}"))?,
            ),
            Term::Path(s) => Compiled::Path(normalize_seps(&s.to_lowercase())),
            Term::In(p) => Compiled::In(p.clone()),
            Term::Ext(v) => Compiled::Ext(v.iter().map(|x| x.to_lowercase()).collect()),
            Term::Kind(k) => Compiled::Kind(*k),
            Term::Size(r) => Compiled::Size(*r),
            Term::Modified(s) => Compiled::Modified(s.bounds_at(now, offset)),
            Term::Created(s) => Compiled::Created(s.bounds_at(now, offset)),
            Term::Tag(t) => Compiled::Tag(t.to_lowercase()),
            Term::Content(_) => Compiled::Content,
        },
    })
}

fn normalize_seps(s: &str) -> String {
    s.chars().map(|c| if c == '/' || c == '\\' { std::path::MAIN_SEPARATOR } else { c }).collect()
}

/// The file name part of an entry (search hits may carry a relative path as `name`).
fn file_name(e: &EntryInfo) -> String {
    e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| e.name.clone())
}

fn is_dir(e: &EntryInfo) -> bool {
    matches!(e.kind, EntryKind::Dir | EntryKind::Junction)
}

fn ext_of(name: &str) -> Option<String> {
    let i = name.rfind('.')?;
    (i > 0 && i + 1 < name.len()).then(|| name[i + 1..].to_lowercase())
}

/// `path` lies strictly below `dir` (component-wise; case-insensitive on Windows).
pub fn is_under(path: &Path, dir: &Path) -> bool {
    let norm = |c: Component<'_>| {
        let s = c.as_os_str().to_string_lossy();
        if cfg!(windows) { s.to_lowercase() } else { s.into_owned() }
    };
    let mut pc = path.components().filter(|c| !matches!(c, Component::CurDir));
    let mut n = 0;
    for dc in dir.components().filter(|c| !matches!(c, Component::CurDir)) {
        match pc.next() {
            Some(p) if norm(p) == norm(dc) => n += 1,
            _ => return false,
        }
    }
    n > 0 && pc.next().is_some()
}

fn in_bounds(t: Option<SystemTime>, b: &TimeBounds) -> bool {
    let Some(t) = t else { return false };
    let s = unix_secs(t);
    b.0.is_none_or(|lo| s >= lo) && b.1.is_none_or(|hi| s < hi)
}

fn eval(c: &Compiled, e: &EntryInfo, tags: TagLookup<'_>) -> bool {
    match c {
        Compiled::And(v) => v.iter().all(|x| eval(x, e, tags)),
        Compiled::Or(v) => v.iter().any(|x| eval(x, e, tags)),
        Compiled::Not(x) => !eval(x, e, tags),
        Compiled::Name(s) => file_name(e).to_lowercase().contains(s.as_str()),
        Compiled::Glob(g) => g.is_match(file_name(e)),
        Compiled::Regex(r) => r.is_match(&file_name(e)),
        Compiled::Path(s) => e.path.to_string_lossy().to_lowercase().contains(s.as_str()),
        Compiled::In(dir) => is_under(&e.path, dir),
        Compiled::Ext(v) => !is_dir(e) && ext_of(&file_name(e)).is_some_and(|x| v.contains(&x)),
        Compiled::Kind(KindClass::File) => e.kind == EntryKind::File,
        Compiled::Kind(KindClass::Folder) => is_dir(e),
        Compiled::Kind(k) => {
            e.kind == EntryKind::File && ext_of(&file_name(e)).is_some_and(|x| k.extensions().contains(&x.as_str()))
        }
        Compiled::Size(r) => e.kind == EntryKind::File && r.contains(e.size),
        Compiled::Modified(b) => in_bounds(e.modified, b),
        Compiled::Created(b) => in_bounds(e.created, b),
        Compiled::Tag(t) => tags(&e.path, t),
        Compiled::Content => true,
    }
}

/// Narrow `base` (roots, hidden, gitignore, limits …) with the filters of `expr` that the
/// walker can apply itself: one required name term, a required extension / kind list,
/// size and date bounds, and one required content term. The result may still return
/// entries that don't match the whole query; check every hit with a [`Matcher`].
///
/// Returns an error when the query needs file contents in a way the walker can't
/// express (a `content:` term under OR / NOT, or more than one).
pub fn to_search_query(expr: &Expr, mut base: SearchQuery, now: SystemTime) -> Result<SearchQuery, String> {
    let secs = unix_secs(now);
    let offset = local_offset(secs);
    let required = expr.required_terms();
    let contents = expr.terms().iter().filter(|t| matches!(t, Term::Content(_))).count();
    let required_contents: Vec<&String> = required
        .iter()
        .filter_map(|t| match t {
            Term::Content(s) => Some(s),
            _ => None,
        })
        .collect();
    if contents > 0 && (contents > 1 || required_contents.len() != 1) {
        return Err("Searching contents inside OR / NOT or with several content: terms needs the index".into());
    }
    if let Some(s) = required_contents.first() {
        base.content = Some(ContentPatternSpec { regex: false, text: (*s).clone(), case_sensitive: false });
    }
    let mut files_only = false;
    for t in required {
        match t {
            Term::Name(s) if base.name.is_none() => {
                base.name = Some(NamePatternSpec::new(PatternKind::Substring, s.clone()));
            }
            Term::NameGlob(g) if base.name.is_none() => {
                base.name = Some(NamePatternSpec::new(PatternKind::Glob, g.clone()));
            }
            Term::Regex(r) if base.name.is_none() => {
                base.name = Some(NamePatternSpec::new(PatternKind::Regex, r.clone()));
            }
            Term::Ext(v) if base.extensions.is_empty() => base.extensions = v.clone(),
            Term::Kind(KindClass::File) => files_only = true,
            Term::Kind(KindClass::Folder) => {
                base.kinds =
                    Some(base.kinds.unwrap_or(EntryKindSet::all()) & (EntryKindSet::DIR | EntryKindSet::JUNCTION))
            }
            Term::Kind(k) => {
                files_only = true;
                if base.extensions.is_empty() {
                    base.extensions = k.extensions().iter().map(|s| s.to_string()).collect();
                }
            }
            Term::Size(r) => {
                files_only = true;
                let (lo, hi) = base.size.unwrap_or((None, None));
                let hi = hi.filter(|h| *h != 0);
                // SearchQuery bounds are inclusive; ours are half-open.
                let new_lo = r.lo.into_iter().chain(lo).max();
                let new_hi = r.hi.map(|h| h.saturating_sub(1)).into_iter().chain(hi).min();
                base.size = Some((new_lo, new_hi));
            }
            Term::Modified(spec) => {
                let (lo, hi) = spec.bounds_at(secs, offset);
                let to_time = |s: i64| {
                    if s >= 0 {
                        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(s as u64))
                    } else {
                        std::time::UNIX_EPOCH.checked_sub(std::time::Duration::from_secs(s.unsigned_abs()))
                    }
                };
                let (blo, bhi) = base.modified.unwrap_or((None, None));
                let lo = lo.and_then(to_time).into_iter().chain(blo).max();
                let hi = hi.and_then(to_time).into_iter().chain(bhi).min();
                base.modified = Some((lo, hi));
            }
            _ => {}
        }
    }
    if files_only || base.content.is_some() {
        base.kinds = Some(base.kinds.unwrap_or(EntryKindSet::all()) & EntryKindSet::FILE);
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;
    use std::path::PathBuf;
    use std::time::Duration;

    fn file(path: &str, size: u64, age_days: u64) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from(path), EntryKind::File);
        e.size = size;
        e.modified = SystemTime::now().checked_sub(Duration::from_secs(age_days * 86_400 + 60));
        e
    }

    fn m(q: &str, e: &EntryInfo) -> bool {
        let no_tags = |_: &Path, _: &str| false;
        Matcher::new(&parse(q).unwrap(), SystemTime::now()).unwrap().matches(e, &no_tags)
    }

    #[test]
    fn evaluates_terms() {
        let e = file("/data/docs/Annual Report.PDF", 6 << 20, 3);
        assert!(m("report", &e));
        assert!(m("annual report", &e));
        assert!(m("\"annual report\"", &e));
        assert!(!m("\"report annual\"", &e));
        assert!(m("*.pdf", &e));
        assert!(m("ext:pdf", &e) && !m("ext:doc", &e));
        assert!(m("kind:document", &e) && m("kind:file", &e) && !m("kind:folder", &e) && !m("kind:image", &e));
        assert!(m("size:>5MB", &e) && !m("size:<5MB", &e));
        assert!(m("modified:<7d", &e) && !m("modified:<1d", &e) && m("modified:>1d", &e));
        assert!(m("path:docs", &e) && !m("path:music", &e));
        assert!(m("in:/data", &e) && m("in:/data/docs", &e) && !m("in:/other", &e) && !m("in:/data/do", &e));
        assert!(m("regex:^annual", &e));
        assert!(m("-draft", &e) && !m("-report", &e));
        assert!(m("draft OR report", &e) && !m("draft OR memo", &e));
        assert!(m("(draft OR report) -(memo)", &e));
        assert!(m("", &e));
    }

    #[test]
    fn folders_and_tags() {
        let d = EntryInfo::minimal(PathBuf::from("/data/photos"), EntryKind::Dir);
        assert!(m("kind:folder", &d) && !m("kind:file", &d) && !m("ext:photos", &d));
        assert!(!m("size:<1k", &d), "size filters only apply to files");
        let tagged = |p: &Path, t: &str| p.ends_with("photos") && t == "red";
        let mt = Matcher::new(&parse("tag:red").unwrap(), SystemTime::now()).unwrap();
        assert!(mt.matches(&d, &tagged));
        assert!(!mt.matches(&EntryInfo::minimal(PathBuf::from("/x"), EntryKind::Dir), &tagged));
        // Unknown times never match a date filter.
        assert!(!m("modified:<7d", &d));
        assert!(m("-modified:<7d", &d));
    }

    #[test]
    fn under() {
        assert!(is_under(Path::new("/a/b/c"), Path::new("/a")));
        assert!(!is_under(Path::new("/a"), Path::new("/a")));
        assert!(!is_under(Path::new("/ab/c"), Path::new("/a")));
        if cfg!(windows) {
            assert!(is_under(Path::new(r"C:\Users\Me\x"), Path::new(r"c:\users\me")));
        }
    }

    #[test]
    fn builds_walker_prefilters() {
        let base = SearchQuery { roots: vec!["/r".into()], ..Default::default() };
        let q =
            to_search_query(&parse("report ext:pdf size:>1k modified:<7d").unwrap(), base.clone(), SystemTime::now())
                .unwrap();
        assert_eq!(q.name.as_ref().unwrap().text, "report");
        assert_eq!(q.extensions, ["pdf"]);
        assert_eq!(q.size, Some((Some(1025), None)));
        assert!(q.modified.unwrap().0.is_some());
        assert_eq!(q.kinds, Some(EntryKindSet::FILE));
        assert_eq!(q.roots, base.roots);

        let q = to_search_query(&parse("a OR b").unwrap(), base.clone(), SystemTime::now()).unwrap();
        assert!(q.name.is_none(), "OR can't narrow the walk");
        let q = to_search_query(&parse("kind:image").unwrap(), base.clone(), SystemTime::now()).unwrap();
        assert!(q.extensions.contains(&"png".to_string()));
        let q = to_search_query(&parse("kind:folder").unwrap(), base.clone(), SystemTime::now()).unwrap();
        assert_eq!(q.kinds, Some(EntryKindSet::DIR | EntryKindSet::JUNCTION));
        let q = to_search_query(&parse("size:1k..2k").unwrap(), base.clone(), SystemTime::now()).unwrap();
        assert_eq!(q.size, Some((Some(1024), Some(2048))), "inclusive walker bounds");

        let q = to_search_query(&parse("content:todo *.rs").unwrap(), base.clone(), SystemTime::now()).unwrap();
        assert_eq!(q.content.unwrap().text, "todo");
        assert_eq!(q.name.unwrap().kind, PatternKind::Glob);
        assert!(to_search_query(&parse("content:a OR x").unwrap(), base.clone(), SystemTime::now()).is_err());
        assert!(to_search_query(&parse("content:a content:b").unwrap(), base, SystemTime::now()).is_err());
    }
}
