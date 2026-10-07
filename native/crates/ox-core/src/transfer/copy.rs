// SPDX-License-Identifier: AGPL-3.0-only
//! Recursive copy into staging. Ports `TransferEngine._copy` in
//! `v2.0.0:desktop/operations.py`.
//!
//! Rules enforced here:
//! - XFER-017: items are inspected without following symbolic links; links
//!   are copied as links and never traversed, so link loops are harmless.
//! - XFER-018: sockets, devices, FIFOs and other special files are refused.
//! - OPS-047: an entry inside a folder that cannot be copied (unreadable,
//!   denied, a special file) is asked about by itself, as Windows Explorer
//!   asks: Retry copies it again, Skip leaves only it out, and the rest of
//!   the folder is still copied ([`ItemFailures::entry_failed`]).
//! - Nesting deeper than [`MAX_DEPTH`] stops the copy.
//! - XFER-016: meeting the engine's own staging name inside the source
//!   means the destination is an alias of a folder inside the source (for
//!   example the same share under another host name); the copy stops
//!   instead of copying itself forever.
//! - XFER-004 and XFER-005: staged local folders are owner-only while being
//!   built; the source's mode is recorded and applied only when publishing.
//! - XFER-028: a file larger than the destination file system stores is
//!   refused with a message that says so, and names and links it cannot
//!   store are renamed or left out as the user answers.
//! - XFER-013: for a move finished by copying, every copied source item is
//!   recorded with what it was before its copy, so only those are removed
//!   afterwards, and only while they are unchanged.

use std::ffi::OsString;

use super::cancellation::Cancellation;
use super::error::TransferError;
use super::guard::{nesting_error, MAX_DEPTH};
use super::item_failure::{EntryDecision, ItemFailures};
use super::labels::copy_label;
use super::modes::{path_for_unix_modes, secure_local_staging, DirectoryModes, PRIVATE_DIRECTORY_MODE};
use super::names::child_node;
use super::node::{Node, NodeInfo, NodeKind};
use super::source_removal::CopiedItem;
use super::staging::clean_staging;
use super::types::{progress_fraction, ByteProgress, Progress, ProgressScope};
use super::unstorable::{Fix, Unstorable};

/// Copies one source tree into staging, reporting byte progress.
pub(crate) struct Copier<'a> {
    cancel: &'a Cancellation,
    /// The name of the staging item this copy builds (see the alias rule).
    own_stage_name: &'a str,
    /// Final modes of the staged local folders, applied when publishing.
    modes: &'a mut DirectoryModes,
    /// What the destination cannot store, and the user's answers about it.
    unstorable: &'a mut Unstorable,
    emit: &'a mut dyn FnMut(Progress),
    /// Receives each source item below the top once it is copied, children
    /// before their folder, when the copy finishes a move (XFER-013).
    copied: Option<&'a mut Vec<CopiedItem>>,
    /// Asks about an entry inside a folder that cannot be copied, and
    /// keeps what was left out (OPS-047).
    failures: Option<&'a mut ItemFailures>,
    /// The names from the top item down to the entry being copied.
    entry_path: Vec<String>,
    /// Set when a safety rule stopped the copy (the nesting limit, an
    /// alias of the destination inside the source): the whole item fails,
    /// whatever entry it was found in.
    stopped: bool,
}

impl<'a> Copier<'a> {
    /// A copier building into the staging item named `own_stage_name`.
    pub(crate) fn new(
        cancel: &'a Cancellation,
        own_stage_name: &'a str,
        modes: &'a mut DirectoryModes,
        unstorable: &'a mut Unstorable,
        emit: &'a mut dyn FnMut(Progress),
    ) -> Self {
        Self {
            cancel,
            own_stage_name,
            modes,
            unstorable,
            emit,
            copied: None,
            failures: None,
            entry_path: Vec::new(),
            stopped: false,
        }
    }

    /// Asks `failures` about an entry inside a folder that cannot be
    /// copied, instead of failing the whole item (OPS-047).
    pub(crate) fn asking(mut self, failures: &'a mut ItemFailures) -> Self {
        self.failures = Some(failures);
        self
    }

    /// Records every source item below the top into `copied` once it is
    /// copied (XFER-013).
    pub(crate) fn recording(mut self, copied: Option<&'a mut Vec<CopiedItem>>) -> Self {
        self.copied = copied;
        self
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
        self.check_item(source, depth)?;
        // XFER-017: inspected without following a symbolic link.
        let info = source.info(Some(self.cancel))?;
        self.copy_inspected(source, target, &info, depth)
    }

