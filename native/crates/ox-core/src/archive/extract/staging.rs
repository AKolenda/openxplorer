// SPDX-License-Identifier: AGPL-3.0-only
//! The private folder an extraction is built in, and how it is published
//! or removed. Ports the staging steps of `ZipExtractor.extract` in
//! `v2.0.0:desktop/zip_extraction.py`. Like the Python extractor, which calls
//! `_secure_local_staging` and `_clean_staging` of `v2.0.0:desktop/operations.py`,
//! it applies the transfer engine's own staging rules: its random names,
//! `secure_local_staging` and [`Node::delete_staging`].
//!
//! Safety rules (ARC-013, ARC-018):
//!
//! - The staging folder gets an unpredictable name and is created
//!   exclusively, so only a folder this extraction created is ever removed.
//! - On local disks it is owner-only (`0700`) from the start; the
//!   published folder keeps that mode.
//! - It is published by a rename that never replaces an existing item.
//! - Any failure removes it; a folder that cannot be removed is reported
//!   with its location.

use std::ffi::OsStr;

use crate::archive::ArchiveError;
use crate::random::{random_hex, NAME_BYTES};
use crate::transfer::{secure_local_staging, Cancellation, ItemIdentity, Node, TransferError};

/// Staging folders are `.openxplorer-extract-<32 hex digits>.part`.
const STAGING_PREFIX: &str = ".openxplorer-extract-";
const STAGING_SUFFIX: &str = ".part";

/// A staging folder this extraction created, which it alone may remove.
pub(super) struct ExtractionStaging {
    folder: Box<dyn Node>,
    /// The local folder's identity, recorded when it was made private, so
    /// cleanup never empties another folder moved in under its name.
    created: Option<ItemIdentity>,
}

impl ExtractionStaging {
    /// Creates a new staging folder in `destination`.
    ///
    /// # Errors
    ///
    /// When no random name can be generated or the folder cannot be
    /// created; nothing needs cleaning up then.
    pub(super) fn create(destination: &dyn Node, cancel: &Cancellation) -> Result<Self, ArchiveError> {
        let folder = destination.child(OsStr::new(&staging_name()?));
        // Only a successful, exclusive creation grants cleanup ownership:
        // a name that was taken is never removed.
        folder.create_directory(Some(cancel))?;
        Ok(Self {
            folder,
            created: None,
        })
    }

    /// The staging folder.
    pub(super) fn folder(&self) -> &dyn Node {
        self.folder.as_ref()
    }

    /// ARC-018: makes a local staging folder owner-only (`0700`) with the
    /// transfer engine's rule for its own staging, as
    /// `v2.0.0:desktop/zip_extraction.py` calls `_secure_local_staging`. Only
    /// `file:` folders with a local path get Unix modes: MTP, AFC and many
    /// SMB backends expose a FUSE path but cannot `chmod` (XFER-004); their
    /// random staging name keeps the folder private instead. The mode is set
    /// through a descriptor opened without following links, so a folder
    /// swapped for a link cannot redirect it, and the folder's identity is
    /// recorded for [`Self::discard`].
    ///
    /// # Errors
    ///
    /// When the folder cannot be opened that way or its mode cannot change.
    pub(super) fn make_private(&mut self) -> Result<(), ArchiveError> {
        self.created = secure_local_staging(self.folder.as_ref())?;
        Ok(())
    }

    /// ARC-013: renames the finished staging folder to `target` in the same
    /// folder. The rename never replaces anything, so a folder another
    /// program created under the name meanwhile is kept.
    ///
    /// # Errors
    ///
    /// [`TransferError::Exists`] (as [`ArchiveError::Backend`]) when
    /// `target` exists, or the backend's error; the staging folder is then
    /// still there for [`Self::discard`].
    pub(super) fn publish(&self, target: &dyn Node, cancel: &Cancellation) -> Result<(), ArchiveError> {
        self.folder.publish(target, Some(cancel))?;
        Ok(())
    }

    /// ARC-013: removes the staging folder after `cause` stopped the
    /// extraction, and returns the error to report: `cause`, or
    /// [`ArchiveError::StagingLeftBehind`] with the folder's location when
    /// it could not be removed. Removal never follows links inside it.
    pub(super) fn discard(self, cause: ArchiveError) -> ArchiveError {
        match self.folder.delete_staging(self.created) {
            Ok(()) => cause,
            Err(cleanup) => ArchiveError::StagingLeftBehind {
                cause: Box::new(cause),
                staging_uri: self.folder.uri(),
                cleanup,
            },
        }
    }
}

/// A new `.openxplorer-extract-<32 hex digits>.part` name. The digits come
/// from the kernel's random source, like the transfer engine's staging
/// names, so another program cannot predict the name and prepare an item
/// under it.
///
/// # Errors
///
/// When the kernel's random source cannot be read.
fn staging_name() -> Result<String, ArchiveError> {
    let digits = random_hex(NAME_BYTES).map_err(|error| {
        TransferError::failed(format!("Could not reserve a private staging name. {error}"))
    })?;
    Ok(format!("{STAGING_PREFIX}{digits}{STAGING_SUFFIX}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ARC-013
    #[test]
    fn staging_names_are_hidden_random_and_distinct() {
        let first = staging_name().expect("urandom is readable");
        let second = staging_name().expect("urandom is readable");

        let digits = first
            .strip_prefix(".openxplorer-extract-")
            .and_then(|rest| rest.strip_suffix(".part"))
            .expect("the staging name format");
        assert_eq!(digits.len(), 32);
        assert!(digits
            .bytes()
            .all(|digit| digit.is_ascii_hexdigit() && !digit.is_ascii_uppercase()));
        assert_ne!(first, second);
    }
}
