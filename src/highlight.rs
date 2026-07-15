//! Text highlighting utilities for search results.

use crate::models::TextPart;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Tokenize text into matched and non-matched parts for highlighting.
/// Returns a vector of lines, where each line is a vector of TextParts.
pub fn tokenize_with_highlighting<'a>(
    text: &'a str,
    terms: &[String],
) -> Vec<Vec<TextPart<'a>>> {
    if terms.is_empty() {
        return text
            .lines()
            .map(|line| vec![TextPart::Normal(line)])
            .collect();
    }

    text.lines().map(|line| {
        let mut parts = Vec::new();
        let line_lower = line.to_ascii_lowercase();
        let mut last_end = 0;

        while last_end < line.len() {
            let earliest_match = terms
                .iter()
                .filter_map(|term| {
                    line_lower[last_end..]
                        .find(term)
                        .map(|pos| (last_end + pos, term.len()))
                })
                .min_by_key(|(pos, _)| *pos);

            match earliest_match {
                Some((start, len)) => {
                    if start > last_end {
                        parts.push(TextPart::Normal(&line[last_end..start]));
                    }
                    parts.push(TextPart::Matched(&line[start..start + len]));
                    last_end = start + len;
                }
                None => {
                    if last_end < line.len() {
                        parts.push(TextPart::Normal(&line[last_end..]));
                    }
                    break;
                }
            }
        }
        parts
    }).collect()
}

/// Convert TextParts into ratatui Spans with appropriate styling.
pub fn parts_to_spans<'a>(parts: &[TextPart<'a>]) -> Vec<Span<'a>> {
    parts.iter().map(|part| match part {
        TextPart::Matched(s) => Span::styled(
            *s,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        TextPart::Normal(s) => Span::raw(*s),
    }).collect()
}

/// Get highlighted text with search terms highlighted.
/// Highlights exact matching substrings, works for all languages including Japanese.
pub fn get_highlighted_text<'a>(text: &'a str, terms: &[String]) -> Vec<Line<'a>> {
    let tokenized = tokenize_with_highlighting(text, terms);

    tokenized
        .into_iter()
        .map(|parts| Line::from(parts_to_spans(&parts)))
        .collect()
}
