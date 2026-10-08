//! Compiled form of a [`SearchQuery`]: name / kind / extension / size / date filters and
//! the nucleo-backed fuzzy scorer.

use std::time::SystemTime;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use soyuz_core::{EntryInfo, EntryKind, EntryKindSet, NamePattern, PatternError, PatternKind};

use crate::content::ContentMatcher;
use crate::query::SearchQuery;

/// Fuzzy scorer over file names (nucleo-matcher). Not `Sync`: keep one per thread.
pub struct FuzzyScorer {
    pattern: Pattern,
    matcher: Matcher,
    buf: Vec<char>,
}

impl std::fmt::Debug for FuzzyScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuzzyScorer").finish_non_exhaustive()
    }
}

impl FuzzyScorer {
    /// `needle` is matched as one fuzzy atom per whitespace-separated word.
    pub fn new(needle: &str, case_sensitive: bool) -> Self {
        let case = if case_sensitive { CaseMatching::Respect } else { CaseMatching::Ignore };
        FuzzyScorer {
            pattern: Pattern::new(needle, case, Normalization::Smart, AtomKind::Fuzzy),
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
        }
    }

    /// `Some(score)` (higher is better) when `name` matches.
    pub fn score(&mut self, name: &str) -> Option<u32> {
        self.pattern.score(Utf32Str::new(name, &mut self.buf), &mut self.matcher)
    }
}

/// One-shot fuzzy score of `name` against `needle` (case-insensitive); `None` = no match.
/// The search streams fuzzy hits unsorted; the app ranks them with this (or a cached
/// [`FuzzyScorer`]).
pub fn fuzzy_score(needle: &str, name: &str) -> Option<u32> {
    FuzzyScorer::new(needle, false).score(name)
}

/// Name matcher used by the walker threads.
#[derive(Clone, Debug)]
pub(crate) enum NameMatch {
    Any,
    Pattern(NamePattern),
    Fuzzy { needle: String, case_sensitive: bool },
}

/// Everything a worker needs to decide whether an entry is a hit.
#[derive(Debug)]
pub(crate) struct Compiled {
    pub name: NameMatch,
    pub content: Option<ContentMatcher>,
    pub content_max_size: u64,
    pub size: Option<(u64, u64)>,
    pub modified: Option<(Option<SystemTime>, Option<SystemTime>)>,
    pub kinds: Option<EntryKindSet>,
    /// Lower-cased, without leading dot.
    pub extensions: Vec<String>,
}

impl Compiled {
    pub fn new(q: &SearchQuery) -> Result<Self, PatternError> {
        let name = match &q.name {
            None => NameMatch::Any,
            Some(spec) if spec.text.is_empty() => NameMatch::Any,
            Some(spec) if spec.kind == PatternKind::Fuzzy => {
                NameMatch::Fuzzy { needle: spec.text.clone(), case_sensitive: spec.case_sensitive }
            }
            Some(spec) => NameMatch::Pattern(spec.compile()?),
        };
        let content = match &q.content {
            Some(c) if !c.text.is_empty() => Some(ContentMatcher::new(c)?),
            _ => None,
        };
        let size = q.size.map(|(lo, hi)| (lo.unwrap_or(0), hi.filter(|&h| h != 0).unwrap_or(u64::MAX)));
        let extensions = q
            .extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        Ok(Compiled {
            name,
            content,
            content_max_size: q.content_max_size,
            size,
            modified: q.modified.filter(|(a, b)| a.is_some() || b.is_some()),
            kinds: q.kinds,
            extensions,
        })
    }

    pub fn scorer(&self) -> Option<FuzzyScorer> {
        match &self.name {
            NameMatch::Fuzzy { needle, case_sensitive } => Some(FuzzyScorer::new(needle, *case_sensitive)),
            _ => None,
        }
    }

    /// Cheap pre-filter on the name and the walker's file type (before any `stat`).
    /// `is_dir` is `None` when unknown.
    pub fn pre_match(&self, name: &str, is_dir: Option<bool>, scorer: Option<&mut FuzzyScorer>) -> bool {
        let dir = is_dir == Some(true);
        if dir && (self.content.is_some() || !self.extensions.is_empty() || self.size.is_some()) {
            return false;
        }
        if !self.extensions.is_empty() {
            let Some(ext) = name_extension(name) else { return false };
            if !self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                return false;
            }
        }
        if let Some(kinds) = self.kinds {
            match is_dir {
                Some(true) if !kinds.intersects(EntryKindSet::DIR | EntryKindSet::SYMLINK | EntryKindSet::JUNCTION) => {
                    return false;
                }
                Some(false) if kinds == EntryKindSet::DIR => return false,
                _ => {}
            }
        }
        match &self.name {
            NameMatch::Any => true,
            NameMatch::Pattern(p) => p.is_match(name),
            NameMatch::Fuzzy { .. } => scorer.is_some_and(|s| s.score(name).is_some()),
        }
    }

    /// Metadata filters (after `stat`).
    pub fn meta_match(&self, e: &EntryInfo) -> bool {
        if let Some(kinds) = self.kinds
            && !kinds.contains_kind(e.kind)
        {
            return false;
        }
        if let Some((lo, hi)) = self.size {
            // Size filters only make sense for files.
            if e.is_dir_like() || e.size < lo || e.size > hi {
                return false;
            }
        }
        if let Some((lo, hi)) = self.modified {
            let Some(m) = e.modified else { return false };
            if lo.is_some_and(|lo| m < lo) || hi.is_some_and(|hi| m > hi) {
                return false;
            }
        }
        if self.content.is_some() && !is_file_like(e.kind) {
            return false;
        }
        true
    }
}

/// Regular files and symlinks to files.
pub(crate) fn is_file_like(k: EntryKind) -> bool {
    matches!(k, EntryKind::File | EntryKind::Symlink { target: soyuz_core::LinkTarget::File })
}

fn name_extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| &name[dot + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_scores_rank() {
        let tight = fuzzy_score("rep", "report.txt").unwrap();
        let loose = fuzzy_score("rep", "xrxxexxp.txt").unwrap();
        assert!(tight > loose);
        assert!(fuzzy_score("zzz", "report.txt").is_none());
        let mut s = FuzzyScorer::new("REP", true);
        assert!(s.score("report").is_none());
        assert!(s.score("REPORT").is_some());
    }

    #[test]
    fn size_bounds_zero_is_unbounded() {
        let q = SearchQuery { size: Some((Some(10), Some(0))), ..Default::default() };
        assert_eq!(Compiled::new(&q).unwrap().size, Some((10, u64::MAX)));
    }

    #[test]
    fn ext_prefilter() {
        let q = SearchQuery { extensions: vec![".RS".into()], ..Default::default() };
        let c = Compiled::new(&q).unwrap();
        assert!(c.pre_match("main.rs", Some(false), None));
        assert!(!c.pre_match("main.rsx", Some(false), None));
        assert!(!c.pre_match("src.rs", Some(true), None));
        assert!(!c.pre_match(".rs", Some(false), None));
    }
}
