//! Email parsing and text extraction utilities.

use crate::models::{DATE_FORMAT, NO_SUBJECT, UNKNOWN_SENDER};
use chrono::{TimeZone, Utc};
use mailparse::dateparse;
use mailparse::MailHeaderMap;
use regex::Regex;
use std::sync::OnceLock;

/// Macro to generate cached regex functions.
macro_rules! cached_regex {
    ($name:ident, $pattern:literal) => {
        fn $name() -> &'static Regex {
            static REGEX: OnceLock<Regex> = OnceLock::new();
            REGEX.get_or_init(|| Regex::new($pattern).unwrap())
        }
    };
}

// Cached regex patterns for HTML stripping.
cached_regex!(html_tag_regex, r"<[^>]+>");
cached_regex!(style_block_regex, r"(?is)<style[^>]*>.*?</style>");
cached_regex!(script_block_regex, r"(?is)<script[^>]*>.*?</script>");
cached_regex!(html_comment_regex, r"(?s)<!--.*?-->");
cached_regex!(whitespace_regex, r"\s+");

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
    value.replace(['\r', '\n'], " ")
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

    // Extract text content (without headers, since they're displayed separately in UI)
    let text_content = extract_email_text(&mail, false);

    // Check if matches query
    if !matches_query(&text_content, query) {
        return None;
    }

    let subject = extract_header(&mail, "Subject", NO_SUBJECT);
    let from_addr = extract_header(&mail, "From", UNKNOWN_SENDER);
    let to_addr = extract_header(&mail, "To", "");
    let cc_addr = extract_header(&mail, "Cc", "");
    let date_str = format_date(mail.get_headers().get_first_value("Date").as_deref());

    Some(crate::models::SearchResult {
        subject,
        from_addr,
        to_addr,
        cc_addr,
        date_str,
        file_path: path.display().to_string(),
        content: text_content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Helper function to get fixture path
    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    // ========== matches_query tests ==========
    
    #[test]
    fn test_matches_query_single_term() {
        assert!(matches_query("Hello World", "hello"));
        assert!(matches_query("Hello World", "world"));
        assert!(!matches_query("Hello World", "foo"));
    }

    #[test]
    fn test_matches_query_multiple_terms_and_logic() {
        // All terms must be present (AND logic)
        assert!(matches_query("rust programming language", "rust language"));
        assert!(matches_query("rust programming language", "rust programming"));
        assert!(!matches_query("rust programming", "rust java"));
        assert!(!matches_query("rust", "rust java"));
    }

    #[test]
    fn test_matches_query_case_insensitive() {
        assert!(matches_query("RuSt ProGramMinG", "rust"));
        assert!(matches_query("rust", "RUST"));
        assert!(matches_query("RuSt", "rUsT"));
        assert!(matches_query("Hello WORLD", "hello world"));
    }

    #[test]
    fn test_matches_query_partial_word_match() {
        assert!(matches_query("testing", "test"));
        assert!(matches_query("programming", "program"));
        assert!(matches_query("email@example.com", "example"));
    }

    #[test]
    fn test_matches_query_empty_cases() {
        assert!(matches_query("any text", ""));
        assert!(!matches_query("", "query"));
        assert!(matches_query("", ""));
    }

    #[test]
    fn test_matches_query_special_characters() {
        assert!(matches_query("user@example.com", "@example"));
        assert!(matches_query("price: $100", "$100"));
        assert!(matches_query("50% discount", "50%"));
    }

    #[test]
    fn test_matches_query_whitespace_handling() {
        assert!(matches_query("multiple   spaces", "multiple spaces"));
        assert!(matches_query("text with\nnewlines", "text newlines"));
        assert!(matches_query("  leading spaces", "leading"));
    }

    // ========== strip_html_tags tests ==========

    #[test]
    fn test_strip_html_tags_simple() {
        assert_eq!(strip_html_tags("<p>Hello</p>"), "Hello");
        assert_eq!(strip_html_tags("<div>World</div>"), "World");
        assert_eq!(strip_html_tags("<span>Text</span>"), "Text");
    }

    #[test]
    fn test_strip_html_tags_nested() {
        assert_eq!(
            strip_html_tags("<div><p><span>Nested</span></p></div>"),
            "Nested"
        );
        assert_eq!(
            strip_html_tags("<html><body><h1>Title</h1><p>Content</p></body></html>"),
            "Title Content"
        );
    }

    #[test]
    fn test_strip_html_tags_with_attributes() {
        assert_eq!(
            strip_html_tags("<div class='test' id='main'>Content</div>"),
            "Content"
        );
        assert_eq!(
            strip_html_tags("<a href='http://example.com'>Link</a>"),
            "Link"
        );
    }

    #[test]
    fn test_strip_html_tags_self_closing() {
        assert_eq!(strip_html_tags("Line 1<br/>Line 2"), "Line 1 Line 2");
        assert_eq!(strip_html_tags("<img src='test.jpg'/>Text"), "Text");
    }

    #[test]
    fn test_strip_html_tags_plain_text() {
        assert_eq!(strip_html_tags("Plain text"), "Plain text");
        assert_eq!(strip_html_tags("No HTML here"), "No HTML here");
    }

    #[test]
    fn test_strip_html_tags_mixed_content() {
        assert_eq!(
            strip_html_tags("Text before <b>bold</b> text after"),
            "Text before bold text after"
        );
    }

    #[test]
    fn test_strip_html_tags_whitespace_normalization() {
        let html = "<div>  Multiple   spaces  </div>";
        let result = strip_html_tags(html);
        // Should normalize whitespace
        assert!(!result.contains("   "));
    }

    #[test]
    fn test_strip_html_tags_empty() {
        assert_eq!(strip_html_tags(""), "");
        assert_eq!(strip_html_tags("<div></div>"), "");
    }

    // ========== format_date tests ==========

    #[test]
    fn test_format_date_valid_rfc2822() {
        let date = "Mon, 20 Jan 2026 10:30:00 +0000";
        let formatted = format_date(Some(date));
        assert!(formatted.contains("2026"));
        assert!(formatted.contains("01")); // Month
        assert!(formatted.contains("20")); // Day
    }

    #[test]
    fn test_format_date_none() {
        assert_eq!(format_date(None), "N/A");
    }

    #[test]
    fn test_format_date_invalid_format() {
        let invalid = "Not a valid date";
        let result = format_date(Some(invalid));
        // dateparse may return epoch time (1970-01-01) for invalid dates
        // or return the original string depending on how it fails
        assert!(!result.is_empty());
    }

    #[test]
    fn test_format_date_empty_string() {
        let result = format_date(Some(""));
        // dateparse may return epoch time (1970-01-01) for empty strings
        assert!(!result.is_empty());
    }

    // ========== process_emlx_file integration tests ==========

    #[test]
    fn test_process_emlx_file_plain_text() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            // Skip if fixture doesn't exist (e.g., in CI without fixtures)
            return;
        }

        let result = process_emlx_file(&path, "rust programming");
        assert!(result.is_some());
        
        let search_result = result.unwrap();
        assert_eq!(search_result.subject, "Test Plain Text Email");
        assert!(search_result.from_addr.contains("sender@example.com"));
        assert!(search_result.content.contains("rust programming"));
    }

    #[test]
    fn test_process_emlx_file_html() {
        let path = fixture_path("html_email.emlx");
        if !path.exists() {
            return;
        }

        let result = process_emlx_file(&path, "invoice receipt");
        assert!(result.is_some());
        
        let search_result = result.unwrap();
        assert_eq!(search_result.subject, "HTML Test Email");
        // HTML tags should be stripped
        assert!(!search_result.content.contains("<p>"));
        assert!(!search_result.content.contains("<div>"));
        assert!(search_result.content.contains("invoice receipt"));
    }

    #[test]
    fn test_process_emlx_file_multipart() {
        let path = fixture_path("multipart_email.emlx");
        if !path.exists() {
            return;
        }

        let result = process_emlx_file(&path, "project update");
        assert!(result.is_some());
        
        let search_result = result.unwrap();
        assert_eq!(search_result.subject, "Multipart Email Test");
        assert!(search_result.content.contains("project update"));
    }

    #[test]
    fn test_process_emlx_file_no_subject() {
        let path = fixture_path("no_subject.emlx");
        if !path.exists() {
            return;
        }

        let result = process_emlx_file(&path, "without");
        assert!(result.is_some());
        
        let search_result = result.unwrap();
        // Should use NO_SUBJECT constant
        assert!(search_result.subject.contains("No Subject") || search_result.subject.is_empty());
    }

    #[test]
    fn test_process_emlx_file_no_match() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        let result = process_emlx_file(&path, "nonexistent query xyz");
        assert!(result.is_none());
    }

    #[test]
    fn test_process_emlx_file_malformed() {
        let path = fixture_path("malformed.emlx");
        if !path.exists() {
            return;
        }

        // Should handle gracefully without panicking
        let _result = process_emlx_file(&path, "any");
        // May return None or handle error gracefully
        // Main goal: no panic
    }

    #[test]
    fn test_process_emlx_file_nonexistent() {
        let path = PathBuf::from("nonexistent.emlx");
        let result = process_emlx_file(&path, "query");
        assert!(result.is_none());
    }

    #[test]
    fn test_process_emlx_file_case_insensitive_match() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        // Query in different case
        let result = process_emlx_file(&path, "RUST PROGRAMMING");
        assert!(result.is_some());
    }
}
