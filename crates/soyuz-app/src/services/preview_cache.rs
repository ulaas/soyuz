//! Byte-budgeted LRU of finished previews (`[preview] preview_cache_mib`), shared by the
//! preview pane and Quick Look: going back to a file shows its preview instantly.
//!
//! Only self-contained results are cached (images, text, Markdown, SVG, archive
//! listings). Hex views page from disk and folder previews stream sizes, so they always
//! reload. Sizes are estimates of the decoded data (texture pixels, laid-out text).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use parking_lot::Mutex;

use super::preview::{Content, Loaded};

/// Default budget (bytes) until the config is applied.
pub const DEFAULT_BUDGET: u64 = 96 * 1024 * 1024;

static BUDGET: AtomicU64 = AtomicU64::new(DEFAULT_BUDGET);

/// What a cached preview was made from (and for).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
    /// Rendering inputs that change the result (image size limit, theme, text cap).
    pub variant: (u32, bool, usize),
}

struct Entry {
    loaded: Loaded,
    bytes: u64,
    used: u64,
}

#[derive(Default)]
struct Cache {
    map: HashMap<CacheKey, Entry>,
    total: u64,
    tick: u64,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Set the memory budget (`[preview] preview_cache_mib`); shrinks the cache at once.
pub fn set_budget_bytes(bytes: u64) {
    BUDGET.store(bytes, Ordering::Relaxed);
    let mut g = CACHE.lock();
    if let Some(c) = g.as_mut() {
        trim(c, bytes);
    }
}

/// `(bytes, previews)` currently cached.
pub fn memory() -> (u64, u64) {
    CACHE.lock().as_ref().map_or((0, 0), |c| (c.total, c.map.len() as u64))
}

/// Estimated memory of a preview, or `None` when it is not cacheable.
pub fn cost(l: &Loaded) -> Option<u64> {
    let facts: u64 = l.facts.iter().map(|(a, b)| (a.len() + b.len()) as u64).sum();
    let body = match &l.content {
        Content::Image(img) => {
            let [w, h] = img.tex.size();
            (w * h * 4) as u64
        }
        Content::Text(t) => t
            .lines
            .iter()
            .map(|j| (j.text.len() + j.sections.len() * std::mem::size_of::<egui::text::LayoutSection>()) as u64)
            .sum(),
        Content::Markdown { blocks, .. } => blocks.len() as u64 * 256,
        Content::Svg { bytes, .. } => bytes.len() as u64,
        Content::Archive(a) => a.entries.iter().map(|(p, _, _)| p.len() as u64 + 24).sum(),
        Content::Hex(_) | Content::Folder(_) | Content::Media { .. } | Content::Message { .. } => return None,
    };
    Some(body + facts + 256)
}

/// A cached preview for `key` (refreshes its LRU position).
pub fn get(key: &CacheKey) -> Option<Loaded> {
    let mut g = CACHE.lock();
    let c = g.as_mut()?;
    c.tick += 1;
    let tick = c.tick;
    let e = c.map.get_mut(key)?;
    e.used = tick;
    Some(e.loaded.clone())
}

/// Remember a finished preview (ignored when not cacheable or larger than the budget).
pub fn put(key: CacheKey, loaded: &Loaded) {
    let Some(bytes) = cost(loaded) else { return };
    let budget = BUDGET.load(Ordering::Relaxed);
    if bytes > budget / 2 {
        return;
    }
    let mut g = CACHE.lock();
    let c = g.get_or_insert_with(Cache::default);
    c.tick += 1;
    let used = c.tick;
    if let Some(old) = c.map.insert(key, Entry { loaded: loaded.clone(), bytes, used }) {
        c.total -= old.bytes;
    }
    c.total += bytes;
    trim(c, budget);
}

fn trim(c: &mut Cache, budget: u64) {
    while c.total > budget {
        let Some(oldest) = c.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone()) else { break };
        if let Some(e) = c.map.remove(&oldest) {
            c.total -= e.bytes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::preview::{ArchiveListing, Kind};

    fn listing(n: usize) -> Loaded {
        Loaded {
            kind: Kind::Archive,
            content: Content::Archive(ArchiveListing {
                entries: (0..n).map(|i| (format!("entry-{i:04}"), false, 1)).collect(),
                ..Default::default()
            }),
            facts: Vec::new(),
            meta: None,
        }
    }

    fn key(i: u32) -> CacheKey {
        CacheKey { path: format!("/a{i}.zip").into(), modified: None, size: 1, variant: (0, true, 0) }
    }

    #[test]
    fn lru_within_budget() {
        let one = cost(&listing(100)).unwrap();
        set_budget_bytes(one * 3);
        for i in 0..4 {
            put(key(i), &listing(100));
            if i == 2 {
                assert!(get(&key(0)).is_some(), "touch 0 so it outlives 1");
            }
        }
        let (bytes, n) = memory();
        assert!(bytes <= one * 3 && n == 3, "{bytes} {n}");
        assert!(get(&key(0)).is_some(), "recently used survives");
        assert!(get(&key(1)).is_none(), "least recently used evicted");
        assert!(cost(&Loaded { content: Content::Folder(Default::default()), ..listing(0) }).is_none());
        set_budget_bytes(0);
        assert_eq!(memory().1, 0);
        set_budget_bytes(DEFAULT_BUDGET);
    }
}
