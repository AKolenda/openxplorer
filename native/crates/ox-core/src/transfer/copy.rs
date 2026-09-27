// SPDX-License-Identifier: AGPL-3.0-only
//! Recursive copy into staging. Port of `TransferEngine._copy` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - Items are inspected without following symbolic links; links are
//!   copied as links and never traversed, so link loops are harmless.
//! - Sockets, devices, FIFOs and other special files are refused.
//! - Nesting deeper than [`MAX_DEPTH`] stops the copy.
//! - Meeting the engine's own staging name inside the source means the
//!   destination is an alias of a folder inside the source (for example the
//!   same share under another host name); the copy stops instead of copying
//!   itself forever.
//! - Staged local folders are owner-only while being built; the source's
//!   mode is recorded and applied only when publishing.

use super::error::TransferError;
use super::guard::{nesting_error, MAX_DEPTH};
use super::labels::copy_label;
use super::modes::{secure_local_staging, DirectoryModes, PRIVATE_DIRECTORY_MODE};
use super::names::child_node;
use super::node::{Cancellation, Node, NodeInfo, NodeKind};
use super::types::Progress;

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
        let info = source.info(Some(self.cancel))?;
        match info.kind {
            NodeKind::Directory => self.copy_directory(source, target, &info, depth),
            NodeKind::File | NodeKind::Symlink => self.copy_file(source, target),
            NodeKind::Special => Err(TransferError::failed(
                "Sockets, devices and other special files are not copied.",
            )),
        }
    }

    fn copy_directory(
        &mut self,
        source: &dyn Node,
        target: &dyn Node,
        info: &NodeInfo,
        depth: usize,
    ) -> Result<(), TransferError> {
        target.mkdir(Some(self.cancel))?;
        let local_path = target.path().filter(|_| target.uri().starts_with("file:"));
        if let Some(path) = local_path {
            let mode = match info.mode {
                Some(mode) => Some(mode),
                None => target.info(Some(self.cancel))?.mode,
            };
            let final_mode = mode.unwrap_or(PRIVATE_DIRECTORY_MODE);
            self.modes.record(target.uri(), path, final_mode);
            secure_local_staging(target)?;
        }
        for child in source.children(Some(self.cancel))? {
            let child_target = child_node(target, child.name())?;
            self.copy(child.as_ref(), child_target.as_ref(), depth + 1)?;
        }
        Ok(())
    }

    fn copy_file(&mut self, source: &dyn Node, target: &dyn Node) -> Result<(), TransferError> {
        let name = source.display_name();
        let cancel = self.cancel;
        let emit = &mut *self.emit;
        let mut progress = |current: u64, total: u64| {
            // Nothing more is reported once the user cancelled.
            if cancel.is_cancelled() {
                return;
            }
            let fraction = if total > 0 {
                (current as f64 / total as f64).min(1.0)
            } else {
                0.0
            };
            emit(Progress {
                label: copy_label(&name, current, total),
                fraction,
            });
        };
        source.copy_file(target, cancel, &mut progress)
    }
}
