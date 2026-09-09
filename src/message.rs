//! Reading one message: its headers, its attachments, and its body as text.
//!
//! The search path only ever asks "does this match?", and [`crate::email`]
//! shapes its text extraction for that — every text part concatenated, HTML
//! flattened, whitespace collapsed. That is the wrong shape for showing a
//! message to a person, where the paragraph breaks *are* the content. This
//! module is the other half: it reads a single message for display.
//!
//! Character encoding is `mailparse`'s job and it does it well. Japanese mail
//! from Outlook arrives as ISO-2022-JP carrying NEC/IBM extension characters —
//! `髙`, `①`, `㈱` — that most decoders cannot represent; the encoding
//! behind `mailparse` handles them, in both body text and attachment filenames.
//! A filename Outlook writes as an RFC 2047 encoded word inside a quoted string
//! (which the standard does not allow, but Outlook does anyway) comes back
//! decoded, so nothing here has to unpick it.

use crate::email::rfc822_slice;
use anyhow::{Context, Result};
use mailparse::{MailHeaderMap, ParsedMail};
use std::path::{Path, PathBuf};

/// Header that Apple Mail puts on a part whose content is still on the server.
///
/// This is the reliable marker for "not downloaded yet". Judging by an empty
/// payload instead would also flag a genuinely empty attachment, and judging by
/// the `.partial.emlx` filename says nothing about *which* part is missing —
/// a message can have one attachment on disk and another still on the server.
const APPLE_CONTENT_LENGTH: &str = "X-Apple-Content-Length";

/// Headers shown when a message is displayed, in this order.
const DISPLAY_HEADERS: [&str; 6] = ["Date", "From", "To", "Cc", "Subject", "Message-ID"];

/// One attachment, as described by the message structure alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mimetype: String,
    /// Size in bytes: the decoded payload when it is on disk, otherwise the
    /// size the sender or Apple Mail declared. `None` when neither is known.
    pub size: Option<usize>,
    /// An embedded part — a signature logo, an image the HTML body references —
    /// rather than a file the sender attached.
    pub inline: bool,
    /// Whether the content is on disk. `false` means Apple Mail still has it on
    /// the server; the name and size are known but the bytes are not.
    pub downloaded: bool,
}

/// A single message read from an `.emlx` file.
pub struct Message {
    pub path: PathBuf,
    raw: Vec<u8>,
}

/// Shows the path and size rather than the bytes: a derived `Debug` would dump
/// the whole message into any failing assertion that mentions one.
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("path", &self.path)
            .field("bytes", &self.raw.len())
            .finish()
    }
}

impl Message {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        // Parsed once here purely to fail early with a good message; the borrow
        // cannot be stored alongside the bytes it borrows from.
        Self::parse_bytes(&raw).with_context(|| format!("cannot parse {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            raw,
        })
    }

    fn parse_bytes(raw: &[u8]) -> Result<ParsedMail<'_>> {
        let slice =
            rfc822_slice(raw).context("not an .emlx file (no newline in the first line)")?;
        Ok(mailparse::parse_mail(slice)?)
    }

    /// The parsed message. Cheap enough to re-parse per accessor, and it keeps
    /// callers from having to thread a lifetime through everything.
    pub fn parsed(&self) -> ParsedMail<'_> {
        Self::parse_bytes(&self.raw).expect("already parsed successfully in load()")
    }

    /// `(name, value)` for the headers worth displaying, in [`DISPLAY_HEADERS`]
    /// order, skipping those the message does not carry.
    pub fn display_headers(&self) -> Vec<(&'static str, String)> {
        let mail = self.parsed();
        DISPLAY_HEADERS
            .iter()
            .filter_map(|name| {
                mail.headers
                    .get_first_header(name)
                    .map(|header| (*name, crate::email::clean_header_value(&header.get_value())))
            })
            .collect()
    }

    /// Every attachment in the message, outermost part first.
    pub fn attachments(&self) -> Vec<Attachment> {
        let mut out = Vec::new();
        collect_attachments(&self.parsed(), false, &mut out);
        out.into_iter().map(|(attachment, _)| attachment).collect()
    }

    /// The same, each paired with its decoded bytes where they are on disk.
    ///
    /// Separate from [`Message::attachments`] because decoding is what costs:
    /// listing a message with a 30 MB attachment should not decode it.
    pub fn attachment_payloads(&self) -> Vec<(Attachment, Option<Vec<u8>>)> {
        let mut out = Vec::new();
        collect_attachments(&self.parsed(), true, &mut out);
        out
    }

    /// The message body as readable text.
    ///
    /// Prefers `text/plain` and returns it as the sender wrote it — no
    /// whitespace normalising, because the line breaks are the paragraphs. Falls
    /// back to converting the HTML part when there is no plain one, and
    /// `prefer_html` swaps that order for mail whose plain part is a stub.
    ///
    /// `None` when the message has no text part at all, which for a
    /// `.partial.emlx` usually means the body is still on the server.
    pub fn body_text(&self, prefer_html: bool) -> Option<String> {
        let mail = self.parsed();
        let mut plain = None;
        let mut html = None;
        collect_body(&mail, &mut plain, &mut html);
        let (first, second) = if prefer_html {
            (html.map(|h| html_to_text(&h)), plain)
        } else {
            (plain, html.map(|h| html_to_text(&h)))
        };
        // Each candidate is checked for emptiness on its own, so a sender that
        // ships a blank `text/plain` next to the real HTML body falls through to
        // it rather than reporting the message as having no body.
        let non_empty = |text: String| (!text.trim().is_empty()).then_some(text);
        first
            .and_then(non_empty)
            .or_else(|| second.and_then(non_empty))
    }
}

