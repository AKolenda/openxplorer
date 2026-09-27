// SPDX-License-Identifier: AGPL-3.0-only
//! Recursive copy into staging. Port of `TransferEngine._copy` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - XFER-017: items are inspected without following symbolic links; links
//!   are copied as links and never traversed, so link loops are harmless.
//! - XFER-018: sockets, devices, FIFOs and other special files are refused.
//! - Nesting deeper than [`MAX_DEPTH`] stops the copy.
//! - XFER-016: meeting the engine's own staging name inside the source
//!   means the destination is an alias of a folder inside the source (for
//!   example the same share under another host name); the copy stops
//!   instead of copying itself forever.
//! - XFER-004 and XFER-005: staged local folders are owner-only while being
//!   built; the source's mode is recorded and applied only when publishing.

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::guard::{nesting_error, MAX_DEPTH};
use super::labels::copy_label;
use super::modes::{path_for_unix_modes, secure_local_staging, DirectoryModes, PRIVATE_DIRECTORY_MODE};
use super::names::child_node;
use super::node::{Node, NodeInfo, NodeKind};
use super::types::{progress_fraction, Progress};

/// Copies one source tree into staging, reporting byte progress.
pub(crate) struct Copier<'a> {
    cancel: &'a Cancellation,
    /// The name of the staging item this copy builds (see the alias rule).
    own_stage_name: &'a str,
    /// Final modes of the staged local folders, applied when publishing.
    modes: &'a mut DirectoryModes,
    emit: &'a mut dyn FnMut(Progress),
}

impl<'a> Copier<'a> {
    /// A copier building into the staging item named `own_stage_name`.
    pub(crate) fn new(
        cancel: &'a Cancellation,
        own_stage_name: &'a str,
        modes: &'a mut DirectoryModes,
        emit: &'a mut dyn FnMut(Progress),
    ) -> Self {
        Self {
            cancel,
            own_stage_name,
            modes,
            emit,
        }
    }

    /// Copies `source` (at nesting `depth`) to the new name `target`.
    ///
    /// # Errors
    ///
    /// The first item that cannot be copied, a refused special file, the
    /// nesting limit, an alias of the destination inside the source, or
    /// [`TransferError::Cancelled`].
    pub(crate) fn copy(
        &mut self,
        source: &dyn Node,
        target: &dyn Node,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(nesting_error());
        }
        if source.name() == self.own_stage_name {
            return Err(TransferError::failed(
                "The destination resolves inside the source through an alias. Copy stopped.",
            ));
        }
        // XFER-017: inspected without following a symbolic link.
        let info = source.info(Some(self.cancel))?;
        match info.kind {
            NodeKind::Directory => self.copy_directory(source, target, &info, depth),
            NodeKind::File | NodeKind::Symlink => self.copy_file(source, target),
            NodeKind::Special => Err(TransferError::failed(
                "Sockets, devices and other special files are not copied.",
            )),
        }
    }

    /// Creates the folder `target` for the folder `source`, whose metadata
    /// is `info`, and copies its items.
    fn copy_directory(
        &mut self,
        source: &dyn Node,
        target: &dyn Node,
        info: &NodeInfo,
        depth: usize,
    ) -> Result<(), TransferError> {
        target.mkdir(Some(self.cancel))?;
        if let Some(path) = path_for_unix_modes(target) {
            let final_mode = self.published_mode(info, target)?;
            self.modes.record(target.uri(), path, final_mode);
            secure_local_staging(target)?;
        }
        self.copy_children(source, target, depth + 1)
    }

    /// The mode the staged local folder `target` gets when it is published:
    /// the source folder's mode; else, for a source backend without Unix
    /// modes, the mode the new folder was created with; else owner-only.
    fn published_mode(&self, source_info: &NodeInfo, target: &dyn Node) -> Result<u32, TransferError> {
        if let Some(source_mode) = source_info.mode {
            return Ok(source_mode);
        }
        let created_mode = target.info(Some(self.cancel))?.mode;
        Ok(created_mode.unwrap_or(PRIVATE_DIRECTORY_MODE))
    }

    /// Copies every item of the folder `source` into the existing folder
    /// `target`, at nesting `depth`.
    ///
    /// # Errors
    ///
    /// As [`Copier::copy`], for the first item that fails.
    pub(crate) fn copy_children(
        &mut self,
        source: &dyn Node,
        target: &dyn Node,
        depth: usize,
    ) -> Result<(), TransferError> {
        for child in source.children(Some(self.cancel))? {
            let child_target = child_node(target, child.name())?;
            self.copy(child.as_ref(), child_target.as_ref(), depth)?;
        }
        Ok(())
    }

    /// Copies one file, or one link as a link, reporting its byte progress.
    fn copy_file(&mut self, source: &dyn Node, target: &dyn Node) -> Result<(), TransferError> {
        let name = source.display_name();
        let cancel = self.cancel;
        let emit = &mut *self.emit;
        let mut progress = |current: u64, total: u64| {
            // Nothing more is reported once the user cancelled.
            if cancel.is_cancelled() {
                return;
            }
            emit(Progress {
                label: copy_label(&name, current, total),
                fraction: progress_fraction(current, total),
            });
        };
        source.copy_file(target, cancel, &mut progress)
    }
}
