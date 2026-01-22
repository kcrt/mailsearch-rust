//! Apple Mail Full-Text Search Tool
//!
//! Performs fast full-text search on Apple Mail .emlx files.
//!
//! Usage:
//!     cargo run -- search terms here
//!     cargo run -- "exact phrase"
//!     cargo run -- --mail-root ~/Library/Mail/V10 project

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use clap::Parser;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use mailparse::{dateparse, MailHeaderMap, ParsedMail};
use rayon::prelude::*;
use regex::Regex;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use walkdir::WalkDir;

// Default Mail directory
const DEFAULT_MAIL_ROOT: &str = "Library/Mail/V10";

// Constants
const DEFAULT_LIMIT: usize = 50;
const DATE_FORMAT: &str = "%Y-%m-%d %H:%M";
const NO_SUBJECT: &str = "(No Subject)";
const UNKNOWN_SENDER: &str = "Unknown";

/// Configuration for the search operation.
#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
struct Config {
    /// Search query (multiple words = AND search)
    #[arg(required_unless_present_any(["--help", "-h"]))]
    query: String,

    /// Path to Mail directory
    #[arg(short = 'r', long = "mail-root", default_value = DEFAULT_MAIL_ROOT)]
    mail_root: PathBuf,

    /// Maximum number of results
    #[arg(short = 'l', long = "limit", default_value_t = DEFAULT_LIMIT)]
    limit: usize,
}

/// Represents a single search result from an email message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchResult {
    subject: String,
    from_addr: String,
    date_str: String,
    file_path: String,
}

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
fn format_date(date_header: Option<&str>) -> String {
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
fn strip_html_tags(html: &str) -> String {
    let text = html_tag_regex().replace_all(html, " ");
    whitespace_regex().replace_all(&text, " ").trim().to_string()
}

/// Clean embedded newlines from header values.
fn clean_header_value(value: &str) -> String {
    value.replace('\r', " ").replace('\n', " ")
}

/// Extract all text content from an email message.
fn extract_email_text(mail: &ParsedMail<'_>, include_headers: bool) -> String {
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
    }

    // Extract body content
    extract_body_text(mail, &mut text_parts);

    text_parts.join("\n")
}

/// Extract body text from email parts recursively.
fn extract_body_text(mail: &ParsedMail<'_>, text_parts: &mut Vec<String>) {
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
fn matches_query(text: &str, query: &str) -> bool {
    let text_lower = text.to_ascii_lowercase();
    query
        .split_whitespace()
        .all(|term| text_lower.contains(&term.to_ascii_lowercase()))
}

/// Extract header value safely.
fn extract_header(mail: &ParsedMail<'_>, header: &str, default: &str) -> String {
    mail.headers
        .get_first_header(header)
        .map(|h| clean_header_value(&h.get_value()))
        .unwrap_or_else(|| default.to_string())
}

/// Find all .emlx files in the Mail directory.
fn find_emlx_files(mail_root: &Path) -> Vec<PathBuf> {
    WalkDir::new(mail_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "emlx"))
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

/// Process a single .emlx file and return SearchResult if it matches the query.
fn process_emlx_file(path: &Path, query: &str) -> Option<SearchResult> {
    let content = fs::read_to_string(path).ok()?;

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

    Some(SearchResult {
        subject,
        from_addr,
        date_str,
        file_path: path.display().to_string(),
    })
}

/// Search for messages matching the query.
fn search_messages(mail_root: &Path, query: &str, limit: usize) -> Vec<SearchResult> {
    let files = find_emlx_files(mail_root);
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    let results: Vec<SearchResult> = files
        .into_par_iter()
        .progress_with(pb)
        .filter_map(|emlx_file| process_emlx_file(&emlx_file, query))
        .collect();

    results.into_iter().take(limit).collect()
}

/// Print search results in a formatted table.
fn print_results(results: &[SearchResult], query: &str) {
    if results.is_empty() {
        println!("\nNo messages found matching: {}", query);
        return;
    }

    println!("\n{}", "=".repeat(100));
    println!("Search results for: {}", query);
    println!("Found {} messages", results.len());
    println!("{}\n", "=".repeat(100));

    for result in results {
        println!("From:    {}", result.from_addr);
        println!("   Subject: {}", result.subject);
        println!("   Date:    {}", result.date_str);
        println!("{}", "-".repeat(100));
    }
}

fn main() -> Result<()> {
    let mut config = Config::parse();

    // Expand tilde in path
    if config.mail_root.starts_with("~") {
        let home = env::var("HOME").context("Could not determine HOME environment variable")?;
        let rest = config
            .mail_root
            .strip_prefix("~")
            .unwrap_or_else(|_| config.mail_root.as_path());
        config.mail_root = PathBuf::from(home).join(rest);
    }

    // Handle relative path from home directory
    if !config.mail_root.is_absolute() {
        config.mail_root = dirs::home_dir()
            .context("Could not determine home directory")?
            .join(&config.mail_root);
    }

    if !config.mail_root.exists() {
        eprintln!("Error: Mail directory not found:");
        eprintln!("   {}", config.mail_root.display());
        eprintln!("\nTo fix this:");
        eprintln!("   1. Open System Settings → Privacy & Security → Full Disk Access");
        eprintln!("   2. Add Terminal or your IDE to the allowed applications");
        eprintln!("   3. Restart Terminal/IDE and try again");
        std::process::exit(1);
    }

    println!("Searching Mail files...");
    println!("   Directory: {}", config.mail_root.display());
    println!("   Query: {}", config.query);
    println!("   Scanning .emlx files...\n");

    let results = search_messages(&config.mail_root, &config.query, config.limit);
    print_results(&results, &config.query);

    Ok(())
}
