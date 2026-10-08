//! Name patterns and entry filters.

use serde::{Deserialize, Serialize};

use crate::entry::{Attributes, EntryInfo, EntryKindSet};
use crate::locale::{fold_char_for_match, fold_for_match};

/// Which matcher a [`NamePatternSpec`] compiles to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    #[default]
    Substring,
    Glob,
    Regex,
    Fuzzy,
}

/// Serializable pattern (saved searches, session); [`NamePatternSpec::compile`] yields
/// the matcher.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct NamePatternSpec {
    pub kind: PatternKind,
    pub text: String,
    pub case_sensitive: bool,
}

/// Invalid glob / regex.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    #[error("invalid glob: {0}")]
    Glob(String),
    #[error("invalid regex: {0}")]
    Regex(String),
}

impl NamePatternSpec {
    pub fn new(kind: PatternKind, text: impl Into<String>) -> Self {
        NamePatternSpec { kind, text: text.into(), case_sensitive: false }
    }

    pub fn compile(&self) -> Result<NamePattern, PatternError> {
        let ci = !self.case_sensitive;
        Ok(match self.kind {
            PatternKind::Substring => {
                NamePattern::Substring { needle: if ci { fold_for_match(&self.text) } else { self.text.clone() }, ci }
            }
            PatternKind::Glob => {
                let glob = globset::GlobBuilder::new(&self.text)
                    .case_insensitive(ci)
                    .literal_separator(false)
                    .backslash_escape(false)
                    .build()
                    .map_err(|e| PatternError::Glob(e.to_string()))?;
                NamePattern::Glob(glob.compile_matcher())
            }
            PatternKind::Regex => NamePattern::Regex(
                regex::RegexBuilder::new(&self.text)
                    .case_insensitive(ci)
                    .build()
                    .map_err(|e| PatternError::Regex(e.to_string()))?,
            ),
            PatternKind::Fuzzy => NamePattern::Fuzzy { needle: fold_for_match(&self.text) },
        })
    }
}

/// A compiled name matcher (`Send + Sync`).
#[derive(Clone, Debug)]
pub enum NamePattern {
    /// `needle` is already lower-cased when `ci`.
    Substring {
        needle: String,
        ci: bool,
    },
    Glob(globset::GlobMatcher),
    Regex(regex::Regex),
    /// Case-insensitive subsequence match; `needle` is lower-cased.
    Fuzzy {
        needle: String,
    },
}

impl NamePattern {
    pub fn is_match(&self, name: &str) -> bool {
        match self {
            NamePattern::Substring { needle, ci } => {
                if *ci {
                    fold_for_match(name).contains(needle.as_str())
                } else {
                    name.contains(needle.as_str())
                }
            }
            NamePattern::Glob(g) => g.is_match(name),
            NamePattern::Regex(r) => r.is_match(name),
            NamePattern::Fuzzy { needle } => {
                let mut hay = name.chars().map(fold_char_for_match);
                needle.chars().all(|n| hay.any(|h| h == n))
            }
        }
    }
}

/// Pane / search filter.
#[derive(Clone, Default, Debug)]
pub struct Filter {
    pub name: Option<NamePattern>,
    pub show_hidden: bool,
    pub show_system: bool,
    pub kinds: Option<EntryKindSet>,
    /// Allowed extensions (without dot, case-insensitive); entries without a matching
    /// extension (including directories) are rejected.
    pub ext: Option<Vec<String>>,
}

impl Filter {
    pub fn matches(&self, e: &EntryInfo) -> bool {
        if !self.show_hidden && e.is_hidden() {
            return false;
        }
        if !self.show_system && e.attrs.contains(Attributes::SYSTEM) {
            return false;
        }
        if let Some(kinds) = self.kinds
            && !kinds.contains_kind(e.kind)
        {
            return false;
        }
        if let Some(exts) = &self.ext {
            match e.extension() {
                Some(x) if exts.iter().any(|w| w.trim_start_matches('.').eq_ignore_ascii_case(x)) => {}
                _ => return false,
            }
        }
        if let Some(p) = &self.name
            && !p.is_match(&e.name)
        {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryKind;
    use std::path::PathBuf;

    fn m(kind: PatternKind, text: &str, name: &str) -> bool {
        NamePatternSpec::new(kind, text).compile().unwrap().is_match(name)
    }

    #[test]
    fn turkish_i_matching() {
        use PatternKind::{Fuzzy, Substring};
        assert!(m(Substring, "istanbul", "İSTANBUL.jpg"));
        assert!(m(Substring, "İst", "istanbul.jpg"));
        assert!(m(Substring, "ilik", "ılık su.txt"));
        assert!(m(Substring, "img", "IMG_0001.JPG"));
        assert!(m(Substring, "ışık", "IŞIK.png"));
        assert!(m(Fuzzy, "izm", "İzmir notları.md"));
        assert!(m(Substring, "çağ", "ÇAĞRI.txt"));
        assert!(!m(Substring, "cag", "ÇAĞRI.txt"));
    }

    #[test]
    fn patterns() {
        assert!(m(PatternKind::Substring, "REP", "my report.pdf"));
        assert!(m(PatternKind::Glob, "*.PDF", "my report.pdf"));
        assert!(!m(PatternKind::Glob, "*.pdf", "my report.pdfx"));
        assert!(m(PatternKind::Regex, r"^img_\d+\.jpe?g$", "IMG_0012.JPG"));
        assert!(m(PatternKind::Fuzzy, "mrp", "My Report.pdf"));
        assert!(!m(PatternKind::Fuzzy, "zz", "My Report.pdf"));
        let cs = NamePatternSpec { case_sensitive: true, ..NamePatternSpec::new(PatternKind::Substring, "REP") };
        assert!(!cs.compile().unwrap().is_match("report"));
        assert!(matches!(NamePatternSpec::new(PatternKind::Regex, "(").compile(), Err(PatternError::Regex(_))));
    }

    #[test]
    fn filter_rules() {
        let mut f = Filter::default();
        let mut hidden = EntryInfo::minimal(PathBuf::from("/a/.git"), EntryKind::Dir);
        assert!(!f.matches(&hidden));
        f.show_hidden = true;
        assert!(f.matches(&hidden));
        hidden.attrs |= Attributes::SYSTEM;
        assert!(!f.matches(&hidden));

        let file = EntryInfo::minimal(PathBuf::from("/a/clip.MP4"), EntryKind::File);
        let dir = EntryInfo::minimal(PathBuf::from("/a/videos"), EntryKind::Dir);
        f.ext = Some(vec!["mp4".into()]);
        assert!(f.matches(&file));
        assert!(!f.matches(&dir));
        f.ext = None;
        f.kinds = Some(EntryKindSet::DIR);
        assert!(!f.matches(&file));
        assert!(f.matches(&dir));
        f.kinds = None;
        f.name = Some(NamePatternSpec::new(PatternKind::Glob, "*.mp4").compile().unwrap());
        assert!(f.matches(&file));
        assert!(!f.matches(&dir));
    }
}
