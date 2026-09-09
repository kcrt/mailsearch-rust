//! Configuration and CLI argument parsing.

pub use clap::Parser;
use crate::models::{Filters, DEFAULT_LIMIT, DEFAULT_MAIL_ROOT};
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
    //
    // Optional when `--or` is given, so a search written entirely as OR groups
    // (`mailsearch --or foo --or bar`) is accepted as well as the equivalent
    // `mailsearch foo --or bar`. Requiring the positional made the all-`--or`
    // form fail with "the following required arguments were not provided",
    // which reads as a bug in the caller's quoting rather than a usage rule.
    //
    // Also optional when a filter narrows the scan on its own: "every message
    // from this sender" (`--from`) and "every message with an attachment"
    // (`--has-attachment`) are complete searches with no text query at all.
    #[arg(
        num_args = 1..,
        value_name = "QUERY",
        required_unless_present_any = ["or_terms", "from", "has_attachment"]
    )]
    pub query: Vec<String>,

    /// Additional OR group(s); repeatable. Each value is AND-matched internally,
    /// groups are OR-combined. e.g. `--or "foo bar" --or baz`
    #[arg(short = 'o', long = "or", value_name = "TERMS")]
    pub or_terms: Vec<String>,

    /// Only match messages whose `From` header contains PATTERN; repeatable
    /// (any of them matches). Matched against the decoded header, so both the
    /// display name and the address work.
    // Repeatable and OR-combined because the question behind it is usually
    // "which of these people wrote to me", e.g. three correspondents who share
    // a surname and hold three different addresses.
    #[arg(long = "from", value_name = "PATTERN")]
    pub from: Vec<String>,

    /// Only match messages carrying a real attachment (inline parts such as
    /// signature images do not count)
    #[arg(long = "has-attachment", default_value_t = false)]
    pub has_attachment: bool,

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

    /// Ignore Apple Mail's Envelope Index and read every message file
    // An escape hatch, not a mode: the index only decides which files are worth
    // opening, and every file it keeps is still parsed and matched normally, so
    // both settings return the same messages. Worth having anyway - if a search
    // ever does disagree, re-running it with this flag says in one step whether
    // the index was involved.
    #[arg(long = "no-index", default_value_t = false)]
    pub no_index: bool,

    /// Output results as JSON to stdout instead of the interactive TUI
    #[arg(long = "json", default_value_t = false)]
    pub json: bool,

    /// Output one TAB-separated line per result instead of the interactive TUI:
    /// date, from, subject, message-id, path
    // TAB rather than a printable separator: a tab cannot survive inside a
    // header value (it is folding whitespace there), while `|` legitimately
    // appears in subjects. Any tab that does turn up in a value is replaced
    // with a space when the line is written, so the column count is fixed.
    #[arg(long = "tsv", default_value_t = false, conflicts_with = "json")]
    pub tsv: bool,
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

    /// Whether output goes to stdout for another program to read, rather than
    /// to the interactive TUI.
    ///
    /// Status messages have to stay off stdout in these modes or they would
    /// corrupt the output.
    pub fn machine_output(&self) -> bool {
        self.json || self.tsv
    }

    /// The header/structure filters this invocation asks for.
    ///
    /// Patterns are lowercased here, once, because matching lowercases the
    /// header it compares them against.
    pub fn filters(&self) -> Filters {
        Filters {
            from_patterns: self
                .from
                .iter()
                .map(|pattern| pattern.trim().to_ascii_lowercase())
                .filter(|pattern| !pattern.is_empty())
                .collect(),
            require_attachment: self.has_attachment,
        }
    }

    /// The text query as one human-readable line.
    ///
    /// Empty groups are dropped the same way [`crate::email::parse_query_groups`]
    /// drops them, so an all-`--or` search reads as `foo OR bar` rather than
    /// carrying a leading separator for the absent positional query.
    fn query_groups_display(&self) -> String {
        std::iter::once(self.query_string())
            .chain(self.or_terms.iter().cloned())
            .map(|group| group.trim().to_string())
            .filter(|group| !group.is_empty())
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    /// The whole search as one human-readable line, for status messages and the
    /// TUI header.
    ///
    /// Filters are appended as `+from:…` / `+attachment` rather than folded into
    /// the query, because they are AND-ed with it. A filter-only search shows
    /// `(any)` for the query so the line cannot be misread as an empty search.
    pub fn display_query(&self) -> String {
        let query = self.query_groups_display();
        let mut line = if query.is_empty() {
            "(any)".to_string()
        } else {
            query
        };
        if !self.from.is_empty() {
            line.push_str(&format!(" +from:{}", self.from.join(",")));
        }
        if self.has_attachment {
            line.push_str(" +attachment");
        }
        line
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
    fn json_and_tsv_conflict() {
        assert!(Config::try_parse_from(["mailsearch", "q", "--json", "--tsv"]).is_err());
    }

    #[test]
    fn machine_output_covers_both_stdout_modes() {
        assert!(parse(&["q", "--json"]).machine_output());
        assert!(parse(&["q", "--tsv"]).machine_output());
        assert!(!parse(&["q"]).machine_output());
    }

    #[test]
    fn from_alone_is_accepted() {
        // "everything this person sent" is a complete search on its own.
        let config = parse(&["--from", "t.suzuki"]);
        assert_eq!(config.query_string(), "");
        assert_eq!(config.filters().from_patterns, ["t.suzuki"]);
    }

    #[test]
    fn has_attachment_alone_is_accepted() {
        let config = parse(&["--has-attachment"]);
        assert!(config.filters().require_attachment);
    }

    #[test]
    fn from_patterns_are_lowercased_and_trimmed() {
        // Matching lowercases the header, so the patterns must arrive lowercased.
        let config = parse(&["--from", " T.Suzuki ", "--from", "TANAKA"]);
        assert_eq!(config.filters().from_patterns, ["t.suzuki", "tanaka"]);
    }

    #[test]
    fn no_filter_flags_means_no_filters() {
        let filters = parse(&["query"]).filters();
        assert!(filters.from_patterns.is_empty());
        assert!(!filters.require_attachment);
    }

    #[test]
    fn display_query_shows_the_filters() {
        assert_eq!(
            parse(&["鶏卵", "--from", "t.suzuki", "--has-attachment"]).display_query(),
            "鶏卵 +from:t.suzuki +attachment"
        );
    }

    #[test]
    fn display_query_of_a_filter_only_search_is_not_blank() {
        assert_eq!(
            parse(&["--from", "t.suzuki", "--from", "tanaka"]).display_query(),
            "(any) +from:t.suzuki,tanaka"
        );
    }

    #[test]
    fn or_groups_alone_are_accepted() {
        // `--or` without a positional query used to be a usage error.
        let config = parse(&["--or", "foo", "--or", "bar"]);
        assert_eq!(config.query_string(), "");
        assert_eq!(config.or_terms, ["foo", "bar"]);
    }

    #[test]
    fn display_query_joins_the_groups() {
        assert_eq!(parse(&["foo", "--or", "bar"]).display_query(), "foo OR bar");
    }

    #[test]
    fn display_query_drops_the_absent_positional() {
        // Otherwise the header would read " OR foo OR bar".
        assert_eq!(
            parse(&["--or", "foo", "--or", "bar"]).display_query(),
            "foo OR bar"
        );
    }

    #[test]
    fn display_query_of_a_plain_query_is_the_query() {
        assert_eq!(parse(&["オボムコイド", "鶏卵"]).display_query(), "オボムコイド 鶏卵");
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
