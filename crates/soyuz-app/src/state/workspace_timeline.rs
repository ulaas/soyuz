//! Timeline (F7): files changed today / this week. Pure parts — calendar buckets in
//! local time and merging candidates from several sources (watcher journal, recent
//! store, folder scans, the search index when one is present).

use std::collections::HashMap;
use std::path::PathBuf;

const DAY: i64 = 86_400;

/// Calendar bucket of a change, in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bucket {
    Today,
    Yesterday,
    /// Earlier this (ISO, Monday-first) week.
    ThisWeek,
    /// Older than this week but within the timeline's range.
    Earlier,
}

impl Bucket {
    pub fn label(self) -> &'static str {
        match self {
            Bucket::Today => "Today",
            Bucket::Yesterday => "Yesterday",
            Bucket::ThisWeek => "This week",
            Bucket::Earlier => "Earlier",
        }
    }
}

/// Local midnight (as unix seconds) of the day containing `now`, given the local UTC
/// offset in seconds.
pub fn day_start(now: i64, offset: i64) -> i64 {
    (now + offset).div_euclid(DAY) * DAY - offset
}

/// Local Monday 00:00 of the week containing `now`.
pub fn week_start(now: i64, offset: i64) -> i64 {
    let today = day_start(now, offset);
    // 1970-01-01 was a Thursday: days since a Monday = (days + 3) mod 7.
    let days = (now + offset).div_euclid(DAY);
    today - (days + 3).rem_euclid(7) * DAY
}

/// Bucket of a change at `ts` seen at `now` (both unix seconds).
pub fn bucket(ts: i64, now: i64, offset: i64) -> Bucket {
    let today = day_start(now, offset);
    if ts >= today {
        Bucket::Today
    } else if ts >= today - DAY {
        Bucket::Yesterday
    } else if ts >= week_start(now, offset) {
        Bucket::ThisWeek
    } else {
        Bucket::Earlier
    }
}

/// Start of the timeline range: `days` local days back (today counts as one), and never
/// later than the start of this week so "this week" is always complete.
pub fn range_start(now: i64, offset: i64, days: u32) -> i64 {
    let back = day_start(now, offset) - (i64::from(days.max(1)) - 1) * DAY;
    back.min(week_start(now, offset))
}

/// Merge candidate `(path, changed at)` lists: one entry per path (the latest time), only
/// changes at or after `since`, newest first, at most `max`.
pub fn merge(sources: impl IntoIterator<Item = Vec<(PathBuf, i64)>>, since: i64, max: usize) -> Vec<(PathBuf, i64)> {
    let mut best: HashMap<PathBuf, i64> = HashMap::new();
    for src in sources {
        for (p, t) in src {
            if t < since {
                continue;
            }
            let e = best.entry(p).or_insert(t);
            *e = (*e).max(t);
        }
    }
    let mut v: Vec<(PathBuf, i64)> = best.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(max);
    v
}

/// Count per bucket (for the timeline's summary line).
pub fn counts(items: &[(PathBuf, i64)], now: i64, offset: i64) -> [(Bucket, usize); 4] {
    let mut c = [(Bucket::Today, 0), (Bucket::Yesterday, 0), (Bucket::ThisWeek, 0), (Bucket::Earlier, 0)];
    for (_, t) in items {
        let b = bucket(*t, now, offset);
        if let Some(slot) = c.iter_mut().find(|(k, _)| *k == b) {
            slot.1 += 1;
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wednesday 2026-10-07 15:00:00 UTC.
    const NOW: i64 = 1_791_385_200;

    #[test]
    fn calendar_buckets() {
        let off = 0;
        assert_eq!(day_start(NOW, off), NOW - 15 * 3600);
        // Monday 2026-10-05 00:00 UTC.
        assert_eq!(week_start(NOW, off), NOW - 15 * 3600 - 2 * DAY);
        assert_eq!(bucket(NOW - 60, NOW, off), Bucket::Today);
        assert_eq!(bucket(day_start(NOW, off), NOW, off), Bucket::Today, "midnight belongs to today");
        assert_eq!(bucket(day_start(NOW, off) - 1, NOW, off), Bucket::Yesterday);
        assert_eq!(bucket(week_start(NOW, off), NOW, off), Bucket::ThisWeek);
        assert_eq!(bucket(week_start(NOW, off) - 1, NOW, off), Bucket::Earlier);
        // A local offset moves the day boundary: 01:00 at UTC+3 is still "today".
        let off3 = 3 * 3600;
        let local_1am = day_start(NOW, off3) + 3600;
        assert_eq!(bucket(local_1am, NOW, off3), Bucket::Today);
        assert_eq!(bucket(local_1am, NOW, 0), Bucket::Yesterday, "…but 22:00 UTC yesterday");
        assert_eq!(bucket(day_start(NOW, off3) - 60, NOW, off3), Bucket::Yesterday);
    }

    #[test]
    fn week_starts_on_monday() {
        // Monday itself, at noon UTC.
        let monday = week_start(NOW, 0) + 12 * 3600;
        assert_eq!(week_start(monday, 0), week_start(NOW, 0));
        // Sunday belongs to the previous week.
        let sunday = week_start(NOW, 0) - 3600;
        assert_eq!(week_start(sunday, 0), week_start(NOW, 0) - 7 * DAY);
        // On a Monday "yesterday" (Sunday) is older than this week.
        assert_eq!(bucket(sunday, monday, 0), Bucket::Yesterday);
    }

    #[test]
    fn range_covers_the_week() {
        assert_eq!(range_start(NOW, 0, 7), day_start(NOW, 0) - 6 * DAY);
        assert_eq!(range_start(NOW, 0, 1), week_start(NOW, 0), "at least this week");
        assert_eq!(range_start(NOW, 0, 0), week_start(NOW, 0));
    }

    #[test]
    fn merge_dedupes_filters_and_sorts() {
        let p = |s: &str| PathBuf::from(s);
        let a = vec![(p("/a"), 10), (p("/b"), 30), (p("/old"), 1)];
        let b = vec![(p("/a"), 50), (p("/c"), 20)];
        let m = merge([a, b], 5, 10);
        assert_eq!(m, vec![(p("/a"), 50), (p("/b"), 30), (p("/c"), 20)]);
        assert_eq!(merge([m.clone()], 0, 2).len(), 2);
        let c = counts(&[(p("/x"), NOW - 10), (p("/y"), NOW - DAY), (p("/z"), NOW - 30 * DAY)], NOW, 0);
        assert_eq!(c[0], (Bucket::Today, 1));
        assert_eq!(c[1], (Bucket::Yesterday, 1));
        assert_eq!(c[3], (Bucket::Earlier, 1));
    }
}
