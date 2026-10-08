//! Date formatting and RFC 3339 serde helpers for `SystemTime`.

use std::fmt::Write as _;
use std::time::SystemTime;

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, Datelike, Local};
use serde::{Deserialize, Serialize};

use crate::locale::{self, Locale};

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

/// Format `t` in local time, in the current [`locale`](crate::locale).
pub fn format_time(t: SystemTime, fmt: &DateFormat) -> String {
    format_time_at(t, fmt, SystemTime::now())
}

/// [`format_time`] with an explicit "now" (for relative formats; testable).
pub fn format_time_at(t: SystemTime, fmt: &DateFormat, now: SystemTime) -> String {
    format_time_at_in(t, fmt, now, locale::current())
}

/// [`format_time_at`] in an explicit locale. Turkish uses `08.10.2026 14:03` for the
/// short form, Turkish month / weekday names (also inside custom `%B %A %b %a` patterns)
/// and Turkish relative words ("Az önce", "Bugün 14:03", "Dün 09:12").
pub fn format_time_at_in(t: SystemTime, fmt: &DateFormat, now: SystemTime, l: Locale) -> String {
    let dt: DateTime<Local> = t.into();
    match fmt {
        DateFormat::Named(DateFormatName::Short) => dt.format(short_pattern(l)).to_string(),
        DateFormat::Named(DateFormatName::Long) => custom(dt, "%A, %-d %B %Y %H:%M:%S", l),
        DateFormat::Named(DateFormatName::Iso) => dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        DateFormat::Named(DateFormatName::Relative) => relative(dt, now.into(), l),
        DateFormat::Custom(pattern) => custom(dt, pattern, l),
    }
}

fn short_pattern(l: Locale) -> &'static str {
    match l {
        Locale::EnUs => "%Y-%m-%d %H:%M",
        Locale::TrTr => "%d.%m.%Y %H:%M",
    }
}

fn relative(dt: DateTime<Local>, now: DateTime<Local>, l: Locale) -> String {
    let w = locale::words(l);
    let secs = (now - dt).num_seconds();
    if (0..60).contains(&secs) {
        return w.just_now.into();
    }
    if (60..3600).contains(&secs) {
        return w.min_ago.replace("{}", &(secs / 60).to_string());
    }
    let days = now.date_naive().signed_duration_since(dt.date_naive()).num_days();
    match days {
        0 => format!("{} {}", w.today, dt.format("%H:%M")),
        1 => format!("{} {}", w.yesterday, dt.format("%H:%M")),
        2..=6 => custom(dt, "%A %H:%M", l),
        _ if dt.year() == now.year() && days > 0 => custom(dt, "%-d %b %H:%M", l),
        _ => match l {
            Locale::EnUs => dt.format("%Y-%m-%d").to_string(),
            Locale::TrTr => dt.format("%d.%m.%Y").to_string(),
        },
    }
}

/// Replace the name specifiers (`%A %a %B %b %h`, with optional flags) of a strftime
/// pattern by the locale's literal names; chrono itself only knows English.
fn localize_pattern(dt: &DateTime<Local>, pattern: &str, l: Locale) -> String {
    if l == Locale::EnUs {
        return pattern.to_string();
    }
    let month = dt.month0() as usize;
    let wd = dt.weekday().num_days_from_monday() as usize;
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut it = pattern.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut token = String::from("%");
        while let Some(&f) = it.peek() {
            if matches!(f, '-' | '_' | '0' | '^' | '#') {
                token.push(f);
                it.next();
            } else {
                break;
            }
        }
        let Some(spec) = it.next() else {
            out.push_str(&token);
            break;
        };
        let name = match spec {
            'A' => Some(locale::weekday_names(l)[wd]),
            'a' => Some(locale::weekday_abbr(l)[wd]),
            'B' => Some(locale::month_names(l)[month]),
            'b' | 'h' => Some(locale::month_abbr(l)[month]),
            _ => None,
        };
        match name {
            Some(n) => out.push_str(n),
            None => {
                out.push_str(&token);
                out.push(spec);
            }
        }
    }
    out
}

fn custom(dt: DateTime<Local>, pattern: &str, l: Locale) -> String {
    let pattern = localize_pattern(&dt, pattern, l);
    let items: Vec<Item<'_>> = StrftimeItems::new(&pattern).collect();
    if items.iter().any(|i| matches!(i, Item::Error)) {
        return dt.format(short_pattern(l)).to_string();
    }
    let mut out = String::new();
    if write!(out, "{}", dt.format_with_items(items.into_iter())).is_err() {
        return dt.format(short_pattern(l)).to_string();
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
    fn turkish_dates() {
        use chrono::TimeZone;
        let tr = Locale::TrTr;
        // Thursday 8 October 2026, 14:03:27 local time.
        let dt = Local.with_ymd_and_hms(2026, 10, 8, 14, 3, 27).unwrap();
        let t: SystemTime = dt.into();
        let far = t + Duration::from_secs(400 * 86400);
        let long = DateFormat::Named(DateFormatName::Long);
        assert_eq!(format_time_at_in(t, &long, far, tr), "Perşembe, 8 Ekim 2026 14:03:27");
        assert_eq!(format_time_at_in(t, &long, far, Locale::EnUs), "Thursday, 8 October 2026 14:03:27");
        let short = DateFormat::Named(DateFormatName::Short);
        assert_eq!(format_time_at_in(t, &short, far, tr), "08.10.2026 14:03");
        assert_eq!(format_time_at_in(t, &short, far, Locale::EnUs), "2026-10-08 14:03");
        let custom = DateFormat::Custom("%-d %b %Y, %a".into());
        assert_eq!(format_time_at_in(t, &custom, far, tr), "8 Eki 2026, Per");
        // `%%B` is a literal "%B", not a month name.
        assert_eq!(format_time_at_in(t, &DateFormat::Custom("%%B".into()), far, tr), "%B");
        let rel = DateFormat::Named(DateFormatName::Relative);
        let after = |s: u64| t + Duration::from_secs(s);
        assert_eq!(format_time_at_in(t, &rel, after(5), tr), "Az önce");
        assert_eq!(format_time_at_in(t, &rel, after(300), tr), "5 dk önce");
        assert_eq!(format_time_at_in(t, &rel, after(3 * 3600), tr), "Bugün 14:03");
        assert_eq!(format_time_at_in(t, &rel, after(86400), tr), "Dün 14:03");
        assert_eq!(format_time_at_in(t, &rel, after(3 * 86400), tr), "Perşembe 14:03");
        assert_eq!(format_time_at_in(t, &rel, far, tr), "08.10.2026");
        let later = Local.with_ymd_and_hms(2026, 12, 20, 10, 0, 0).unwrap();
        assert_eq!(format_time_at_in(t, &rel, later.into(), tr), "8 Eki 14:03");
        assert_eq!(format_time_at_in(t, &rel, later.into(), Locale::EnUs), "8 Oct 14:03");
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
