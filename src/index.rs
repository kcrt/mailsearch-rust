//! Apple Mail's Envelope Index, used to narrow the candidate set before any
//! message file is read.
//!
//! Mail keeps a SQLite database at `MailData/Envelope Index` holding one row per
//! message: sender, recipients, subject, dates, mailbox, and attachment names.
//! Reading it is orders of magnitude cheaper than opening a quarter of a million
//! `.emlx` files, so a search that only needs to know *who wrote* a message can
//! decide which files are worth parsing before touching any of them. Measured on
//! a real mailbox, `--from yamada --has-attachment` over 30 days dropped from
//! 4.85s to under a second — the sender does the narrowing; see
//! [`matching_clause`] for why the attachment filter cannot help.
//!
//! **The index can only ever remove candidates.** Every file it selects is still
//! parsed and matched from the `.emlx` itself, exactly as it would be without
//! the index, so a search returns the same results either way. Two consequences
//! follow, and both matter more than the speed:
//!
//! 1. Falling back to a full scan is always safe. If the database is missing,
//!    unreadable (no Full Disk Access), locked, or shaped differently than a
//!    future macOS writes it, [`Candidates::load`] returns `None` and the caller
//!    walks everything as before.
//! 2. A message the index does not know about is **kept**, never dropped. This
//!    is not hypothetical: a real mailbox held 233,847 `.emlx` files against
//!    231,469 indexed messages, and one of those 2,552 orphans was a legitimate
//!    search hit. See [`Candidates::keeps`].
//!
//! The link between a row and a file is the message's `ROWID`, which Apple Mail
//! also uses as the `.emlx` filename (`963826.emlx`, or `963826.partial.emlx`
//! when the body has not been downloaded). Matching on that number avoids having
//! to reconstruct Mail's directory layout, which is undocumented and has changed
//! between releases.

use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::Path;

use crate::models::Filters;

/// Location of the index within the Mail root.
const INDEX_RELATIVE_PATH: &str = "MailData/Envelope Index";

/// Character used to escape SQL `LIKE` wildcards in user-supplied patterns.
const LIKE_ESCAPE: char = '\\';

/// The set of messages worth reading, as decided by the Envelope Index.
pub struct Candidates {
    /// Every `ROWID` the index holds. Needed to tell "the index says no" apart
    /// from "the index has never heard of this file"; see [`Candidates::keeps`].
    known: HashSet<i64>,
    /// The `ROWID`s satisfying the filter.
    matching: HashSet<i64>,
}

impl Candidates {
    /// Whether the file with this `ROWID` should still be read.
    ///
    /// Unknown files are kept. Mail leaves `.emlx` files behind that the index
    /// no longer references — stale copies from a move, and messages pruned from
    /// the database — and one of them being a real hit is a silent wrong answer,
    /// which is far worse than reading a couple of thousand extra files. The
    /// orphans put a floor under how much the index can narrow, and that floor
    /// is the price of never losing a message.
    pub fn keeps(&self, rowid: i64) -> bool {
        self.matching.contains(&rowid) || !self.known.contains(&rowid)
    }

    /// How many messages the index holds, for the caller's progress reporting.
    pub fn known_count(&self) -> usize {
        self.known.len()
    }

