//! Data models for the mail search application.

use serde::Serialize;

/// Text part for highlighting - either matched search term or normal text.
#[derive(Debug, Clone, Copy)]
pub enum TextPart<'a> {
    Matched(&'a str),
    Normal(&'a str),
}

/// Represents a single search result from an email message.
///
/// Serializes to metadata only; the message body (`content`) is intentionally
/// omitted from JSON output.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub subject: String,
    #[serde(rename = "from")]
    pub from_addr: String,
    #[serde(rename = "to")]
    pub to_addr: String,
    #[serde(rename = "cc")]
    pub cc_addr: String,
    #[serde(rename = "date")]
    pub date_str: String,
    #[serde(rename = "path")]
    pub file_path: String,
    #[serde(skip)]
    pub content: String,
}

// Default Mail directory
pub const DEFAULT_MAIL_ROOT: &str = "Library/Mail/V10";

// Constants
pub const DEFAULT_LIMIT: usize = usize::MAX;  // Unlimited by default
pub const DATE_FORMAT: &str = "%Y-%m-%d %H:%M";
pub const NO_SUBJECT: &str = "(No Subject)";
pub const UNKNOWN_SENDER: &str = "Unknown";
