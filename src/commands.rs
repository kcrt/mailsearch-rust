//! The `dump` and `attachments` subcommands: working with a message you have
//! already found, rather than finding one.
//!
//! Both take the same kind of target — an `.emlx` path or a `Message-ID` — so
//! the output of a search pipes straight into either:
//!
//! ```text
//! mailsearch --tsv --days 7 --from yamada | cut -f5 | xargs mailsearch dump
//! ```
//!
//! Everything here stays offline. Apple Mail is never asked for anything, which
//! matters because scripting it blocks its interface while it answers, and a
//! message whose content is still on the server simply reports that rather than
//! triggering a download. Fetching such content is a separate job for a separate
//! tool.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::message::{strip_quote, Attachment, Message};
use crate::models::{normalise_message_id, Filters};
use crate::search::search_messages;

/// Resolve one target to a file on disk.
///
/// A path is used as given. Anything else is treated as a `Message-ID` and
/// looked up by scanning, because the Envelope Index stores a hash of the id
/// rather than the id itself and cannot be asked. Scanning is why `--days`
/// exists on both subcommands: it is the difference between reading a few
/// thousand files and a quarter of a million.
pub fn resolve_target(target: &str, mail_root: &Path, days: Option<u32>) -> Result<PathBuf> {
    let path = Path::new(target);
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    if !looks_like_message_id(target) {
        bail!("no such file, and not a Message-ID: {target}");
    }
    let wanted = normalise_message_id(target);
    // Matched as a filter rather than a text query: `Message-ID` is not one of
    // the headers a query is compared against, so querying for it would never
    // match. The filter is also checked before the body is extracted, so the
    // scan reads headers only.
    let filters = Filters {
        message_id: Some(wanted.clone()),
        ..Filters::default()
    };
    let window = days.map(crate::timewindow::window_now);
    let outcome = search_messages(mail_root, &[], &filters, usize::MAX, window, true);
    outcome
        .results
        .first()
        .map(|result| PathBuf::from(&result.file_path))
        .with_context(|| match days {
            Some(days) => format!("<{wanted}> not found in the last {days} days"),
            None => format!("<{wanted}> not found"),
        })
}

/// Whether a target that is not a path could be a `Message-ID`.
fn looks_like_message_id(target: &str) -> bool {
    target.contains('@') && !target.contains('/')
}

/// Render one attachment as a listing line.
fn attachment_line(attachment: &Attachment) -> String {
    let size = match attachment.size {
        Some(size) => format!("{} bytes", thousands(size)),
        None => "size unknown".to_string(),
    };
    let mut notes = vec![attachment.mimetype.clone(), size];
    if attachment.inline {
        notes.push("inline".to_string());
    }
    if !attachment.downloaded {
        // Said plainly, because the previous tooling reported these as "0 bytes"
        // and an empty attachment reads as a real one.
        notes.push("NOT DOWNLOADED".to_string());
    }
    format!("  {}  ({})", attachment.name, notes.join(", "))
}

/// Group digits for readability: `3265430` -> `3,265,430`.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Print a message as text: headers, attachment list, then the body.
pub fn dump(path: &Path, headers_only: bool, strip_quotes: bool, prefer_html: bool) -> Result<()> {
    let message = Message::load(path)?;
    println!("===== {} =====", path.display());
    for (name, value) in message.display_headers() {
        println!("{name}: {value}");
    }

    let attachments = message.attachments();
    if !attachments.is_empty() {
        println!("--- attachments ---");
        for attachment in &attachments {
            println!("{}", attachment_line(attachment));
        }
    }
    if headers_only {
        return Ok(());
    }

    println!("--- body ---");
    match message.body_text(prefer_html) {
        Some(body) => println!(
            "{}",
            if strip_quotes {
                strip_quote(&body)
            } else {
                body
            }
        ),
        // Distinguished from an empty message: for a `.partial.emlx` the body is
        // on the server, and saying so is more use than printing nothing.
        None => eprintln!("  [no body on disk — Apple Mail has not downloaded it]"),
    }
    Ok(())
}