/// Decode any RFC 2047 encoded words left in a MIME parameter value.
///
/// `mailparse` decodes a plain `filename="=?iso-2022-jp?B?…?="`, but not one
/// Outlook splits across RFC 2231 continuations:
///
/// ```text
/// filename*0="=?iso-2022-jp?B?GyRCPzc1LBsoQjI2MDQxNSAbJEJOURsoQjIwMjYtMDAxIBsk";
/// filename*1="QjszGyhC?=  =?iso-2022-jp?B?GyRCRURCQE86GyhCKBskQj9XQi4bKEIpGyRC";
/// filename*2="M1gycRsoQi56aXA=?=";
/// ```
///
/// The base64 is split mid-word, so the segments have to be joined before
/// anything can be decoded — which `mailparse` does — and only then decoded,
/// which it does not. Without this pass the name is shown as its own encoding.
///
/// The decoding itself is handed back to `mailparse` by wrapping the value in a
/// synthetic header, rather than reimplemented: encoded-word parsing is fiddly,
/// and the charset support behind it is the reason NEC/IBM extension characters
/// survive at all.
pub fn decode_encoded_words(value: &str) -> String {
    if !value.contains("=?") {
        return value.to_string();
    }
    let synthetic = format!("X: {value}");
    mailparse::parse_header(synthetic.as_bytes())
        .map(|(header, _)| header.get_value())
        .unwrap_or_else(|_| value.to_string())
}

/// Walk the message, collecting the parts that are attachments.
///
/// `with_payload` decides whether each part's content is decoded as well; see
/// [`Message::attachment_payloads`].
fn collect_attachments(
    part: &ParsedMail<'_>,
    with_payload: bool,
    out: &mut Vec<(Attachment, Option<Vec<u8>>)>,
) {
    if part.ctype.mimetype.to_lowercase().starts_with("multipart/") {
        for subpart in &part.subparts {
            collect_attachments(subpart, with_payload, out);
        }
        return;
    }
    let disposition = part.get_content_disposition();
    let name = disposition
        .params
        .get("filename")
        .or_else(|| part.ctype.params.get("name"));
    let Some(name) = name else { return };

    // Apple only writes this header on a part it has not fetched, so its
    // presence is the answer; its value is the encoded length, which is why the
    // declared `size` parameter is preferred for display.
    let pending = part.headers.get_first_value(APPLE_CONTENT_LENGTH);
    let downloaded = pending.is_none();
    let declared = disposition
        .params
        .get("size")
        .and_then(|s| s.parse::<usize>().ok());
    // Decoding is needed for the exact size as well as for saving, so a listing
    // that does not want the bytes falls back to the declared size instead.
    let payload = (downloaded && with_payload)
        .then(|| part.get_body_raw().ok())
        .flatten();
    let size = if downloaded {
        match &payload {
            Some(bytes) => Some(bytes.len()),
            None => declared.or_else(|| part.get_body_raw().ok().map(|b| b.len())),
        }
    } else {
        declared.or_else(|| pending.and_then(|v| v.trim().parse().ok()))
    };

    out.push((
        Attachment {
            name: decode_encoded_words(name),
            mimetype: part.ctype.mimetype.clone(),
            size,
            inline: is_inline(part),
            downloaded,
        },
        payload,
    ));
}

