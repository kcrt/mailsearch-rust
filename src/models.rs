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
    /// Message date in epoch seconds, used for sorting and date filtering.
    /// `None` when neither the `Date:` header nor the file mtime could supply one.
    pub timestamp: Option<i64>,
    /// RFC 5322 `Message-ID`, angle brackets included, as it appears in the header.
    /// `None` when the message carries no usable one. Included so callers can
    /// address a message (e.g. to draft a reply) without re-parsing the file.
    pub message_id: Option<String>,
    #[serde(rename = "path")]
    pub file_path: String,
    #[serde(skip)]
    pub content: String,
}

/// Filters that narrow the scan by header or message structure, alongside the
/// text query.
///
/// Kept apart from the query groups because these are AND-ed with the query
/// (and with each other) rather than OR-ed into it: `--from a --from b` means
/// "from a or b", but a `--from` hit must still satisfy the query.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    /// Lowercased patterns matched against the `From` header; a message passes
    /// when its `From` contains **any** of them. Empty = no sender filter.
    pub from_patterns: Vec<String>,
    /// Require at least one real attachment. Inline parts (signature images and
    /// the like) do not count.
    pub require_attachment: bool,
}

// Default Mail directory
pub const DEFAULT_MAIL_ROOT: &str = "Library/Mail/V10";

// Constants
pub const DEFAULT_LIMIT: usize = usize::MAX;  // Unlimited by default
pub const DATE_FORMAT: &str = "%Y-%m-%d %H:%M";
pub const NO_SUBJECT: &str = "(No Subject)";
pub const UNKNOWN_SENDER: &str = "Unknown";
