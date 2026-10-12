//! Back/forward navigation history of a pane.

use soyuz_core::Location;

/// One remembered place: the location plus where the user was inside it, so going
/// back restores the cursor and scroll position.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub location: Location,
    pub cursor: Option<String>,
    pub scroll: f32,
}

impl HistoryEntry {
    pub fn new(location: Location) -> Self {
        HistoryEntry { location, cursor: None, scroll: 0.0 }
    }
}

#[derive(Clone, Debug)]
pub struct NavHistory {
    /// Most recent last.
    back: Vec<HistoryEntry>,
    /// Next-forward last.
    forward: Vec<HistoryEntry>,
    limit: usize,
}

impl Default for NavHistory {
    fn default() -> Self {
        NavHistory { back: Vec::new(), forward: Vec::new(), limit: 100 }
    }
}

impl NavHistory {
    /// Record `current` before navigating somewhere new: it becomes the back target and
    /// the forward stack is discarded. Navigating to the same location is not recorded.
    pub fn push(&mut self, current: HistoryEntry, target: &Location) {
        if &current.location == target {
            return;
        }
        if self.back.last().is_some_and(|e| e.location == current.location) {
            self.back.pop();
        }
        self.back.push(current);
        if self.back.len() > self.limit {
            let excess = self.back.len() - self.limit;
            self.back.drain(..excess);
        }
        self.forward.clear();
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Go `n` (≥ 1) steps back; `current` is pushed onto the forward stack (and the
    /// skipped entries with it). Returns the entry to show.
    pub fn back_n(&mut self, n: usize, current: HistoryEntry) -> Option<HistoryEntry> {
        if n == 0 || n > self.back.len() {
            return None;
        }
        self.forward.push(current);
        for _ in 1..n {
            let e = self.back.pop()?;
            self.forward.push(e);
        }
        self.back.pop()
    }

    /// Go `n` (≥ 1) steps forward.
    pub fn forward_n(&mut self, n: usize, current: HistoryEntry) -> Option<HistoryEntry> {
        if n == 0 || n > self.forward.len() {
            return None;
        }
        self.back.push(current);
        for _ in 1..n {
            let e = self.forward.pop()?;
            self.back.push(e);
        }
        self.forward.pop()
    }

    /// Back entries, most recent first (for the history dropdown).
    pub fn back_entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.back.iter().rev()
    }

    /// Forward entries, nearest first.
    pub fn forward_entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.forward.iter().rev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(s: &str) -> Location {
        Location::Dir(s.into())
    }

    fn e(s: &str) -> HistoryEntry {
        HistoryEntry::new(loc(s))
    }

    #[test]
    fn back_and_forward() {
        let mut h = NavHistory::default();
        assert!(!h.can_back());
        h.push(e("/a"), &loc("/b"));
        h.push(e("/b"), &loc("/c"));
        // at /c
        let back = h.back_n(1, e("/c")).unwrap();
        assert_eq!(back.location, loc("/b"));
        assert!(h.can_forward());
        let fwd = h.forward_n(1, e("/b")).unwrap();
        assert_eq!(fwd.location, loc("/c"));
        assert!(!h.can_forward());
        // jump two back
        let b2 = h.back_n(2, e("/c")).unwrap();
        assert_eq!(b2.location, loc("/a"));
        let names: Vec<_> = h.forward_entries().map(|e| e.location.clone()).collect();
        assert_eq!(names, [loc("/b"), loc("/c")]);
        assert!(h.back_n(1, e("/a")).is_none());
    }

    #[test]
    fn new_navigation_clears_forward_and_skips_duplicates() {
        let mut h = NavHistory::default();
        h.push(e("/a"), &loc("/b"));
        h.back_n(1, e("/b"));
        h.push(e("/a"), &loc("/x"));
        assert!(!h.can_forward());
        h.push(e("/x"), &loc("/x")); // same place: ignored
        assert_eq!(h.back_entries().count(), 1);
        h.push(e("/x"), &loc("/y"));
        h.push(e("/x"), &loc("/z")); // repeated current replaces, no duplicate
        let back: Vec<_> = h.back_entries().map(|e| e.location.clone()).collect();
        assert_eq!(back, [loc("/x"), loc("/a")]);
    }

    #[test]
    fn cursor_is_kept() {
        let mut h = NavHistory::default();
        h.push(HistoryEntry { location: loc("/a"), cursor: Some("file.txt".into()), scroll: 120.0 }, &loc("/b"));
        let back = h.back_n(1, e("/b")).unwrap();
        assert_eq!(back.cursor.as_deref(), Some("file.txt"));
        assert_eq!(back.scroll, 120.0);
    }

    #[test]
    fn limit_is_enforced() {
        let mut h = NavHistory { limit: 3, ..Default::default() };
        for i in 0..10 {
            h.push(e(&format!("/{i}")), &loc(&format!("/{}", i + 1)));
        }
        assert_eq!(h.back_entries().count(), 3);
        assert_eq!(h.back_entries().next().unwrap().location, loc("/9"));
    }
}
