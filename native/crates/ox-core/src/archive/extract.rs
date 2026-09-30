// SPDX-License-Identifier: AGPL-3.0-only
//! Extracting a ZIP into a new folder, all or nothing. Ports
//! `ZipExtractor` of `desktop/zip_extraction.py` and the checks the
//! `archiveExtract` branch of `dispatch` in `desktop/winspace.py` runs
//! before it.
//!
//! An extraction never touches existing content:
//!
//! - ARC-012: the output is always a new folder in a real destination
//!   folder; an existing name, even a file or a dangling link, stops it.
//! - ARC-014 to ARC-017: every member is checked before anything is
//!   written (see `plan`).
//! - ARC-020: the write guard is asked about the destination folder, the
//!   new folder and every path inside it before anything is written.
//! - ARC-013: the contents are built in a private staging folder and
//!   published by a rename that never replaces anything. Any failure,
//!   damaged data or cancellation removes the staging folder (see
//!   `staging`).
//! - ARC-018: files are created new and owner-only through an
//!   [`ExtractionOutput`]; archive permissions are never applied.
//!
//! | Module | Responsibility |
//! |---|---|
//! | `plan` | Checking every member and summarising the archive |
//! | `limits` | [`ExtractionLimits`] and the byte checks while writing |
//! | `staging` | The staging folder: creating, securing, publishing, removing |
//! | `output` | [`ExtractionOutput`]: creating and writing the files |
//! | `unpack` | Writing the planned members into the staging folder |
//! | `lift` | Extract here: a lone top-level folder becomes the output |

mod lift;
mod limits;
mod output;
mod plan;
mod staging;
mod unpack;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::sync::Arc;

pub use lift::lift_single_folder;
pub use limits::ExtractionLimits;
pub use output::{ExtractionOutput, GioExtractionOutput, OutputFile};
pub use plan::ExtractionSummary;

use super::source::{open_archive, ArchiveOpener};
use super::worker::on_worker;
use super::ArchiveError;
use crate::location::{is_smb_server, normalise, validate_name};
use crate::transfer::{
    Cancellation, Node, NodeFactory, NodeKind, Progress, ProgressScope, TransferError, WriteGuard,
};
use plan::{plan, ExtractionPlan};
use staging::ExtractionStaging;
use unpack::Unpacking;

/// Receives progress on the worker thread the extraction runs on.
type ProgressCallback = Box<dyn FnMut(Progress) + Send>;

/// What to extract, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionRequest {
    /// The ZIP to extract.
    pub archive_uri: String,
    /// The folder to create the new folder in.
    pub destination_uri: String,
    /// The new folder's name; nothing may have it yet.
    pub folder_name: String,
}

/// A finished extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFolder {
    /// The new folder holding the archive's contents.
    pub uri: String,
    /// The new folder's name.
    pub name: String,
    /// The archive, which is unchanged.
    pub archive_uri: String,
    /// The folder the new folder was created in.
    pub destination_uri: String,
    /// What was extracted.
    pub summary: ExtractionSummary,
}

/// Checks and extracts ZIP archives.
///
/// Like the transfer engine, it resolves URIs with a [`NodeFactory`], asks
/// an optional write guard before writing, and reports [`Progress`].
pub struct ZipExtractor {
    opener: Arc<dyn ArchiveOpener>,
    factory: NodeFactory,
    output: Arc<dyn ExtractionOutput>,
    /// Named after `self.emit` of the Python extractor.
    emit: ProgressCallback,
    write_guard: Option<Box<WriteGuard>>,
    limits: ExtractionLimits,
}

impl fmt::Debug for ZipExtractor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ZipExtractor")
            .field("has_write_guard", &self.write_guard.is_some())
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// The destination of an extraction, checked and resolved.
struct Destination {
    /// The folder the new folder goes in.
    folder: Box<dyn Node>,
    /// The new folder, which does not exist yet.
    new_folder: Box<dyn Node>,
}

impl ZipExtractor {
    /// An extractor reading archives with `opener`, resolving destination
    /// URIs with `factory` and writing files with `output`, without
    /// progress reports or a write guard, and with the default limits.
    pub fn new(
        opener: Arc<dyn ArchiveOpener>,
        factory: NodeFactory,
        output: Arc<dyn ExtractionOutput>,
    ) -> Self {
        Self {
            opener,
            factory,
            output,
            emit: Box::new(|_| {}),
            write_guard: None,
            limits: ExtractionLimits::default(),
        }
    }

    /// Receives progress for the transfer panel. It is called on the
    /// thread the extraction runs on.
    #[must_use]
    pub fn with_progress(mut self, emit: impl FnMut(Progress) + Send + 'static) -> Self {
        self.emit = Box::new(emit);
        self
    }

    /// ARC-020: rejects writes into protected locations such as snapshot
    /// folders.
    #[must_use]
    pub fn with_write_guard(
        mut self,
        guard: impl Fn(&str) -> Result<(), TransferError> + Send + Sync + 'static,
    ) -> Self {
        self.write_guard = Some(Box::new(guard));
        self
    }

