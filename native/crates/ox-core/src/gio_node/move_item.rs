// SPDX-License-Identifier: AGPL-3.0-only
//! Native moves and the MTP device rename rules.
//!
//! Ports `GioNode.move_native`, `GioNode.replace_native` and
//! `GioNode._rename_mtp` in `desktop/gio_backend.py`.
//!
//! Rules enforced here:
//! - No move ever falls back to copy-then-delete (`NO_FALLBACK_FOR_MOVE`).
//! - A move without Replace never overwrites. GIO checks the target and
//!   then renames, which leaves a tiny window for another program to create
//!   the name in between; the Python app has the same window. A kernel
//!   no-replace rename would close it, but it bypasses GIO's local move,
//!   which also moves the item's `GVfs` metadata (emblems, icon positions).
//! - On MTP, a move within one folder is a rename (`set_display_name`, MTP
//!   `SetObjectPropValue`), a move to another folder keeps the item's name
//!   (MTP `MoveObject`), and Replace is never done in one step, because
//!   `GVfs` deletes the existing item before it moves and cannot restore it.

use gio::prelude::*;

use super::{check, raw, GioNode};
use crate::transfer::{verify_installation, Cancellation, Node, TransferError};

/// Whether a move may overwrite an existing item at the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Overwrite {
    /// The target name must be free.
    Never,
    /// The user explicitly chose Replace for this item.
    Replace,
}

/// `MOVE_FLAGS` in `desktop/gio_backend.py`.
const MOVE_FLAGS: gio::FileCopyFlags =
    gio::FileCopyFlags::NOFOLLOW_SYMLINKS.union(gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE);

impl GioNode {
    /// Moves or renames this item to `target` natively.
    pub(super) fn move_item(
        &self,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
        overwrite: Overwrite,
    ) -> Result<(), TransferError> {
        check(cancel)?;
        self.require_item()?;
        let target_file = gio::File::for_uri(&target.uri());
        if self.is_mtp() {
            if overwrite == Overwrite::Replace {
                return Err(TransferError::ReplaceUnsupported(
                    "This device cannot replace an item in one step.".into(),
                ));
            }
            if self.has_same_parent(&target_file) {
                return self.rename_mtp(target, cancel);
            }
            // GVfs would report success and keep the old name.
            if self.name() != target.name() {
                return Err(TransferError::failed(
                    "This device can move an item to another folder or rename it, \
                     but not both in one step. Nothing was changed.",
                ));
            }
        }
        let flags = match overwrite {
            Overwrite::Never => MOVE_FLAGS,
            Overwrite::Replace => MOVE_FLAGS | gio::FileCopyFlags::OVERWRITE,
        };
        self.file
            .move_(&target_file, flags, raw(cancel), None)
            .map_err(|error| move_error(error, overwrite))
    }

    fn has_same_parent(&self, target_file: &gio::File) -> bool {
        match (self.file.parent(), target_file.parent()) {
            (Some(source_parent), Some(target_parent)) => source_parent.equal(&target_parent),
            _ => false,
        }
    }

    /// A same-folder rename through MTP `SetObjectPropValue`. The device
    /// refuses a taken name (reported as a failure); it never replaces the
    /// other item.
    fn rename_mtp(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        check(cancel)?;
        if target.exists(cancel) {
            return Err(name_taken(target));
        }
        // MTP object names are text; an item is never given a lossily
        // converted name.
        let Some(new_name) = target.name().to_str().map(str::to_owned) else {
            return Err(TransferError::failed(
                "This device only accepts names that are valid UTF-8. Nothing was changed.",
            ));
        };
        match self.file.set_display_name(&new_name, raw(cancel)) {
            Ok(_) => Ok(()),
            Err(error) => self.settle_failed_rename(target, error),
        }
    }

    /// A device can finish a rename after the client stopped waiting for
    /// it (cancellation, timeout), so a reported failure is checked against
    /// what actually happened. Both folders are relisted, and success needs
    /// the definite absence of the old name: a disconnected device is never
    /// counted as success.
    fn settle_failed_rename(&self, target: &dyn Node, error: glib::Error) -> Result<(), TransferError> {
        if verify_installation(self, target).is_ok() {
            return Ok(());
        }
        if self.info(None).is_ok() && target.info(None).is_ok() {
            return Err(name_taken(target));
        }
        Err(error.into())
    }
}

fn name_taken(target: &dyn Node) -> TransferError {
    TransferError::Exists(format!(
        "An item named “{}” already exists. Nothing was overwritten.",
        target.display_name()
    ))
}

/// Maps a failed native move. With Replace, "cannot overwrite here" asks the
/// engine for its reversible replacement; without it, an unsupported move
/// (for example across filesystems) is refused with an explanation.
fn move_error(error: glib::Error, overwrite: Overwrite) -> TransferError {
    let code = error.kind::<gio::IOErrorEnum>();
    let unsupported = matches!(
        code,
        Some(gio::IOErrorEnum::NotSupported | gio::IOErrorEnum::WouldRecurse)
    );
    let exists = code == Some(gio::IOErrorEnum::Exists);
    if overwrite == Overwrite::Replace && (unsupported || exists) {
        return TransferError::ReplaceUnsupported("The backend does not support direct overwrite.".into());
    }
    if unsupported {
        return TransferError::NotSupported(
            "A native move is not supported here. Cross-filesystem/cross-share moves are \
             deliberately disabled. Copy, verify, then trash the source separately."
                .into(),
        );
    }
    error.into()
}
