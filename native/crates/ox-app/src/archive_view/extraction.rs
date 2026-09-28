// SPDX-License-Identifier: AGPL-3.0-only
//! What an extraction reports, and the names Extract here tries.
//!
//! Ports the end of `extractDialog` in `desktop/ui/app.js` (ARC-011): a
//! failure opens "Extraction stopped" with the reason and the promise
//! that nothing was changed, and a success says how many files went into
//! which folder. Extract here (ARC-025) names the new folder after the
//! archive, then `<name> (2)`, `<name> (3)` and so on while a name is
//! taken; the extraction itself refuses a taken name, so no check can
//! race with another program.

use ox_core::archive::{ArchiveError, ExtractedFolder};

/// The title of the dialog of a failed extraction.
pub(crate) const EXTRACTION_STOPPED: &str = "Extraction stopped";

/// How many names Extract here tries before it gives up.
const MAX_NAME_TRIES: u32 = 100;

/// The text of "Extraction stopped": the reason, then that the archive
/// and existing files were not touched.
pub(crate) fn extraction_failure_text(error: &ArchiveError) -> String {
    format!("{error}\n\nThe ZIP is unchanged. Existing files were not overwritten.")
}

/// The toast after an extraction: `Extracted 3 files into Assets.`
pub(crate) fn extraction_success_text(folder: &ExtractedFolder) -> String {
    format!(
        "Extracted {} files into {}.",
        folder.summary.file_count, folder.name
    )
}

/// The folder names Extract here tries, in order: `name`, `name (2)`, …
pub(crate) fn unique_folder_names(name: &str) -> impl Iterator<Item = String> + '_ {
    let first = std::iter::once(name.to_owned());
    let numbered = (2..=MAX_NAME_TRIES).map(move |number| format!("{name} ({number})"));
    first.chain(numbered)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-025
    #[test]
    fn extract_here_tries_the_name_then_numbered_names() {
        let names: Vec<String> = unique_folder_names("Assets").take(3).collect();

        assert_eq!(names, ["Assets", "Assets (2)", "Assets (3)"]);
    }

    /// parity: ARC-011
    #[test]
    fn a_failure_says_the_zip_and_existing_files_are_untouched() {
        let text = extraction_failure_text(&ArchiveError::DestinationExists);

        assert!(
            text.ends_with("\n\nThe ZIP is unchanged. Existing files were not overwritten."),
            "{text}"
        );
    }
}
