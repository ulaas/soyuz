//! Fuzzy matching for the palette (nucleo-matcher): scores plus matched character
//! indices for highlighting.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// A compiled query. An empty query matches everything with score 0.
pub struct Fuzzy {
    pattern: Option<Pattern>,
    matcher: Matcher,
    buf: Vec<char>,
    indices: Vec<u32>,
}

impl Fuzzy {
    pub fn new(query: &str) -> Self {
        let q = query.trim();
        let pattern = (!q.is_empty()).then(|| Pattern::parse(q, CaseMatching::Smart, Normalization::Smart));
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Fuzzy { pattern, matcher: Matcher::new(config), buf: Vec::new(), indices: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
    }

    /// Score of `hay` (`None` = no match).
    pub fn score(&mut self, hay: &str) -> Option<u32> {
        match &self.pattern {
            None => Some(0),
            Some(p) => p.score(Utf32Str::new(hay, &mut self.buf), &mut self.matcher),
        }
    }

    /// Score plus the sorted char indices of the matched characters.
    pub fn indices(&mut self, hay: &str) -> Option<(u32, Vec<u32>)> {
        let Some(p) = &self.pattern else { return Some((0, Vec::new())) };
        self.indices.clear();
        let s = p.indices(Utf32Str::new(hay, &mut self.buf), &mut self.matcher, &mut self.indices)?;
        let mut v = self.indices.clone();
        v.sort_unstable();
        v.dedup();
        Some((s, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_and_indices() {
        let mut f = Fuzzy::new("nf");
        assert!(f.score("New folder").is_some());
        assert!(f.score("Refresh").is_none());
        let (_, idx) = f.indices("New folder").unwrap();
        assert_eq!(idx, vec![0, 4]);
        let mut e = Fuzzy::new("   ");
        assert!(e.is_empty());
        assert_eq!(e.score("anything"), Some(0));
    }

    #[test]
    fn prefix_beats_scattered() {
        let mut f = Fuzzy::new("copy");
        let a = f.score("Copy").unwrap();
        let b = f.score("Compare checksum by pattern yes").unwrap_or(0);
        assert!(a > b);
    }
}
