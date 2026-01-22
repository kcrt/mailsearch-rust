//! Search functionality for finding and processing email files.

use crate::email::process_emlx_file;
use crate::models::SearchResult;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressIterator, ProgressStyle};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Find all .emlx files in the Mail directory.
pub fn find_emlx_files(mail_root: &Path) -> Vec<PathBuf> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message("Searching for .emlx files...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let files: Vec<PathBuf> = WalkDir::new(mail_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "emlx"))
        .map(|entry| entry.path().to_path_buf())
        .collect();

    spinner.finish_with_message(format!("Found {} .emlx files", files.len()));
    files
}

/// Search for messages matching the query.
pub fn search_messages(mail_root: &Path, query: &str, limit: usize) -> Vec<SearchResult> {
    let files = find_emlx_files(mail_root);
    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    if limit < usize::MAX {
        // Use sequential iteration with early termination when limit is set
        files
            .into_iter()
            .progress_with(pb)
            .filter_map(|emlx_file| process_emlx_file(&emlx_file, query))
            .take(limit)
            .collect()
    } else {
        // Use parallel iteration for unlimited search
        files
            .into_par_iter()
            .progress_with(pb)
            .filter_map(|emlx_file| process_emlx_file(&emlx_file, query))
            .collect()
    }
}
