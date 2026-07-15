//! Terminal User Interface for displaying search results.

use crate::highlight::get_highlighted_text;
use crate::models::SearchResult;
use crate::sort::{compare_results, parse_date_from_result, SortMode};
use anyhow::{Context, Result};
use chrono::NaiveDate;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::time::Duration;

/// Filter type for filtering search results.
#[derive(Debug, Clone)]
enum FilterType {
    From(String),
    Subject(String),
    All(String), // Search across all fields (from, subject, content)
    After(NaiveDate),
    Before(NaiveDate),
}

/// TUI Application state.
pub struct App {
    pub results: Vec<SearchResult>,
    pub query: String,
    pub highlight_terms: Vec<String>,
    pub selected: usize,
    pub content_scroll: usize,
    pub should_quit: bool,
    pub filter_input: String,
    pub filter_mode: bool,
    pub filtered_indices: Option<Vec<usize>>,
    sort_mode: SortMode,
    sorted_indices: Option<Vec<usize>>,
}

impl App {
    pub fn new(results: Vec<SearchResult>, query: String, highlight_terms: Vec<String>) -> Self {
        Self {
            results,
            query,
            highlight_terms,
            selected: 0,
            content_scroll: 0,
            should_quit: false,
            filter_input: String::new(),
            filter_mode: false,
            filtered_indices: None,
            sort_mode: SortMode::NoSort,
            sorted_indices: None,
        }
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.apply_sort();
        self.selected = 0;
        self.content_scroll = 0;
    }

    /// Set the initial sort mode (e.g. from the `--sort` CLI flag) and apply it.
    pub fn set_initial_sort(&mut self, mode: SortMode) {
        self.sort_mode = mode;
        self.apply_sort();
    }

    fn apply_sort(&mut self) {
        if self.sort_mode == SortMode::NoSort {
            self.sorted_indices = None;
            return;
        }

        // Get base indices (either filtered or all)
        let base_indices: Vec<usize> = if let Some(ref filtered) = self.filtered_indices {
            filtered.clone()
        } else {
            (0..self.results.len()).collect()
        };

        let mut sorted: Vec<usize> = base_indices;
        sorted.sort_by(|&a, &b| {
            compare_results(&self.results[a], &self.results[b], self.sort_mode)
        });

        self.sorted_indices = Some(sorted);
    }

    pub fn selected_result(&self) -> Option<&SearchResult> {
        // Get the index based on sorted -> filtered -> direct
        // If both filters are applied, sorted contains filtered + sorted indices
        let idx = if let Some(ref sorted) = self.sorted_indices {
            sorted.get(self.selected).copied()
        } else if let Some(ref filtered) = self.filtered_indices {
            filtered.get(self.selected).copied()
        } else {
            Some(self.selected)
        };
        idx.and_then(|i| self.results.get(i))
    }

    pub fn visible_results_count(&self) -> usize {
        if let Some(ref sorted) = self.sorted_indices {
            sorted.len()
        } else {
            self.filtered_indices.as_ref().map_or(self.results.len(), |v| v.len())
        }
    }

    fn get_visible_indices(&self) -> Vec<usize> {
        if let Some(ref sorted) = self.sorted_indices {
            sorted.clone()
        } else if let Some(ref filtered) = self.filtered_indices {
            filtered.clone()
        } else {
            (0..self.results.len()).collect()
        }
    }

    pub fn next(&mut self) {
        let count = self.visible_results_count();
        if self.selected < count.saturating_sub(1) {
            self.selected += 1;
            self.content_scroll = 0;
        }
    }

