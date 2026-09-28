// SPDX-License-Identifier: AGPL-3.0-only
//! ZIP archives in the window: browsing one read-only, extracting it into
//! a new folder, and compressing a selection into a new ZIP.
//!
//! Ports `archiveDialog`, `extractDialog` and `zipOutputName` of
//! `desktop/ui/app.js` over ox-core's [`archive`](ox_core::archive)
//! service, which ports `desktop/archives.py` and
//! `desktop/zip_extraction.py` with every safety rule: nothing is
//! decompressed to browse, only the member the user opens is copied (to a
//! private read-only file), and an extraction checks every member first,
//! builds the new folder privately and publishes it all or nothing, so
//! zip-slip paths, links, bombs and name clashes never reach the disk.
//!
//! Beyond the Python app, from the Dolphin baseline: Extract here, without
//! a dialog (ARC-025), and Compress to ZIP file (ARC-023).
//!
//! | Module | Responsibility |
//! |---|---|
//! | `browser` | The "Compressed folder" dialog: [`ArchiveBrowserView`] |
//! | `extract_dialog` | The "Extract compressed folder" dialog |
//! | `extraction` | Running an extraction and the texts that report it |
//! | `operation_panel` | [`OperationPanel`]: progress and Cancel of a running extraction or compression |
//! | `compress` | The name of a new ZIP and the texts that report compressing |

mod browser;
mod compress;
mod extract_dialog;
mod extraction;
mod operation_panel;

use ox_core::archive::{ArchiveEntry, ArchiveEntryKind};

use crate::icons::Art;

#[cfg(test)]
pub(crate) use browser::ArchiveBrowserView;
pub(crate) use browser::{archive_dialog, ArchiveDialogActions};
pub(crate) use compress::{
    compressed_file_name, compression_failure_text, compression_success_text, COMPRESSION_STOPPED,
};
pub(crate) use extract_dialog::{extract_dialog, ExtractDialogSetup, ExtractionChoice};
pub(crate) use extraction::{
    extraction_failure_text, extraction_success_text, unique_folder_names, EXTRACTION_STOPPED,
};
pub(crate) use operation_panel::OperationPanel;

/// A ZIP archive the user acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveTarget {
    /// The archive's URI.
    pub uri: String,
    /// The archive's file name.
    pub name: String,
}

/// The picture of an archive row: a folder, or the file's type
/// (`fileIcon(item, 25)`).
fn archive_art(entry: &ArchiveEntry) -> Art {
    match entry.kind {
        ArchiveEntryKind::Folder => Art::Folder,
        ArchiveEntryKind::File { .. } => Art::for_file_name(&entry.name),
    }
}
