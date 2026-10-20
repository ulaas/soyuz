//! Text of documents (PDF) for content search and the index's content words.
//!
//! Only the hook lives here: the app registers an extractor with [`set_extractor`] (the
//! PDF reader is an app service), and `soyuz-search` / `soyuz-index` call [`extract`]
//! for files with one of its extensions instead of reading them as plain text. With no
//! extractor registered, documents are skipped as binary, as before.

use std::path::Path;
use std::sync::{Arc, RwLock};

/// Returns the text of the document at the path, or why it couldn't be read.
pub type Extractor = dyn Fn(&Path) -> Result<String, String> + Send + Sync;

struct Hook {
    extensions: Vec<String>,
    max_bytes: u64,
    extract: Arc<Extractor>,
}

static HOOK: RwLock<Option<Hook>> = RwLock::new(None);

/// Register `extract` for files with these extensions (lower-case, no dot) up to
/// `max_bytes` on disk. Replaces any earlier extractor.
pub fn set_extractor(extensions: &[&str], max_bytes: u64, extract: Arc<Extractor>) {
    let extensions = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
    if let Ok(mut h) = HOOK.write() {
        *h = Some(Hook { extensions, max_bytes, extract });
    }
}

/// Remove the extractor: documents are skipped again.
pub fn clear_extractor() {
    if let Ok(mut h) = HOOK.write() {
        *h = None;
    }
}

/// Extensions the registered extractor reads (empty when none is registered).
pub fn extensions() -> Vec<String> {
    HOOK.read().ok().and_then(|h| h.as_ref().map(|h| h.extensions.clone())).unwrap_or_default()
}

/// Largest document read (bytes on disk); 0 when no extractor is registered.
pub fn max_bytes() -> u64 {
    HOOK.read().ok().and_then(|h| h.as_ref().map(|h| h.max_bytes)).unwrap_or(0)
}

/// `path` has an extension the registered extractor reads.
pub fn handles(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    HOOK.read()
        .ok()
        .is_some_and(|h| h.as_ref().is_some_and(|h| h.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))))
}

/// The document's text: `None` when no registered extractor handles `path`.
pub fn extract(path: &Path) -> Option<Result<String, String>> {
    if !handles(path) {
        return None;
    }
    // Clone the function out so a slow document doesn't hold the lock.
    let f = HOOK.read().ok()?.as_ref()?.extract.clone();
    Some(f(path))
}

/// `text` cut to at most `max` bytes, on a character boundary.
pub fn truncate(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut cut = max;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_round_trip() {
        clear_extractor();
        assert!(!handles(Path::new("a.pdf")));
        assert!(extract(Path::new("a.pdf")).is_none());
        set_extractor(&["PDF"], 100, Arc::new(|p| Ok(format!("text of {}", p.display()))));
        assert!(handles(Path::new("x/A.Pdf")));
        assert!(!handles(Path::new("a.txt")) && !handles(Path::new("pdf")));
        assert_eq!(extract(Path::new("a.pdf")), Some(Ok("text of a.pdf".to_string())));
        assert_eq!((extensions(), max_bytes()), (vec!["pdf".to_string()], 100));
        clear_extractor();
        assert_eq!(max_bytes(), 0);
        assert_eq!(truncate("héllo".into(), 2), "h");
        assert_eq!(truncate("héllo".into(), 3), "hé");
    }
}