/// Whether a named part is embedded in the message rather than attached to it.
///
/// Mirrors the reasoning in `crate::email::part_is_attachment`, which is what
/// `--has-attachment` uses; the two must agree or a message could be found by a
/// filter and then listed as having nothing.
fn is_inline(part: &ParsedMail<'_>) -> bool {
    let disposition = part.get_content_disposition();
    if matches!(
        disposition.disposition,
        mailparse::DispositionType::Attachment
    ) {
        return false;
    }
    // `mailparse` reports `Inline` both for an explicit header and for no header
    // at all, so the header's presence has to be checked separately.
    if part
        .headers
        .get_first_header("Content-Disposition")
        .is_some()
    {
        return true;
    }
    // No disposition at all: a `Content-ID` means the HTML body references it as
    // `cid:`, which is how older Outlook and Word mark a signature image.
    part.headers.get_first_header("Content-ID").is_some()
}

/// Find the first plain and the first HTML body part.
///
/// Attachments are skipped, so a message whose only HTML is an attached `.html`
/// file does not have it mistaken for the body.
fn collect_body(part: &ParsedMail<'_>, plain: &mut Option<String>, html: &mut Option<String>) {
    if matches!(
        part.get_content_disposition().disposition,
        mailparse::DispositionType::Attachment
    ) {
        return;
    }
    let mimetype = part.ctype.mimetype.to_lowercase();
    if mimetype.starts_with("multipart/") {
        for subpart in &part.subparts {
            collect_body(subpart, plain, html);
        }
    } else if mimetype == "text/plain" && plain.is_none() {
        *plain = part.get_body().ok();
    } else if mimetype == "text/html" && html.is_none() {
        *html = part.get_body().ok();
    }
}

/// Convert an HTML body to text, keeping the line structure.
///
/// Deliberately not `crate::email::strip_html_tags`, which collapses every run
/// of whitespace into one space. That is right for matching a query and wrong
/// for reading: it turns a message into a single paragraph.
fn html_to_text(html: &str) -> String {
    let mut text = crate::email::strip_html_blocks(html);
    for (pattern, replacement) in [
        (r"(?i)<br\s*/?>", "\n"),
        (r"(?i)</(p|div|tr|li|h[1-6])\s*>", "\n"),
        (r"(?i)</t[dh]\s*>", "\t"),
    ] {
        text = regex::Regex::new(pattern)
            .expect("static pattern")
            .replace_all(&text, replacement)
            .into_owned();
    }
    text = regex::Regex::new("<[^>]+>")
        .expect("static pattern")
        .replace_all(&text, "")
        .into_owned();
    text = unescape_entities(&text);
    // Collapse runs of blank lines, but keep single ones: HTML mail is full of
    // layout newlines that would otherwise leave the text full of holes.
    regex::Regex::new(r"\n[ \t]*(\n[ \t]*)+")
        .expect("static pattern")
        .replace_all(&text, "\n\n")
        .trim()
        .to_string()
}

/// Resolve the HTML entities that actually turn up in mail bodies.
fn unescape_entities(text: &str) -> String {
    let mut out = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    // Last, so an entity written `&amp;lt;` survives as the text `&lt;`.
    out = out.replace("&amp;", "&");
    out
}

/// Markers that begin a quoted reply or the signature after it.
///
/// Anything from the first match onwards is what someone else wrote, or a
/// signature block, and is dropped by [`strip_quote`].
fn quote_markers() -> &'static [regex::Regex] {
    static MARKERS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    MARKERS.get_or_init(|| {
        [
            r"^>",                    // the universal quote prefix
            r"^-{3,}\s*$",            // Outlook's "-----Original Message-----" rule
            r"^_{5,}\s*$",            // Outlook's other divider
            r"^\s*差出人:\s",         // Japanese Outlook's quoted header block
            r"^\s*From:\s.*\bwrote:", // some webmail clients
            r"^On .+ wrote:\s*$",     // Apple Mail, Gmail
            r"^.{0,40}のメール:\s*$", // Japanese Apple Mail
        ]
        .iter()
        .map(|p| regex::Regex::new(p).expect("static pattern"))
        .collect()
    })
}

