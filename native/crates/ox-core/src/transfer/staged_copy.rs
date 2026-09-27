// SPDX-License-Identifier: AGPL-3.0-only
//! Copying one top-level item: build it in private staging, then publish it
//! under its final name. Port of the copy branch of `_run_items` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - XFER-001: a copy is built under an unguessable
//!   `.winspace-transfer-<32 hex>.part` name and published only when
//!   complete, so a partial copy is never visible under its final name.
//!   Local and network destinations get a private staging folder (created
//!   exclusively with `create_directory`, owner-only when local) holding
//!   `payload`.
//! - XFER-021: device destinations (MTP) build the item itself under a
//!   hidden sibling name and publish it with a same-folder rename (MTP
//!   `SetObjectPropValue`). Uploads therefore never need MTP `MoveObject`,
//!   which devices such as Android 7 and 8 do not offer. A device's success
//!   report is never trusted: the final name must exist and the staged name
//!   must be gone.
//! - XFER-023: a copy within one device (MTP `CopyObject`) keeps the
//!   source's name whatever target is requested, so it is built under that
//!   name inside a private folder, renamed there, and moved out under the
//!   same name. As in the Python app, that last move needs MTP
//!   `MoveObject`; a device without it gets an error that says so.
//! - XFER-002: only staging this item created is recorded for cleanup: a
//!   failed exclusive `create_directory`, or an upload refused because its
//!   name exists, grants no right to delete anything.

use std::ffi::OsString;

use super::cancellation::Cancellation;
use super::commit::{commit_replace, publish_staged, verify_installation};
use super::copy::Copier;
use super::error::TransferError;
use super::modes::{secure_local_staging, DirectoryModes};
use super::names::{child_node, staging_name, PAYLOAD_NAME};
use super::node::{ItemIdentity, Node, NodeKind, WriteGuard};
use super::staging::StagingPlace;
use super::types::{ConflictPolicy, Progress};

/// Staging the engine created for one item.
pub(crate) enum Stage {
    /// The item itself under a hidden name beside its final name (device
    /// uploads).
    Sibling(Box<dyn Node>),
    /// A private folder holding the item: `payload`, or the source's own
    /// name for a copy within one device.
    Folder {
        /// The private `.winspace-transfer-<hex>.part` folder.
        folder: Box<dyn Node>,
        /// The item being built inside it.
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
            Stage::Sibling(item) | Stage::Folder { item, .. } => item.as_ref(),
        }
    }
}

/// The staging one item owns, for cleanup after it finishes or fails.
#[derive(Default)]
pub(crate) struct ItemStaging {
    /// Set as soon as staging exists; cleared once nothing is left to remove.
    pub(crate) stage: Option<Stage>,
    /// Where the staging lives, which decides how cleanup is retried.
    pub(crate) place: StagingPlace,
    /// The local staging folder the engine created, recorded when it was
    /// made private; `None` for remote and device staging.
    pub(crate) created: Option<ItemIdentity>,
}

impl ItemStaging {
    /// Removes the private folder of a published copy.
    ///
    /// The folder is empty by then. A plain delete removes only an empty
    /// folder, so it can never take the published item with it.
    ///
    /// # Errors
    ///
    /// The backend's error; the folder then stays recorded for cleanup.
    pub(crate) fn remove_empty_folder(&mut self) -> Result<(), TransferError> {
        if let Some(stage) = &self.stage {
            stage.root().delete()?;
        }
        self.stage = None;
        Ok(())
    }
}

/// How one copy is staged, chosen from the destination's capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// `payload` inside a private folder: local and network destinations.
    Payload,
    /// The item itself under a hidden sibling name: device uploads.
    Sibling,
    /// The source's own name inside a private folder: a file copy within
    /// one device, where MTP `CopyObject` keeps the source name.
    SameDeviceCopy,
}

/// One top-level copy into `destination_folder` under the name
/// `destination`.
pub(crate) struct StagedCopy<'a> {
    /// The user's item; it is only read.
    pub(crate) source: &'a dyn Node,
    /// What `source` is, inspected without following links.
    pub(crate) source_kind: NodeKind,
    /// The folder the copy is published in.
    pub(crate) destination_folder: &'a dyn Node,
    /// The final name, chosen by the conflict policy.
    pub(crate) destination: &'a dyn Node,
    /// Replace overwrites files and merges folders; every other policy
    /// publishes without overwriting.
    pub(crate) policy: ConflictPolicy,
    pub(crate) cancel: &'a Cancellation,
    /// Asked about every destination a Replace changes.
    pub(crate) guard: Option<&'a WriteGuard>,
    /// Receives byte progress.
    pub(crate) emit: &'a mut dyn FnMut(Progress),
}