    /// Read the index and select the messages matching `filters`.
    ///
    /// Returns `None` whenever the index cannot answer, which the caller must
    /// treat as "scan everything" rather than "nothing matched".
    ///
    /// Only filters that the index can evaluate *without changing the answer*
    /// are pushed down here. The date window deliberately is not: the scan
    /// compares the `Date:` header against the cutoff and pre-filters on file
    /// mtime, and `date_received` agrees with neither, so pushing it down would
    /// change which messages a window includes rather than merely narrowing the
    /// files read. The two mechanisms compose instead — the index picks the
    /// senders, the existing mtime pre-filter picks the window.
    pub fn load(mail_root: &Path, filters: &Filters) -> Option<Self> {
        if !narrows(filters) {
            return None;
        }
        let path = mail_root.join(INDEX_RELATIVE_PATH);
        // Read-only, and without `immutable`: Mail commits through a WAL, and an
        // immutable open would silently ignore it and read a stale snapshot that
        // omits mail received minutes ago.
        let connection = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .ok()?;

        let known = collect_ids(&connection, "SELECT ROWID FROM messages", &[])?;
        // An empty index is indistinguishable from one we failed to understand,
        // and narrowing to nothing would report "no results" for every search.
        if known.is_empty() {
            return None;
        }

        let (clause, params) = matching_clause(filters);
        let sql = format!("SELECT ROWID FROM messages WHERE {clause}");
        let matching = collect_ids(&connection, &sql, &params)?;

        Some(Self { known, matching })
    }
}

/// Whether these filters give the index anything to narrow by.
///
/// Without one, every message matches and the pass would cost a query and a
/// quarter of a million-entry set to remove nothing.
fn narrows(filters: &Filters) -> bool {
    !filters.from_patterns.is_empty()
}

/// Run a `ROWID`-returning query, or give up on the index entirely.
///
/// Any error — a renamed table, a changed column, a locked database — means the
/// index cannot be trusted for this search, and the caller falls back to a full
/// scan. Nothing here is worth failing the search over.
fn collect_ids(connection: &Connection, sql: &str, params: &[String]) -> Option<HashSet<i64>> {
    let mut statement = connection.prepare(sql).ok()?;
    let bound = rusqlite::params_from_iter(params.iter());
    let rows = statement
        .query_map(bound, |row| row.get::<_, i64>(0))
        .ok()?;
    rows.collect::<Result<HashSet<i64>, _>>().ok()
}

/// The `WHERE` clause selecting messages that pass `filters`, plus its parameters.
///
/// Every clause here is deliberately at least as permissive as the real check
/// the scan performs afterwards, because a false positive only costs one file
/// read while a false negative loses a message:
///
/// - `--from` matches the address *or* the display name, since the scan matches
///   the decoded header containing both. `LIKE` is also case-insensitive for
///   ASCII, which the scan's lowercasing already assumes.
///
/// `--has-attachment` is deliberately **not** among them, however tempting the
/// `attachments` table looks. Mail's attachment index is incomplete: on a real
/// mailbox, 32 messages carrying ordinary `.docx`/`.xlsx`/`.pptx` attachments —
/// fully downloaded, not stubs — had no `attachments` row at all. Requiring one
/// silently lost every one of them. The table is a record of what Mail happened
/// to index, not of which messages have attachments, so only the scan's own MIME
/// walk can answer that.
fn matching_clause(filters: &Filters) -> (String, Vec<String>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();

    if !filters.from_patterns.is_empty() {
        let mut senders: Vec<String> = Vec::new();
        for pattern in &filters.from_patterns {
            params.push(format!("%{}%", escape_like(pattern)));
            let placeholder = format!("?{}", params.len());
            senders.push(format!(
                "a.address LIKE {placeholder} ESCAPE '{LIKE_ESCAPE}' \
                 OR a.comment LIKE {placeholder} ESCAPE '{LIKE_ESCAPE}'"
            ));
        }
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM addresses a WHERE a.ROWID = messages.sender AND ({}))",
            senders.join(" OR ")
        ));
    }

    (clauses.join(" AND "), params)
}

/// Escape `LIKE` wildcards so a pattern is matched literally.
///
/// Sender patterns are plain substrings to the user; an address containing `_`
/// is ordinary and must not silently become "any character".
fn escape_like(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    for ch in pattern.chars() {
        if matches!(ch, '%' | '_' | LIKE_ESCAPE) {
            out.push(LIKE_ESCAPE);
        }
        out.push(ch);
    }
    out
}

