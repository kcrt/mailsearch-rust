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

/// Remove HTML tags and normalize whitespace.
pub fn strip_html_tags(html: &str) -> String {
    let text = html_tag_regex().replace_all(html, " ");
    whitespace_regex().replace_all(&text, " ").trim().to_string()
}

/// Clean embedded newlines from header values.
pub fn clean_header_value(value: &str) -> String {
    value.chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect()
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
/// 
/// Note: This function processes query terms on each call. For batch processing
/// of multiple documents with the same query, use `matches_query_with_terms()` 
/// with pre-processed terms for better performance.
pub fn matches_query(text: &str, query: &str) -> bool {
    // Pre-process query terms to lowercase once
    let lowercase_terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect();
    
    matches_query_with_terms(text, &lowercase_terms)
}

/// Check if text matches pre-processed lowercase search terms.
/// This is more efficient when matching multiple documents with the same query.
pub fn matches_query_with_terms(text: &str, lowercase_terms: &[String]) -> bool {
    let text_lower = text.to_ascii_lowercase();
    lowercase_terms.iter().all(|term| text_lower.contains(term))
}

/// Process a single .emlx file and return SearchResult if it matches the query.
pub fn process_emlx_file(
    path: &std::path::Path,
    query: &str,
) -> Option<crate::models::SearchResult> {
    // Pre-process query terms once
    let lowercase_terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect();
    
    process_emlx_file_with_terms(path, &lowercase_terms)
}

/// Process a single .emlx file with pre-processed search terms.
/// This is more efficient when processing multiple files with the same query.
pub fn process_emlx_file_with_terms(
    path: &std::path::Path,
    lowercase_terms: &[String],
) -> Option<crate::models::SearchResult> {
    // Read file as bytes to avoid UTF-8 validation overhead initially
    let content = std::fs::read(path).ok()?;

    // .emlx format:
    // Line 1: Byte count (ASCII)
    // Line 2+: MIME content
    // Find the first newline to skip the byte count line
    let newline_pos = content.iter().position(|&b| b == b'\n')?;
    
    // MIME content starts after first newline
    let mime_content = &content[newline_pos + 1..];

    // Parse as email
    let mail = mailparse::parse_mail(mime_content).ok()?;

    // Extract text content (includes headers and body)
    let text_content = extract_email_text(&mail, true);
    
    // Check if matches query
    if !matches_query_with_terms(&text_content, lowercase_terms) {
        return None;
    }

    // Only extract metadata if we have a match
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
    fn test_matches_query_single_term() {
        assert!(matches_query("Hello World", "hello"));
        assert!(matches_query("Hello World", "world"));
        assert!(!matches_query("Hello World", "foo"));
    }

    #[test]
    fn test_matches_query_multiple_terms() {
        assert!(matches_query("Hello World from Rust", "hello rust"));
        assert!(matches_query("Hello World from Rust", "world from"));
        assert!(!matches_query("Hello World from Rust", "hello python"));
    }

    #[test]
    fn test_matches_query_case_insensitive() {
        assert!(matches_query("Hello World", "HELLO"));
        assert!(matches_query("HELLO WORLD", "hello"));
        assert!(matches_query("HeLLo WoRLd", "hELLo wORld"));
    }

    #[test]
    fn test_matches_query_with_terms() {
        let terms = vec!["hello".to_string(), "world".to_string()];
        assert!(matches_query_with_terms("Hello World", &terms));
        assert!(!matches_query_with_terms("Hello There", &terms));
    }

    #[test]
    fn test_matches_query_with_terms_empty() {
        let terms: Vec<String> = vec![];
        assert!(matches_query_with_terms("Hello World", &terms));
    }

    #[test]
    fn test_strip_html_tags() {
        let html = "<p>Hello <strong>World</strong>!</p>";
        let text = strip_html_tags(html);
        assert_eq!(text, "Hello World !");
    }

    #[test]
    fn test_clean_header_value() {
        let value = "Subject:\r\n with newlines";
        let cleaned = clean_header_value(value);
        assert_eq!(cleaned, "Subject:   with newlines"); // \r and \n each become space
    }

    #[test]
    fn test_process_emlx_file_with_terms() {
        // Create a temporary test email file
        let test_dir = std::env::temp_dir().join("mailsearch_test");
        std::fs::create_dir_all(&test_dir).unwrap();
        
        let test_file = test_dir.join("test.emlx");
        let content = "365\nFrom: test@example.com\nTo: user@example.com\nSubject: Rust Programming\nDate: Mon, 1 Jan 2024 12:00:00 +0000\nContent-Type: text/plain; charset=utf-8\n\nThis is a test email about Rust performance.";
        std::fs::write(&test_file, content).unwrap();

        // Test matching query
        let terms = vec!["rust".to_string(), "performance".to_string()];
        let result = process_emlx_file_with_terms(&test_file, &terms);
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.subject.contains("Rust"));
        assert!(result.content.contains("performance"));

        // Test non-matching query
        let terms = vec!["python".to_string()];
        let result = process_emlx_file_with_terms(&test_file, &terms);
        assert!(result.is_none());

        // Cleanup
        std::fs::remove_file(test_file).ok();
        std::fs::remove_dir(test_dir).ok();
    }
}
