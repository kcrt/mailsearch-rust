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
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use mailparse::{dateparse, MailHeaderMap, ParsedMail};
use rayon::prelude::*;
use regex::Regex;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;
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
    query: String,

    /// Path to Mail directory
    #[arg(short = 'r', long = "mail-root", default_value = DEFAULT_MAIL_ROOT)]
    mail_root: PathBuf,

    /// Maximum number of results
    #[arg(short = 'l', long = "limit", default_value_t = DEFAULT_LIMIT)]
    limit: usize,
}

/// Represents a single search result from an email message.
#[derive(Debug, Clone)]
struct SearchResult {
    subject: String,
    from_addr: String,
    date_str: String,
    file_path: String,
    content: String,
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
        content: text_content,
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

/// Print search results in a formatted table (fallback when TUI is not available).
#[allow(dead_code)]
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

/// TUI Application state.
struct App {
    results: Vec<SearchResult>,
    query: String,
    selected: usize,
    content_scroll: usize,
    should_quit: bool,
}

impl App {
    fn new(results: Vec<SearchResult>, query: String) -> Self {
        Self {
            results,
            query,
            selected: 0,
            content_scroll: 0,
            should_quit: false,
        }
    }

    fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }

    fn next(&mut self) {
        if self.selected < self.results.len().saturating_sub(1) {
            self.selected += 1;
            self.content_scroll = 0;
        }
    }

    fn previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.content_scroll = 0;
        }
    }

    fn scroll_content_down(&mut self, amount: usize) {
        self.content_scroll += amount;
    }

    fn scroll_content_up(&mut self, amount: usize) {
        self.content_scroll = self.content_scroll.saturating_sub(amount);
    }

    fn quit(&mut self) {
        self.should_quit = true;
    }
}

/// Get highlighted text with search terms highlighted.
fn get_highlighted_text<'a>(text: &'a str, query: &str) -> Vec<Line<'a>> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut lines = Vec::new();

    // Simple word-wrapping at terminal width (rough approximation)
    let line_width = 100;

    // Process text line-by-line to preserve newlines
    for text_line in text.lines() {
        let mut current_line = Vec::new();
        let mut current_col = 0;

        // Apply word wrapping and highlighting to each line
        for word in text_line.split_whitespace() {
            let word_len = word.width() + 1; // +1 for space
            if current_col + word_len > line_width && !current_line.is_empty() {
                lines.push(Line::from(current_line.clone()));
                current_line.clear();
                current_col = 0;
            }

            let word_lower = word.to_ascii_lowercase();
            let is_match = terms.iter().any(|term| word_lower.contains(term));

            let span = if is_match {
                Span::styled(
                    word.to_string(),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(word.to_string())
            };

            current_line.push(span);
            current_line.push(Span::raw(" "));
            current_col += word_len;
        }

        // Push the current line (even if empty) to preserve blank lines
        if !current_line.is_empty() {
            lines.push(Line::from(current_line));
        } else {
            // Empty line - preserve it
            lines.push(Line::from(Span::raw("")));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::raw("(No content)")));
    }

    lines
}

/// Draw the TUI interface.
fn draw_ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Split into top (results list) and bottom (content preview)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
        .split(size);

    // Results list
    let items: Vec<ListItem> = app
        .results
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let style = if i == app.selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("{}: {}", result.from_addr, result.subject)).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Results for: {} ({}) ", app.query, app.results.len()))
                .title_style(Style::default().fg(Color::Green)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(list, chunks[0], &mut ratatui::widgets::ListState::default().with_selected(Some(app.selected)));

    // Content preview
    let content = if let Some(result) = app.selected_result() {
        let header = format!(
            "From: {}\nSubject: {}\nDate: {}",
            result.from_addr,
            result.subject,
            result.date_str
        );
        format!("{}\n\n{}", header, result.content)
    } else {
        "No results".to_string()
    };

    let highlighted_text = get_highlighted_text(&content, &app.query);

    let content_paragraph = Paragraph::new(highlighted_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Content ")
                .title_style(Style::default().fg(Color::Green)),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.content_scroll as u16, 0));

    f.render_widget(content_paragraph, chunks[1]);

    // Help footer
    let help_text = vec![
        Line::from(vec![
            Span::styled(" ↑↓ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Navigate "),
            Span::styled(" Space ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" QuickLook "),
            Span::styled(" Enter ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Open "),
            Span::styled(" PgUp/Dn ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Scroll content "),
            Span::styled(" q ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" Quit "),
        ]),
    ];

    let help_paragraph = Paragraph::new(help_text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));

    let help_height = 3;
    let help_rect = Rect {
        x: chunks[1].x,
        y: chunks[1].bottom() - help_height,
        width: chunks[1].width,
        height: help_height,
    };

    // Draw help over the content area
    f.render_widget(help_paragraph, help_rect);
}

/// Run the TUI application.
fn run_tui(results: Vec<SearchResult>, query: String) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(results, query);

    // Event loop
    while !app.should_quit {
        terminal.draw(|f| draw_ui(f, &app))?;

        // Handle input with timeout
        if event::poll(Duration::from_millis(100))? {
            if let event::Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => app.quit(),
                        KeyCode::Down | KeyCode::Char('j') => app.next(),
                        KeyCode::Up | KeyCode::Char('k') => app.previous(),
                        KeyCode::PageDown => app.scroll_content_down(10),
                        KeyCode::PageUp => app.scroll_content_up(10),
                        KeyCode::Enter => {
                            if let Some(result) = app.selected_result() {
                                // Leave TUI mode temporarily
                                disable_raw_mode()?;
                                execute!(
                                    terminal.backend_mut(),
                                    LeaveAlternateScreen,
                                    DisableMouseCapture
                                )?;

                                // Open file
                                #[allow(clippy::empty_single_line)]
                                if let Err(e) = open::that(&result.file_path) {
                                    eprintln!("Failed to open file: {}", e);
                                }

                                // Restore TUI mode
                                execute!(
                                    terminal.backend_mut(),
                                    EnterAlternateScreen,
                                    EnableMouseCapture
                                )?;
                                enable_raw_mode()?;
                            }
                        }
                        KeyCode::Char(' ') => {
                            if let Some(result) = app.selected_result() {
                                // Leave TUI mode temporarily
                                disable_raw_mode()?;
                                execute!(
                                    terminal.backend_mut(),
                                    LeaveAlternateScreen,
                                    DisableMouseCapture
                                )?;

                                // QuickLook
                                #[allow(clippy::empty_single_line)]
                                if let Err(e) = ql_command(&result.file_path) {
                                    eprintln!("QuickLook failed: {}", e);
                                }

                                // Restore TUI mode
                                execute!(
                                    terminal.backend_mut(),
                                    EnterAlternateScreen,
                                    EnableMouseCapture
                                )?;
                                enable_raw_mode()?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// Run QuickLook on a file (macOS only).
fn ql_command(path: &str) -> Result<()> {
    use std::process::Command;
    Command::new("qlmanage")
        .args(["-p", path])
        .status()
        .context("Failed to execute qlmanage")?;
    Ok(())
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

    if results.is_empty() {
        println!("\nNo messages found matching: {}", config.query);
    } else {
        // Run TUI
        run_tui(results, config.query)?;
    }

    Ok(())
}
