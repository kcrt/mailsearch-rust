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
    REGEX.get_or_init(|| Regex::new(r"<!--.*?-->").unwrap())
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
        if let Ok(body) = mail.get_body_raw() {
            if let Ok(text) = std::str::from_utf8(&body) {
                text_parts.push(text.to_string());
            }
        }
    } else if content_type.starts_with("text/html") {
        if let Ok(body) = mail.get_body_raw() {
            if let Ok(text) = std::str::from_utf8(&body) {
                text_parts.push(strip_html_tags(text));
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags_basic() {
        let html = "<p>Hello <b>World</b></p>";
        assert_eq!(strip_html_tags(html), "Hello World");
    }

    #[test]
    fn test_strip_html_tags_with_style_block() {
        let html = "<style>body { color: red; }</style><p>Hello</p>";
        assert_eq!(strip_html_tags(html), "Hello");
    }

    #[test]
    fn test_strip_html_tags_with_multiline_style() {
        let html = r#"<style>
            body {
                color: red;
                font-size: 14px;
            }
        </style>
        <p>Content</p>"#;
        assert_eq!(strip_html_tags(html), "Content");
    }

    #[test]
    fn test_strip_html_tags_with_script_block() {
        let html = "<script>alert('test');</script><p>World</p>";
        assert_eq!(strip_html_tags(html), "World");
    }

    #[test]
    fn test_strip_html_tags_with_multiline_script() {
        let html = r#"<script type="text/javascript">
            function test() {
                alert('test');
            }
        </script>
        <p>Text</p>"#;
        assert_eq!(strip_html_tags(html), "Text");
    }

    #[test]
    fn test_strip_html_tags_with_html_comments() {
        let html = "<!-- This is a comment --><p>Text</p>";
        assert_eq!(strip_html_tags(html), "Text");
    }

    #[test]
    fn test_strip_html_tags_with_multiline_comments() {
        let html = r#"<!-- This is
        a multiline
        comment -->
        <p>Content</p>"#;
        assert_eq!(strip_html_tags(html), "Content");
    }

    #[test]
    fn test_strip_html_tags_combined() {
        let html = r#"
        <style>body { color: red; }</style>
        <script>alert('test');</script>
        <!-- comment -->
        <p>Final <b>Text</b></p>
        "#;
        assert_eq!(strip_html_tags(html), "Final Text");
    }

    #[test]
    fn test_strip_html_tags_case_insensitive() {
        let html = "<STYLE>body { color: red; }</STYLE><p>Hello</p>";
        assert_eq!(strip_html_tags(html), "Hello");
    }

    #[test]
    fn test_strip_html_tags_with_attributes() {
        let html = r#"<style type="text/css">body { color: red; }</style><p>Hello</p>"#;
        assert_eq!(strip_html_tags(html), "Hello");
    }

    #[test]
    fn test_strip_html_tags_whitespace_normalization() {
        let html = "<p>Hello    \n\n   World</p>";
        assert_eq!(strip_html_tags(html), "Hello World");
    }

    #[test]
    fn test_strip_html_tags_empty() {
        let html = "";
        assert_eq!(strip_html_tags(html), "");
    }

    #[test]
    fn test_strip_html_tags_plain_text() {
        let html = "Plain text without HTML";
        assert_eq!(strip_html_tags(html), "Plain text without HTML");
    }
}
