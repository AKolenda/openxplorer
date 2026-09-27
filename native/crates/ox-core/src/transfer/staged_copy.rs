// SPDX-License-Identifier: AGPL-3.0-only
//! Copying one top-level item: build it in private staging, then publish it
//! under its final name. Port of the copy branch of `_run_items` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - A copy is built under an unguessable `.winspace-transfer-<32 hex>.part`
//!   name and published only when complete, so a partial copy is never
//!   visible under its final name.
//! - Local and network destinations get a private staging folder (created
//!   with an exclusive `mkdir`, owner-only when local) holding `payload`.
//! - Device directories (MTP) are built under a hidden sibling name and
//!   published with a same-folder rename. Device files are built under their
//!   final filename inside a private folder and moved out without renaming.
//!   Every cleanup root is reserved by an exclusive mkdir before copying.
//! - A copy within one device (MTP CopyObject) keeps the source's name
//!   whatever target is requested, so it is built under that name inside a
//!   private folder, renamed there, and moved out under the same name.
//! - A device's success report is never trusted: the final name must exist
//!   and the staged name must be gone.
//! - Only staging this item created is recorded for cleanup; a failed
//!   `mkdir` grants no right to delete anything.

use super::commit::{commit_replace, publish_staged, verify_installation};
use super::copy::Copier;
use super::modes::{secure_local_staging, DirectoryModes};
use super::names::{child_node, staging_name, PAYLOAD_NAME};
use super::node::{Cancellation, Node, TransferError, WriteGuard};
use super::types::Progress;

/// Staging the engine created for one item.
pub(crate) enum Stage {
    /// The item itself under a hidden name beside its final name (device
    /// destinations).
    Sibling(Box<dyn Node>),
    /// A private folder holding the item: `payload`, or the source's own
    /// name for a copy within one device, or the final name for an upload.
    Folder {
        folder: Box<dyn Node>,
        item: Box<dyn Node>,
    },
}

impl Stage {
    /// What cleanup removes: the hidden item or the private folder.
    pub(crate) fn root(&self) -> &dyn Node {
        match self {
            Stage::Sibling(item) => item.as_ref(),
            Stage::Folder { folder, .. } => folder.as_ref(),
        }
    }

    /// The completed item that gets published.
    fn item(&self) -> &dyn Node {
        match self {
            Stage::Sibling(item) => item.as_ref(),
            Stage::Folder { item, .. } => item.as_ref(),
        }
    }
}

/// The staging one item owns, for cleanup after it finishes or fails.
#[derive(Default)]
pub(crate) struct StageSlot {
    /// Set as soon as staging exists; cleared once nothing is left to remove.
    pub(crate) stage: Option<Stage>,
    /// The destination is a device: cleanup is retried and reported as an
    /// "item".
    pub(crate) device: bool,
}

/// One top-level copy into `dest_dir` under the name `destination`.
pub(crate) struct StagedCopy<'a> {
    pub(crate) source: &'a dyn Node,
    pub(crate) is_directory: bool,
    pub(crate) dest_dir: &'a dyn Node,
    pub(crate) destination: &'a dyn Node,
    /// The user chose Replace: overwrite files and merge folders.
    pub(crate) replace: bool,
    pub(crate) cancel: &'a Cancellation,
    pub(crate) guard: Option<&'a WriteGuard>,
    pub(crate) emit: &'a mut dyn FnMut(Progress),
}

