//! Apple Mail Full-Text Search Tool
//!
//! Performs fast full-text search on Apple Mail .emlx files.
//!
//! Usage:
//!     cargo run -- search terms here
//!     cargo run -- "exact phrase"
//!     cargo run -- --mail-root ~/Library/Mail/V10 project

mod commands;
mod config;
mod email;
mod highlight;
mod index;
mod message;
mod models;
mod search;
mod sort;
mod timewindow;
mod tui;

use anyhow::{Context, Result};
use config::{Cli, Command, Config, Parser, TargetArgs};
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

/// One result as a TAB-separated line: date, from, subject, message-id, path.
///
/// No header row, so the output pipes straight into `cut`/`awk`; the column
/// order is documented in the README. A tab inside a value would shift every
/// later column, so values are cleaned rather than quoted — headers cannot
/// legitimately contain one, and a path with a tab is pathological.
fn tsv_line(result: &models::SearchResult) -> String {
    let field = |value: &str| value.replace('\t', " ");
    [
        field(&result.date_str),
        field(&result.from_addr),
        field(&result.subject),
        field(result.message_id.as_deref().unwrap_or("")),
        field(&result.file_path),
    ]
    .join("\t")
}

/// Resolve every target a subcommand was given, reporting failures as they
/// happen rather than abandoning the whole run for one bad path.
///
/// Returns the paths that resolved plus whether anything failed, so the process
/// can still exit non-zero when some of the work did not happen.
fn resolve_targets(args: &TargetArgs) -> (Vec<std::path::PathBuf>, bool) {
    let mail_root = expand_mail_root_path(args.mail_root.clone()).unwrap_or_default();
    let mut paths = Vec::new();
    let mut failed = false;
    for target in &args.targets {
        match commands::resolve_target(target, &mail_root, args.days) {
            Ok(path) => paths.push(path),
            Err(error) => {
                eprintln!("{error}");
                failed = true;
            }
        }
    }
    (paths, failed)
}

/// Run a subcommand over its targets, continuing past a failure on any one.
///
/// Exits non-zero if anything failed, so a caller in a pipeline still hears
/// about it even though the remaining targets were processed.
fn run_command(command: &Command) -> Result<()> {
    let failed = match command {
        Command::Dump(args) => {
            let (paths, failed) = resolve_targets(&args.target);
            paths.iter().fold(failed, |failed, path| {
                match commands::dump(path, args.headers_only, args.strip_quote, args.html) {
                    Ok(()) => failed,
                    Err(error) => {
                        eprintln!("{error:#}");
                        true
                    }
                }
            })
        }
        Command::Attachments(args) => {
            // Checked before any target is resolved: a missing directory should
            // not be discovered after a Message-ID lookup has scanned the mailbox.
            if let Some(directory) = &args.save {
                if !directory.is_dir() {
                    anyhow::bail!("no such directory: {}", directory.display());
                }
            }
            let (paths, mut failed) = resolve_targets(&args.target);
            let mut total = 0;
            for path in &paths {
                match commands::attachments(path, args.save.as_deref(), args.include_inline) {
                    Ok(count) => total += count,
                    Err(error) => {
                        eprintln!("{error:#}");
                        failed = true;
                    }
                }
            }
            // Only for a multi-message run, where the per-message lines scroll
            // away and the total is the part worth reading.
            if args.target.targets.len() > 1 {
                let verb = if args.save.is_some() { "saved" } else { "listed" };
                println!("\n{verb}: {total}");
            }
            failed
        }
    };
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = &cli.command {
        return run_command(command);
    }
    let mut config: Config = cli.search;

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
    let query = config.query_string();
    let groups = email::parse_query_groups(&query, &config.or_terms);
    // Flattened, pre-lowercased term list for highlighting any matched term.
    let highlight_terms: Vec<String> = groups.iter().flatten().cloned().collect();
    // Header/structure filters (--from, --has-attachment), AND-ed with the query.
    let filters = config.filters();
    // Human-readable query used for status messages and the TUI header.
    let display_query = config.display_query();

    // Restrict the scan to a date window when asked. This is both a filter and the
    // main speedup: most files can be skipped without being read.
    let days = config.days_window();
    let window = days.map(timewindow::window_now);

    // Status messages would corrupt stdout in JSON/TSV mode; suppress them there.
    if !config.machine_output() {
        println!("Searching Mail files...");
        println!("   Directory: {}", config.mail_root.display());
        println!("   Query: {}", display_query);
        if let (Some(days), Some(window)) = (days, window) {
            // Dates are displayed in UTC throughout, so name the zone here: this
            // is the one line a reader would compare against their wall clock.
            println!(
                "   Period: last {} day{} (since {} UTC)",
                days,
                if days == 1 { "" } else { "s" },
                email::format_timestamp(window.date_cutoff)
                    .unwrap_or_else(|| "unknown".to_string())
            );
        }
        println!();
    }

    // Only early-terminate during the scan when no sort is requested; otherwise we must
    // see every match before we can sort and take the top-N. A date window already cuts
    // the candidate set down to very little, so keep the parallel path in that case
    // rather than dropping to the sequential scan for an arbitrary top-N.
    let scan_limit = if config.sort == SortMode::NoSort && window.is_none() {
        config.limit
    } else {
        usize::MAX
    };
    let outcome = search_messages(
        &config.mail_root,
        &groups,
        &filters,
        scan_limit,
        window,
        !config.no_index,
    );
    if outcome.total_seen == 0 {
        eprintln!("\nError: No .emlx files found in the Mail directory.");
        eprintln!("\nPlease ensure that the Mail directory is correct and accessible from this tool.");
        eprintln!("\n    Directory: {}", config.mail_root.display());
        std::process::exit(1);
    }

    let mut results = outcome.results;
    sort_results(&mut results, config.sort);
    if results.len() > config.limit {
        results.truncate(config.limit);
    }

    if config.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if config.tsv {
        for result in &results {
            println!("{}", tsv_line(result));
        }
    } else if results.is_empty() {
        // Name the window explicitly, so an empty result set doesn't read as
        // "the query matched nothing" when it was the date restriction.
        match days {
            Some(days) => println!(
                "\nNo messages from the last {} day{} matching: {}",
                days,
                if days == 1 { "" } else { "s" },
                display_query
            ),
            None => println!("\nNo messages found matching: {}", display_query),
        }
    } else {
        // Run TUI
        run_tui(results, display_query, highlight_terms, config.sort)?;
    }

    Ok(())
}