/// The message `ROWID` encoded in an `.emlx` filename, if it has one.
///
/// Mail names message files after their row: `963826.emlx`, and
/// `963826.partial.emlx` while the body is still on the server. Anything else is
/// not a message this index can speak about.
pub fn rowid_from_path(path: &Path) -> Option<i64> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".emlx")?;
    // `.partial` is the only infix Mail uses, and stripping it is what lets a
    // not-yet-downloaded message be narrowed like any other.
    let digits = stem.strip_suffix(".partial").unwrap_or(stem);
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn filters(from: &[&str], attachment: bool) -> Filters {
        Filters {
            from_patterns: from.iter().map(|s| s.to_string()).collect(),
            require_attachment: attachment,
            ..Filters::default()
        }
    }

    /// Build an index-shaped database holding the given messages.
    ///
    /// `messages` is (rowid, address, display name, attachment names).
    fn fixture(messages: &[(i64, &str, &str, &[&str])]) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE addresses (ROWID INTEGER PRIMARY KEY, address TEXT, comment TEXT);
                 CREATE TABLE messages (ROWID INTEGER PRIMARY KEY, sender INTEGER);
                 CREATE TABLE attachments (ROWID INTEGER PRIMARY KEY, message INTEGER, name TEXT);",
            )
            .unwrap();
        for (index, (rowid, address, comment, attachments)) in messages.iter().enumerate() {
            let sender = index as i64 + 1;
            connection
                .execute(
                    "INSERT INTO addresses (ROWID, address, comment) VALUES (?1, ?2, ?3)",
                    (sender, address, comment),
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO messages (ROWID, sender) VALUES (?1, ?2)",
                    (rowid, sender),
                )
                .unwrap();
            for name in *attachments {
                connection
                    .execute(
                        "INSERT INTO attachments (message, name) VALUES (?1, ?2)",
                        (rowid, name),
                    )
                    .unwrap();
            }
        }
        connection
    }

    /// Select against a fixture the way [`Candidates::load`] does.
    fn select(connection: &Connection, filters: &Filters) -> Candidates {
        let known = collect_ids(connection, "SELECT ROWID FROM messages", &[]).unwrap();
        let (clause, params) = matching_clause(filters);
        let sql = format!("SELECT ROWID FROM messages WHERE {clause}");
        let matching = collect_ids(connection, &sql, &params).unwrap();
        Candidates { known, matching }
    }

    #[test]
    fn a_sender_pattern_matches_the_address() {
        let db = fixture(&[(1, "yamada.hanako@example.jp", "Yamada", &[])]);
        assert!(select(&db, &filters(&["yamada"], false)).keeps(1));
    }

    #[test]
    fn a_sender_pattern_matches_the_display_name() {
        // The scan matches the decoded header, where the display name sits next
        // to the address; a Japanese name never appears in the address itself.
        let db = fixture(&[(1, "yamada.hanako@example.jp", "山田　花子", &[])]);
        assert!(select(&db, &filters(&["山田"], false)).keeps(1));
    }

    #[test]
    fn a_sender_pattern_is_case_insensitive_for_ascii() {
        // Patterns arrive lowercased from `Config::filters`.
        let db = fixture(&[(1, "Yamada.Hanako@Example.jp", "Yamada", &[])]);
        assert!(select(&db, &filters(&["yamada"], false)).keeps(1));
    }

    #[test]
    fn repeated_sender_patterns_are_or_combined() {
        let db = fixture(&[
            (1, "a@example.jp", "A", &[]),
            (2, "b@example.jp", "B", &[]),
            (3, "c@example.jp", "C", &[]),
        ]);
        let candidates = select(&db, &filters(&["a@", "b@"], false));
        assert!(candidates.keeps(1));
        assert!(candidates.keeps(2));
        assert!(!candidates.keeps(3));
    }

    #[test]
    fn a_non_matching_sender_is_dropped() {
        let db = fixture(&[(1, "someone@example.jp", "Someone", &[])]);
        assert!(!select(&db, &filters(&["yamada"], false)).keeps(1));
    }

    #[test]
    fn underscores_in_a_pattern_are_literal() {
        // `_` is a single-character wildcard in LIKE; a caller means the character.
        let db = fixture(&[
            (1, "a_b@example.jp", "A", &[]),
            (2, "axb@example.jp", "B", &[]),
        ]);
        let candidates = select(&db, &filters(&["a_b"], false));
        assert!(candidates.keeps(1));
        assert!(!candidates.keeps(2));
    }

    #[test]
    fn percent_in_a_pattern_is_literal() {
        let db = fixture(&[
            (1, "100%@example.jp", "A", &[]),
            (2, "other@example.jp", "B", &[]),
        ]);
        let candidates = select(&db, &filters(&["100%"], false));
        assert!(candidates.keeps(1));
        assert!(!candidates.keeps(2));
    }

    #[test]
    fn an_attachment_filter_never_narrows() {
        // Mail's `attachments` table misses real attachments (32 of them on a
        // real mailbox), so requiring a row would silently lose messages. The
        // message with no attachment row must survive the index and be left for
        // the scan's MIME walk to judge.
        let db = fixture(&[
            (1, "yamada@example.jp", "Yamada", &["report.pdf"]),
            (2, "yamada@example.jp", "Yamada", &[]),
        ]);
        let candidates = select(&db, &filters(&["yamada"], true));
        assert!(candidates.keeps(1));
        assert!(candidates.keeps(2));
    }

    #[test]
    fn an_attachment_filter_alone_is_not_worth_opening_the_index() {
        // Nothing to narrow by, so the whole pass is skipped.
        let dir = tempfile::tempdir().unwrap();
        assert!(Candidates::load(dir.path(), &filters(&[], true)).is_none());
    }

    #[test]
    fn a_message_the_index_never_saw_is_kept() {
        // The orphan case: 2,552 such files on a real mailbox, one of them a hit.
        let db = fixture(&[(1, "someone@example.jp", "Someone", &[])]);
        assert!(select(&db, &filters(&["yamada"], false)).keeps(999));
    }

    #[test]
    fn only_a_sender_pattern_gives_the_index_something_to_do() {
        assert!(!narrows(&Filters::default()));
        assert!(narrows(&filters(&["yamada"], false)));
        // An attachment filter is not pushed down, so on its own it narrows nothing.
        assert!(!narrows(&filters(&[], true)));
    }

    #[test]
    fn a_missing_index_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Candidates::load(dir.path(), &filters(&["yamada"], false)).is_none());
    }

    #[test]
    fn an_index_that_is_not_a_database_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("MailData")).unwrap();
        std::fs::write(dir.path().join(INDEX_RELATIVE_PATH), "not sqlite").unwrap();
        assert!(Candidates::load(dir.path(), &filters(&["yamada"], false)).is_none());
    }

    #[test]
    fn an_index_missing_the_expected_tables_is_not_an_error() {
        // A future macOS reshaping the schema must not break the search.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("MailData")).unwrap();
        let path = dir.path().join(INDEX_RELATIVE_PATH);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE unrelated (id INTEGER PRIMARY KEY);")
            .unwrap();
        drop(connection);
        assert!(Candidates::load(dir.path(), &filters(&["yamada"], false)).is_none());
    }

    #[test]
    fn rowid_comes_from_the_filename() {
        assert_eq!(
            rowid_from_path(&PathBuf::from("/a/b/Messages/963826.emlx")),
            Some(963826)
        );
    }

    #[test]
    fn rowid_comes_from_a_partial_filename_too() {
        // Not-yet-downloaded messages must be narrowable like any other.
        assert_eq!(
            rowid_from_path(&PathBuf::from("/a/b/Messages/964173.partial.emlx")),
            Some(964173)
        );
    }

    #[test]
    fn a_non_numeric_filename_has_no_rowid() {
        assert_eq!(rowid_from_path(&PathBuf::from("/a/b/draft.emlx")), None);
        assert_eq!(rowid_from_path(&PathBuf::from("/a/b/963826.txt")), None);
    }
}
