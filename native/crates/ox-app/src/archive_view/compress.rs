// SPDX-License-Identifier: AGPL-3.0-only
//! The name of a new ZIP, and what compressing reports (ARC-023).
//!
//! Compress to ZIP file is new in the native app, from the Dolphin
//! baseline and Explorer's "Compress to ZIP file": the selection goes into
//! a new ZIP beside it, named after the first selected item (`Photos` or
//! `report.txt` give `Photos.zip` and `report.zip`), then `Photos (2).zip`
//! and so on while a name is taken. The archive is written privately and
//! published under its name without replacing anything.

use ox_core::archive::{ArchiveError, CreatedArchive};

/// The title of the dialog of a failed compression.
pub(crate) const COMPRESSION_STOPPED: &str = "Compression stopped";

/// How many names Compress tries before it gives up.
const MAX_NAME_TRIES: u32 = 100;

/// The ZIP names tried for a selection whose first item is `first_name`,
/// in order: `Photos.zip`, `Photos (2).zip`, …
pub(crate) fn compressed_file_name(first_name: &str) -> impl Iterator<Item = String> + '_ {
    let stem = file_stem(first_name);
    let first = std::iter::once(format!("{stem}.zip"));
    let numbered = (2..=MAX_NAME_TRIES).map(move |number| format!("{stem} ({number}).zip"));
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
    format!("{error}\n\nNo existing file was replaced.")
}

/// The toast after compressing: `Compressed 3 items into Photos.zip.`,
/// and how many links or special files were left out.
pub(crate) fn compression_success_text(archive: &CreatedArchive) -> String {
    let compressed = format!("Compressed {} items into {}.", archive.item_count, archive.name);
    if archive.skipped_count == 0 {
        return compressed;
    }
    format!(
        "{compressed} {} links or special files were left out.",
        archive.skipped_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-023
    #[test]
    fn the_zip_is_named_after_the_first_item_then_numbered() {
        let for_file: Vec<String> = compressed_file_name("report.txt").take(2).collect();
        let for_folder: Vec<String> = compressed_file_name("Photos").take(1).collect();
        let for_dot_file: Vec<String> = compressed_file_name(".bashrc").take(1).collect();

        assert_eq!(for_file, ["report.zip", "report (2).zip"]);
        assert_eq!(for_folder, ["Photos.zip"]);
        assert_eq!(for_dot_file, [".bashrc.zip"]);
    }
}