    pub fn previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.content_scroll = 0;
        }
    }

    pub fn scroll_content_down(&mut self, amount: usize) {
        self.content_scroll += amount;
    }

    pub fn scroll_content_up(&mut self, amount: usize) {
        self.content_scroll = self.content_scroll.saturating_sub(amount);
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn enter_filter_mode(&mut self) {
        self.filter_mode = true;
    }

    pub fn exit_filter_mode(&mut self) {
        self.filter_mode = false;
        self.filter_input.clear();
    }

    pub fn clear_filter(&mut self) {
        self.filter_input.clear();
        self.filtered_indices = None;
        self.sorted_indices = None; // Clear sort when clearing filter
        self.selected = 0;
        self.content_scroll = 0;
        self.apply_sort(); // Reapply sort to all results
    }

    pub fn add_filter_char(&mut self, c: char) {
        self.filter_input.push(c);
    }

    pub fn delete_filter_char(&mut self) {
        self.filter_input.pop();
    }

    pub fn clear_filter_input(&mut self) {
        self.filter_input.clear();
    }

    pub fn apply_filter(&mut self) {
        if self.filter_input.is_empty() {
            self.filtered_indices = None;
        } else {
            let filters = parse_filter(&self.filter_input);

            // Only apply filter if valid filters were parsed
            // (e.g., "unknown:filter" would be invalid, but plain text "hello" is valid)
            if filters.is_empty() {
                // Invalid filter format - show no results
                self.filtered_indices = Some(Vec::new());
            } else {
                let mut matching_indices = Vec::new();

                for (i, result) in self.results.iter().enumerate() {
                    if filters.iter().all(|filter| match_filter(filter, result)) {
                        matching_indices.push(i);
                    }
                }

                self.filtered_indices = Some(matching_indices);
            }
        }
        self.selected = 0;
        self.content_scroll = 0;
        self.filter_mode = false;
        self.apply_sort(); // Reapply sort to filtered results
    }
}

/// Parse filter string into a list of filter criteria.
fn parse_filter(input: &str) -> Vec<FilterType> {
    let mut filters = Vec::new();
    let mut chars = input.chars().peekable();
    let mut current_token = String::new();

    while let Some(c) = chars.next() {
        if c.is_whitespace() && current_token.is_empty() {
            continue;
        }

        if c.is_whitespace() {
            if !current_token.is_empty() {
                if let Some(filter) = parse_single_filter(&current_token) {
                    filters.push(filter);
                }
                current_token.clear();
            }
        } else if c == '"' {
            // Handle quoted strings
            let mut quoted = String::new();
            for qc in chars.by_ref() {
                if qc == '"' {
                    break;
                }
                quoted.push(qc);
            }
            current_token.push_str(&quoted);
        } else {
            current_token.push(c);
        }
    }

    if !current_token.is_empty() {
        if let Some(filter) = parse_single_filter(&current_token) {
            filters.push(filter);
        }
    }

    filters
}

/// Parse a single filter token like "from:john", "after:2025-01-01", or just "hello".
fn parse_single_filter(token: &str) -> Option<FilterType> {
    if let Some((filter_type, value)) = token.split_once(':') {
        match filter_type.to_lowercase().as_str() {
            "from" => Some(FilterType::From(value.to_string())),
            "subject" => Some(FilterType::Subject(value.to_string())),
            "after" => NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .map(FilterType::After),
            "before" => NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .map(FilterType::Before),
            _ => None,
        }
    } else if !token.is_empty() {
        // No colon means search across all fields
        Some(FilterType::All(token.to_string()))
    } else {
        None
    }
}

/// Check if a search result matches a filter.
fn match_filter(filter: &FilterType, result: &SearchResult) -> bool {
    match filter {
        FilterType::From(pattern) => result
            .from_addr
            .to_lowercase()
            .contains(&pattern.to_lowercase()),
        FilterType::Subject(pattern) => result
            .subject
            .to_lowercase()
            .contains(&pattern.to_lowercase()),
        FilterType::All(pattern) => {
            let pattern_lower = pattern.to_lowercase();
            result.from_addr.to_lowercase().contains(&pattern_lower)
                || result.subject.to_lowercase().contains(&pattern_lower)
                || result.content.to_lowercase().contains(&pattern_lower)
        }
        FilterType::After(date) => {
            parse_date_from_result(result)
                .map(|result_date| result_date >= *date)
                .unwrap_or(false)
        }
        FilterType::Before(date) => {
            parse_date_from_result(result)
                .map(|result_date| result_date <= *date)
                .unwrap_or(false)
        }
    }
}

