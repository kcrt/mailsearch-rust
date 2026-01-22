//! Email parsing and text extraction utilities.

use crate::models::{DATE_FORMAT, NO_SUBJECT, UNKNOWN_SENDER};
use chrono::{TimeZone, Utc};
use mailparse::dateparse;
use mailparse::MailHeaderMap;
use regex::Regex;
use std::sync::OnceLock;

/// Cached regex patterns for HTML stripping.
fn html_tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
}

fn style_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap())
}

fn script_block_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap())
}

fn html_comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").unwrap())
}

fn whitespace_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\s+").unwrap())
}

/// Format email date header to readable string.
pub fn format_date(date_header: Option<&str>) -> String {
    if let Some(date_str) = date_header {
        if let Ok(timestamp) = dateparse(date_str) {
            if let Some(dt) = Utc.timestamp_opt(timestamp, 0).single() {
                return dt.format(DATE_FORMAT).to_string();
            }
        }
        // Return original if parsing fails
        return date_str.to_string();
    }
    "N/A".to_string()
}

/// Remove HTML tags, CSS, scripts, and normalize whitespace.
pub fn strip_html_tags(html: &str) -> String {
    // Remove style blocks
    let text = style_block_regex().replace_all(html, " ");
    // Remove script blocks
    let text = script_block_regex().replace_all(&text, " ");
    // Remove HTML comments
    let text = html_comment_regex().replace_all(&text, " ");
    // Remove remaining HTML tags
    let text = html_tag_regex().replace_all(&text, " ");
    // Normalize whitespace
    whitespace_regex().replace_all(&text, " ").trim().to_string()
}

/// Clean embedded newlines from header values.
pub fn clean_header_value(value: &str) -> String {
    value.replace('\r', " ").replace('\n', " ")
}

/// Extract header value safely.
pub fn extract_header(mail: &mailparse::ParsedMail<'_>, header: &str, default: &str) -> String {
    mail.headers
        .get_first_header(header)
        .map(|h| clean_header_value(&h.get_value()))
        .unwrap_or_else(|| default.to_string())
}

/// Extract all text content from an email message.
pub fn extract_email_text(mail: &mailparse::ParsedMail<'_>, include_headers: bool) -> String {
    let mut text_parts = Vec::new();

    // Extract headers if requested
    if include_headers {
        let headers = ["Subject", "From", "To", "Cc", "Reply-To"];
        for header in headers {
            if let Some(value) = mail.headers.get_first_header(header) {
                let cleaned = clean_header_value(&value.get_value());
                text_parts.push(format!("{}: {}", header, cleaned));
            }
        }
        // Add blank line separator between headers and body
        text_parts.push(String::new());
    }

    // Extract body content
    extract_body_text(mail, &mut text_parts);

    text_parts.join("\n")
}

/// Extract body text from email parts recursively.
fn extract_body_text(mail: &mailparse::ParsedMail<'_>, text_parts: &mut Vec<String>) {
    let content_type = mail.ctype.mimetype.to_lowercase();

    // Skip attachments
    let cd = mail.get_content_disposition();
    if let mailparse::DispositionType::Attachment = cd.disposition {
        return;
    }

    if content_type.starts_with("text/plain") {
        // Use get_body() which handles character encoding automatically
        if let Ok(text) = mail.get_body() {
            text_parts.push(text);
        }
    } else if content_type.starts_with("text/html") {
        // Use get_body() which handles character encoding automatically
        if let Ok(text) = mail.get_body() {
            text_parts.push(strip_html_tags(&text));
        }
    } else if content_type.starts_with("multipart/") {
        for subpart in &mail.subparts {
            extract_body_text(subpart, text_parts);
        }
    }
}

/// Check if text matches the search query (all terms must be present).
pub fn matches_query(text: &str, query: &str) -> bool {
    let text_lower = text.to_ascii_lowercase();
    query
        .split_whitespace()
        .all(|term| text_lower.contains(&term.to_ascii_lowercase()))
}

/// Process a single .emlx file and return SearchResult if it matches the query.
pub fn process_emlx_file(
    path: &std::path::Path,
    query: &str,
) -> Option<crate::models::SearchResult> {
    let content = std::fs::read_to_string(path).ok()?;

    // .emlx format:
    // Line 1: Byte count
    // Line 2+: MIME content
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    // First line is byte count, skip it
    let mime_content = &content[lines[0].len() + 1..];
    let bytes = mime_content.as_bytes();

    // Parse as email
    let mail = mailparse::parse_mail(bytes).ok()?;

    // Extract text content
    let text_content = extract_email_text(&mail, true);

    // Check if matches query
    if !matches_query(&text_content, query) {
        return None;
    }

    let subject = extract_header(&mail, "Subject", NO_SUBJECT);
    let from_addr = extract_header(&mail, "From", UNKNOWN_SENDER);
    let date_str = format_date(mail.get_headers().get_first_value("Date").as_deref());

    Some(crate::models::SearchResult {
        subject,
        from_addr,
        date_str,
        file_path: path.display().to_string(),
        content: text_content,
    })
}
