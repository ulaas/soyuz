//! Date formatting and RFC 3339 serde helpers for `SystemTime`.

use std::fmt::Write as _;
use std::time::SystemTime;

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, Datelike, Local};
use serde::{Deserialize, Serialize};

/// Named date formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DateFormatName {
    /// "Just now", "5 min ago", "Today 14:03", "Yesterday 09:12", weekday, then short date.
    #[default]
    Relative,
    /// `2026-10-08 14:03`
    Short,
    /// `Thursday, 8 October 2026 14:03:27`
    Long,
    /// RFC 3339 with local offset.
    Iso,
}

/// A named format or a custom strftime pattern (e.g. `"%Y-%m-%d %H:%M"`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DateFormat {
    Named(DateFormatName),
    Custom(String),
}

impl Default for DateFormat {
    fn default() -> Self {
        DateFormat::Named(DateFormatName::Relative)
    }
}

/// Format `t` in local time.
pub fn format_time(t: SystemTime, fmt: &DateFormat) -> String {
    format_time_at(t, fmt, SystemTime::now())
}

/// [`format_time`] with an explicit "now" (for relative formats; testable).
pub fn format_time_at(t: SystemTime, fmt: &DateFormat, now: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    match fmt {
        DateFormat::Named(DateFormatName::Short) => dt.format("%Y-%m-%d %H:%M").to_string(),
        DateFormat::Named(DateFormatName::Long) => dt.format("%A, %-d %B %Y %H:%M:%S").to_string(),
        DateFormat::Named(DateFormatName::Iso) => dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        DateFormat::Named(DateFormatName::Relative) => relative(dt, now.into()),
        DateFormat::Custom(pattern) => custom(dt, pattern),
    }
}

fn relative(dt: DateTime<Local>, now: DateTime<Local>) -> String {
    let secs = (now - dt).num_seconds();
    if (0..60).contains(&secs) {
        return "Just now".into();
    }
    if (60..3600).contains(&secs) {
        return format!("{} min ago", secs / 60);
    }
    let days = now.date_naive().signed_duration_since(dt.date_naive()).num_days();
    match days {
        0 => format!("Today {}", dt.format("%H:%M")),
        1 => format!("Yesterday {}", dt.format("%H:%M")),
        2..=6 => dt.format("%A %H:%M").to_string(),
        _ if dt.year() == now.year() && days > 0 => dt.format("%-d %b %H:%M").to_string(),
        _ => dt.format("%Y-%m-%d").to_string(),
    }
}

fn custom(dt: DateTime<Local>, pattern: &str) -> String {
    let items: Vec<Item<'_>> = StrftimeItems::new(pattern).collect();
    if items.iter().any(|i| matches!(i, Item::Error)) {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    let mut out = String::new();
    if write!(out, "{}", dt.format_with_items(items.into_iter())).is_err() {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    out
}

/// `#[serde(with = "soyuz_core::time::serde_rfc3339")]` for `SystemTime` fields
/// (and `serde_rfc3339::option` for `Option<SystemTime>`).
pub mod serde_rfc3339 {
    use std::time::SystemTime;

    use chrono::{DateTime, SecondsFormat, Utc};
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn to_string(t: &SystemTime) -> String {
        DateTime::<Utc>::from(*t).to_rfc3339_opts(SecondsFormat::AutoSi, true)
    }

    pub fn parse(s: &str) -> Result<SystemTime, chrono::ParseError> {
        DateTime::parse_from_rfc3339(s).map(SystemTime::from)
    }

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&to_string(t))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let s = String::deserialize(d)?;
        parse(&s).map_err(D::Error::custom)
    }

    pub mod option {
        use std::time::SystemTime;

        use serde::{Deserialize, Deserializer, Serializer, de::Error};

        pub fn serialize<S: Serializer>(t: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
            match t {
                Some(t) => s.serialize_some(&super::to_string(t)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
            match Option::<String>::deserialize(d)? {
                Some(s) => super::parse(&s).map(Some).map_err(D::Error::custom),
                None => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn relative_formats() {
        let now = SystemTime::now();
        let f = DateFormat::Named(DateFormatName::Relative);
        assert_eq!(format_time_at(now - Duration::from_secs(5), &f, now), "Just now");
        assert_eq!(format_time_at(now - Duration::from_secs(300), &f, now), "5 min ago");
        let old = now - Duration::from_secs(400 * 86400);
        let s = format_time_at(old, &f, now);
        assert_eq!(s.len(), 10, "{s}");
    }

    #[test]
    fn custom_and_invalid() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let s = format_time(t, &DateFormat::Custom("%Y".into()));
        assert!(s == "2023" || s == "2024", "{s}");
        // Invalid patterns fall back instead of panicking.
        let s = format_time(t, &DateFormat::Custom("%Q %".into()));
        assert!(!s.is_empty());
    }

    #[test]
    fn date_format_serde() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            f: DateFormat,
        }
        let w: W = toml::from_str("f = \"relative\"").unwrap();
        assert_eq!(w.f, DateFormat::Named(DateFormatName::Relative));
        let w: W = toml::from_str("f = \"%Y-%m-%d\"").unwrap();
        assert_eq!(w.f, DateFormat::Custom("%Y-%m-%d".into()));
    }

    #[test]
    fn rfc3339_round_trip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            #[serde(with = "serde_rfc3339")]
            t: SystemTime,
            #[serde(with = "serde_rfc3339::option", default)]
            o: Option<SystemTime>,
        }
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        let w = W { t, o: Some(t) };
        let s = toml::to_string(&w).unwrap();
        assert!(s.contains("2023-11-14T22:13:20.123Z"), "{s}");
        assert_eq!(toml::from_str::<W>(&s).unwrap(), w);
    }
}
