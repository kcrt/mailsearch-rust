//! Search functionality for finding and processing email files.

use crate::models::SearchResult;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressIterator, ProgressStyle};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Find all .emlx files in the Mail directory.
pub fn find_emlx_files(mail_root: &Path) -> Vec<PathBuf> {
    WalkDir::new(mail_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| entry.path().extension().map_or(false, |ext| ext == "emlx"))
        .map(|entry| entry.path().to_path_buf())
        .collect()
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

    // Pre-process query terms once for all file processing
    let lowercase_terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_ascii_lowercase())
        .collect();

    if limit < usize::MAX {
        // Use sequential iteration with early termination when limit is set
        files
            .into_iter()
            .progress_with(pb)
            .filter_map(|emlx_file| {
                crate::email::process_emlx_file_with_terms(&emlx_file, &lowercase_terms)
            })
            .take(limit)
            .collect()
    } else {
        // Use parallel iteration for unlimited search
        files
            .into_par_iter()
            .progress_with(pb)
            .filter_map(|emlx_file| {
                crate::email::process_emlx_file_with_terms(&emlx_file, &lowercase_terms)
            })
            .collect()
    }
}
