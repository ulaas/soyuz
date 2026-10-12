//! Fuzzy matching for the palette (nucleo-matcher): scores plus matched character
//! indices for highlighting. Query and haystack are case-folded with
//! [`soyuz_core::locale::fold_char_for_match`] (one char per char, so indices still point into
//! the original text): Turkish dotted / dotless i match each other and plain `i`.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// A compiled query. An empty query matches everything with score 0.
pub struct Fuzzy {
    pattern: Option<Pattern>,
    matcher: Matcher,
    buf: Vec<char>,
    folded: String,
    indices: Vec<u32>,
}

impl Fuzzy {
    pub fn new(query: &str) -> Self {
        let q: String = query.trim().chars().map(soyuz_core::locale::fold_char_for_match).collect();
        let pattern = (!q.is_empty()).then(|| Pattern::parse(&q, CaseMatching::Ignore, Normalization::Smart));
        let mut config = Config::DEFAULT;
        config.prefer_prefix = true;
        Fuzzy { pattern, matcher: Matcher::new(config), buf: Vec::new(), folded: String::new(), indices: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
    }

    fn fold(&mut self, hay: &str) {
        self.folded.clear();
        self.folded.extend(hay.chars().map(soyuz_core::locale::fold_char_for_match));
    }

    /// Score of `hay` (`None` = no match).
    pub fn score(&mut self, hay: &str) -> Option<u32> {
        if self.pattern.is_none() {
            return Some(0);
        }
        self.fold(hay);
        let p = self.pattern.as_ref()?;
        p.score(Utf32Str::new(&self.folded, &mut self.buf), &mut self.matcher)
    }

    /// Score plus the sorted char indices of the matched characters.
    pub fn indices(&mut self, hay: &str) -> Option<(u32, Vec<u32>)> {
        if self.pattern.is_none() {
            return Some((0, Vec::new()));
        }
        self.fold(hay);
        self.indices.clear();
        let p = self.pattern.as_ref()?;
        let s = p.indices(Utf32Str::new(&self.folded, &mut self.buf), &mut self.matcher, &mut self.indices)?;
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
    fn turkish_i_folds() {
        // Typing a plain "i" finds İ, ı and I; highlights index the original text.
        let mut f = Fuzzy::new("ic");
        assert!(f.score("İçerik").is_some());
        assert!(f.score("Işık").is_none());
        let (_, idx) = Fuzzy::new("isik").indices("Işık").unwrap();
        assert_eq!(idx, vec![0, 1, 2, 3]);
        assert!(Fuzzy::new("İPTAL").score("iptal").is_some());
        assert!(Fuzzy::new("ı").score("Dosya listesi").is_some());
    }

    #[test]
    fn prefix_beats_scattered() {
        let mut f = Fuzzy::new("copy");
        let a = f.score("Copy").unwrap();
        let b = f.score("Compare checksum by pattern yes").unwrap_or(0);
        assert!(a > b);
    }
}
