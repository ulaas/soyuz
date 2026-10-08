//! Name-keyed selection (PLAN §2.8): survives sorting, filtering, watcher patches and
//! relists because it never stores indices. All operations take the *visible order* of
//! keys when they need ordering (ranges, select-all, invert).

use std::collections::BTreeSet;
use std::sync::Arc;

/// Stable identity of an entry inside one directory listing: its name.
pub type EntryKey = Arc<str>;

/// Modifier state of a click, decoupled from egui for testability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClickMods {
    /// Ctrl (Cmd on macOS): toggle / additive.
    pub toggle: bool,
    /// Shift: range from the anchor.
    pub range: bool,
}

/// How a rubber band combines with the selection that existed when the drag started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandMode {
    /// Plain drag: the band *is* the selection.
    Replace,
    /// Shift+drag: union with the base selection.
    Add,
    /// Ctrl+drag: symmetric difference with the base selection.
    Toggle,
}

impl BandMode {
    pub fn from_mods(m: ClickMods) -> Self {
        if m.toggle {
            BandMode::Toggle
        } else if m.range {
            BandMode::Add
        } else {
            BandMode::Replace
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    keys: BTreeSet<EntryKey>,
    anchor: Option<EntryKey>,
}

impl Selection {
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = &EntryKey> {
        self.keys.iter()
    }

    pub fn anchor(&self) -> Option<&EntryKey> {
        self.anchor.as_ref()
    }

    pub fn set_anchor(&mut self, key: Option<EntryKey>) {
        self.anchor = key;
    }

    pub fn clear(&mut self) {
        self.keys.clear();
        self.anchor = None;
    }

    pub fn insert(&mut self, key: EntryKey) {
        self.keys.insert(key);
    }

    pub fn remove(&mut self, key: &str) {
        self.keys.remove(key);
    }

    /// Select exactly `key` and make it the anchor.
    pub fn select_only(&mut self, key: EntryKey) {
        self.keys.clear();
        self.keys.insert(key.clone());
        self.anchor = Some(key);
    }

    /// Toggle `key` and make it the anchor.
    pub fn toggle(&mut self, key: EntryKey) {
        if !self.keys.remove(&key) {
            self.keys.insert(key.clone());
        }
        self.anchor = Some(key);
    }

    /// Select the inclusive range between `from` and `to` in `order`. With `additive`
    /// the range is added to the existing selection, otherwise it replaces it. Keys
    /// missing from `order` select just `to`. The anchor is left untouched.
    pub fn select_range(&mut self, order: &[EntryKey], from: &str, to: &str, additive: bool) {
        let a = order.iter().position(|k| &**k == from);
        let b = order.iter().position(|k| &**k == to);
        if !additive {
            self.keys.clear();
        }
        match (a, b) {
            (Some(a), Some(b)) => {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                self.keys.extend(order[lo..=hi].iter().cloned());
            }
            (_, Some(b)) => {
                self.keys.insert(order[b].clone());
            }
            _ => {}
        }
    }

    /// Standard click semantics: plain = select only, Ctrl = toggle, Shift = range from
    /// the anchor (replacing), Ctrl+Shift = range added to the selection.
    pub fn click(&mut self, order: &[EntryKey], key: EntryKey, mods: ClickMods) {
        match (mods.toggle, mods.range) {
            (_, true) => match self.anchor.clone() {
                Some(anchor) => self.select_range(order, &anchor, &key, mods.toggle),
                None => self.select_only(key),
            },
            (true, false) => self.toggle(key),
            (false, false) => self.select_only(key),
        }
    }

    pub fn select_all(&mut self, order: &[EntryKey]) {
        self.keys = order.iter().cloned().collect();
    }

    /// Invert within `order` (selected keys outside `order`, e.g. filtered out, are dropped).
    pub fn invert(&mut self, order: &[EntryKey]) {
        let next: BTreeSet<EntryKey> = order.iter().filter(|k| !self.keys.contains(&***k)).cloned().collect();
        self.keys = next;
    }

    /// Drop keys for which `exists` is false (after a relist / patch).
    pub fn retain(&mut self, mut exists: impl FnMut(&str) -> bool) {
        self.keys.retain(|k| exists(k));
        if self.anchor.as_deref().is_some_and(|a| !exists(a)) {
            self.anchor = None;
        }
    }

    /// The selection produced by a rubber band that currently covers `hits`, given the
    /// selection `base` captured when the drag began.
    pub fn from_band(base: &Selection, hits: &[EntryKey], mode: BandMode) -> Selection {
        let mut keys = match mode {
            BandMode::Replace => BTreeSet::new(),
            BandMode::Add | BandMode::Toggle => base.keys.clone(),
        };
        for h in hits {
            match mode {
                BandMode::Replace | BandMode::Add => {
                    keys.insert(h.clone());
                }
                BandMode::Toggle => {
                    if base.keys.contains(h) {
                        keys.remove(h);
                    } else {
                        keys.insert(h.clone());
                    }
                }
            }
        }
        Selection { keys, anchor: hits.first().cloned().or_else(|| base.anchor.clone()) }
    }

    /// Selected keys in `order` order.
    pub fn ordered(&self, order: &[EntryKey]) -> Vec<EntryKey> {
        order.iter().filter(|k| self.keys.contains(&***k)).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[&str]) -> Vec<EntryKey> {
        v.iter().map(|s| EntryKey::from(*s)).collect()
    }

    fn sel(s: &Selection) -> Vec<String> {
        s.iter().map(|k| k.to_string()).collect()
    }

    const PLAIN: ClickMods = ClickMods { toggle: false, range: false };
    const CTRL: ClickMods = ClickMods { toggle: true, range: false };
    const SHIFT: ClickMods = ClickMods { toggle: false, range: true };
    const CTRL_SHIFT: ClickMods = ClickMods { toggle: true, range: true };

    #[test]
    fn click_modes() {
        let order = keys(&["a", "b", "c", "d", "e"]);
        let mut s = Selection::default();
        s.click(&order, "b".into(), PLAIN);
        assert_eq!(sel(&s), ["b"]);
        s.click(&order, "d".into(), SHIFT);
        assert_eq!(sel(&s), ["b", "c", "d"]);
        assert_eq!(s.anchor().map(|a| &**a), Some("b"), "shift keeps the anchor");
        s.click(&order, "a".into(), SHIFT);
        assert_eq!(sel(&s), ["a", "b"], "range replaces and can go upwards");
        s.click(&order, "e".into(), CTRL);
        assert_eq!(sel(&s), ["a", "b", "e"]);
        s.click(&order, "a".into(), CTRL);
        assert_eq!(sel(&s), ["b", "e"]);
        // anchor is now "a" (ctrl-click moves it); ctrl+shift adds a range
        s.click(&order, "c".into(), CTRL_SHIFT);
        assert_eq!(sel(&s), ["a", "b", "c", "e"]);
    }

    #[test]
    fn shift_without_anchor_selects_one() {
        let order = keys(&["a", "b"]);
        let mut s = Selection::default();
        s.click(&order, "b".into(), SHIFT);
        assert_eq!(sel(&s), ["b"]);
    }

    #[test]
    fn invert_and_all() {
        let order = keys(&["a", "b", "c"]);
        let mut s = Selection::default();
        s.insert("b".into());
        s.insert("zzz".into()); // filtered out
        s.invert(&order);
        assert_eq!(sel(&s), ["a", "c"]);
        s.select_all(&order);
        assert_eq!(s.len(), 3);
        assert_eq!(s.ordered(&keys(&["c", "a"])), keys(&["c", "a"]));
    }

    #[test]
    fn retain_drops_missing_and_anchor() {
        let mut s = Selection::default();
        s.select_only("gone".into());
        s.insert("kept".into());
        s.retain(|k| k == "kept");
        assert_eq!(sel(&s), ["kept"]);
        assert!(s.anchor().is_none());
    }

    #[test]
    fn rubber_band_modes() {
        let mut base = Selection::default();
        base.insert("a".into());
        base.insert("b".into());
        let hits = keys(&["b", "c"]);
        assert_eq!(sel(&Selection::from_band(&base, &hits, BandMode::Replace)), ["b", "c"]);
        assert_eq!(sel(&Selection::from_band(&base, &hits, BandMode::Add)), ["a", "b", "c"]);
        assert_eq!(sel(&Selection::from_band(&base, &hits, BandMode::Toggle)), ["a", "c"]);
        // Shrinking the band back to nothing restores the base in additive modes.
        assert_eq!(sel(&Selection::from_band(&base, &[], BandMode::Toggle)), ["a", "b"]);
        assert!(Selection::from_band(&base, &[], BandMode::Replace).is_empty());
        assert_eq!(BandMode::from_mods(CTRL), BandMode::Toggle);
        assert_eq!(BandMode::from_mods(SHIFT), BandMode::Add);
        assert_eq!(BandMode::from_mods(PLAIN), BandMode::Replace);
    }
}