/// Draw the TUI interface.
fn draw_ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Calculate help height based on filter mode
    let help_height = if app.filter_mode { 4 } else { 3 };

    // Split into top (results list) and bottom (content preview + help)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
        .split(size);

    // Split the bottom section into content and help areas
    let bottom_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(help_height),
        ].as_ref())
        .split(chunks[1]);

    // Get visible results based on filter and sort
    let visible_indices = app.get_visible_indices();
    let visible_results: Vec<(usize, &SearchResult)> = visible_indices
        .iter()
        .map(|&i| (i, &app.results[i]))
        .collect();

    // Results list
    let items: Vec<ListItem> = visible_results
        .iter()
        .enumerate()
        .map(|(display_idx, (_, result))| {
            let style = if display_idx == app.selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("{} {}", result.from_addr, result.subject)).style(style)
        })
        .collect();

    // Build title with sort mode indicator
    let sort_indicator = if app.sort_mode != SortMode::NoSort {
        format!(" [sort: {}]", app.sort_mode.as_str())
    } else {
        String::new()
    };

    let title = if let Some(ref indices) = app.filtered_indices {
        format!(" Results for: {} ({} filtered: {}){} ", app.query, app.results.len(), indices.len(), sort_indicator)
    } else {
        format!(" Results for: {} ({}){} ", app.query, app.results.len(), sort_indicator)
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
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
        // Format metadata header
        let mut metadata = format!(
            "From: {}\nDate: {}\nSubject: {}",
            result.from_addr,
            result.date_str,
            result.subject
        );

        // Add To if present
        if !result.to_addr.is_empty() {
            metadata.push_str(&format!("\nTo: {}", result.to_addr));
        }

        // Add Cc if present
        if !result.cc_addr.is_empty() {
            metadata.push_str(&format!("\nCc: {}", result.cc_addr));
        }

        metadata.push_str(&format!("\n---\n{}", result.content));
        metadata
    } else {
        "No results".to_string()
    };

    let highlighted_text = get_highlighted_text(&content, &app.highlight_terms);

    let content_paragraph = Paragraph::new(highlighted_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Content ")
                .title_style(Style::default().fg(Color::Green)),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.content_scroll as u16, 0));

    f.render_widget(content_paragraph, bottom_chunks[0]);

    // Help footer
    let help_text = if app.filter_mode {
        vec![
            Line::from(vec![
                Span::styled(" / ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Filter "),
                Span::styled(" Esc ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Cancel "),
                Span::styled(" Enter ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Apply "),
                Span::styled(" Ctrl+U ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Clear "),
            ]),
            Line::from(vec![
                Span::styled("Filter: ", Style::default().fg(Color::Yellow)),
                Span::raw(&app.filter_input),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled(" ↑↓ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Navigate "),
                Span::styled(" s ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Sort "),
                Span::styled(" Space ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" QuickLook "),
                Span::styled(" Enter ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Open "),
                Span::styled(" / ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Filter "),
                Span::styled(" q ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" Quit "),
            ]),
        ]
    };

    let help_paragraph = Paragraph::new(help_text)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));

    // Draw help in its dedicated area
    f.render_widget(help_paragraph, bottom_chunks[1]);
}

/// Helper to temporarily suspend terminal mode for external commands.
fn with_terminal_suspended<R>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    f: impl FnOnce() -> R,
) -> R {
    disable_raw_mode().unwrap();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    ).unwrap();
    let result = f();
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    ).unwrap();
    enable_raw_mode().unwrap();
    terminal.flush().unwrap();
    result
}

/// Run QuickLook on a file (macOS only).
fn ql_command(path: &str) -> Result<()> {
    std::process::Command::new("qlmanage")
        .args(["-p", path])
        .status()
        .context("Failed to execute qlmanage")?;
    Ok(())
}

/// Handle filter mode input.
fn handle_filter_mode_input(app: &mut App, key: &crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Esc => app.exit_filter_mode(),
        KeyCode::Enter => app.apply_filter(),
        KeyCode::Backspace => app.delete_filter_char(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_filter_input()
        }
        KeyCode::Char(c) => app.add_filter_char(c),
        _ => {}
    }
}

