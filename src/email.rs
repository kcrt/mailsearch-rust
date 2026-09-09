//! Email parsing and text extraction utilities.

use crate::models::{DATE_FORMAT, NO_SUBJECT, UNKNOWN_SENDER};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use mailparse::dateparse;
use mailparse::MailHeaderMap;
use regex::Regex;
use std::sync::OnceLock;

/// Macro to generate cached regex functions.
macro_rules! cached_regex {
    ($name:ident, $pattern:literal) => {
        fn $name() -> &'static Regex {
            static REGEX: OnceLock<Regex> = OnceLock::new();
            REGEX.get_or_init(|| Regex::new($pattern).unwrap())
        }
    };
}

// Cached regex patterns for HTML stripping.
cached_regex!(html_tag_regex, r"<[^>]+>");
cached_regex!(style_block_regex, r"(?is)<style[^>]*>.*?</style>");
cached_regex!(script_block_regex, r"(?is)<script[^>]*>.*?</script>");
cached_regex!(html_comment_regex, r"(?s)<!--.*?-->");
cached_regex!(whitespace_regex, r"\s+");

/// Parse a `Date:` header into epoch seconds.
pub fn parse_date_header(date_header: Option<&str>) -> Option<i64> {
    let date_str = date_header?;
    if let Ok(timestamp) = dateparse(date_str) {
        return Some(timestamp);
    }
    // `dateparse` only understands RFC 2822, and rejects the ISO-style headers
    // some mailers emit (`2026-07-28 10:00:00 +0900`). Those used to be
    // recovered downstream by re-parsing the formatted date string, so they need
    // a fallback here or they would silently lose their date.
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        return Some(dt.timestamp());
    }
    let first_token = date_str.split_whitespace().next()?;
    NaiveDate::parse_from_str(first_token, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_time(NaiveTime::MIN).and_utc().timestamp())
}

/// Convert epoch seconds into the timezone dates are presented in.
///
/// This is the single place that decides the display timezone. `date_str` and the
/// TUI's `after:`/`before:` filters both derive from it, so they cannot drift
/// apart and leave a row excluded by a filter that matches its visible date.
fn display_dt(timestamp: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(timestamp, 0).single()
}

/// Format epoch seconds for display.
pub fn format_timestamp(timestamp: i64) -> Option<String> {
    display_dt(timestamp).map(|dt| dt.format(DATE_FORMAT).to_string())
}

/// Calendar date of a timestamp, as displayed.
pub fn display_date(timestamp: i64) -> Option<NaiveDate> {
    display_dt(timestamp).map(|dt| dt.date_naive())
}

/// Display string for a message's date.
///
/// Prefers the formatted timestamp, falls back to the raw header when it could
/// not be interpreted at all, and finally to a placeholder when there is no
/// header to show.
pub fn date_display(timestamp: Option<i64>, raw_header: Option<&str>) -> String {
    timestamp
        .and_then(format_timestamp)
        .or_else(|| raw_header.map(str::to_string))
        .unwrap_or_else(|| "N/A".to_string())
}

/// Remove the parts of an HTML document that carry no readable text: style and
/// script blocks together with their contents, and comments.
///
/// Split out from [`strip_html_tags`] so the display-oriented conversion in
/// [`crate::message`] can reuse it without also inheriting the whitespace
/// collapsing below — which is right for matching a query against a body and
/// wrong for showing that body to a person.
pub fn strip_html_blocks(html: &str) -> String {
    let text = style_block_regex().replace_all(html, " ");
    let text = script_block_regex().replace_all(&text, " ");
    html_comment_regex().replace_all(&text, " ").into_owned()
}

/// Remove HTML tags, CSS, scripts, and normalize whitespace.
pub fn strip_html_tags(html: &str) -> String {
    let text = strip_html_blocks(html);
    // Remove remaining HTML tags
    let text = html_tag_regex().replace_all(&text, " ");
    // Normalize whitespace
    whitespace_regex()
        .replace_all(&text, " ")
        .trim()
        .to_string()
}

