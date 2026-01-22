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
mod tui;

use anyhow::{Context, Result};
use config::{Config, Parser};
use search::search_messages;
use std::env;
use std::path::PathBuf;
use tui::run_tui;

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