/// Handle normal mode input.
fn handle_normal_mode_input(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    key: &crossterm::event::KeyEvent,
) {
    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Char('s') => app.cycle_sort_mode(),
        KeyCode::Esc => {
            if app.filtered_indices.is_some() {
                app.clear_filter();
            } else {
                app.quit();
            }
        }
        KeyCode::Char('/') => app.enter_filter_mode(),
        KeyCode::Down | KeyCode::Char('j') => app.next(),
        KeyCode::Up | KeyCode::Char('k') => app.previous(),
        KeyCode::PageDown => app.scroll_content_down(10),
        KeyCode::PageUp => app.scroll_content_up(10),
        KeyCode::Enter => {
            if let Some(result) = app.selected_result() {
                with_terminal_suspended(terminal, || {
                    if let Err(e) = open::that(&result.file_path) {
                        eprintln!("Failed to open file: {}", e);
                    }
                });
            }
        }
        KeyCode::Char(' ') => {
            if let Some(result) = app.selected_result() {
                with_terminal_suspended(terminal, || {
                    if let Err(e) = ql_command(&result.file_path) {
                        eprintln!("QuickLook failed: {}", e);
                    }
                });
            }
        }
        _ => {}
    }
}

/// Run the TUI application.
pub fn run_tui(
    results: Vec<SearchResult>,
    query: String,
    highlight_terms: Vec<String>,
    initial_sort: SortMode,
) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(results, query, highlight_terms);
    app.set_initial_sort(initial_sort);

    // Event loop
    while !app.should_quit {
        terminal.draw(|f| draw_ui(f, &app))?;

        // Handle input with timeout
        if event::poll(Duration::from_millis(100))? {
            if let event::Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if app.filter_mode {
                        handle_filter_mode_input(&mut app, &key);
                    } else {
                        handle_normal_mode_input(&mut app, &mut terminal, &key);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_filter_from() {
        let filters = parse_filter("from:john");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            FilterType::From(value) => assert_eq!(value, "john"),
            _ => panic!("Expected From filter"),
        }
    }

    #[test]
    fn test_parse_filter_subject() {
        let filters = parse_filter("subject:meeting");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            FilterType::Subject(value) => assert_eq!(value, "meeting"),
            _ => panic!("Expected Subject filter"),
        }
    }

    #[test]
    fn test_parse_filter_with_quotes() {
        let filters = parse_filter("subject:\"project update\"");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            FilterType::Subject(value) => assert_eq!(value, "project update"),
            _ => panic!("Expected Subject filter"),
        }
    }

    #[test]
    fn test_parse_filter_multiple() {
        let filters = parse_filter("from:john subject:meeting");
        assert_eq!(filters.len(), 2);
        match &filters[0] {
            FilterType::From(value) => assert_eq!(value, "john"),
            _ => panic!("Expected From filter"),
        }
        match &filters[1] {
            FilterType::Subject(value) => assert_eq!(value, "meeting"),
            _ => panic!("Expected Subject filter"),
        }
    }

    #[test]
    fn test_parse_filter_date() {
        let filters = parse_filter("after:2025-01-01");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            FilterType::After(date) => {
                assert_eq!(date.to_string(), "2025-01-01");
            }
            _ => panic!("Expected After filter"),
        }
    }

    #[test]
    fn test_match_filter_from() {
        let result = SearchResult {
            subject: "Test".to_string(),
            from_addr: "John Doe <john@example.com>".to_string(),
            to_addr: "".to_string(),
            cc_addr: "".to_string(),
            date_str: "2025-01-15 10:00".to_string(),
            file_path: "/path".to_string(),
            content: "Content".to_string(),
        };

        let filter = FilterType::From("john".to_string());
        assert!(match_filter(&filter, &result));

        let filter = FilterType::From("jane".to_string());
        assert!(!match_filter(&filter, &result));
    }

    #[test]
    fn test_match_filter_subject() {
        let result = SearchResult {
            subject: "Project Update Meeting".to_string(),
            from_addr: "sender@example.com".to_string(),
            to_addr: "".to_string(),
            cc_addr: "".to_string(),
            date_str: "2025-01-15 10:00".to_string(),
            file_path: "/path".to_string(),
            content: "Content".to_string(),
        };

        let filter = FilterType::Subject("project".to_string());
        assert!(match_filter(&filter, &result));

        let filter = FilterType::Subject("invoice".to_string());
        assert!(!match_filter(&filter, &result));
    }

    #[test]
    fn test_match_filter_after() {
        let result = SearchResult {
            subject: "Test".to_string(),
            from_addr: "sender@example.com".to_string(),
            to_addr: "".to_string(),
            cc_addr: "".to_string(),
            date_str: "2025-01-15 10:00".to_string(),
            file_path: "/path".to_string(),
            content: "Content".to_string(),
        };

        let filter = FilterType::After(NaiveDate::from_ymd_opt(2025, 1, 10).unwrap());
        assert!(match_filter(&filter, &result));

        let filter = FilterType::After(NaiveDate::from_ymd_opt(2025, 1, 20).unwrap());
        assert!(!match_filter(&filter, &result));
    }

    #[test]
    fn test_match_filter_before() {
        let result = SearchResult {
            subject: "Test".to_string(),
            from_addr: "sender@example.com".to_string(),
            to_addr: "".to_string(),
            cc_addr: "".to_string(),
            date_str: "2025-01-15 10:00".to_string(),
            file_path: "/path".to_string(),
            content: "Content".to_string(),
        };

        let filter = FilterType::Before(NaiveDate::from_ymd_opt(2025, 1, 20).unwrap());
        assert!(match_filter(&filter, &result));

        let filter = FilterType::Before(NaiveDate::from_ymd_opt(2025, 1, 10).unwrap());
        assert!(!match_filter(&filter, &result));
    }

    #[test]
    fn test_parse_filter_plain_text_searches_all() {
        // Test that plain text (no colon) creates All filters
        let filters = parse_filter("Hello");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            FilterType::All(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected All filter"),
        }

        // Test multiple words create multiple All filters (AND logic)
        let filters = parse_filter("Hello World");
        assert_eq!(filters.len(), 2);
        match &filters[0] {
            FilterType::All(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected All filter"),
        }
        match &filters[1] {
            FilterType::All(text) => assert_eq!(text, "World"),
            _ => panic!("Expected All filter"),
        }

        // Test that invalid filter type returns empty list
        let filters = parse_filter("unknown:filter");
        assert_eq!(filters.len(), 0);
    }

    #[test]
    fn test_apply_filter_with_invalid_input() {
        let results = vec![
            SearchResult {
                subject: "Test Subject".to_string(),
                from_addr: "john@example.com".to_string(),
                to_addr: "".to_string(),
                cc_addr: "".to_string(),
                date_str: "2025-01-15 10:00".to_string(),
                file_path: "/path1".to_string(),
                content: "Content".to_string(),
            },
        ];

        let mut app = App::new(results, "test".to_string(), vec!["test".to_string()]);

        // Apply invalid filter type (unknown:) - should result in empty filtered_indices
        app.filter_input = "unknown:filter".to_string();
        app.apply_filter();

        // Should have empty Some (no matches) instead of None (no filter)
        assert!(app.filtered_indices.is_some());
        assert_eq!(app.filtered_indices.as_ref().unwrap().len(), 0);

        // Apply plain text filter - should search across all fields (no match)
        app.filter_input = "nomatch".to_string();
        app.apply_filter();

        assert!(app.filtered_indices.is_some());
        assert_eq!(app.filtered_indices.as_ref().unwrap().len(), 0);

        // Apply plain text filter - should search across all fields (match)
        app.filter_input = "test".to_string();
        app.apply_filter();

        assert!(app.filtered_indices.is_some());
        assert_eq!(app.filtered_indices.as_ref().unwrap().len(), 1);

        // Apply valid filter - should work normally
        app.filter_input = "from:john".to_string();
        app.apply_filter();

        assert!(app.filtered_indices.is_some());
        assert_eq!(app.filtered_indices.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_match_filter_all() {
        let result = SearchResult {
            subject: "Project Update Meeting".to_string(),
            from_addr: "john@example.com".to_string(),
            to_addr: "".to_string(),
            cc_addr: "".to_string(),
            date_str: "2025-01-15 10:00".to_string(),
            file_path: "/path".to_string(),
            content: "The project is progressing well with updates".to_string(),
        };

        // Match in subject
        let filter = FilterType::All("project".to_string());
        assert!(match_filter(&filter, &result));

        // Match in from
        let filter = FilterType::All("john".to_string());
        assert!(match_filter(&filter, &result));

        // Match in content
        let filter = FilterType::All("progressing".to_string());
        assert!(match_filter(&filter, &result));

        // No match
        let filter = FilterType::All("xyz".to_string());
        assert!(!match_filter(&filter, &result));

        // Case insensitive
        let filter = FilterType::All("PROJECT".to_string());
        assert!(match_filter(&filter, &result));
    }
}