/// Clean embedded newlines from header values.
pub fn clean_header_value(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

/// Raw value of a header, cleaned of embedded newlines. `None` if absent.
fn header_value(mail: &mailparse::ParsedMail<'_>, header: &str) -> Option<String> {
    mail.headers
        .get_first_header(header)
        .map(|h| clean_header_value(&h.get_value()))
}

/// Extract header value safely.
pub fn extract_header(mail: &mailparse::ParsedMail<'_>, header: &str, default: &str) -> String {
    header_value(mail, header).unwrap_or_else(|| default.to_string())
}

/// Extract the `Message-ID`, treating a blank one as absent.
///
/// Kept as an `Option` because an unsent draft can lack one, and a caller that
/// wants to reply needs to tell "no id" apart from an empty string.
pub fn extract_message_id(mail: &mailparse::ParsedMail<'_>) -> Option<String> {
    header_value(mail, "Message-ID")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Headers that a query is matched against, alongside the body.
///
/// `Reply-To` is here because it often carries an address the visible `From`
/// does not, and searching for a correspondent should find those messages too.
const SEARCHABLE_HEADERS: [&str; 5] = ["Subject", "From", "To", "Cc", "Reply-To"];

/// The searchable headers rendered as `"Subject: …\nFrom: …"`, MIME-decoded.
///
/// Kept apart from the body so a caller can search both while still displaying
/// only the body (see [`process_emlx_file`]).
pub fn extract_header_text(mail: &mailparse::ParsedMail<'_>) -> String {
    SEARCHABLE_HEADERS
        .iter()
        .filter_map(|header| header_value(mail, header).map(|value| format!("{header}: {value}")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the readable body of a message: every text part, HTML stripped,
/// attachments skipped. Headers are [`extract_header_text`]'s job.
pub fn extract_email_text(mail: &mailparse::ParsedMail<'_>) -> String {
    let mut text_parts = Vec::new();
    extract_body_text(mail, &mut text_parts);
    text_parts.join("\n")
}

/// Extract body text from email parts recursively.
fn extract_body_text(mail: &mailparse::ParsedMail<'_>, text_parts: &mut Vec<String>) {
    let content_type = mail.ctype.mimetype.to_lowercase();

    // Skip attachments
    let cd = mail.get_content_disposition();
    if let mailparse::DispositionType::Attachment = cd.disposition {
        return;
    }

    if content_type.starts_with("text/plain") {
        // Use get_body() which handles character encoding automatically
        if let Ok(text) = mail.get_body() {
            text_parts.push(text);
        }
    } else if content_type.starts_with("text/html") {
        // Use get_body() which handles character encoding automatically
        if let Ok(text) = mail.get_body() {
            text_parts.push(strip_html_tags(&text));
        }
    } else if content_type.starts_with("multipart/") {
        for subpart in &mail.subparts {
            extract_body_text(subpart, text_parts);
        }
    }
}

/// Parse the positional query + --or values into OR-groups of AND-terms (DNF).
/// Outer = OR, inner = AND. Terms are lowercased; empty groups are dropped.
pub fn parse_query_groups(query: &str, or_terms: &[String]) -> Vec<Vec<String>> {
    std::iter::once(query)
        .chain(or_terms.iter().map(String::as_str))
        .map(|g| {
            g.split_whitespace()
                .map(|t| t.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .filter(|g: &Vec<String>| !g.is_empty())
        .collect()
}

/// Check if a message, given as the pieces of text it is made of, matches the
/// search query groups.
///
/// Groups are OR-combined; terms within a group are AND-combined (DNF).
/// Terms are expected to be pre-lowercased by [`parse_query_groups`].
/// An empty group list matches everything (preserves empty-query behavior).
///
/// A term is satisfied when *any* part contains it, so an AND-group can be spread
/// across the pieces — `mailsearch grafana raid` matches a message whose header
/// holds one term and whose body holds the other. Taking the parts separately
/// avoids concatenating the body into a fresh allocation for every message.
pub fn matches_query_parts(parts: &[&str], groups: &[Vec<String>]) -> bool {
    if groups.is_empty() {
        return true;
    }
    let lowered: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    groups.iter().any(|group| {
        group
            .iter()
            .all(|term| lowered.iter().any(|part| part.contains(term)))
    })
}

/// Whether a message's `From` header matches any of the given patterns.
///
/// Patterns are expected pre-lowercased. Matching is a substring test against
/// the MIME-decoded header, so both the display name and the address are
/// searchable (`--from 山田` and `--from yamada8010` both work).
/// No patterns means no sender restriction.
pub fn matches_from(mail: &mailparse::ParsedMail<'_>, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let Some(from) = header_value(mail, "From") else {
        return false;
    };
    let from = from.to_ascii_lowercase();
    patterns.iter().any(|pattern| from.contains(pattern))
}

/// Whether a single part counts as a real attachment.
///
/// Embedded images are the whole difficulty here: a signature logo is a named
/// image part just like a genuine attachment is, and mail from the same set of
/// correspondents arrives in both of the shapes below. Both were taken from
/// real mail while this was written.
///
/// ⚠️ `mailparse` reports [`DispositionType::Inline`] both for an explicit
/// `Content-Disposition: inline` and for a part carrying no such header at all,
/// so the header's presence has to be checked separately:
///
/// - Outlook (recent): `Content-Disposition: inline; filename="image001.png"`
///   — an explicit disposition, so the header check rejects it.
/// - Outlook (older, and Word-generated mail): no `Content-Disposition` at all,
///   only `Content-Type: image/png; name="image003.png"` plus a `Content-ID`
///   that the HTML part references as `cid:`. The `Content-ID` is what marks it
///   as embedded rather than attached.
///
/// An explicit `attachment` disposition always wins, `Content-ID` or not: if the
/// sender declared it an attachment, it is one — with the single exception of a
/// cryptographic signature; see [`is_detached_signature`].
fn part_is_attachment(part: &mailparse::ParsedMail<'_>) -> bool {
    if part.ctype.mimetype.to_lowercase().starts_with("multipart/") {
        return false;
    }
    if is_detached_signature(&part.ctype.mimetype) {
        return false;
    }
    let disposition = part.get_content_disposition();
    if matches!(
        disposition.disposition,
        mailparse::DispositionType::Attachment
    ) {
        return true;
    }
    if part
        .headers
        .get_first_header("Content-Disposition")
        .is_some()
    {
        // Explicitly inline (or form-data/extension): not an attachment.
        return false;
    }
    if part.headers.get_first_header("Content-ID").is_some() {
        // Referenced from the HTML body as `cid:`, i.e. embedded, not attached.
        return false;
    }
    disposition.params.contains_key("filename") || part.ctype.params.contains_key("name")
}

/// Whether a MIME type is a detached cryptographic signature.
///
/// S/MIME/PGP signed mail carries the signature as a sibling part declared
/// `Content-Disposition: attachment` with a filename (`smime.p7s`,
/// `signature.asc`), which is indistinguishable from a real attachment by
/// structure alone. Nobody asking for "mail with an attachment" means a signed
/// bank notification, and Apple Mail agrees: it does not list these as
/// attachments either. Measured on real mail, they were **83 of the 831**
/// messages `--has-attachment` returned over 90 days.
///
/// Only *detached* signatures are excluded. `application/pkcs7-mime` — signed or
/// encrypted content rather than a signature over it — can carry the real
/// message and its attachments, so it is left alone.
fn is_detached_signature(mimetype: &str) -> bool {
    matches!(
        mimetype.to_lowercase().as_str(),
        "application/pkcs7-signature"
            | "application/x-pkcs7-signature"
            | "application/pgp-signature"
    )
}

/// Every real attachment's filename, lowercased for matching.
///
/// Uses the same notion of "real" as [`has_attachment`], so `--attachment-name`
/// and `--has-attachment` cannot disagree about what a message contains.
fn attachment_names(mail: &mailparse::ParsedMail<'_>, out: &mut Vec<String>) {
    if part_is_attachment(mail) {
        let disposition = mail.get_content_disposition();
        if let Some(name) = disposition
            .params
            .get("filename")
            .or_else(|| mail.ctype.params.get("name"))
        {
            out.push(crate::message::decode_encoded_words(name).to_lowercase());
        }
    }
    for subpart in &mail.subparts {
        attachment_names(subpart, out);
    }
}

/// Whether any attachment's name contains any of `patterns`.
///
/// Patterns are expected to arrive lowercased, as the names are lowercased here.
pub fn matches_attachment_name(mail: &mailparse::ParsedMail<'_>, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let mut names = Vec::new();
    attachment_names(mail, &mut names);
    names
        .iter()
        .any(|name| patterns.iter().any(|pattern| name.contains(pattern)))
}

/// Whether a message carries at least one real attachment.
///
/// Only the MIME structure is inspected, never the payload, so this still works
/// on a `.partial.emlx` whose attachment bodies have not been downloaded — the
/// part headers and filenames are present even when the content is not.
pub fn has_attachment(mail: &mailparse::ParsedMail<'_>) -> bool {
    if part_is_attachment(mail) {
        return true;
    }
    mail.subparts.iter().any(has_attachment)
}

/// What a message must satisfy to be returned by the scan.
///
/// Grouping the criteria keeps them from being confused with each other at call
/// sites, and lets new filters be added without touching every caller.
pub struct Criteria<'a> {
    /// OR-groups of AND-terms, as produced by [`parse_query_groups`].
    pub groups: &'a [Vec<String>],
    /// Earliest message date to accept, in epoch seconds.
    pub date_cutoff: Option<i64>,
    /// Header/structure filters, AND-ed with the query.
    pub filters: &'a crate::models::Filters,
}

/// The RFC 822 message inside an `.emlx` file.
///
/// An `.emlx` is three parts: a first line giving the message length in bytes,
/// the message itself, and an Apple property list of Mail's own metadata. Every
/// file has the trailing plist — all 233,847 of them on a real mailbox — so
/// **dropping the first line is not enough**; the length has to be used.
///
/// Getting this wrong is easy to miss. In a multipart message the plist lands
/// after the closing boundary, where the parser ignores it. In a single-part
/// message it lands *inside the body*, and for the `Content-Transfer-Encoding:
/// base64` that Apple Mail uses for its own outgoing mail it is decoded along
/// with the message, appending binary rubbish to the text:
///
/// ```text
/// 2026年3月28日 9:00: みどり中央クリニック          <- the message
/// 2026年3月28日 9:00: みどり中央クリニック1[ޮȨ]... <- without the length
/// ```
///
/// A length that is missing, unparsable, or longer than the file falls back to
/// "everything after the first line", which is what this did before: a wrong
/// length should cost the old rubbish, not the whole message.
pub fn rfc822_slice(bytes: &[u8]) -> Option<&[u8]> {
    // `position(..)? + 1` also guards a truncated file with no newline at all,
    // and yields an empty slice when the newline is the final byte.
    let mime_start = bytes.iter().position(|&b| b == b'\n')? + 1;
    let rest = &bytes[mime_start..];
    let length = std::str::from_utf8(&bytes[..mime_start])
        .ok()
        .and_then(|line| line.trim().parse::<usize>().ok());
    Some(match length {
        Some(length) if length <= rest.len() => &rest[..length],
        _ => rest,
    })
}

/// Process a single .emlx file and return SearchResult if it matches the criteria.
///
/// `mtime` is the file's modification time in epoch seconds when it was already
/// stat'd during discovery; it stands in for the message date when the header is
/// missing or unreadable.
pub fn process_emlx_file(
    path: &std::path::Path,
    mtime: Option<i64>,
    criteria: &Criteria<'_>,
) -> Option<crate::models::SearchResult> {
    // Read as bytes rather than a String: some messages are not valid UTF-8, and
    // decoding is `mailparse`'s job anyway (`get_body` honours the charset).
    let bytes = std::fs::read(path).ok()?;

    // Parse as email
    let mail = mailparse::parse_mail(rfc822_slice(&bytes)?).ok()?;

    // Resolve the date first: it is cheap, and lets a message outside the window
    // be rejected before the expensive body extraction and HTML stripping.
    let raw_date = mail.get_headers().get_first_value("Date");
    // A message whose date can't be read falls back to its mtime. It only got
    // here by passing the mtime prefilter, so "date unknown but the file is
    // recent" should be included rather than silently dropped.
    let timestamp = parse_date_header(raw_date.as_deref()).or(mtime);
    if let Some(cutoff) = criteria.date_cutoff {
        if timestamp.is_none_or(|ts| ts < cutoff) {
            return None;
        }
    }

    // Header and structure filters come before the body work below: both only
    // read headers, while the body extraction decodes charsets and strips HTML.
    if !matches_from(&mail, &criteria.filters.from_patterns) {
        return None;
    }
    if let Some(wanted) = &criteria.filters.message_id {
        let found = extract_message_id(&mail).map(|id| crate::models::normalise_message_id(&id));
        if found.as_deref() != Some(wanted.as_str()) {
            return None;
        }
    }
    if criteria.filters.require_attachment && !has_attachment(&mail) {
        return None;
    }
    if !matches_attachment_name(&mail, &criteria.filters.attachment_names) {
        return None;
    }

    // The body is kept header-free because the UI shows the headers separately,
    // but the query is matched against both: a sender, recipient or subject that
    // never appears in the body must still be findable.
    let text_content = extract_email_text(&mail);
    let header_content = extract_header_text(&mail);

    if !matches_query_parts(&[&header_content, &text_content], criteria.groups) {
        return None;
    }

    let subject = extract_header(&mail, "Subject", NO_SUBJECT);
    let from_addr = extract_header(&mail, "From", UNKNOWN_SENDER);
    let to_addr = extract_header(&mail, "To", "");
    let cc_addr = extract_header(&mail, "Cc", "");
    let date_str = date_display(timestamp, raw_date.as_deref());
    let message_id = extract_message_id(&mail);

    Some(crate::models::SearchResult {
        subject,
        from_addr,
        to_addr,
        cc_addr,
        date_str,
        timestamp,
        message_id,
        file_path: path.display().to_string(),
        content: text_content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Helper function to get fixture path
    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name)
    }

    // ========== matches_from / has_attachment tests ==========

    // These parse a message from bytes rather than a fixture file: the point of
    // each case is one header or one part, which reads better inline.
    fn mail(raw: &str) -> mailparse::ParsedMail<'_> {
        mailparse::parse_mail(raw.as_bytes()).unwrap()
    }

    fn patterns(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn no_from_patterns_matches_every_message() {
        assert!(matches_from(&mail("From: a@example.com\n\nbody"), &[]));
    }

    #[test]
    fn from_matches_on_the_address() {
        let msg = mail("From: Taro Suzuki <t.suzuki@example.jp>\n\nbody");
        assert!(matches_from(&msg, &patterns(&["t.suzuki"])));
        assert!(!matches_from(&msg, &patterns(&["tanaka"])));
    }

    #[test]
    fn from_matches_on_the_decoded_display_name() {
        // The header is MIME-encoded; the pattern is the plain name.
        let msg = mail("From: =?utf-8?B?5bGx55Sw?= <s@example.jp>\n\nbody");
        assert!(matches_from(&msg, &patterns(&["山田"])));
    }

    #[test]
    fn from_patterns_are_or_combined() {
        let msg = mail("From: b@example.com\n\nbody");
        assert!(matches_from(
            &msg,
            &patterns(&["a@example.com", "b@example.com"])
        ));
    }

    #[test]
    fn a_message_without_a_from_header_matches_no_pattern() {
        assert!(!matches_from(
            &mail("Subject: x\n\nbody"),
            &patterns(&["a"])
        ));
    }

    #[test]
    fn a_plain_message_has_no_attachment() {
        assert!(!has_attachment(&mail(
            "From: a@example.com\nContent-Type: text/plain\n\nbody"
        )));
    }

    #[test]
    fn text_and_html_alternatives_are_not_attachments() {
        assert!(!has_attachment(&mail(concat!(
            "Content-Type: multipart/alternative; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nplain\n",
            "--b\nContent-Type: text/html\n\n<p>html</p>\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn a_declared_attachment_part_counts() {
        assert!(has_attachment(&mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf\n",
            "Content-Disposition: attachment; filename=\"a.pdf\"\n\nJVBER\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn an_explicitly_inline_image_does_not_count() {
        // Outlook signature logos arrive exactly like this, on almost every
        // message from some correspondents; counting them would make
        // --has-attachment useless.
        assert!(!has_attachment(&mail(concat!(
            "Content-Type: multipart/related; boundary=b\n\n",
            "--b\nContent-Type: text/html\n\n<p>hi</p>\n",
            "--b\nContent-Type: image/png; name=\"image001.png\"\n",
            "Content-Disposition: inline; filename=\"image001.png\"\n\niVBOR\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn a_named_image_with_a_content_id_does_not_count() {
        // The older-Outlook signature shape: no Content-Disposition at all, so
        // only the Content-ID separates it from a real attachment. Counting it
        // let two signature-only messages through --has-attachment.
        assert!(!has_attachment(&mail(concat!(
            "Content-Type: multipart/related; boundary=b\n\n",
            "--b\nContent-Type: text/html\n\n<img src=3D\"cid:image003.png@01DD\">\n",
            "--b\nContent-Type: image/png; name=\"image003.png\"\n",
            "Content-ID: <image003.png@01DD3CA1.F97A3580>\n\niVBOR\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn a_declared_attachment_counts_even_with_a_content_id() {
        // An explicit disposition is the sender's own answer; trust it.
        assert!(has_attachment(&mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: image/png; name=\"chart.png\"\n",
            "Content-ID: <chart@example.com>\n",
            "Content-Disposition: attachment; filename=\"chart.png\"\n\niVBOR\n",
            "--b--\n"
        ))));
    }

    // ========== rfc822_slice tests ==========

    /// An `.emlx` as Mail writes it, without the plist: length line, message.
    ///
    /// The count has to be right — [`rfc822_slice`] slices by it, so a
    /// placeholder would truncate the fixture rather than the plist.
    fn emlx(message: &str) -> String {
        format!("{}\n{message}", message.len())
    }

    /// The same, with the trailing Apple plist every real `.emlx` carries.
    fn emlx_with_plist(message: &str) -> Vec<u8> {
        let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist><dict/></plist>\n";
        format!("{}\n{message}{plist}", message.len()).into_bytes()
    }

    #[test]
    fn the_trailing_plist_is_cut_off() {
        let raw = emlx_with_plist("Subject: hi\n\nbody\n");
        assert_eq!(rfc822_slice(&raw).unwrap(), b"Subject: hi\n\nbody\n");
    }

    #[test]
    fn a_single_part_body_does_not_swallow_the_plist() {
        // The case that appended binary rubbish to Apple Mail's own sent mail.
        let raw = emlx_with_plist("Content-Type: text/plain\n\nhello\n");
        let mail = mailparse::parse_mail(rfc822_slice(&raw).unwrap()).unwrap();
        assert_eq!(extract_email_text(&mail).trim(), "hello");
    }

    #[test]
    fn a_missing_length_falls_back_to_everything_after_the_first_line() {
        // A plain `.eml` handed to the tool, or a file whose first line is not a
        // number: keep the old behaviour rather than losing the message.
        let raw = b"Subject: hi\n\nbody\n".to_vec();
        assert_eq!(rfc822_slice(&raw).unwrap(), b"\nbody\n");
    }

    #[test]
    fn a_length_longer_than_the_file_falls_back() {
        let raw = b"99999\nSubject: hi\n\nbody\n".to_vec();
        assert_eq!(rfc822_slice(&raw).unwrap(), b"Subject: hi\n\nbody\n");
    }

    #[test]
    fn a_file_with_no_newline_at_all_is_rejected() {
        assert!(rfc822_slice(b"12345").is_none());
    }

    #[test]
    fn a_length_line_ending_in_crlf_still_parses() {
        // "Subject: hi\n\nbody\n" is 18 bytes; the trailing text stands in for
        // the plist and must be cut off despite the \r before the newline.
        let raw = b"18\r\nSubject: hi\n\nbody\nTRAILING".to_vec();
        assert_eq!(rfc822_slice(&raw).unwrap(), b"Subject: hi\n\nbody\n");
    }

    // ========== matches_attachment_name ==========

    #[test]
    fn an_attachment_name_pattern_matches_a_substring() {
        let m = mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf\n",
            "Content-Disposition: attachment; filename=\"2026 report final.pdf\"\n\nJVBER\n",
            "--b--\n"
        ));
        assert!(matches_attachment_name(&m, &["report".to_string()]));
        assert!(!matches_attachment_name(&m, &["invoice".to_string()]));
    }

    #[test]
    fn no_attachment_name_pattern_matches_everything() {
        let m = mail("Content-Type: text/plain\n\nbody\n");
        assert!(matches_attachment_name(&m, &[]));
    }

    #[test]
    fn an_attachment_name_pattern_is_case_insensitive_for_ascii() {
        // Patterns arrive lowercased from `Config::filters`.
        let m = mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf\n",
            "Content-Disposition: attachment; filename=\"Report.PDF\"\n\nJVBER\n",
            "--b--\n"
        ));
        assert!(matches_attachment_name(&m, &["report.pdf".to_string()]));
    }

    #[test]
    fn repeated_attachment_name_patterns_are_or_combined() {
        let m = mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\n\nJVBER\n",
            "--b--\n"
        ));
        assert!(matches_attachment_name(
            &m,
            &["invoice".to_string(), "report".to_string()]
        ));
    }

    #[test]
    fn a_signature_image_name_is_not_matchable() {
        // Otherwise `--attachment-name image001` would hit half the mailbox.
        let m = mail(concat!(
            "Content-Type: multipart/related; boundary=b\n\n",
            "--b\nContent-Type: text/html\n\n<img src=\"cid:i\">\n",
            "--b\nContent-Type: image/png; name=\"image001.png\"\n",
            "Content-ID: <i>\n\niVBOR\n",
            "--b--\n"
        ));
        assert!(!matches_attachment_name(&m, &["image001".to_string()]));
    }

    #[test]
    fn a_japanese_attachment_name_is_matchable() {
        // The name arrives RFC 2047 encoded and must be decoded before matching,
        // or a search for the case number could never hit.
        let m = mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/x-zip-compressed\n",
            "Content-Disposition: attachment;\n",
            "\tfilename*0=\"=?iso-2022-jp?B?GyRCPzc1LBsoQjI2MDQxNSAbJEJOURsoQjIwMjYtMDAxIBsk\";\n",
            "\tfilename*1=\"QjszGyhC?=  =?iso-2022-jp?B?GyRCRURCQE86GyhCKBskQj9XQi4bKEIpGyRC\";\n",
            "\tfilename*2=\"M1gycRsoQi56aXA=?=\"\n\nUEsD\n",
            "--b--\n"
        ));
        assert!(matches_attachment_name(&m, &["倫2026-001".to_string()]));
        assert!(matches_attachment_name(&m, &["山田".to_string()]));
    }

    #[test]
    fn an_smime_signature_is_not_an_attachment() {
        // The shape every signed bank notification arrives in: a plain body plus
        // a detached signature declared as an attachment. 83 of 831 hits.
        assert!(!has_attachment(&mail(concat!(
            "Content-Type: multipart/signed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/x-pkcs7-signature; name=\"smime.p7s\"\n",
            "Content-Disposition: attachment; filename=\"smime.p7s\"\n\nMIIN\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn the_unprefixed_smime_signature_type_is_also_excluded() {
        // Both spellings occur in the wild, 19 and 64 times respectively.
        assert!(!has_attachment(&mail(concat!(
            "Content-Type: multipart/signed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pkcs7-signature; name=\"smime.p7s\"\n",
            "Content-Disposition: attachment; filename=\"smime.p7s\"\n\nMIIN\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn a_pgp_signature_is_not_an_attachment() {
        assert!(!has_attachment(&mail(concat!(
            "Content-Type: multipart/signed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pgp-signature; name=\"signature.asc\"\n",
            "Content-Disposition: attachment; filename=\"signature.asc\"\n\n-----BEGIN\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn a_real_attachment_alongside_a_signature_still_counts() {
        // Signed mail that also carries a document must not be filtered out.
        assert!(has_attachment(&mail(concat!(
            "Content-Type: multipart/signed; boundary=b\n\n",
            "--b\nContent-Type: multipart/mixed; boundary=c\n\n",
            "--c\nContent-Type: text/plain\n\nbody\n",
            "--c\nContent-Type: application/pdf; name=\"report.pdf\"\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\n\nJVBER\n",
            "--c--\n",
            "--b\nContent-Type: application/pkcs7-signature; name=\"smime.p7s\"\n",
            "Content-Disposition: attachment; filename=\"smime.p7s\"\n\nMIIN\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn signed_or_encrypted_content_is_not_treated_as_a_signature() {
        // `pkcs7-mime` wraps the real message, attachments and all, so excluding
        // it would hide genuine attachments rather than signature noise.
        assert!(has_attachment(&mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pkcs7-mime; name=\"smime.p7m\"\n",
            "Content-Disposition: attachment; filename=\"smime.p7m\"\n\nMIIN\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn a_named_part_without_a_disposition_header_counts() {
        // mailparse reports Inline for a missing Content-Disposition, so this is
        // the case that the header-presence check exists for.
        assert!(has_attachment(&mail(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf; name=\"x.pdf\"\n\nJVBER\n",
            "--b--\n"
        ))));
    }

    #[test]
    fn an_attachment_only_message_with_no_multipart_counts() {
        assert!(has_attachment(&mail(concat!(
            "Content-Type: application/ms-tnef; name=\"winmail.dat\"\n",
            "Content-Disposition: attachment; filename=\"winmail.dat\"\n\ndata\n"
        ))));
    }

    // ========== matches_query tests ==========

    // Helper: build query groups from a single AND-group string (no --or terms).
    fn q(query: &str) -> Vec<Vec<String>> {
        parse_query_groups(query, &[])
    }

    // Helper: the one-part case, which is what most of these tests are about.
    fn matches_query(text: &str, groups: &[Vec<String>]) -> bool {
        matches_query_parts(&[text], groups)
    }

    // Helpers: run a scan against a file. `Criteria` borrows the groups, so these
    // own them for the duration of the call.
    fn scan(path: &std::path::Path, query: &str) -> Option<crate::models::SearchResult> {
        scan_with(path, query, None, None)
    }

    fn scan_with(
        path: &std::path::Path,
        query: &str,
        mtime: Option<i64>,
        date_cutoff: Option<i64>,
    ) -> Option<crate::models::SearchResult> {
        let groups = q(query);
        let filters = crate::models::Filters::default();
        let criteria = Criteria {
            groups: &groups,
            date_cutoff,
            filters: &filters,
        };
        process_emlx_file(path, mtime, &criteria)
    }

    // Epoch seconds for a UTC timestamp, for readable date assertions.
    fn utc_ts(s: &str) -> i64 {
        DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    #[test]
    fn test_matches_query_single_term() {
        assert!(matches_query("Hello World", &q("hello")));
        assert!(matches_query("Hello World", &q("world")));
        assert!(!matches_query("Hello World", &q("foo")));
    }

    #[test]
    fn test_matches_query_multiple_terms_and_logic() {
        // All terms must be present (AND logic)
        assert!(matches_query(
            "rust programming language",
            &q("rust language")
        ));
        assert!(matches_query(
            "rust programming language",
            &q("rust programming")
        ));
        assert!(!matches_query("rust programming", &q("rust java")));
        assert!(!matches_query("rust", &q("rust java")));
    }

    #[test]
    fn test_matches_query_case_insensitive() {
        assert!(matches_query("RuSt ProGramMinG", &q("rust")));
        assert!(matches_query("rust", &q("RUST")));
        assert!(matches_query("RuSt", &q("rUsT")));
        assert!(matches_query("Hello WORLD", &q("hello world")));
    }

    #[test]
    fn test_matches_query_partial_word_match() {
        assert!(matches_query("testing", &q("test")));
        assert!(matches_query("programming", &q("program")));
        assert!(matches_query("email@example.com", &q("example")));
    }

    #[test]
    fn test_matches_query_empty_cases() {
        assert!(matches_query("any text", &q("")));
        assert!(!matches_query("", &q("query")));
        assert!(matches_query("", &q("")));
    }

    #[test]
    fn test_matches_query_special_characters() {
        assert!(matches_query("user@example.com", &q("@example")));
        assert!(matches_query("price: $100", &q("$100")));
        assert!(matches_query("50% discount", &q("50%")));
    }

    #[test]
    fn test_matches_query_whitespace_handling() {
        assert!(matches_query("multiple   spaces", &q("multiple spaces")));
        assert!(matches_query("text with\nnewlines", &q("text newlines")));
        assert!(matches_query("  leading spaces", &q("leading")));
    }

    // ========== OR search (parse_query_groups + matches_query) tests ==========

    #[test]
    fn test_parse_query_groups_dnf_structure() {
        // Positional query becomes the first group; each --or value a further group.
        let groups = parse_query_groups("hello world", &["today".to_string()]);
        assert_eq!(groups, vec![vec!["hello", "world"], vec!["today"]]);
    }

    #[test]
    fn test_parse_query_groups_drops_empty_groups() {
        // Empty or whitespace-only --or values are ignored.
        let groups = parse_query_groups("hello", &["".to_string(), "   ".to_string()]);
        assert_eq!(groups, vec![vec!["hello"]]);
        // Fully empty query yields no groups (matches everything downstream).
        assert!(parse_query_groups("", &[]).is_empty());
    }

    #[test]
    fn test_matches_query_or_logic() {
        // (rust AND lang) OR (java)
        let groups = parse_query_groups("rust lang", &["java".to_string()]);
        assert!(matches_query("rust lang tutorial", &groups)); // first group matches
        assert!(matches_query("java tutorial", &groups)); // second group matches
        assert!(matches_query("rust lang and java", &groups)); // both match
        assert!(!matches_query("python tutorial", &groups)); // neither matches
    }

    #[test]
    fn test_matches_query_or_preserves_group_and() {
        // A group only matches when ALL its terms are present.
        let groups = parse_query_groups("rust lang", &["python".to_string()]);
        // "rust" alone doesn't satisfy the (rust AND lang) group, and no python either.
        assert!(!matches_query("rust tutorial", &groups));
        // "python" satisfies its single-term group via OR.
        assert!(matches_query("python tutorial", &groups));
    }

    // ========== strip_html_tags tests ==========

    #[test]
    fn test_strip_html_tags_simple() {
        assert_eq!(strip_html_tags("<p>Hello</p>"), "Hello");
        assert_eq!(strip_html_tags("<div>World</div>"), "World");
        assert_eq!(strip_html_tags("<span>Text</span>"), "Text");
    }

    #[test]
    fn test_strip_html_tags_nested() {
        assert_eq!(
            strip_html_tags("<div><p><span>Nested</span></p></div>"),
            "Nested"
        );
        assert_eq!(
            strip_html_tags("<html><body><h1>Title</h1><p>Content</p></body></html>"),
            "Title Content"
        );
    }

    #[test]
    fn test_strip_html_tags_with_attributes() {
        assert_eq!(
            strip_html_tags("<div class='test' id='main'>Content</div>"),
            "Content"
        );
        assert_eq!(
            strip_html_tags("<a href='http://example.com'>Link</a>"),
            "Link"
        );
    }

    #[test]
    fn test_strip_html_tags_self_closing() {
        assert_eq!(strip_html_tags("Line 1<br/>Line 2"), "Line 1 Line 2");
        assert_eq!(strip_html_tags("<img src='test.jpg'/>Text"), "Text");
    }

    #[test]
    fn test_strip_html_tags_plain_text() {
        assert_eq!(strip_html_tags("Plain text"), "Plain text");
        assert_eq!(strip_html_tags("No HTML here"), "No HTML here");
    }

    #[test]
    fn test_strip_html_tags_mixed_content() {
        assert_eq!(
            strip_html_tags("Text before <b>bold</b> text after"),
            "Text before bold text after"
        );
    }

    #[test]
    fn test_strip_html_tags_whitespace_normalization() {
        let html = "<div>  Multiple   spaces  </div>";
        let result = strip_html_tags(html);
        // Should normalize whitespace
        assert!(!result.contains("   "));
    }

    #[test]
    fn test_strip_html_tags_empty() {
        assert_eq!(strip_html_tags(""), "");
        assert_eq!(strip_html_tags("<div></div>"), "");
    }

    // ========== date_display tests ==========

    // Mirrors how `process_emlx_file` builds `date_str` from a raw header.
    fn display_of(header: Option<&str>) -> String {
        date_display(parse_date_header(header), header)
    }

    #[test]
    fn test_date_display_valid_rfc2822() {
        assert_eq!(
            display_of(Some("Mon, 20 Jan 2026 10:30:00 +0000")),
            "2026-01-20 10:30"
        );
    }

    #[test]
    fn test_date_display_no_header() {
        assert_eq!(display_of(None), "N/A");
        assert_eq!(date_display(None, None), "N/A");
    }

    #[test]
    fn test_date_display_invalid_format() {
        // `dateparse` maps unrecognizable input to epoch 0 rather than failing.
        assert_eq!(display_of(Some("Not a valid date")), "1970-01-01 00:00");
        assert_eq!(display_of(Some("")), "1970-01-01 00:00");
    }

    #[test]
    fn test_date_display_uninterpretable_header_falls_back_to_raw() {
        assert_eq!(
            date_display(None, Some("sometime last Tuesday")),
            "sometime last Tuesday"
        );
    }

    // ========== parse_date_header tests ==========

    #[test]
    fn test_parse_date_header_rfc2822_offsets() {
        // Same instant expressed three ways.
        let expected = utc_ts("2026-01-20T10:30:00Z");
        assert_eq!(
            parse_date_header(Some("Tue, 20 Jan 2026 19:30:00 +0900")),
            Some(expected)
        );
        assert_eq!(
            parse_date_header(Some("Tue, 20 Jan 2026 06:30:00 -0400")),
            Some(expected)
        );
        assert_eq!(
            parse_date_header(Some("Tue, 20 Jan 2026 10:30:00 GMT")),
            Some(expected)
        );
    }

    #[test]
    fn test_parse_date_header_none() {
        assert_eq!(parse_date_header(None), None);
    }

    #[test]
    fn test_parse_date_header_garbage_is_epoch_zero() {
        // Pins a `dateparse` quirk: unrecognizable input yields Ok(0) rather than
        // an error, so such mail is dated 1970 and falls outside any date window.
        assert_eq!(parse_date_header(Some("Not a valid date")), Some(0));
        assert_eq!(parse_date_header(Some("")), Some(0));
    }

    #[test]
    fn test_parse_date_header_iso_style() {
        // `dateparse` rejects these; the fallback must recover them, otherwise a
        // message from today would be excluded by a date window.
        assert_eq!(
            parse_date_header(Some("2026-01-20 19:30:00 +0900")),
            Some(utc_ts("2026-01-20T00:00:00Z"))
        );
        assert_eq!(
            parse_date_header(Some("2026-01-20T19:30:00+09:00")),
            Some(utc_ts("2026-01-20T10:30:00Z"))
        );
    }

    #[test]
    fn test_format_timestamp_round_trip() {
        let ts = utc_ts("2026-01-20T10:30:00Z");
        assert_eq!(format_timestamp(ts).as_deref(), Some("2026-01-20 10:30"));
        assert_eq!(
            display_date(ts),
            Some(NaiveDate::from_ymd_opt(2026, 1, 20).unwrap())
        );
    }

    // ========== process_emlx_file integration tests ==========

    #[test]
    fn test_process_emlx_file_plain_text() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            // Skip if fixture doesn't exist (e.g., in CI without fixtures)
            return;
        }

        let result = scan(&path, "rust programming");
        assert!(result.is_some());

        let search_result = result.unwrap();
        assert_eq!(search_result.subject, "Test Plain Text Email");
        assert!(search_result.from_addr.contains("sender@example.com"));
        assert!(search_result.content.contains("rust programming"));
        // The fixture carries the trailing Apple plist every real `.emlx` has.
        // Slicing by the length line is what keeps it out of the body.
        assert!(
            !search_result.content.contains("plist"),
            "the trailing plist leaked into the body: {}",
            search_result.content
        );
    }

    #[test]
    fn test_process_emlx_file_html() {
        let path = fixture_path("html_email.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "invoice receipt");
        assert!(result.is_some());

        let search_result = result.unwrap();
        assert_eq!(search_result.subject, "HTML Test Email");
        // HTML tags should be stripped
        assert!(!search_result.content.contains("<p>"));
        assert!(!search_result.content.contains("<div>"));
        assert!(search_result.content.contains("invoice receipt"));
    }

    #[test]
    fn test_process_emlx_file_multipart() {
        let path = fixture_path("multipart_email.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "project update");
        assert!(result.is_some());

        let search_result = result.unwrap();
        assert_eq!(search_result.subject, "Multipart Email Test");
        assert!(search_result.content.contains("project update"));
    }

    #[test]
    fn test_process_emlx_file_no_subject() {
        let path = fixture_path("no_subject.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "without");
        assert!(result.is_some());

        let search_result = result.unwrap();
        // Should use NO_SUBJECT constant
        assert!(search_result.subject.contains("No Subject") || search_result.subject.is_empty());
    }

    #[test]
    fn test_process_emlx_file_no_match() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "nonexistent query xyz");
        assert!(result.is_none());
    }

    #[test]
    fn a_sender_only_in_the_headers_is_searchable() {
        // The regression this guards: the body was searched but the headers were
        // not, so a message could only be found by words it happened to repeat in
        // its text. An address that appears solely in `From:` must match.
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        let body = {
            let bytes = std::fs::read(&path).unwrap();
            let start = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
            let mail = mailparse::parse_mail(&bytes[start..]).unwrap();
            extract_email_text(&mail)
        };
        assert!(
            !body.to_lowercase().contains("sender@example.com"),
            "fixture must not repeat the sender in its body, or this proves nothing"
        );

        assert!(scan(&path, "sender@example.com").is_some(), "From");
        assert!(scan(&path, "recipient@example.com").is_some(), "To");
        assert!(scan(&path, "Test Plain Text Email").is_some(), "Subject");
    }

    #[test]
    fn an_and_group_may_span_the_headers_and_the_body() {
        // Terms are matched per-part, so this only works if a group is allowed to
        // draw one term from the headers and another from the body.
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        assert!(scan(&path, "sender@example.com plain text").is_some());
        assert!(scan(&path, "sender@example.com nonexistentxyz").is_none());
    }

    #[test]
    fn matches_query_parts_spreads_an_and_group_across_parts() {
        assert!(matches_query_parts(
            &["From: alice@example.com", "the body text"],
            &q("alice body")
        ));
        assert!(!matches_query_parts(
            &["From: alice@example.com", "the body text"],
            &q("alice missing")
        ));
        // OR-groups still work across parts.
        let groups = parse_query_groups("nothing", &["alice".to_string()]);
        assert!(matches_query_parts(
            &["From: alice@example.com", "the body text"],
            &groups
        ));
    }

    #[test]
    fn test_process_emlx_file_malformed() {
        let path = fixture_path("malformed.emlx");
        if !path.exists() {
            return;
        }

        // Should handle gracefully without panicking
        let _result = scan(&path, "any");
        // May return None or handle error gracefully
        // Main goal: no panic
    }

    #[test]
    fn test_process_emlx_file_nonexistent() {
        let path = PathBuf::from("nonexistent.emlx");
        let result = scan(&path, "query");
        assert!(result.is_none());
    }

    #[test]
    fn test_process_emlx_file_case_insensitive_match() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        // Query in different case
        let result = scan(&path, "RUST PROGRAMMING");
        assert!(result.is_some());
    }

    // ========== date window tests ==========

    #[test]
    fn test_process_emlx_file_records_message_id() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "rust programming").unwrap();
        // Angle brackets are preserved, matching the header and what Python's
        // `msg["message-id"]` returns.
        assert_eq!(
            result.message_id.as_deref(),
            Some("<plain-text-fixture@example.com>")
        );
    }

    #[test]
    fn test_process_emlx_file_without_message_id() {
        let path = fixture_path("no_subject.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "without").unwrap();
        assert_eq!(result.message_id, None);
    }

    #[test]
    fn test_extract_message_id_treats_blank_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blank_id.emlx");
        std::fs::write(
            &path,
            emlx("From: sender@example.com\nSubject: Blank\nMessage-ID:   \n\nbody text here\n"),
        )
        .unwrap();

        let result = scan(&path, "body text").unwrap();
        assert_eq!(result.message_id, None, "a blank Message-ID is not usable");
    }

    #[test]
    fn test_process_emlx_file_records_timestamp() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        let result = scan(&path, "rust programming").unwrap();
        assert_eq!(result.timestamp, Some(utc_ts("2026-01-20T10:30:00Z")));
        assert_eq!(result.date_str, "2026-01-20 10:30");
    }

    #[test]
    fn test_process_emlx_file_cutoff_after_message_rejects() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        // Fixture is dated 2026-01-20; a February cutoff must exclude it.
        let cutoff = utc_ts("2026-02-01T00:00:00Z");
        assert!(scan_with(&path, "rust programming", None, Some(cutoff)).is_none());
    }

    #[test]
    fn test_process_emlx_file_cutoff_before_message_accepts() {
        let path = fixture_path("plain_text.emlx");
        if !path.exists() {
            return;
        }

        let cutoff = utc_ts("2025-01-01T00:00:00Z");
        assert!(scan_with(&path, "rust programming", None, Some(cutoff)).is_some());
    }

    #[test]
    fn test_process_emlx_file_dateless_falls_back_to_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no_date.emlx");
        // Byte-count line, then a message with no Date: header at all.
        std::fs::write(
            &path,
            emlx("From: sender@example.com\nSubject: No date here\n\nquarterly report body\n"),
        )
        .unwrap();

        let cutoff = utc_ts("2026-07-01T00:00:00Z");
        let recent_mtime = utc_ts("2026-07-27T12:00:00Z");
        let old_mtime = utc_ts("2026-01-01T12:00:00Z");

        // The file passed the mtime prefilter, so "date unknown but recent" is kept.
        let kept = scan_with(&path, "quarterly report", Some(recent_mtime), Some(cutoff));
        assert_eq!(kept.map(|r| r.timestamp), Some(Some(recent_mtime)));

        // An mtime outside the window is still rejected.
        assert!(scan_with(&path, "quarterly report", Some(old_mtime), Some(cutoff)).is_none());

        // Without any date information there is nothing to place it in the window.
        assert!(scan_with(&path, "quarterly report", None, Some(cutoff)).is_none());
    }

    #[test]
    fn test_process_emlx_file_single_line_does_not_panic() {
        // A truncated .emlx with no newline at all: the byte-count line is the
        // whole file. Must return None rather than panicking on a bad slice.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.emlx");
        std::fs::write(&path, "1234").unwrap();
        assert!(scan(&path, "anything").is_none());

        // Newline as the final byte leaves an empty message body.
        let empty_body = dir.path().join("empty_body.emlx");
        std::fs::write(&empty_body, "1234\n").unwrap();
        assert!(scan(&empty_body, "anything").is_none());
    }

    #[test]
    fn test_process_emlx_file_non_utf8_is_searchable() {
        // Shift_JIS encoded body: reading as a String used to drop these silently.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sjis.emlx");
        let mut bytes = b"100\nFrom: sender@example.com\nSubject: SJIS\n\
             Content-Type: text/plain; charset=Shift_JIS\n\n"
            .to_vec();
        // "会議" in Shift_JIS.
        bytes.extend_from_slice(&[0x89, 0xEF, 0x8B, 0x63]);
        bytes.push(b'\n');
        std::fs::write(&path, &bytes).unwrap();

        let result = scan(&path, "会議");
        assert!(
            result.is_some(),
            "Shift_JIS body should be decoded and matched"
        );
    }
}
