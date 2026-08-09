//! Configuration and CLI argument parsing.

pub use clap::Parser;
use crate::models::{DEFAULT_LIMIT, DEFAULT_MAIL_ROOT};
use crate::sort::SortMode;
use std::path::PathBuf;

/// Configuration for the search operation.
#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Search query (multiple words = AND search; use --or for OR groups)
    // Taken as a list so the words may be quoted as one argument or left bare:
    // `mailsearch "オボムコイド 鶏卵"` and `mailsearch オボムコイド 鶏卵` mean the
    // same AND search. A single `String` rejected the unquoted form with
    // "unexpected argument", which reads like a failed search rather than a
    // usage error once stderr is discarded.
    #[arg(required = true, num_args = 1.., value_name = "QUERY")]
    pub query: Vec<String>,

    /// Additional OR group(s); repeatable. Each value is AND-matched internally,
    /// groups are OR-combined. e.g. `--or "foo bar" --or baz`
    #[arg(short = 'o', long = "or", value_name = "TERMS")]
    pub or_terms: Vec<String>,

    /// Path to Mail directory
    #[arg(short = 'r', long = "mail-root", default_value = DEFAULT_MAIL_ROOT)]
    pub mail_root: PathBuf,

    /// Maximum number of results (unlimited by default; setting this enables early termination)
    #[arg(short = 'l', long = "limit", default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,

    /// Sort order for results (applied before --limit when set)
    #[arg(long = "sort", value_enum, default_value_t = SortMode::NoSort)]
    pub sort: SortMode,

    /// Limit the search to mail from the last 7 days (shorthand for --days 7)
    // No `default_value_t` here: on an arg that participates in `conflicts_with`
    // it makes clap treat the flag as always present, so the conflict would fire
    // even when neither flag was passed.
    #[arg(long = "this-week")]
    pub this_week: bool,

    /// Limit the search to mail from the last N days (0 = today only)
    // `Option` rather than a defaulted value so "not given" stays distinct from
    // `--days 0`. The upper bound keeps `days * 86400` far from overflowing.
    #[arg(
        long = "days",
        value_name = "N",
        conflicts_with = "this_week",
        value_parser = clap::value_parser!(u32).range(0..=MAX_DAYS)
    )]
    pub days: Option<u32>,

    /// Output results as JSON to stdout instead of the interactive TUI
    #[arg(long = "json", default_value_t = false)]
    pub json: bool,
}

/// Upper bound for `--days`, roughly a century.
const MAX_DAYS: i64 = 36_500;

impl Config {
    /// The query words as a single whitespace-separated string.
    ///
    /// Query parsing splits on whitespace anyway, so joining here keeps quoted
    /// and unquoted invocations indistinguishable downstream.
    pub fn query_string(&self) -> String {
        self.query.join(" ")
    }

    /// Number of days to restrict the search to, if a date window was requested.
    pub fn days_window(&self) -> Option<u32> {
        self.days.or_else(|| self.this_week.then_some(7))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Config {
        Config::try_parse_from(std::iter::once("mailsearch").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn bare_words_are_one_and_query() {
        // The unquoted form used to be rejected outright.
        assert_eq!(parse(&["オボムコイド", "鶏卵"]).query_string(), "オボムコイド 鶏卵");
    }

    #[test]
    fn a_quoted_query_is_unchanged() {
        assert_eq!(parse(&["オボムコイド 鶏卵"]).query_string(), "オボムコイド 鶏卵");
    }

    #[test]
    fn bare_words_still_take_flags_after_them() {
        let config = parse(&["hello", "world", "--days", "30"]);
        assert_eq!(config.query_string(), "hello world");
        assert_eq!(config.days_window(), Some(30));
    }

    #[test]
    fn an_empty_query_is_rejected() {
        assert!(Config::try_parse_from(["mailsearch"]).is_err());
    }

    #[test]
    fn no_date_flags_means_no_window() {
        assert_eq!(parse(&["query"]).days_window(), None);
    }

    #[test]
    fn this_week_is_seven_days() {
        assert_eq!(parse(&["query", "--this-week"]).days_window(), Some(7));
    }

    #[test]
    fn days_sets_the_window() {
        assert_eq!(parse(&["query", "--days", "30"]).days_window(), Some(30));
    }

    #[test]
    fn days_zero_is_distinct_from_unset() {
        assert_eq!(parse(&["query", "--days", "0"]).days_window(), Some(0));
    }

    #[test]
    fn this_week_and_days_conflict() {
        assert!(
            Config::try_parse_from(["mailsearch", "query", "--this-week", "--days", "3"]).is_err()
        );
    }

    #[test]
    fn days_beyond_the_cap_is_rejected() {
        assert!(Config::try_parse_from(["mailsearch", "query", "--days", "36501"]).is_err());
    }
}
