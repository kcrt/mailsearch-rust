//! Sorting logic for search results, shared between the CLI and the TUI.

use crate::models::SearchResult;
use chrono::NaiveDate;
use clap::ValueEnum;
use std::cmp::Ordering;

/// Sort mode for ordering search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortMode {
    /// Keep the original discovery order.
    #[value(name = "none")]
    NoSort,
    /// Date, oldest first.
    #[value(name = "date-asc")]
    DateAsc,
    /// Date, newest first.
    #[value(name = "date-desc")]
    DateDesc,
    /// Subject, alphabetical.
    #[value(name = "subject")]
    Subject,
    /// Sender address, alphabetical.
    #[value(name = "from")]
    From,
    /// Recipient address, alphabetical.
    #[value(name = "to")]
    To,
}

impl SortMode {
    /// Next mode in the cycle, used by the TUI's `s` key.
    pub fn next(self) -> Self {
        match self {
            SortMode::NoSort => SortMode::DateAsc,
            SortMode::DateAsc => SortMode::DateDesc,
            SortMode::DateDesc => SortMode::Subject,
            SortMode::Subject => SortMode::From,
            SortMode::From => SortMode::To,
            SortMode::To => SortMode::NoSort,
        }
    }

    /// Human-readable label for the TUI title indicator.
    pub fn as_str(&self) -> &str {
        match self {
            SortMode::NoSort => "no sort",
            SortMode::DateAsc => "date asc",
            SortMode::DateDesc => "date desc",
            SortMode::Subject => "subject",
            SortMode::From => "from",
            SortMode::To => "to",
        }
    }
}

/// Calendar date of a result, in the timezone dates are displayed in.
///
/// Derived from the parsed timestamp rather than re-parsing `date_str`, and
/// deliberately reuses [`crate::email::display_date`] so a date filter can never
/// disagree with the date shown in the results list.
pub fn result_date(result: &SearchResult) -> Option<NaiveDate> {
    result.timestamp.and_then(crate::email::display_date)
}

/// Compare two results according to the given sort mode.
pub fn compare_results(a: &SearchResult, b: &SearchResult, mode: SortMode) -> Ordering {
    match mode {
        SortMode::NoSort => Ordering::Equal,
        // Compare timestamps directly: `date_str` only carries minutes and used to
        // be truncated to the day, so same-day mail tied and ordered arbitrarily.
        // `None` sorts before `Some`, keeping undated mail first under `date-asc`
        // and last under `date-desc` as before.
        SortMode::DateAsc => a.timestamp.cmp(&b.timestamp),
        SortMode::DateDesc => b.timestamp.cmp(&a.timestamp),
        SortMode::Subject => a.subject.to_lowercase().cmp(&b.subject.to_lowercase()),
        SortMode::From => a.from_addr.to_lowercase().cmp(&b.from_addr.to_lowercase()),
        SortMode::To => a.to_addr.to_lowercase().cmp(&b.to_addr.to_lowercase()),
    }
}

/// Sort results in place. No-op for `NoSort`; stable to preserve original order on ties.
pub fn sort_results(results: &mut [SearchResult], mode: SortMode) {
    if mode == SortMode::NoSort {
        return;
    }
    results.sort_by(|a, b| compare_results(a, b, mode));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn ts(rfc3339: &str) -> Option<i64> {
        Some(DateTime::parse_from_rfc3339(rfc3339).unwrap().timestamp())
    }

    /// A result identified by its subject, with everything else left inert.
    fn result(subject: &str, timestamp: Option<i64>) -> SearchResult {
        SearchResult {
            subject: subject.to_string(),
            from_addr: String::new(),
            to_addr: String::new(),
            cc_addr: String::new(),
            date_str: timestamp
                .and_then(crate::email::format_timestamp)
                .unwrap_or_else(|| "N/A".to_string()),
            timestamp,
            message_id: None,
            file_path: String::new(),
            content: String::new(),
        }
    }

    fn subjects(results: &[SearchResult]) -> Vec<&str> {
        results.iter().map(|r| r.subject.as_str()).collect()
    }

    #[test]
    fn same_day_results_order_by_time() {
        // The reason sorting moved off `date_str`: both of these are 2026-01-20,
        // so a day-granularity comparison tied and left the order arbitrary.
        let mut results = vec![
            result("evening", ts("2026-01-20T18:00:00Z")),
            result("morning", ts("2026-01-20T08:00:00Z")),
            result("noon", ts("2026-01-20T12:00:00Z")),
        ];

        sort_results(&mut results, SortMode::DateAsc);
        assert_eq!(subjects(&results), ["morning", "noon", "evening"]);

        sort_results(&mut results, SortMode::DateDesc);
        assert_eq!(subjects(&results), ["evening", "noon", "morning"]);
    }

    #[test]
    fn undated_results_sort_first_ascending_and_last_descending() {
        let mut results = vec![
            result("dated", ts("2026-01-20T08:00:00Z")),
            result("undated", None),
        ];

        sort_results(&mut results, SortMode::DateAsc);
        assert_eq!(subjects(&results), ["undated", "dated"]);

        sort_results(&mut results, SortMode::DateDesc);
        assert_eq!(subjects(&results), ["dated", "undated"]);
    }

    #[test]
    fn equal_timestamps_keep_their_original_order() {
        let same = ts("2026-01-20T08:00:00Z");
        let mut results = vec![result("first", same), result("second", same)];

        sort_results(&mut results, SortMode::DateAsc);
        assert_eq!(subjects(&results), ["first", "second"]);
    }

    #[test]
    fn no_sort_leaves_order_untouched() {
        let mut results = vec![
            result("b", ts("2026-01-20T08:00:00Z")),
            result("a", ts("2026-01-21T08:00:00Z")),
        ];

        sort_results(&mut results, SortMode::NoSort);
        assert_eq!(subjects(&results), ["b", "a"]);
    }

    #[test]
    fn result_date_matches_the_displayed_date() {
        let r = result("x", ts("2026-01-20T10:30:00Z"));
        assert_eq!(r.date_str, "2026-01-20 10:30");
        assert_eq!(
            result_date(&r),
            Some(NaiveDate::from_ymd_opt(2026, 1, 20).unwrap())
        );
        assert_eq!(result_date(&result("y", None)), None);
    }

    #[test]
    fn text_modes_are_case_insensitive() {
        let mut results = vec![result("Zebra", None), result("apple", None)];
        sort_results(&mut results, SortMode::Subject);
        assert_eq!(subjects(&results), ["apple", "Zebra"]);
    }
}
