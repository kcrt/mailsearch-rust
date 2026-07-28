//! Search functionality for finding and processing email files.

use crate::email::{process_emlx_file, Criteria};
use crate::models::SearchResult;
use crate::timewindow::Window;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressIterator, ProgressStyle};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

/// Outcome of scanning the Mail directory for candidate files.
pub struct FileScan {
    /// Candidate files paired with their mtime in epoch seconds. The mtime is
    /// `None` when the file was not stat'd, or when its metadata was unreadable.
    pub files: Vec<(PathBuf, Option<i64>)>,
    /// How many `.emlx` files the walk saw, before any date filtering. Lets the
    /// caller distinguish "no mail at all" from "no mail in the requested window".
    pub total_seen: usize,
}

/// Convert a file timestamp to epoch seconds.
fn to_epoch(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
}

/// Find all .emlx files in the Mail directory, optionally restricted by mtime.
///
/// When `window` is set, files whose mtime predates [`Window::mtime_cutoff`] are
/// dropped without ever being read. See [`crate::timewindow`] for why that is
/// sound. Without a window no file is stat'd at all, so an unrestricted search
/// pays nothing for this.
pub fn find_emlx_files(mail_root: &Path, window: Option<Window>) -> FileScan {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message("Searching for .emlx files...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    // The walk itself makes no stat calls: `file_type()` and the extension check
    // both come from the directory read. Excluding directories keeps them out of
    // the mtime stat below; note this must not be `is_file()`, which would also
    // reject symlinks, since `follow_links` is off and a symlinked message should
    // still be searched.
    let candidates: Vec<PathBuf> = WalkDir::new(mail_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| !entry.file_type().is_dir())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "emlx"))
        .map(|entry| entry.path().to_path_buf())
        .collect();
    let total_seen = candidates.len();

    let files: Vec<(PathBuf, Option<i64>)> = match window {
        None => candidates.into_iter().map(|path| (path, None)).collect(),
        Some(window) => {
            spinner.set_message(format!("Checking dates of {total_seen} .emlx files..."));
            // One stat per file, so do them in parallel. `par_iter` on a `Vec`
            // preserves order, keeping the walk order (and thus `--limit`)
            // deterministic.
            candidates
                .into_par_iter()
                .filter_map(|path| {
                    // `path.metadata()` follows symlinks, unlike walkdir's
                    // `DirEntry::metadata()`, which would report the link's own
                    // mtime and could wrongly exclude a linked message. Both cost
                    // the same single stat.
                    match path.metadata().and_then(|m| m.modified()).map(to_epoch) {
                        Ok(Some(mtime)) => {
                            (mtime >= window.mtime_cutoff).then_some((path, Some(mtime)))
                        }
                        // Metadata unreadable: keep the file and let the Date
                        // header decide rather than dropping mail silently.
                        Ok(None) | Err(_) => Some((path, None)),
                    }
                })
                .collect()
        }
    };

    let message = match window {
        None => format!("Found {} .emlx files", files.len()),
        Some(_) => format!(
            "Found {} of {} .emlx files recent enough to check",
            files.len(),
            total_seen
        ),
    };
    spinner.finish_with_message(message);

    FileScan { files, total_seen }
}

/// Matching messages, plus enough context to explain an empty result set.
pub struct SearchOutcome {
    pub results: Vec<SearchResult>,
    /// How many `.emlx` files existed before any date filtering.
    pub total_seen: usize,
}

