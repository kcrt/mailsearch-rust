//! Data models for the mail search application.

/// Text part for highlighting - either matched search term or normal text.
#[derive(Debug, Clone, Copy)]
pub enum TextPart<'a> {
    Matched(&'a str),
    Normal(&'a str),
}

/// Represents a single search result from an email message.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub subject: String,
    pub from_addr: String,
    pub date_str: String,
    pub file_path: String,
    pub content: String,
}

// Default Mail directory
pub const DEFAULT_MAIL_ROOT: &str = "Library/Mail/V10";

// Constants
pub const DEFAULT_LIMIT: usize = usize::MAX;  // Unlimited by default
pub const DATE_FORMAT: &str = "%Y-%m-%d %H:%M";
pub const NO_SUBJECT: &str = "(No Subject)";
pub const UNKNOWN_SENDER: &str = "Unknown";
