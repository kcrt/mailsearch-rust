//! Search functionality for finding and processing email files.

use crate::email::{process_emlx_file, Criteria};
use crate::index::{rowid_from_path, Candidates};
use crate::models::Filters;
use crate::models::SearchResult;
use crate::timewindow::Window;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressIterator, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashMap;
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

/// Find all .emlx files in the Mail directory, optionally restricted by mtime
/// and by what Apple Mail's Envelope Index says about each message.
///
/// When `window` is set, files whose mtime predates [`Window::mtime_cutoff`] are
/// dropped without ever being read. See [`crate::timewindow`] for why that is
/// sound. Without a window no file is stat'd at all, so an unrestricted search
/// pays nothing for this.
///
/// When `narrow` is set, files the index has ruled out are dropped before the
/// mtime pass, so a sender filter also saves the stat. The index only ever
/// removes candidates and never changes what a kept file matches, so `None`
/// here — a missing or unreadable index — costs speed and nothing else. See
/// [`crate::index`].
pub fn find_emlx_files(
    mail_root: &Path,
    window: Option<Window>,
    narrow: Option<&Candidates>,
) -> FileScan {
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
    // Counted before narrowing: this is what tells "no mail directory / no Full
    // Disk Access" apart from "the filters matched nothing", and the index must
    // not be able to turn the former diagnostic on.
    let total_seen = candidates.len();

    let candidates: Vec<PathBuf> = match narrow {
        None => candidates,
        Some(narrow) => {
            spinner.set_message(format!(
                "Narrowing {total_seen} .emlx files against {} indexed messages...",
                narrow.known_count()
            ));
            candidates
                .into_iter()
                // A file whose name is not a message row number is kept, the same
                // way an unknown row number is: the index cannot speak about it.
                .filter(|path| rowid_from_path(path).is_none_or(|rowid| narrow.keeps(rowid)))
                .collect()
        }
    };

    let files: Vec<(PathBuf, Option<i64>)> = match window {
        None => candidates.into_iter().map(|path| (path, None)).collect(),
        Some(window) => {
            spinner.set_message(format!(
                "Checking dates of {} .emlx files...",
                candidates.len()
            ));
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

    // Say how much was skipped whenever anything was, so a fast search does not
    // look like a search that quietly missed most of the mailbox.
    let message = if window.is_none() && narrow.is_none() {
        format!("Found {} .emlx files", files.len())
    } else {
        format!(
            "Found {} of {} .emlx files worth reading",
            files.len(),
            total_seen
        )
    };
    spinner.finish_with_message(message);

    FileScan { files, total_seen }
}

/// Collects results while collapsing copies of the same message.
///
/// Apple Mail keeps more than one `.emlx` for the same message: adjacent
/// sequence numbers in one mailbox, plus a stale file left behind when a message
/// is moved (INBOX → Archive). A scan therefore sees the same mail two or three
/// times — measured on real mail, 11 messages with attachments came back as 22
/// rows — and every caller of `--json` had to collapse them again by
/// `message_id`. Doing it here means no caller has to.
///
/// Messages with no `Message-ID` are never merged: without an identity there is
/// nothing to merge them by, and an unsent draft is a legitimate result.
#[derive(Default)]
struct Deduped {
    results: Vec<SearchResult>,
    /// `Message-ID` → index into `results`.
    by_message_id: HashMap<String, usize>,
}

impl Deduped {
    /// Whether `candidate` is a better copy to keep than `current`.
    ///
    /// A fully downloaded file beats a `.partial.emlx`: the `path` is what a
    /// caller feeds to a tool that reads the message, and a complete file can be
    /// read straight from disk without asking Apple Mail for the body. Beyond
    /// that the first copy seen wins, so the order stays stable.
    fn is_better(candidate: &SearchResult, current: &SearchResult) -> bool {
        let partial = |result: &SearchResult| result.file_path.ends_with(".partial.emlx");
        partial(current) && !partial(candidate)
    }

    fn push(&mut self, result: SearchResult) {
        let Some(message_id) = result.message_id.clone() else {
            self.results.push(result);
            return;
        };
        match self.by_message_id.get(&message_id) {
            Some(&index) => {
                if Self::is_better(&result, &self.results[index]) {
                    self.results[index] = result;
                }
            }
            None => {
                self.by_message_id.insert(message_id, self.results.len());
                self.results.push(result);
            }
        }
    }

    /// How many distinct messages have been collected so far.
    fn len(&self) -> usize {
        self.results.len()
    }

    fn into_results(self) -> Vec<SearchResult> {
        self.results
    }
}

/// Matching messages, plus enough context to explain an empty result set.
pub struct SearchOutcome {
    pub results: Vec<SearchResult>,
    /// How many `.emlx` files existed before any date filtering.
    pub total_seen: usize,
}

/// Search for messages matching the query, optionally restricted to a date window.
///
/// `use_index` asks for Apple Mail's Envelope Index to pre-select which files
/// are worth reading. It is a pure optimisation: the same messages come back
/// either way, so a caller that turns it off only makes the search slower.
pub fn search_messages(
    mail_root: &Path,
    groups: &[Vec<String>],
    filters: &Filters,
    limit: usize,
    window: Option<Window>,
    use_index: bool,
) -> SearchOutcome {
    let narrow = use_index
        .then(|| Candidates::load(mail_root, filters))
        .flatten();
    let scan = find_emlx_files(mail_root, window, narrow.as_ref());
    let criteria = Criteria {
        groups,
        date_cutoff: window.map(|w| w.date_cutoff),
        filters,
    };

    let pb = ProgressBar::new(scan.files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} files ({eta})")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut deduped = Deduped::default();
    if limit < usize::MAX {
        // Sequential with early termination when a limit is set. The limit counts
        // distinct messages, not files, so duplicates cannot eat into it.
        for (path, mtime) in scan.files.into_iter().progress_with(pb) {
            if let Some(result) = process_emlx_file(&path, mtime, &criteria) {
                deduped.push(result);
                if deduped.len() >= limit {
                    break;
                }
            }
        }
    } else {
        // Parallel for an unlimited search, then collapse duplicates in walk
        // order. Collecting first keeps the scan itself lock-free.
        let found: Vec<SearchResult> = scan
            .files
            .into_par_iter()
            .progress_with(pb)
            .filter_map(|(path, mtime)| process_emlx_file(&path, mtime, &criteria))
            .collect();
        for result in found {
            deduped.push(result);
        }
    }
    let results = deduped.into_results();

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

    /// Wrap a message as Apple Mail writes it: byte-count line, then the message.
    ///
    /// The count has to be right — [`crate::email`] slices by it to cut off the
    /// trailing plist, so a placeholder here would truncate the fixture instead.
    fn emlx(message: &str) -> String {
        format!("{}\n{message}", message.len())
    }

    /// Create an `.emlx`-shaped file and set its mtime to `epoch` seconds.
    fn write_file(dir: &Path, name: &str, epoch: i64) {
        let path = dir.join(name);
        std::fs::write(&path, emlx("Subject: test\n\nbody\n")).unwrap();
        let file = File::options().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(epoch as u64))
            .unwrap();
    }

    /// Create an `.emlx`-shaped file carrying `message_id`, for dedupe tests.
    fn write_message(dir: &Path, name: &str, message_id: &str) {
        let raw = emlx(&format!("Subject: test\nMessage-ID: <{message_id}>\n\nbody\n"));
        std::fs::write(dir.join(name), raw).unwrap();
    }

    /// Scan for the fixture messages above, with no window and no filters.
    fn scan_all(dir: &Path, limit: usize) -> SearchOutcome {
        let groups = crate::email::parse_query_groups("test", &[]);
        search_messages(dir, &groups, &Filters::default(), limit, None, false)
    }

    #[test]
    fn copies_of_one_message_collapse_to_one_result() {
        // Apple Mail really does keep several .emlx for one message.
        let dir = tempfile::tempdir().unwrap();
        write_message(dir.path(), "1.emlx", "same@example.com");
        write_message(dir.path(), "2.emlx", "same@example.com");

        let outcome = scan_all(dir.path(), usize::MAX);
        assert_eq!(outcome.results.len(), 1);
        // Both files were still scanned; only the results collapsed.
        assert_eq!(outcome.total_seen, 2);
    }

    #[test]
    fn a_complete_copy_is_preferred_over_a_partial_one() {
        let dir = tempfile::tempdir().unwrap();
        write_message(dir.path(), "1.partial.emlx", "same@example.com");
        write_message(dir.path(), "2.emlx", "same@example.com");

        let outcome = scan_all(dir.path(), usize::MAX);
        assert_eq!(outcome.results.len(), 1);
        // Asserted as a property rather than a filename, since the walk order of
        // the two files is not guaranteed.
        assert!(!outcome.results[0].file_path.ends_with(".partial.emlx"));
    }

    #[test]
    fn distinct_messages_are_not_collapsed() {
        let dir = tempfile::tempdir().unwrap();
        write_message(dir.path(), "1.emlx", "a@example.com");
        write_message(dir.path(), "2.emlx", "b@example.com");

        assert_eq!(scan_all(dir.path(), usize::MAX).results.len(), 2);
    }

    #[test]
    fn messages_without_a_message_id_are_kept_apart() {
        // No identity to merge by, and an unsent draft is a real result.
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "1.emlx", 1_000_000);
        write_file(dir.path(), "2.emlx", 2_000_000);

        assert_eq!(scan_all(dir.path(), usize::MAX).results.len(), 2);
    }

    #[test]
    fn a_limit_counts_distinct_messages_not_files() {
        // Two messages, two copies each: --limit 2 must still yield both.
        let dir = tempfile::tempdir().unwrap();
        write_message(dir.path(), "1.emlx", "a@example.com");
        write_message(dir.path(), "2.emlx", "a@example.com");
        write_message(dir.path(), "3.emlx", "b@example.com");
        write_message(dir.path(), "4.emlx", "b@example.com");

        let results = scan_all(dir.path(), 2).results;
        assert_eq!(results.len(), 2);
        let mut ids: Vec<&str> = results
            .iter()
            .map(|r| r.message_id.as_deref().unwrap())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["<a@example.com>", "<b@example.com>"]);
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

        let scan = find_emlx_files(dir.path(), None, None);
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

        let scan = find_emlx_files(dir.path(), Some(window_at(2_000_000)), None);
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

        let scan = find_emlx_files(dir.path(), Some(window_at(9_000_000)), None);
        assert!(scan.files.is_empty());
        // The caller needs this to say "nothing in the window" rather than
        // "no mail directory".
        assert_eq!(scan.total_seen, 1);
    }

    #[test]
    fn an_empty_directory_returns_empty_without_exiting() {
        let dir = tempfile::tempdir().unwrap();
        let scan = find_emlx_files(dir.path(), None, None);
        assert_eq!(scan.total_seen, 0);
        assert!(scan.files.is_empty());
    }

    #[test]
    fn partial_emlx_files_are_included() {
        // Apple Mail writes IMAP stubs as `*.partial.emlx`; the extension check
        // must still see them.
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "1234.partial.emlx", 3_000_000);

        let scan = find_emlx_files(dir.path(), Some(window_at(2_000_000)), None);
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

        let scan = find_emlx_files(dir.path(), Some(window_at(2_000_000)), None);
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

        let scan = find_emlx_files(dir.path(), None, None);
        assert_eq!(scan.total_seen, 1);
    }

    #[test]
    fn search_messages_applies_both_the_query_and_the_window() {
        let dir = tempfile::tempdir().unwrap();
        // Dated 2026-01-20, with an mtime well after it.
        let path = dir.path().join("msg.emlx");
        std::fs::write(
            &path,
            emlx("Subject: Report\nDate: Tue, 20 Jan 2026 10:30:00 +0000\n\nquarterly report\n"),
        )
        .unwrap();
        let file = File::options().write(true).open(&path).unwrap();
        let mtime = 1_780_000_000; // 2026-05-29
        file.set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
            .unwrap();

        let groups = crate::email::parse_query_groups("quarterly report", &[]);

        // No window: found.
        assert_eq!(
            search_messages(dir.path(), &groups, &Filters::default(), usize::MAX, None, false)
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
            search_messages(dir.path(), &groups, &Filters::default(), usize::MAX, Some(covering), false)
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
        let scan = search_messages(dir.path(), &groups, &Filters::default(), usize::MAX, Some(date_only), false);
        assert!(scan.results.is_empty());
        assert_eq!(scan.total_seen, 1);

        // A non-matching query is still a non-match inside the window.
        let other = crate::email::parse_query_groups("unrelated", &[]);
        assert!(
            search_messages(dir.path(), &other, &Filters::default(), usize::MAX, Some(covering), false)
                .results
                .is_empty()
        );
    }
}
