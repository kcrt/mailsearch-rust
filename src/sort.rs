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

/// Extract the date from a `SearchResult`'s `date_str` (`YYYY-MM-DD ...`).
pub fn parse_date_from_result(result: &SearchResult) -> Option<NaiveDate> {
    result
        .date_str
        .split_whitespace()
        .next()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

/// Compare two results according to the given sort mode.
pub fn compare_results(a: &SearchResult, b: &SearchResult, mode: SortMode) -> Ordering {
    match mode {
        SortMode::NoSort => Ordering::Equal,
        SortMode::DateAsc => parse_date_from_result(a).cmp(&parse_date_from_result(b)),
        SortMode::DateDesc => parse_date_from_result(b).cmp(&parse_date_from_result(a)),
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
