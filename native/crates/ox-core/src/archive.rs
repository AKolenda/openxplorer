// SPDX-License-Identifier: AGPL-3.0-only
//! ZIP archives: browsing them read-only, opening one member as a private
//! copy, extracting them into a new folder, and compressing items into a
//! new ZIP.
//!
//! Ports `v2.0.0:desktop/archives.py`, `v2.0.0:desktop/zip_extraction.py`, the archive
//! reader of `v2.0.0:desktop/native_opening.py` and the archive branches of
//! `dispatch` in `v2.0.0:desktop/winspace.py`. The archive is never changed, and:
//!
//! - ARC-003: a listing reads the central directory only; nothing is
//!   decompressed or written.
//! - ARC-004: members with unsafe names, links and special files are never
//!   shown or opened, only counted.
//! - ARC-005: a central directory over 32 MiB or more than 100,000 members
//!   is left to an archive manager.
//! - ARC-006: only the one chosen document, image or media member is ever
//!   decompressed for viewing, into a private read-only copy.
//! - ARC-007: archives on shares are read in place through seekable GIO
//!   streams.
//! - ARC-012 to ARC-020: an extraction creates a new folder, checks every
//!   member first, builds it privately and publishes it all or nothing.
//! - ARC-023: compressing writes a new ZIP privately and publishes it
//!   without replacing anything; links are never followed.
//!
//! Every operation blocks on I/O; each has an `…_in_background` variant
//! that runs it on a GIO worker thread for the main loop to await. All of
//! them take a [`Cancellation`](crate::transfer::Cancellation) and stop
//! soon after it is cancelled.
//!
//! | Module | Responsibility | Ports |
//! |---|---|---|
//! | `browse` | Listing an archive folder | `archives.py` |
//! | `preview` | Opening one member as a private copy | `archives.py` |
//! | `copies` | Copies of members taken out of a ZIP browsed as a folder | new (Explorer) |
//! | `extract` | Checking and extracting a whole archive | `zip_extraction.py`, `winspace.py` |
//! | `create` | Compressing items into a new ZIP | new (Dolphin, Explorer) |
//! | `member_names` | Safe member names, previewable types, suggested folder names | `archives.py`, `zip_extraction.py` |
//! | `source` | Opening archives: [`ArchiveOpener`] | `archives.py`, `native_opening.py` |
//! | `gio_reader` | Reading archives on shares in place | `native_opening.py` |
//! | `zip` | The ZIP format, as Python's `zipfile` reads it | `zipfile` |
//! | `worker` | Running operations on a worker thread | `winspace.py` |
//! | `error` | [`ArchiveError`] with the app's messages | all of them |

mod browse;
mod copies;
mod create;
mod error;
mod extract;
mod gio_reader;
mod member_names;
mod preview;
mod source;
mod tar;
mod worker;
mod zip;

pub use browse::{
    default_preview_root, ArchiveBrowser, ArchiveEntry, ArchiveEntryKind, ArchiveListing, MAX_LISTED_ENTRIES,
};
pub use copies::{
    copied_member, copies_root, copy_folder_name, prepare_copies_root, remove_old_copies, COPY_LIFETIME,
};
pub use create::{CompressionRequest, CreatedArchive, ZipCompressor};
pub use error::ArchiveError;
pub use extract::{
    lift_same_named_folder, lift_single_folder, private_extraction_name, ExtractedFolder, ExtractionLimits,
    ExtractionOutput, ExtractionRequest, ExtractionSummary, GioExtractionOutput, OutputFile, ZipExtractor,
};
pub use gio_reader::GioArchiveReader;
pub use member_names::{is_safe_member as is_safe_member_name, is_supported_archive, suggested_folder_name};
pub use preview::{remove_old_previews, remove_preview_copy, PreviewCopy, PREVIEW_LIFETIME, PREVIEW_NOTICE};
pub use source::{ArchiveOpener, ArchiveStream, GioArchiveOpener};
pub use zip::{Zip64Field, ZipFormatError};