    /// Replaces the default [`ExtractionLimits`].
    #[must_use]
    pub fn with_limits(mut self, limits: ExtractionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// ARC-008: checks the whole archive at `archive_uri` with the
    /// extraction rules and summarises it. Nothing is written. Blocking;
    /// see [`Self::inspect_in_background`].
    ///
    /// # Errors
    ///
    /// The errors of reading the archive, the first refusal of the
    /// extraction rules, or [`ArchiveError::Cancelled`].
    pub fn inspect(
        &self,
        archive_uri: &str,
        cancel: &Cancellation,
    ) -> Result<ExtractionSummary, ArchiveError> {
        let archive = open_archive(self.opener.as_ref(), archive_uri, cancel)?;
        let plan = plan(archive.members(), &self.limits, cancel)?;
        Ok(plan.summary)
    }

    /// [`Self::inspect`] on a GIO worker thread, for the main loop to await.
    ///
    /// # Errors
    ///
    /// See [`Self::inspect`].
    pub async fn inspect_in_background(
        self,
        archive_uri: String,
        cancel: Cancellation,
    ) -> Result<ExtractionSummary, ArchiveError> {
        on_worker(move || self.inspect(&archive_uri, &cancel)).await
    }

    /// Extracts the archive into a new folder, all or nothing. Blocking;
    /// see [`Self::extract_in_background`].
    ///
    /// # Errors
    ///
    /// A refusal of the rules in the module documentation, the errors of
    /// reading the archive or writing the destination, or
    /// [`ArchiveError::Cancelled`]. Nothing is left behind, except a
    /// staging folder that could not be removed, which
    /// [`ArchiveError::StagingLeftBehind`] reports.
    pub fn extract(
        &mut self,
        request: &ExtractionRequest,
        cancel: &Cancellation,
    ) -> Result<ExtractedFolder, ArchiveError> {
        let archive_uri = normalise(&request.archive_uri)?;
        let destination_uri = normalise(&request.destination_uri)?;
        let destination = self.prepare_destination(&destination_uri, &request.folder_name, cancel)?;
        let name = request.folder_name.clone();
        self.report("Checking ZIP contents…".to_owned(), 0.0);
        let mut archive = open_archive(self.opener.as_ref(), &archive_uri, cancel)?;
        let plan = plan(archive.members(), &self.limits, cancel)?;
        self.check_member_destinations(destination.new_folder.as_ref(), &plan, cancel)?;
        cancel.check()?;
        let mut staging = ExtractionStaging::create(destination.folder.as_ref(), cancel)?;
        let mut unpacking = Unpacking::new(self, plan.summary, cancel);
        if let Err(error) = unpacking.run(&mut archive, &plan, &mut staging, destination.new_folder.as_ref())
        {
            return Err(staging.discard(error.unless_cancelled(cancel)));
        }
        let files = plan.summary.file_count;
        self.report(format!("Extracted {files} files into {name}"), 1.0);
        Ok(ExtractedFolder {
            uri: destination.new_folder.uri(),
            name,
            archive_uri,
            destination_uri,
            summary: plan.summary,
        })
    }

    /// [`Self::extract`] on a GIO worker thread, for the main loop to await.
    ///
    /// # Errors
    ///
    /// See [`Self::extract`].
    pub async fn extract_in_background(
        mut self,
        request: ExtractionRequest,
        cancel: Cancellation,
    ) -> Result<ExtractedFolder, ArchiveError> {
        on_worker(move || self.extract(&request, &cancel)).await
    }

    /// Checks the destination folder and the new folder's name before
    /// anything is read or written, in the order of the Python app.
    fn prepare_destination(
        &self,
        destination_uri: &str,
        folder_name: &str,
        cancel: &Cancellation,
    ) -> Result<Destination, ArchiveError> {
        self.check_writable(destination_uri)?;
        // A server's list of shares cannot hold folders.
        if is_smb_server(destination_uri) {
            return Err(ArchiveError::ServerListingDestination);
        }
        // ARC-012: the new folder's name is one plain path component.
        let folder_name = validate_name(folder_name)?;
        let folder = (self.factory)(destination_uri)?;
        // ARC-012: `info` does not follow links, so a link to a folder is
        // refused too.
        if folder.info(Some(cancel))?.kind != NodeKind::Directory {
            return Err(ArchiveError::NotARealFolder);
        }
        let new_folder = folder.child(OsStr::new(folder_name));
        self.check_writable(&new_folder.uri())?;
        // ARC-012: an existing file, folder or dangling link keeps its name;
        // nothing is ever merged into it.
        if new_folder.exists(Some(cancel)) {
            return Err(ArchiveError::DestinationExists);
        }
        Ok(Destination { folder, new_folder })
    }

    /// ARC-020: asks the write guard about every path the extraction would
    /// create below `folder`, so a member such as `.snapshot/…` stops it
    /// before anything is written.
    fn check_member_destinations(
        &self,
        folder: &dyn Node,
        plan: &ExtractionPlan,
        cancel: &Cancellation,
    ) -> Result<(), ArchiveError> {
        if self.write_guard.is_none() {
            return Ok(());
        }
        let mut checked = HashSet::new();
        for member in &plan.members {
            cancel.check()?;
            for uri in descendant_uris(folder, &member.segments) {
                if !checked.contains(&uri) {
                    self.check_writable(&uri)?;
                    checked.insert(uri);
                }
            }
        }
        Ok(())
    }

    /// Asks the write guard, if there is one, about `uri`.
    fn check_writable(&self, uri: &str) -> Result<(), ArchiveError> {
        match &self.write_guard {
            Some(guard) => Ok(guard(uri)?),
            None => Ok(()),
        }
    }

    /// Reports progress to the transfer panel.
    fn report(&mut self, label: String, fraction: f64) {
        (self.emit)(Progress {
            label,
            fraction,
            scope: ProgressScope::Batch,
        });
    }
}

/// The URIs of `folder`'s descendants along `segments`: `folder/a`,
/// `folder/a/b` and so on.
fn descendant_uris(folder: &dyn Node, segments: &[String]) -> Vec<String> {
    let mut uris = Vec::with_capacity(segments.len());
    let mut current: Option<Box<dyn Node>> = None;
    for segment in segments {
        // ARC-014: every segment is a checked, single path component.
        let parent = current.as_deref().unwrap_or(folder);
        let child = parent.child(OsStr::new(segment));
        uris.push(child.uri());
        current = Some(child);
    }
    uris
}