/// One message's attachments as `--json` reports them.
///
/// Carries what a follow-up tool needs to act: which file each attachment came
/// from, where it was written, and — the point of the whole thing — which are
/// still on the server and therefore need Apple Mail.
#[derive(serde::Serialize)]
pub struct AttachmentReport {
    pub path: String,
    /// The message's `Message-ID`, so a caller can find it in Apple Mail without
    /// re-parsing the file.
    pub message_id: Option<String>,
    pub attachments: Vec<ReportedAttachment>,
}

#[derive(serde::Serialize)]
pub struct ReportedAttachment {
    #[serde(flatten)]
    pub attachment: Attachment,
    /// Where it was written, when `--save` was given and the bytes were on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_to: Option<String>,
}

/// List a message's attachments, or write them to `save_to`.
///
/// Returns a report per message; the caller prints it as JSON or as a listing.
/// Inline parts are dropped here unless asked for, so neither output has to
/// filter again.
pub fn attachments(
    path: &Path,
    save_to: Option<&Path>,
    include_inline: bool,
) -> Result<AttachmentReport> {
    let message = Message::load(path)?;

    // Listing never decodes: a message with a 30 MB attachment should report its
    // name instantly.
    let entries: Vec<(Attachment, Option<Vec<u8>>)> = match save_to {
        None => message
            .attachments()
            .into_iter()
            .map(|attachment| (attachment, None))
            .collect(),
        Some(_) => message.attachment_payloads(),
    };

    let mut reported = Vec::new();
    for (attachment, payload) in entries {
        if attachment.inline && !include_inline {
            continue;
        }
        let saved_to = match (save_to, payload) {
            (Some(directory), Some(bytes)) => {
                let target = unique_path(directory, &safe_file_name(&attachment.name));
                std::fs::write(&target, bytes)
                    .with_context(|| format!("cannot write {}", target.display()))?;
                Some(target.display().to_string())
            }
            // Either a plain listing, or a part whose bytes are on the server.
            _ => None,
        };
        reported.push(ReportedAttachment {
            attachment,
            saved_to,
        });
    }

    Ok(AttachmentReport {
        path: path.display().to_string(),
        message_id: crate::email::extract_message_id(&message.parsed()),
        attachments: reported,
    })
}

/// Print an attachment report the way a person reads it.
pub fn print_attachment_report(report: &AttachmentReport, saving: bool) {
    let path = Path::new(&report.path);
    println!(
        "● {}",
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
    );
    let mut pending = 0;
    for reported in &report.attachments {
        if !reported.attachment.downloaded {
            pending += 1;
        }
        match &reported.saved_to {
            Some(target) => println!(
                "  saved: {target}  ({} bytes)",
                thousands(reported.attachment.size.unwrap_or(0))
            ),
            None if saving => {
                eprintln!("  [skipped] {} — not downloaded", reported.attachment.name)
            }
            None => println!("{}", attachment_line(&reported.attachment)),
        }
    }
    if pending > 0 {
        eprintln!(
            "  {pending} attachment(s) are still on the server. \
             Apple Mail has to fetch those; this tool does not drive it."
        );
    }
    if report.attachments.is_empty() {
        eprintln!("  no attachments");
    }
}

/// Reduce a sender-supplied filename to something safe to write.
///
/// Only the last path component is kept. The name comes from the message, so it
/// must not be able to choose where the file lands, and the leading components
/// carry nothing worth keeping — `../../etc/passwd` is `passwd`. Backslashes
/// count as separators too, since a Windows mailer writes them that way and the
/// platform's own path handling would not split on them.
fn safe_file_name(name: &str) -> String {
    let normalised = name.replace('\\', "/");
    let base = normalised.rsplit('/').next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|ch| if (ch as u32) < 0x20 { '_' } else { ch })
        .collect();
    // A leading dot would hide the file; trailing space is a nuisance on any
    // platform the directory might later be copied to.
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned.to_string()
    }
}

