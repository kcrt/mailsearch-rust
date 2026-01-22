//! Terminal User Interface for displaying search results.

use crate::highlight::get_highlighted_text;
use crate::models::SearchResult;
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
    After(NaiveDate),
    Before(NaiveDate),
}

/// TUI Application state.
pub struct App {
    pub results: Vec<SearchResult>,
    pub query: String,
    pub selected: usize,
    pub content_scroll: usize,
    pub should_quit: bool,
    pub filter_input: String,
    pub filter_mode: bool,
    pub filtered_indices: Option<Vec<usize>>,
}

impl App {
    pub fn new(results: Vec<SearchResult>, query: String) -> Self {
        Self {
            results,
            query,
            selected: 0,
            content_scroll: 0,
            should_quit: false,
            filter_input: String::new(),
            filter_mode: false,
            filtered_indices: None,
        }
    }

    pub fn selected_result(&self) -> Option<&SearchResult> {
        if let Some(ref indices) = self.filtered_indices {
            indices.get(self.selected).and_then(|&i| self.results.get(i))
        } else {
            self.results.get(self.selected)
        }
    }

    pub fn visible_results_count(&self) -> usize {
        self.filtered_indices.as_ref().map_or(self.results.len(), |v| v.len())
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
        self.selected = 0;
        self.content_scroll = 0;
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
            let mut matching_indices = Vec::new();

            for (i, result) in self.results.iter().enumerate() {
                if filters.iter().all(|filter| match_filter(filter, result)) {
                    matching_indices.push(i);
                }
            }

            self.filtered_indices = Some(matching_indices);
        }
        self.selected = 0;
        self.content_scroll = 0;
        self.filter_mode = false;
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

/// Parse a single filter token like "from:john" or "after:2025-01-01".
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
        FilterType::After(date) => {
            // Parse date from result.date_str (format: "%Y-%m-%d %H:%M")
            result.date_str
                .split_whitespace()
                .next()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .map(|result_date| result_date >= *date)
                .unwrap_or(false)
        }
        FilterType::Before(date) => {
            result.date_str
                .split_whitespace()
                .next()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
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

    // Get visible results based on filter
    let visible_results: Vec<(usize, &SearchResult)> = if let Some(ref indices) = app.filtered_indices {
        indices.iter().map(|&i| (i, &app.results[i])).collect()
    } else {
        app.results.iter().enumerate().collect()
    };

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
            ListItem::new(format!("{} [{}] {}", result.from_addr, result.date_str, result.subject)).style(style)
        })
        .collect();

    let title = if let Some(ref indices) = app.filtered_indices {
        format!(" Results for: {} ({} filtered: {}) ", app.query, app.results.len(), indices.len())
    } else {
        format!(" Results for: {} ({}) ", app.query, app.results.len())
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
        result.content.clone()
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

/// Run QuickLook on a file (macOS only).
fn ql_command(path: &str) -> Result<()> {
    std::process::Command::new("qlmanage")
        .args(["-p", path])
        .status()
        .context("Failed to execute qlmanage")?;
    Ok(())
}

/// Run the TUI application.
pub fn run_tui(results: Vec<SearchResult>, query: String) -> Result<()> {
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
                    if app.filter_mode {
                        // Filter input mode
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
                    } else {
                        // Normal mode
                        match key.code {
                            KeyCode::Char('q') => app.quit(),
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