impl StagedCopy<'_> {
    /// Builds, publishes and verifies the copy. `slot` receives the staging
    /// as soon as it exists, so the caller can clean it up on any failure.
    pub(crate) fn run(mut self, slot: &mut StageSlot) -> Result<(), TransferError> {
        let token = staging_name()?;
        let mut modes = DirectoryModes::default();
        let device = self.dest_dir.stage_as_sibling();
        slot.device = device;
        let same_device_file = !self.is_directory && self.source.native_copy_keeps_name(self.dest_dir);
        if device && self.is_directory {
            self.build_sibling_directory(&token, &mut modes, slot)?;
        } else {
            self.build_in_folder(&token, same_device_file, &mut modes, slot)?;
        }
        self.cancel.check()?;
        let stage = slot
            .stage
            .as_ref()
            .ok_or_else(|| TransferError::failed("The copy was not staged. Nothing was published."))?;
        // A native rename in the destination folder. Replace is only reached
        // after the user explicitly chose it; every other policy keeps the
        // no-overwrite race guard.
        if self.replace {
            commit_replace(
                stage.item(),
                self.destination,
                self.cancel,
                self.guard,
                Some(&mut modes),
            )?;
        } else {
            publish_staged(stage.item(), self.destination, &mut modes, self.cancel)?;
        }
        if device {
            verify_device_publication(stage, self.destination)?;
            if let Stage::Sibling(_) = stage {
                // The staged item now is the published item: nothing is
                // left to clean up.
                slot.stage = None;
            }
        }
        Ok(())
    }

    /// Exclusively reserves and builds a directory under its sibling name.
    fn build_sibling_directory(
        &mut self,
        token: &str,
        modes: &mut DirectoryModes,
        slot: &mut StageSlot,
    ) -> Result<(), TransferError> {
        let staged_item = child_node(self.dest_dir, token)?;
        // A failed exclusive mkdir grants no right to clean up this path.
        staged_item.mkdir(Some(self.cancel))?;
        let stage = slot.stage.insert(Stage::Sibling(staged_item));
        let children = self.source.children(Some(self.cancel))?;
        let mut copier = Copier::new(self.cancel, token, modes, &mut *self.emit);
        for child in children {
            let target = child_node(stage.item(), &child.name())?;
            copier.copy(child.as_ref(), target.as_ref(), 1)?;
        }
        Ok(())
    }

    /// Builds the item inside a new private folder named `token`.
    fn build_in_folder(
        &mut self,
        token: &str,
        same_device_file: bool,
        modes: &mut DirectoryModes,
        slot: &mut StageSlot,
    ) -> Result<(), TransferError> {
        let folder = child_node(self.dest_dir, token)?;
        // A copy within one MTP device runs as CopyObject, which keeps the
        // SOURCE name whatever target is requested: copy under that name.
        let item_name = if same_device_file {
            self.source.name()
        } else if self.dest_dir.stage_as_sibling() {
            // MTP can move across folders only when the name is unchanged.
            // Upload under the final name, safely inside our own namespace.
            self.destination.name()
        } else {
            PAYLOAD_NAME.to_string()
        };
        let item = child_node(folder.as_ref(), &item_name)?;
        // Reserve a private namespace. A failed mkdir never grants
        // permission to delete that name during cleanup.
        folder.mkdir(Some(self.cancel))?;
        let stage = slot.stage.insert(Stage::Folder { folder, item });
        secure_local_staging(stage.root())?;
        let mut copier = Copier::new(self.cancel, token, modes, &mut *self.emit);
        copier.copy(self.source, stage.item(), 0)?;
        if same_device_file {
            self.rename_device_copy(stage)?;
        }
        Ok(())
    }

    /// Checks where a same-device copy landed and gives it its final name
    /// inside the private folder, so the later move out keeps the name.
    fn rename_device_copy(&self, stage: &mut Stage) -> Result<(), TransferError> {
        let Stage::Folder { folder, item } = stage else {
            return Ok(());
        };
        if !item.exists(Some(self.cancel)) {
            return Err(TransferError::failed(
                "The device did not place the copy in its private staging folder. Nothing was published.",
            ));
        }
        let final_name = self.destination.name();
        if final_name != self.source.name() {
            let renamed = child_node(folder.as_ref(), &final_name)?;
            item.move_native(renamed.as_ref(), Some(self.cancel))?;
            *item = renamed;
        }
        Ok(())
    }
}

/// Never trusts a device's success report for the final name.
///
/// MTP keeps resolving a moved item's old path until its folder is listed
/// again, so both parents are relisted before checking metadata. A query
/// failure is never mistaken for absence of the staged item.
fn verify_device_publication(stage: &Stage, destination: &dyn Node) -> Result<(), TransferError> {
    verify_installation(stage.item(), destination).map_err(|error| {
        TransferError::failed(format!(
            "The device reported success, but the copy could not be verified at {}. {error}",
            destination.uri()
        ))
    })
}