    /// Stops before `source` (at nesting `depth`) when the user cancelled,
    /// the nesting limit is reached, or it is the copy's own staging.
    fn check_item(&mut self, source: &dyn Node, depth: usize) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            self.stopped = true;
            return Err(nesting_error());
        }
        // XFER-016: the copy met its own staging, so the destination is an
        // alias of a folder inside the source that `guard_destination`
        // could not prove.
        if source.name() == self.own_stage_name {
            self.stopped = true;
            return Err(TransferError::failed(crate::i18n::gettext(
                "The destination resolves inside the source through an alias. Copy stopped.",
            )));
        }
        Ok(())
    }

    /// Copies `source`, whose metadata is `info`, to the new name `target`.
    fn copy_inspected(
        &mut self,
        source: &dyn Node,
        target: &dyn Node,
        info: &NodeInfo,
        depth: usize,
    ) -> Result<(), TransferError> {
        match info.kind {
            NodeKind::Directory => self.copy_directory(source, target, info, depth)?,
            NodeKind::File => {
                // XFER-028: FAT stores files up to 4 GiB only.
                let rules = self.unstorable.rules;
                rules.check_file_size(&source.display_name(), info.size)?;
                self.copy_file(source, target)?;
            }
            NodeKind::Symlink => self.copy_file(source, target)?,
            NodeKind::Special => {
                return Err(TransferError::failed(crate::i18n::gettext(
                    "Sockets, devices and other special files are not copied.",
                )));
            }
        }
        Ok(())
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
        target.create_directory(Some(self.cancel))?;
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
            self.check_item(child.as_ref(), depth)?;
            self.entry_path.push(child.display_name());
            let copied = self.copy_entry(child, target, depth);
            self.entry_path.pop();
            copied?;
        }
        Ok(())
    }

    /// Copies the entry `child` of a folder into the folder `target`, at
    /// nesting `depth`. When it cannot be copied, the user is asked about
    /// it alone (OPS-047): what it left in staging is removed, then it is
    /// copied again or left out. An error the whole item stops for (a
    /// cancellation, a location that is gone) is returned.
    fn copy_entry(
        &mut self,
        child: Box<dyn Node>,
        target: &dyn Node,
        depth: usize,
    ) -> Result<(), TransferError> {
        loop {
            let mut made = None;
            let error = match self.try_entry(child.as_ref(), target, depth, &mut made) {
                Ok(Some(info)) => {
                    if let Some(copied) = self.copied.as_deref_mut() {
                        copied.push(CopiedItem { node: child, info });
                    }
                    return Ok(());
                }
                Ok(None) => return Ok(()),
                Err(error) => error,
            };
            let decision = match self.failures.as_deref_mut() {
                // The user's cancellation explains any error, and a safety
                // stop is never one entry's problem.
                _ if self.cancel.is_cancelled() || self.stopped => EntryDecision::Fail,
                Some(failures) => failures.entry_failed(&self.entry_path.join("/"), &child.uri(), &error),
                None => EntryDecision::Fail,
            };
            match decision {
                EntryDecision::Fail => return Err(error),
                EntryDecision::Cancel => return Err(TransferError::Cancelled),
                EntryDecision::Retry | EntryDecision::Skip => {
                    if let Some(name) = made {
                        self.discard_partial(target, name)?;
                    }
                }
            }
            if decision == EntryDecision::Skip {
                return Ok(());
            }
        }
    }

    /// Copies the entry `child` into the folder `target` once; `made`
    /// receives the name it is copied under before anything is created.
    /// Returns what `child` was when it was read, or `None` when the
    /// destination cannot store it and the user left it out (XFER-028).
    fn try_entry(
        &mut self,
        child: &dyn Node,
        target: &dyn Node,
        depth: usize,
        made: &mut Option<OsString>,
    ) -> Result<Option<NodeInfo>, TransferError> {
        // XFER-017: inspected without following a symbolic link.
        let info = child.info(Some(self.cancel))?;
        // XFER-028: a name or link the destination cannot store.
        let Fix::Name(name) = self.unstorable.fix(child, Some(info.kind), self.cancel)? else {
            return Ok(None);
        };
        let child_target = child_node(target, &name)?;
        *made = Some(name);
        self.copy_inspected(child, child_target.as_ref(), &info, depth)?;
        Ok(Some(info))
    }

    /// Removes what a failed entry left in staging under `name` in the
    /// folder `target`: a part of a file, or a folder and what was copied
    /// into it. Staging is the engine's own, so this never touches the
    /// user's files.
    fn discard_partial(&mut self, target: &dyn Node, name: OsString) -> Result<(), TransferError> {
        let partial = child_node(target, name)?;
        if !partial.exists(Some(self.cancel)) {
            return Ok(());
        }
        self.modes.forget_below(&partial.uri());
        clean_staging(partial.as_ref())
    }

    /// Copies one file, or one link as a link, reporting its byte progress.
    /// While the user pauses, the copy waits between blocks (OPS-021).
    fn copy_file(&mut self, source: &dyn Node, target: &dyn Node) -> Result<(), TransferError> {
        let name = source.display_name();
        let cancel = self.cancel;
        let emit = &mut *self.emit;
        let mut progress = |current: u64, total: u64| {
            cancel.wait_while_paused();
            // Nothing more is reported once the user cancelled.
            if cancel.is_cancelled() {
                return;
            }
            emit(Progress {
                label: copy_label(&name, current, total),
                fraction: progress_fraction(current, total),
                scope: ProgressScope::File,
                bytes: Some(ByteProgress {
                    file_written: current,
                    file_size: total,
                    // The engine fills in what the batch writes.
                    batch_size: None,
                    batch_written: 0,
                }),
            });
        };
        // An explicit boundary lets the engine accumulate bytes even when
        // adjacent files have the same size. It is throttled after accounting.
        progress(0, 0);
        source.copy_file(target, cancel, &mut progress)
    }
}