/// Drop the quoted reply and the signature, leaving what this sender wrote.
///
/// A blunt instrument by nature — there is no reliable marker, only conventions
/// — so it is opt-in rather than the default.
pub fn strip_quote(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        // "-- " on its own is the RFC 3676 signature separator.
        if line.trim_end() == "--" && line.starts_with("--") {
            break;
        }
        if quote_markers().iter().any(|marker| marker.is_match(line)) {
            break;
        }
        kept.push(line);
    }
    kept.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write an `.emlx` with a correct byte-count line and the trailing plist.
    fn write_emlx(dir: &Path, name: &str, message: &str) -> PathBuf {
        let path = dir.join(name);
        let plist = "<?xml version=\"1.0\"?>\n<plist><dict/></plist>\n";
        std::fs::write(&path, format!("{}\n{message}{plist}", message.len())).unwrap();
        path
    }

    fn load(message: &str) -> (tempfile::TempDir, Message) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_emlx(dir.path(), "m.emlx", message);
        let loaded = Message::load(&path).unwrap();
        (dir, loaded)
    }

    #[test]
    fn headers_come_back_in_display_order() {
        let (_dir, message) = load(concat!(
            "Subject: hello\n",
            "From: a@example.com\n",
            "Date: Tue, 20 Jan 2026 10:30:00 +0000\n",
            "\nbody\n"
        ));
        let names: Vec<&str> = message.display_headers().iter().map(|(n, _)| *n).collect();
        // Date before From before Subject, regardless of the order in the file.
        assert_eq!(names, ["Date", "From", "Subject"]);
    }

    #[test]
    fn a_missing_header_is_skipped_not_blank() {
        let (_dir, message) = load("Subject: hello\n\nbody\n");
        assert_eq!(
            message.display_headers(),
            [("Subject", "hello".to_string())]
        );
    }

    #[test]
    fn the_plain_body_keeps_its_line_breaks() {
        // The whole reason this module exists rather than reusing the search
        // path's extraction, which would collapse this to one line.
        let (_dir, message) = load("Content-Type: text/plain\n\nfirst\n\nsecond\n");
        assert_eq!(message.body_text(false).unwrap(), "first\n\nsecond\n");
    }

    #[test]
    fn the_trailing_plist_stays_out_of_the_body() {
        let (_dir, message) = load("Content-Type: text/plain\n\nhello\n");
        assert!(!message.body_text(false).unwrap().contains("plist"));
    }

    #[test]
    fn plain_is_preferred_over_html() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/alternative; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nplain version\n",
            "--b\nContent-Type: text/html\n\n<p>html version</p>\n",
            "--b--\n"
        ));
        assert_eq!(message.body_text(false).unwrap().trim(), "plain version");
    }

    #[test]
    fn html_is_used_when_there_is_no_plain_part() {
        let (_dir, message) = load(concat!(
            "Content-Type: text/html\n\n",
            "<div>first line</div><div>second line</div>\n"
        ));
        assert_eq!(message.body_text(false).unwrap(), "first line\nsecond line");
    }

    #[test]
    fn prefer_html_swaps_the_order() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/alternative; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nplain version\n",
            "--b\nContent-Type: text/html\n\n<p>html version</p>\n",
            "--b--\n"
        ));
        assert_eq!(message.body_text(true).unwrap(), "html version");
    }

    #[test]
    fn an_empty_plain_part_falls_through_to_the_html() {
        // Senders that ship an empty text/plain alongside the real HTML body.
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/alternative; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\n   \n",
            "--b\nContent-Type: text/html\n\n<p>the real body</p>\n",
            "--b--\n"
        ));
        assert_eq!(message.body_text(false).unwrap(), "the real body");
    }

    #[test]
    fn an_attached_html_file_is_not_mistaken_for_the_body() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nsee attached\n",
            "--b\nContent-Type: text/html\n",
            "Content-Disposition: attachment; filename=\"page.html\"\n\n<p>not the body</p>\n",
            "--b--\n"
        ));
        assert_eq!(message.body_text(false).unwrap().trim(), "see attached");
    }

    #[test]
    fn a_message_with_no_text_part_has_no_body() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: application/pdf; name=\"a.pdf\"\n",
            "Content-Disposition: attachment; filename=\"a.pdf\"\n\nJVBER\n",
            "--b--\n"
        ));
        assert!(message.body_text(false).is_none());
    }

    #[test]
    fn attachments_are_listed_with_type_and_size() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf; name=\"report.pdf\"\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\n\nJVBERi0x\n",
            "--b--\n"
        ));
        let attachments = message.attachments();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "report.pdf");
        assert_eq!(attachments[0].mimetype, "application/pdf");
        assert!(attachments[0].downloaded);
        assert!(!attachments[0].inline);
        assert!(attachments[0].size.is_some_and(|size| size > 0));
    }

    #[test]
    fn an_undownloaded_attachment_is_flagged_and_keeps_its_declared_size() {
        // What every `.partial.emlx` looks like: the structure is on disk, the
        // bytes are not. Judging by an empty payload would call this a 0-byte
        // attachment, which is what the previous tooling reported.
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/zip; name=\"big.zip\"\n",
            "Content-Disposition: attachment; size=\"3265430\"; filename=\"big.zip\"\n",
            "X-Apple-Content-Length: 4411197\n\n",
            "--b--\n"
        ));
        let attachments = message.attachments();
        assert_eq!(attachments.len(), 1);
        assert!(!attachments[0].downloaded);
        assert_eq!(attachments[0].size, Some(3_265_430));
    }

    #[test]
    fn an_inline_image_is_listed_but_marked_inline() {
        // Listing has to show everything; it is the caller that decides whether a
        // signature logo is worth saving.
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/related; boundary=b\n\n",
            "--b\nContent-Type: text/html\n\n<img src=\"cid:logo\">\n",
            "--b\nContent-Type: image/png; name=\"logo.png\"\n",
            "Content-ID: <logo>\n\niVBOR\n",
            "--b--\n"
        ));
        let attachments = message.attachments();
        assert_eq!(attachments.len(), 1);
        assert!(attachments[0].inline);
    }

    #[test]
    fn an_explicitly_inline_part_is_marked_inline() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/related; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: image/png\n",
            "Content-Disposition: inline; filename=\"image001.png\"\n\niVBOR\n",
            "--b--\n"
        ));
        assert!(message.attachments()[0].inline);
    }

    #[test]
    fn a_declared_attachment_with_a_content_id_is_not_inline() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: image/png; name=\"chart.png\"\n",
            "Content-ID: <chart>\n",
            "Content-Disposition: attachment; filename=\"chart.png\"\n\niVBOR\n",
            "--b--\n"
        ));
        assert!(!message.attachments()[0].inline);
    }

    #[test]
    fn nested_attachments_are_found() {
        // Forwarding from Apple Mail nests multipart/mixed inside
        // multipart/alternative, which a top-level-only walk misses entirely.
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/alternative; boundary=a\n\n",
            "--a\nContent-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf; name=\"deep.pdf\"\n",
            "Content-Disposition: attachment; filename=\"deep.pdf\"\n\nJVBER\n",
            "--b--\n",
            "--a--\n"
        ));
        assert_eq!(message.attachments().len(), 1);
        assert_eq!(message.attachments()[0].name, "deep.pdf");
    }

    #[test]
    fn a_message_with_no_attachments_lists_none() {
        let (_dir, message) = load("Content-Type: text/plain\n\nbody\n");
        assert!(message.attachments().is_empty());
    }

    #[test]
    fn loading_a_file_that_is_not_an_emlx_fails_with_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.emlx");
        std::fs::write(&path, "no newline here").unwrap();
        let error = Message::load(&path).unwrap_err().to_string();
        assert!(error.contains("nope.emlx"), "{error}");
    }

    #[test]
    fn a_filename_split_across_rfc2231_continuations_is_decoded() {
        // Outlook's shape: RFC 2231 segments whose joined value is an RFC 2047
        // encoded word, with the base64 split mid-word. Taken from real mail.
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/x-zip-compressed\n",
            "Content-Disposition: attachment;\n",
            "\tsize=57942;\n",
            "\tfilename*0=\"=?iso-2022-jp?B?GyRCPzc1LBsoQjI2MDQxNSAbJEJOURsoQjIwMjYtMDAxIBsk\";\n",
            "\tfilename*1=\"QjszGyhC?=  =?iso-2022-jp?B?GyRCRURCQE86GyhCKBskQj9XQi4bKEIpGyRC\";\n",
            "\tfilename*2=\"M1gycRsoQi56aXA=?=\"\n\nUEsD\n",
            "--b--\n"
        ));
        assert_eq!(
            message.attachments()[0].name,
            "新規260415 倫2026-001 山田太郎(迅速)学会.zip"
        );
    }

    #[test]
    fn a_plain_filename_is_left_alone() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\n\nJVBER\n",
            "--b--\n"
        ));
        assert_eq!(message.attachments()[0].name, "report.pdf");
    }

    #[test]
    fn a_filename_that_merely_contains_a_question_mark_is_not_mangled() {
        let (_dir, message) = load(concat!(
            "Content-Type: multipart/mixed; boundary=b\n\n",
            "--b\nContent-Type: text/plain\n\nbody\n",
            "--b\nContent-Type: application/pdf\n",
            "Content-Disposition: attachment; filename=\"what=?.pdf\"\n\nJVBER\n",
            "--b--\n"
        ));
        assert_eq!(message.attachments()[0].name, "what=?.pdf");
    }

    // ========== strip_quote ==========

    #[test]
    fn a_quote_prefix_ends_the_body() {
        assert_eq!(strip_quote("my reply\n\n> what they wrote\n"), "my reply");
    }

    #[test]
    fn the_signature_separator_ends_the_body() {
        assert_eq!(strip_quote("my reply\n-- \nKyohei\n"), "my reply");
    }

    #[test]
    fn a_japanese_outlook_quote_header_ends_the_body() {
        assert_eq!(
            strip_quote("お世話になっております。\n\n差出人: 山田 花子\n件名: Re:\n"),
            "お世話になっております。"
        );
    }

    #[test]
    fn an_apple_mail_attribution_ends_the_body() {
        assert_eq!(
            strip_quote("thanks\n\nOn 20 Jan 2026, at 10:30, A wrote:\n> hi\n"),
            "thanks"
        );
    }

    #[test]
    fn text_with_no_quote_is_returned_whole() {
        assert_eq!(
            strip_quote("just the body\nsecond line"),
            "just the body\nsecond line"
        );
    }

    #[test]
    fn a_dashed_rule_is_not_confused_with_a_signature_separator() {
        // "--" alone separates a signature; "-----" is Outlook's divider. Both
        // end the body, but a line merely containing dashes must not.
        assert_eq!(strip_quote("a -- b\nstill body"), "a -- b\nstill body");
    }

    // ========== html_to_text ==========

    #[test]
    fn block_tags_become_line_breaks() {
        assert_eq!(html_to_text("<p>one</p><p>two</p>"), "one\ntwo");
    }

    #[test]
    fn br_becomes_a_line_break() {
        assert_eq!(html_to_text("one<br>two<br/>three"), "one\ntwo\nthree");
    }

    #[test]
    fn scripts_and_styles_are_dropped_with_their_content() {
        assert_eq!(
            html_to_text("<style>p{color:red}</style><p>visible</p><script>x()</script>"),
            "visible"
        );
    }

    #[test]
    fn entities_are_resolved() {
        assert_eq!(
            html_to_text("<p>a &amp; b &lt;c&gt; &nbsp;d</p>"),
            "a & b <c>  d"
        );
    }

    #[test]
    fn a_double_escaped_entity_survives_as_text() {
        assert_eq!(html_to_text("<p>&amp;lt;</p>"), "&lt;");
    }

    #[test]
    fn runs_of_blank_lines_collapse_to_one() {
        // HTML mail is mostly layout; without this the text is full of holes.
        assert_eq!(
            html_to_text("<div>a</div><div></div><div></div><div>b</div>"),
            "a\n\nb"
        );
    }
}