/// Search for messages matching the query, optionally restricted to a date window.
pub fn search_messages(
    mail_root: &Path,
    groups: &[Vec<String>],
    limit: usize,
    window: Option<Window>,
) -> SearchOutcome {
    let scan = find_emlx_files(mail_root, window);
    let criteria = Criteria {
        groups,
        date_cutoff: window.map(|w| w.date_cutoff),
    };

    let pb = ProgressBar::new(scan.files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    let results = if limit < usize::MAX {
        // Use sequential iteration with early termination when limit is set
        scan.files
            .into_iter()
            .progress_with(pb)
            .filter_map(|(path, mtime)| process_emlx_file(&path, mtime, &criteria))
            .take(limit)
            .collect()
    } else {
        // Use parallel iteration for unlimited search
        scan.files
            .into_par_iter()
            .progress_with(pb)
            .filter_map(|(path, mtime)| process_emlx_file(&path, mtime, &criteria))
            .collect()
    };

    SearchOutcome {
        results,
        total_seen: scan.total_seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::Duration;

    /// Create an `.emlx`-shaped file and set its mtime to `epoch` seconds.
    fn write_file(dir: &Path, name: &str, epoch: i64) {
        let path = dir.join(name);
        std::fs::write(&path, "42\nSubject: test\n\nbody\n").unwrap();
        let file = File::options().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(epoch as u64))
            .unwrap();
    }

    fn window_at(cutoff: i64) -> Window {
        Window {
            date_cutoff: cutoff,
            mtime_cutoff: cutoff,
        }
    }

    #[test]
    fn without_a_window_all_emlx_files_are_returned_unstatted() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "a.emlx", 1_000_000);
        write_file(dir.path(), "b.emlx", 2_000_000);
        write_file(dir.path(), "c.emlx", 3_000_000);
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let scan = find_emlx_files(dir.path(), None);
        assert_eq!(scan.total_seen, 3);
        assert_eq!(scan.files.len(), 3);
        // No window means no stat, so no mtime is reported.
        assert!(scan.files.iter().all(|(_, mtime)| mtime.is_none()));
    }

    #[test]
    fn a_window_drops_files_older_than_the_mtime_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "old_one.emlx", 1_000_000);
        write_file(dir.path(), "old_two.emlx", 1_500_000);
        write_file(dir.path(), "recent.emlx", 3_000_000);

        let scan = find_emlx_files(dir.path(), Some(window_at(2_000_000)));
        // total_seen still reflects everything the walk found.
        assert_eq!(scan.total_seen, 3);
        assert_eq!(scan.files.len(), 1);
        let (path, mtime) = &scan.files[0];
        assert_eq!(path.file_name().unwrap(), "recent.emlx");
        assert_eq!(*mtime, Some(3_000_000));
    }

    #[test]
    fn a_window_matching_nothing_reports_what_it_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "old.emlx", 1_000_000);

        let scan = find_emlx_files(dir.path(), Some(window_at(9_000_000)));
        assert!(scan.files.is_empty());
        // The caller needs this to say "nothing in the window" rather than
        // "no mail directory".
        assert_eq!(scan.total_seen, 1);
    }

    #[test]
    fn an_empty_directory_returns_empty_without_exiting() {
        let dir = tempfile::tempdir().unwrap();
        let scan = find_emlx_files(dir.path(), None);
        assert_eq!(scan.total_seen, 0);
        assert!(scan.files.is_empty());
    }

    #[test]
    fn partial_emlx_files_are_included() {
        // Apple Mail writes IMAP stubs as `*.partial.emlx`; the extension check
        // must still see them.
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "1234.partial.emlx", 3_000_000);

        let scan = find_emlx_files(dir.path(), Some(window_at(2_000_000)));
        assert_eq!(scan.files.len(), 1);
    }

    #[test]
    fn symlinked_messages_are_followed_not_dropped() {
        // `follow_links` is off, so the walk sees the link itself. It must still be
        // a candidate, and its mtime must come from the target rather than the link.
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("target");
        std::fs::create_dir(&target_dir).unwrap();
        write_file(&target_dir, "real.emlx", 3_000_000);

        let link = dir.path().join("linked.emlx");
        std::os::unix::fs::symlink(target_dir.join("real.emlx"), &link).unwrap();

        let scan = find_emlx_files(dir.path(), Some(window_at(2_000_000)));
        // Both the original and the link.
        assert_eq!(scan.files.len(), 2);
        let linked = scan
            .files
            .iter()
            .find(|(path, _)| path.file_name().unwrap() == "linked.emlx")
            .expect("symlinked .emlx should be a candidate");
        assert_eq!(linked.1, Some(3_000_000), "mtime should come from the target");
    }

    #[test]
    fn nested_directories_are_walked() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("INBOX.mbox").join("Data").join("Messages");
        std::fs::create_dir_all(&nested).unwrap();
        write_file(&nested, "deep.emlx", 3_000_000);

        let scan = find_emlx_files(dir.path(), None);
        assert_eq!(scan.total_seen, 1);
    }

    #[test]
    fn search_messages_applies_both_the_query_and_the_window() {
        let dir = tempfile::tempdir().unwrap();
        // Dated 2026-01-20, with an mtime well after it.
        let path = dir.path().join("msg.emlx");
        std::fs::write(
            &path,
            "42\nSubject: Report\nDate: Tue, 20 Jan 2026 10:30:00 +0000\n\nquarterly report\n",
        )
        .unwrap();
        let file = File::options().write(true).open(&path).unwrap();
        let mtime = 1_780_000_000; // 2026-05-29
        file.set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
            .unwrap();

        let groups = crate::email::parse_query_groups("quarterly report", &[]);

        // No window: found.
        assert_eq!(
            search_messages(dir.path(), &groups, usize::MAX, None)
                .results
                .len(),
            1
        );

        // Window covering the message date: found.
        let covering = Window {
            date_cutoff: 1_700_000_000, // 2023-11
            mtime_cutoff: 1_700_000_000,
        };
        assert_eq!(
            search_messages(dir.path(), &groups, usize::MAX, Some(covering))
                .results
                .len(),
            1
        );

        // The mtime survives the prefilter but the Date header is older than the
        // cutoff: this is the case mtime alone would get wrong.
        let date_only = Window {
            date_cutoff: 1_775_000_000, // 2026-03-31
            mtime_cutoff: 1_700_000_000,
        };
        let scan = search_messages(dir.path(), &groups, usize::MAX, Some(date_only));
        assert!(scan.results.is_empty());
        assert_eq!(scan.total_seen, 1);

        // A non-matching query is still a non-match inside the window.
        let other = crate::email::parse_query_groups("unrelated", &[]);
        assert!(
            search_messages(dir.path(), &other, usize::MAX, Some(covering))
                .results
                .is_empty()
        );
    }
}
