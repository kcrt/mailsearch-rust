//! Terminal User Interface for displaying search results.

use crate::highlight::get_highlighted_text;
use crate::models::SearchResult;
use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind},
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

/// TUI Application state.
pub struct App {
    pub results: Vec<SearchResult>,
    pub query: String,
    pub selected: usize,
    pub content_scroll: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(results: Vec<SearchResult>, query: String) -> Self {
        Self {
            results,
            query,
            selected: 0,
            content_scroll: 0,
            should_quit: false,
        }
    }

    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }

    pub fn next(&mut self) {
        if self.selected < self.results.len().saturating_sub(1) {
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
}

/// Draw the TUI interface.
fn draw_ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // Split into top (results list) and bottom (content preview + help)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
        .split(size);

    // Split the bottom section into content and help areas
    let help_height = 3;
    let bottom_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(help_height),
        ].as_ref())
        .split(chunks[1]);

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
            ListItem::new(format!("{} [{}] {}", result.from_addr, result.date_str, result.subject)).style(style)
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
        // Format metadata header
        let metadata = format!(
            "From: {}\nSubject: {}\n---\n{}",
            result.from_addr,
            result.subject,
            result.content
        );
        metadata
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
