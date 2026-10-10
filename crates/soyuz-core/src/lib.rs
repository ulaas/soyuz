//! `soyuz-core`: the shared, GUI-free vocabulary of the Soyuz file explorer.
//!
//! Every type that is used by two or more Soyuz crates lives here:
//! entries and locations, errors, cooperative operation control, progress sinks,
//! prompt policies, formatting helpers, sorting/filtering and the configuration schema
//! (including key bindings and the [`Action`](config::Action) enum).
//!
//! This crate performs no filesystem I/O except reading/writing the config file.
#![forbid(unsafe_code)]

pub mod columns;
pub mod config;
pub mod control;
pub mod entry;
pub mod error;
pub mod filter;
pub mod locale;
pub mod location;
pub mod naming;
pub mod policy;
pub mod progress;
pub mod size;
pub mod sort;
pub mod time;

pub use columns::{Column, ColumnSpec, GroupBy, ViewMode};
pub use control::{Control, OpControl};
pub use entry::{Attributes, DeviceId, EntryInfo, EntryKind, EntryKindSet, LinkTarget, SearchId};
pub use error::{FsError, FsErrorKind, FsResult};
pub use filter::{Filter, NamePattern, NamePatternSpec, PatternError, PatternKind};
pub use locale::{Locale, fold_for_match};
pub use location::{Location, join_vpath, split_vpath};
pub use naming::{NameError, fs_needs_windows_names, natural_cmp, unique_name, validate_name, validate_name_with};
pub use policy::{ConflictResolution, ErrorAction, Overwrite};
pub use progress::{ByteProgress, HashAlgo, ProgressSink};
pub use size::{SizeStyle, format_eta, format_size, format_speed};
pub use sort::{SortDir, SortSpec, compare_entries, sort_entries};
pub use time::{DateFormat, DateFormatName, format_time};

/// Generic `Display`/`FromStr` helpers for unit-only enums that use
/// `#[serde(rename_all = "snake_case")]`: the string form equals the serde form.
pub(crate) mod enum_str {
    use serde::{Deserialize, Serialize, de::IntoDeserializer};

    /// Serialize a unit enum variant to its serde name (e.g. `Column::Name` → `"name"`).
    pub fn to_str<T: Serialize>(v: &T) -> String {
        match toml::Value::try_from(v) {
            Ok(toml::Value::String(s)) => s,
            _ => String::new(),
        }
    }

    /// Parse a unit enum variant from its serde name; case-insensitive, `-` accepted for `_`.
    pub fn from_str<T: for<'de> Deserialize<'de>>(s: &str) -> Option<T> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        let de: serde::de::value::StringDeserializer<serde::de::value::Error> = norm.into_deserializer();
        T::deserialize(de).ok()
    }
}
