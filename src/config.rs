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
    pub query: String,

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

    /// Output results as JSON to stdout instead of the interactive TUI
    #[arg(long = "json", default_value_t = false)]
    pub json: bool,
}
