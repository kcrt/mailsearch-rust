//! Resolves `--this-week` / `--days N` into the epoch-second cutoffs used to
//! skip mail outside the requested period.
//!
//! Two cutoffs are needed because the search filters in two stages:
//!
//! 1. File `mtime`, checked during the directory walk without reading the file.
//! 2. The `Date:` header, checked after parsing.
//!
//! For Apple Mail's `.emlx` files `mtime >= Date` holds (the file is written at
//! or after the message date, and later edits only push `mtime` forward), so a
//! file whose `mtime` predates the cutoff cannot possibly contain a message
//! inside the window and can be skipped unread. That prunes the candidate set
//! by orders of magnitude. Stage 2 is still required: changing a flag or moving
//! a message rewrites the file, so plenty of surviving candidates are old mail
//! with a freshly bumped `mtime`.

use chrono::{DateTime, NaiveTime, TimeZone};

/// How much earlier the `mtime` cutoff sits relative to the `Date` cutoff.
///
/// The `mtime >= Date` invariant holds, but the observed margin can be as small
/// as a few seconds. A sender whose clock runs fast would then produce a `Date`
/// just inside the window backed by an `mtime` just outside it, and the message
/// would be dropped unread. Loosening only the file-side cutoff closes that hole
/// at negligible cost, since the extra candidates are still a tiny fraction of
/// the mailbox and stage 2 rejects the ones that don't belong.
pub const MTIME_SLACK_SECS: i64 = 2 * 86_400;

const SECS_PER_DAY: i64 = 86_400;

/// The date window a search is restricted to.
///
/// Both fields are epoch seconds and mail is kept when its date is at or after
/// them. They are deliberately different numbers, so they live in a struct
/// rather than being passed around as two interchangeable `i64`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Cutoff applied to the parsed `Date:` header.
    pub date_cutoff: i64,
    /// Cutoff applied to the file's mtime. Always [`MTIME_SLACK_SECS`] before
    /// `date_cutoff`.
    pub mtime_cutoff: i64,
}

/// Build a window covering the last `days` days relative to `now`.
///
/// The cutoff is midnight at the start of `now`'s day, minus `days` days, so the
/// window is intentionally a little wider than requested: `days = 7` run at
/// 15:00 covers 7 days plus 15 hours. "The last week" is an approximation
/// anyway, and erring wide never hides mail the user expected to see.
///
/// Generic over the timezone so tests can pin `now` to a fixed offset instead of
/// depending on the machine's local zone.
pub fn window_from<Tz: TimeZone>(now: DateTime<Tz>, days: u32) -> Window {
    let midnight = now.date_naive().and_time(NaiveTime::MIN);
    // A local midnight can be skipped entirely by a DST transition (and can be
    // ambiguous where clocks fall back at midnight). `earliest()` picks the
    // wider window; the UTC reading is a last resort so this never panics.
    let start_of_today = now
        .timezone()
        .from_local_datetime(&midnight)
        .earliest()
        .map_or_else(|| midnight.and_utc().timestamp(), |dt| dt.timestamp());

    // Subtracting a fixed number of seconds avoids chrono's `Duration::days`,
    // which panics on out-of-range input. `days` is capped by the CLI parser
    // well below the point where this could overflow.
    let date_cutoff = start_of_today - i64::from(days) * SECS_PER_DAY;

    Window {
        date_cutoff,
        mtime_cutoff: date_cutoff - MTIME_SLACK_SECS,
    }
}

/// Build a window covering the last `days` days relative to the current time.
pub fn window_now(days: u32) -> Window {
    window_from(chrono::Local::now(), days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// 2026-07-28 15:30:00 +0900.
    fn jst_now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-07-28T15:30:00+09:00").unwrap()
    }

    /// Epoch seconds for the given JST midnight.
    fn jst_midnight(date: &str) -> i64 {
        DateTime::parse_from_rfc3339(&format!("{date}T00:00:00+09:00"))
            .unwrap()
            .timestamp()
    }

    #[test]
    fn days_zero_covers_today_only() {
        let window = window_from(jst_now(), 0);
        assert_eq!(window.date_cutoff, jst_midnight("2026-07-28"));
    }

    #[test]
    fn days_one_reaches_back_to_yesterday_midnight() {
        let window = window_from(jst_now(), 1);
        assert_eq!(window.date_cutoff, jst_midnight("2026-07-27"));
    }

    #[test]
    fn days_seven_reaches_back_a_week() {
        let window = window_from(jst_now(), 7);
        assert_eq!(window.date_cutoff, jst_midnight("2026-07-21"));
    }

    #[test]
    fn cutoff_ignores_time_of_day() {
        let morning = DateTime::parse_from_rfc3339("2026-07-28T00:00:01+09:00").unwrap();
        let evening = DateTime::parse_from_rfc3339("2026-07-28T23:59:59+09:00").unwrap();
        assert_eq!(window_from(morning, 7), window_from(evening, 7));
    }

    #[test]
    fn mtime_cutoff_is_slacker_than_date_cutoff() {
        let window = window_from(jst_now(), 7);
        assert_eq!(window.mtime_cutoff, window.date_cutoff - MTIME_SLACK_SECS);
        assert!(window.mtime_cutoff < window.date_cutoff);
    }

    #[test]
    fn timezone_is_respected() {
        let utc = DateTime::parse_from_rfc3339("2026-07-28T15:30:00+00:00").unwrap();
        // Same instant, different zone: JST is already 9 hours into the day, so
        // its midnight is 9 hours earlier than UTC's.
        assert_eq!(
            window_from(jst_now(), 0).date_cutoff,
            window_from(utc, 0).date_cutoff - 9 * 3600
        );
    }

    #[test]
    fn large_day_counts_do_not_panic() {
        // The CLI caps `--days` at 36500, but the function must not panic even
        // at the edge of that range.
        let window = window_from(jst_now(), 36_500);
        assert_eq!(
            window.date_cutoff,
            jst_midnight("2026-07-28") - 36_500 * SECS_PER_DAY
        );
    }
}
