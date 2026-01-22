//! Configuration and CLI argument parsing.

pub use clap::Parser;
use crate::models::{DEFAULT_LIMIT, DEFAULT_MAIL_ROOT};
use std::path::PathBuf;

/// Configuration for the search operation.
#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Search query (multiple words = AND search)
    pub query: String,

    /// Path to Mail directory
    #[arg(short = 'r', long = "mail-root", default_value = DEFAULT_MAIL_ROOT)]
    pub mail_root: PathBuf,

    /// Maximum number of results (unlimited by default; setting this enables early termination)
    #[arg(short = 'l', long = "limit", default_value_t = DEFAULT_LIMIT)]
    pub limit: usize,
}
