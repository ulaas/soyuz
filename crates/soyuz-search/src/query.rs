//! Serializable search queries (saved searches, session).

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::{EntryKindSet, NamePatternSpec};

/// Content (grep-like) pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ContentPatternSpec {
    pub regex: bool,
    pub text: String,
    pub case_sensitive: bool,
}

/// A complete search request. Fully serializable; `#[serde(default)]` so the
/// `[[search.saved]] query = {...}` tables in `config.toml` may give any subset.
///
/// `size` / `modified` bounds are `(min, max)`; TOML has no null, so an upper size bound
/// of `Some(0)` means "unbounded".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchQuery {
    pub roots: Vec<PathBuf>,
    pub name: Option<NamePatternSpec>,
    pub content: Option<ContentPatternSpec>,
    /// Files larger than this are not content-searched (default 8 MiB).
    pub content_max_size: u64,
    pub size: Option<(Option<u64>, Option<u64>)>,
    #[serde(with = "time_range")]
    pub modified: Option<(Option<SystemTime>, Option<SystemTime>)>,
    pub kinds: Option<EntryKindSet>,
    pub extensions: Vec<String>,
    pub max_depth: Option<usize>,
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    pub respect_gitignore: bool,
    pub include_archives: bool,
    /// Stop after this many hits (0 = unlimited).
    pub max_results: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            roots: Vec::new(),
            name: None,
            content: None,
            content_max_size: 8 * 1024 * 1024,
            size: None,
            modified: None,
            kinds: None,
            extensions: Vec::new(),
            max_depth: None,
            follow_symlinks: false,
            include_hidden: false,
            respect_gitignore: false,
            include_archives: false,
            max_results: 100_000,
        }
    }
}

/// A named query stored in `config.toml` / `session.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    pub query: SearchQuery,
}

impl SavedSearch {
    /// Convert a `[[search.saved]]` config entry.
    pub fn from_config(entry: &soyuz_core::config::SavedSearchEntry) -> Result<Self, String> {
        let query = SearchQuery::deserialize(toml::Value::Table(entry.query.clone())).map_err(|e| e.to_string())?;
        Ok(SavedSearch { name: entry.name.clone(), query })
    }
}

/// RFC 3339 serde for `Option<(Option<SystemTime>, Option<SystemTime>)>`; missing bounds
/// are written as empty strings.
mod time_range {
    use std::time::SystemTime;

    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
    use soyuz_core::time::serde_rfc3339::{parse, to_string};

    type Range = Option<(Option<SystemTime>, Option<SystemTime>)>;

    pub fn serialize<S: Serializer>(v: &Range, s: S) -> Result<S::Ok, S::Error> {
        let f = |t: &Option<SystemTime>| t.as_ref().map(to_string).unwrap_or_default();
        match v {
            Some((a, b)) => Some((f(a), f(b))).serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Range, D::Error> {
        let raw = Option::<(String, String)>::deserialize(d)?;
        let p = |s: &str| -> Result<Option<SystemTime>, D::Error> {
            if s.is_empty() { Ok(None) } else { parse(s).map(Some).map_err(D::Error::custom) }
        };
        match raw {
            Some((a, b)) => Ok(Some((p(&a)?, p(&b)?))),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn config_entry_parses() {
        let text = r#"
            name = "Large videos"
            query = { roots = ["~"], extensions = ["mp4", "mkv"], size = [1073741824, 0] }
        "#;
        let entry: soyuz_core::config::SavedSearchEntry = toml::from_str(text).unwrap();
        let s = SavedSearch::from_config(&entry).unwrap();
        assert_eq!(s.query.extensions, ["mp4", "mkv"]);
        assert_eq!(s.query.size, Some((Some(1_073_741_824), Some(0))));
        assert_eq!(s.query.content_max_size, 8 * 1024 * 1024);
    }

    #[test]
    fn json_round_trip_with_times() {
        let q = SearchQuery {
            roots: vec!["/x".into()],
            modified: Some((Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)), None)),
            name: Some(NamePatternSpec::new(soyuz_core::PatternKind::Glob, "*.rs")),
            ..Default::default()
        };
        let s = toml::to_string(&SavedSearch { name: "n".into(), query: q.clone() }).unwrap();
        let back: SavedSearch = toml::from_str(&s).unwrap();
        assert_eq!(back.query, q);
    }
}
