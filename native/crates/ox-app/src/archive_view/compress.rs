// SPDX-License-Identifier: AGPL-3.0-only
//! The name of a new ZIP, and what compressing reports (ARC-023).
//!
//! Compress to ZIP file is new in the native app, from the Dolphin
//! baseline and Explorer's "Compress to ZIP file": the selection goes into
//! a new ZIP beside it, named after the first selected item (`Photos` or
//! `report.txt` give `Photos.zip` and `report.zip`), then `Photos (2).zip`
//! and so on while a name is taken. The archive is written privately and
//! published under its name without replacing anything.

use ox_core::archive::{ArchiveError, CreatedArchive, LeftOutReason};

/// The title of the dialog of a failed compression.
pub(crate) const COMPRESSION_STOPPED: &str = crate::i18n::message_id("Compression stopped");

/// How many names Compress tries before it gives up.
const MAX_NAME_TRIES: u32 = 100;

/// The ending of the archive Compress to › ZIP File writes.
pub(crate) const ZIP_ENDING: &str = ".zip";
/// The ending of the archive Compress to › TAR File writes: a plain TAR.
pub(crate) const TAR_ENDING: &str = ".tar";
/// The ending of the archive Compress to › 7z File writes.
pub(crate) const SEVEN_ZIP_ENDING: &str = ".7z";

/// The archive names tried for a selection whose first item is
/// `first_name`, with `ending` (`.zip` or `.tar`), in order:
/// `Photos.zip`, `Photos (2).zip`, …
pub(crate) fn compressed_file_name<'a>(
    first_name: &'a str,
    ending: &'a str,
) -> impl Iterator<Item = String> + 'a {
    let stem = file_stem(first_name);
    let first = std::iter::once(format!("{stem}{ending}"));
    let numbered = (2..=MAX_NAME_TRIES).map(move |number| format!("{stem} ({number}){ending}"));
    first.chain(numbered)
}

/// `report` for `report.txt`, `Photos` for a folder `Photos`, `.bashrc`
/// for `.bashrc`: the name without its last extension.
fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

/// The text of "Compression stopped": the reason, then that nothing was
/// replaced.
pub(crate) fn compression_failure_text(error: &ArchiveError) -> String {
    ox_core::i18n::format_message(
        "{error}\n\nNo existing file was replaced.",
        &[("error", &error.to_string())],
    )
}

/// The title of the message after a compression that left items out.
pub(crate) const ITEMS_LEFT_OUT: &str = crate::i18n::message_id("Some items were not compressed");

/// How many left-out items the message names before it counts the rest.
const MAX_NAMED_LEFT_OUT: usize = 20;

/// The message after compressing, when the archive lacks items the user
/// selected: it names each one and why, as Windows' Compressed (zipped)
/// Folder names a file it could not add, rather than leaving an
/// incomplete archive without a word. `None` when nothing was left out.
pub(crate) fn left_out_text(archive: &CreatedArchive) -> Option<String> {
    if archive.left_out.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = archive
        .left_out
        .iter()
        .take(MAX_NAMED_LEFT_OUT)
        .map(|item| {
            ox_core::i18n::format_message(
                "{name} ({reason})",
                &[("name", &item.name), ("reason", &left_out_reason(item.reason))],
            )
        })
        .collect();
    let more = archive.left_out.len().saturating_sub(MAX_NAMED_LEFT_OUT);
    if more > 0 {
        lines.push(ox_core::i18n::format_message(
            "…and {count} more.",
            &[("count", &more.to_string())],
        ));
    }
    Some(ox_core::i18n::format_message(
        "{name} was created without these items:\n\n{items}",
        &[("name", &archive.name), ("items", &lines.join("\n"))],
    ))
}

/// Why an item is not in the archive, in a few words.
fn left_out_reason(reason: LeftOutReason) -> String {
    match reason {
        LeftOutReason::SpecialFile => ox_core::i18n::gettext("a special file, which holds no data"),
        LeftOutReason::TargetMissing => ox_core::i18n::gettext("a link whose target cannot be found"),
        LeftOutReason::Loop => ox_core::i18n::gettext("a link to a folder that holds it"),
        LeftOutReason::RemoteLink => ox_core::i18n::gettext("a link on a server or device"),
        LeftOutReason::UnreadableName => ox_core::i18n::gettext("a name that is not valid text"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-023
    #[test]
    fn the_zip_is_named_after_the_first_item_then_numbered() {
        let for_file: Vec<String> = compressed_file_name("report.txt", ZIP_ENDING).take(2).collect();
        let for_folder: Vec<String> = compressed_file_name("Photos", ZIP_ENDING).take(1).collect();
        let for_dot_file: Vec<String> = compressed_file_name(".bashrc", ZIP_ENDING).take(1).collect();
        let tar: Vec<String> = compressed_file_name("Photos", TAR_ENDING).take(2).collect();
        assert_eq!(tar, ["Photos.tar", "Photos (2).tar"]);

        assert_eq!(for_file, ["report.zip", "report (2).zip"]);
        assert_eq!(for_folder, ["Photos.zip"]);
        assert_eq!(for_dot_file, [".bashrc.zip"]);
    }
}