/// A path in `directory` that does not exist yet, suffixing `_2`, `_3`, … .
///
/// Never overwrites: two messages in one run routinely carry the same
/// `image001.png` or `様式2 研究計画書ひな形.docx`.
fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    let extension = extension.unwrap_or_default();
    for index in 2.. {
        let candidate = directory.join(format!("{stem}_{index}{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("the loop returns once a free name is found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(3_265_430), "3,265,430");
    }

    #[test]
    fn a_message_id_is_recognised_without_brackets() {
        assert!(looks_like_message_id("abc@example.com"));
        assert!(looks_like_message_id("<abc@example.com>"));
    }

    #[test]
    fn a_path_is_not_mistaken_for_a_message_id() {
        // Mail directories contain `@` nowhere, but a user-supplied path might.
        assert!(!looks_like_message_id("/Users/a/Messages/1.emlx"));
        assert!(!looks_like_message_id("some/dir@x/1.emlx"));
    }

    #[test]
    fn brackets_do_not_affect_message_id_comparison() {
        assert_eq!(
            normalise_message_id("<abc@example.com>"),
            normalise_message_id(" abc@example.com ")
        );
    }

    #[test]
    fn a_filename_cannot_escape_the_destination() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("/absolute/path.pdf"), "path.pdf");
        assert_eq!(safe_file_name("a\\b.pdf"), "b.pdf");
    }

    #[test]
    fn a_leading_dot_is_stripped_so_files_are_not_hidden() {
        assert_eq!(safe_file_name(".hidden.pdf"), "hidden.pdf");
    }

    #[test]
    fn an_unusable_filename_gets_a_placeholder() {
        assert_eq!(safe_file_name("   "), "attachment");
        assert_eq!(safe_file_name("///"), "attachment");
        assert_eq!(safe_file_name("..."), "attachment");
    }

    #[test]
    fn a_japanese_filename_is_left_alone() {
        assert_eq!(
            safe_file_name("新規260416 倫2026-002 ①様式0 臨床研究実施申請書.docx"),
            "新規260416 倫2026-002 ①様式0 臨床研究実施申請書.docx"
        );
    }

    #[test]
    fn a_second_file_of_the_same_name_gets_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.pdf"), b"x").unwrap();
        assert_eq!(unique_path(dir.path(), "a.pdf"), dir.path().join("a_2.pdf"));
    }

    #[test]
    fn suffixes_keep_counting_past_the_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.pdf"), b"x").unwrap();
        std::fs::write(dir.path().join("a_2.pdf"), b"x").unwrap();
        assert_eq!(unique_path(dir.path(), "a.pdf"), dir.path().join("a_3.pdf"));
    }

    #[test]
    fn a_free_name_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.pdf"), dir.path().join("a.pdf"));
    }

    #[test]
    fn an_extensionless_name_still_gets_a_suffix() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "README"),
            dir.path().join("README_2")
        );
    }

    #[test]
    fn an_undownloaded_attachment_says_so_in_the_listing() {
        let attachment = Attachment {
            name: "big.zip".to_string(),
            mimetype: "application/zip".to_string(),
            size: Some(3_265_430),
            inline: false,
            downloaded: false,
        };
        let line = attachment_line(&attachment);
        assert!(line.contains("3,265,430 bytes"), "{line}");
        assert!(line.contains("NOT DOWNLOADED"), "{line}");
    }

    #[test]
    fn an_ordinary_attachment_line_is_name_type_and_size() {
        let attachment = Attachment {
            name: "report.pdf".to_string(),
            mimetype: "application/pdf".to_string(),
            size: Some(1_234),
            inline: false,
            downloaded: true,
        };
        assert_eq!(
            attachment_line(&attachment),
            "  report.pdf  (application/pdf, 1,234 bytes)"
        );
    }
}