impl StagedCopy<'_> {
    /// Builds, publishes and verifies the copy. `staging` receives the
    /// staging as soon as it exists, so the caller can clean it up on any
    /// failure.
    ///
    /// # Errors
    ///
    /// Any failure while staging, publishing or verifying. Nothing is
    /// visible under the final name unless publishing succeeded.
    pub(crate) fn run(mut self, staging: &mut ItemStaging) -> Result<(), TransferError> {
        let stage_name = staging_name()?;
        let mut modes = DirectoryModes::default();
        staging.place = StagingPlace::of(self.destination_folder);
        let layout = self.layout();
        match layout {
            Layout::Sibling => self.build_sibling(&stage_name, &mut modes, staging)?,
            Layout::Payload | Layout::SameDeviceCopy => {
                self.build_in_folder(&stage_name, layout, &mut modes, staging)?;
            }
        }
        self.cancel.check()?;
        // Building succeeded, so the stage is recorded; this keeps that
        // invariant without a panic.
        let stage = staging
            .stage
            .as_ref()
            .ok_or_else(|| TransferError::failed("The copy was not staged. Nothing was published."))?;
        self.publish(stage, &mut modes)
            .map_err(|error| explain_publish_error(error, layout))?;
        if staging.place == StagingPlace::Device {
            // XFER-021: a device's success report is not proof.
            verify_device_publication(stage, self.destination)?;
            if matches!(stage, Stage::Sibling(_)) {
                // The staged item now is the published item: nothing is
                // left to clean up.
                staging.stage = None;
            }
        }
        Ok(())
    }

    /// Where this copy is built: a same-device file copy must follow MTP
    /// `CopyObject`, other device copies stage beside the final name, and
    /// everything else uses a private folder.
    fn layout(&self) -> Layout {
        let is_file = self.source_kind != NodeKind::Directory;
        if is_file && self.source.native_copy_keeps_name(self.destination_folder) {
            Layout::SameDeviceCopy
        } else if self.destination_folder.has_sibling_staging() {
            Layout::Sibling
        } else {
            Layout::Payload
        }
    }

    /// XFER-021: builds the item itself under the hidden name `stage_name`
    /// beside its final name.
    fn build_sibling(
        &mut self,
        stage_name: &str,
        modes: &mut DirectoryModes,
        staging: &mut ItemStaging,
    ) -> Result<(), TransferError> {
        let staged_item = child_node(self.destination_folder, stage_name)?;
        // Nothing can exist under a fresh random name unless another program
        // created it, and then it is not ours to use or remove.
        if staged_item.exists(Some(self.cancel)) {
            return Err(TransferError::failed(
                "Could not reserve a private staging name. Nothing was changed.",
            ));
        }
        let mut copier = Copier::new(self.cancel, stage_name, modes, &mut *self.emit);
        if self.source_kind == NodeKind::Directory {
            // XFER-002: a failed exclusive folder creation grants no right to
            // clean up this path.
            staged_item.create_directory(Some(self.cancel))?;
            let stage = staging.stage.insert(Stage::Sibling(staged_item));
            return copier.copy_children(self.source, stage.item(), 1);
        }
        // Recorded before copying, so a failed or cancelled upload is removed.
        let stage = staging.stage.insert(Stage::Sibling(staged_item));
        let uploaded = copier.copy(self.source, stage.item(), 0);
        if matches!(uploaded, Err(TransferError::Exists(_))) {
            // XFER-002: the copy never overwrites, so another program took
            // the name after the check above, and its item must survive
            // cleanup.
            staging.stage = None;
        }
        uploaded
    }

    /// Builds the item inside a new private folder named `stage_name`.
    fn build_in_folder(
        &mut self,
        stage_name: &str,
        layout: Layout,
        modes: &mut DirectoryModes,
        staging: &mut ItemStaging,
    ) -> Result<(), TransferError> {
        let folder = child_node(self.destination_folder, stage_name)?;
        // XFER-023: a copy within one MTP device runs as CopyObject, which
        // keeps the SOURCE name whatever target is requested: copy under
        // that name.
        let item_name = if layout == Layout::SameDeviceCopy {
            self.source.name()
        } else {
            OsString::from(PAYLOAD_NAME)
        };
        let item = child_node(folder.as_ref(), item_name)?;
        // XFER-002: reserve a private namespace. A failed folder creation
        // never grants permission to delete that name during cleanup.
        folder.create_directory(Some(self.cancel))?;
        let stage = staging.stage.insert(Stage::Folder { folder, item });
        // XFER-004: the folder that was made private is the only one cleanup
        // may empty.
        staging.created = secure_local_staging(stage.root())?;
        let mut copier = Copier::new(self.cancel, stage_name, modes, &mut *self.emit);
        copier.copy(self.source, stage.item(), 0)?;
        if layout == Layout::SameDeviceCopy {
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
        // XFER-023: a device's success report is not proof that the copy
        // exists where the private folder expects it.
        if !item.exists(Some(self.cancel)) {
            return Err(TransferError::failed(
                "The device did not place the copy in its private staging folder. \
                 Nothing was published.",
            ));
        }
        let final_name = self.destination.name();
        if final_name != self.source.name() {
            let renamed = child_node(folder.as_ref(), final_name)?;
            item.move_native(renamed.as_ref(), Some(self.cancel))?;
            *item = renamed;
        }
        Ok(())
    }

    /// XFER-001: installs the completed item under its final name with a
    /// native rename in the destination folder. Replace is only reached
    /// after the user explicitly chose it; every other policy keeps the
    /// no-overwrite race guard (XFER-007).
    fn publish(&self, stage: &Stage, modes: &mut DirectoryModes) -> Result<(), TransferError> {
        if self.policy == ConflictPolicy::Replace {
            commit_replace(
                stage.item(),
                self.destination,
                self.cancel,
                self.guard,
                Some(modes),
            )
        } else {
            publish_staged(stage.item(), self.destination, modes, self.cancel)
        }
    }
}

/// A copy within one device ends with a move out of its private folder
/// (MTP `MoveObject`), which devices such as Android 7 and 8 phones do not
/// offer. Their refusal is explained in those terms rather than as the
/// generic unsupported move, which blames a cross-filesystem move.
fn explain_publish_error(error: TransferError, layout: Layout) -> TransferError {
    let is_move_object_refusal =
        layout == Layout::SameDeviceCopy && matches!(error, TransferError::NotSupported(_));
    if !is_move_object_refusal {
        return error;
    }
    TransferError::NotSupported(
        "This device cannot move items between folders, so a copy within the device \
         cannot be finished. Nothing was published. Copy the item to this computer \
         first, then copy it back to the device."
            .into(),
    )
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
