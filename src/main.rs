//! Apple Mail Full-Text Search Tool
//!
//! Performs fast full-text search on Apple Mail .emlx files.
//!
//! Usage:
//!     cargo run -- search terms here
//!     cargo run -- "exact phrase"
//!     cargo run -- --mail-root ~/Library/Mail/V10 project

mod config;
mod email;
mod highlight;
mod models;
mod search;
mod sort;
mod tui;

use anyhow::{Context, Result};
use config::{Config, Parser};
use search::search_messages;
use sort::{sort_results, SortMode};
use std::env;
use std::path::PathBuf;
use tui::run_tui;

/// Expand tilde and resolve relative paths from home directory.
fn expand_mail_root_path(mail_root: PathBuf) -> Result<PathBuf> {
    // Expand tilde in path
    let mut expanded = if mail_root.starts_with("~") {
        let home = env::var("HOME").context("Could not determine HOME environment variable")?;
        let rest = mail_root
            .strip_prefix("~")
            .unwrap_or(mail_root.as_path());
        PathBuf::from(home).join(rest)
    } else {
        mail_root
    };

    // Handle relative path from home directory
    if !expanded.is_absolute() {
        expanded = dirs::home_dir()
            .context("Could not determine home directory")?
            .join(&expanded);
    }

    Ok(expanded)
}

fn main() -> Result<()> {
    let mut config = Config::parse();

    // Expand tilde and resolve relative paths
    config.mail_root = expand_mail_root_path(config.mail_root)?;

    if !config.mail_root.exists() {
        eprintln!("Error: Mail directory not found:");
        eprintln!("   {}", config.mail_root.display());
        eprintln!("\nTo fix this:");
        eprintln!("   1. Open System Settings → Privacy & Security → Full Disk Access");
        eprintln!("   2. Add Terminal or your IDE to the allowed applications");
        eprintln!("   3. Restart Terminal/IDE and try again");
        std::process::exit(1);
    }

    // Build OR-groups (outer = OR, inner = AND terms) once, then reuse across the scan.
    let groups = email::parse_query_groups(&config.query, &config.or_terms);
    // Flattened, pre-lowercased term list for highlighting any matched term.
    let highlight_terms: Vec<String> = groups.iter().flatten().cloned().collect();
    // Human-readable query used for status messages and the TUI header.
    let display_query = if config.or_terms.is_empty() {
        config.query.clone()
    } else {
        format!("{} OR {}", config.query, config.or_terms.join(" OR "))
    };

    // Status messages would corrupt stdout in JSON mode; suppress them there.
    if !config.json {
        println!("Searching Mail files...");
        println!("   Directory: {}", config.mail_root.display());
        println!("   Query: {}\n", display_query);
    }

    // Only early-terminate during the scan when no sort is requested; otherwise we must
    // see every match before we can sort and take the top-N.
    let scan_limit = if config.sort == SortMode::NoSort {
        config.limit
    } else {
        usize::MAX
    };
    let mut results = search_messages(&config.mail_root, &groups, scan_limit);
    sort_results(&mut results, config.sort);
    if results.len() > config.limit {
        results.truncate(config.limit);
    }

    if config.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if results.is_empty() {
        println!("\nNo messages found matching: {}", display_query);
    } else {
        // Run TUI
        run_tui(results, display_query, highlight_terms, config.sort)?;
    }

    Ok(())
}
