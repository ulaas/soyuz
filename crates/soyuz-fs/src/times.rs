//! Timestamps.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;
use soyuz_core::time::serde_rfc3339;

use crate::baseline;

/// Timestamps to set; `None` leaves a timestamp unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimesSet {
    #[serde(with = "serde_rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<SystemTime>,
    #[serde(with = "serde_rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub created: Option<SystemTime>,
    #[serde(with = "serde_rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<SystemTime>,
}

/// Set timestamps (`created` only where the platform supports it; ignored elsewhere).
pub fn set_times(path: &Path, t: &TimesSet, follow: bool) -> FsResult<()> {
    baseline::set_times(path, t, follow)
}
