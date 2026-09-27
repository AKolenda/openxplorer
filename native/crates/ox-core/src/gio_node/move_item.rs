// SPDX-License-Identifier: AGPL-3.0-only
//! Native move and device rename rules. No operation falls back to copy/delete.

use gio::prelude::*;

use super::{check, raw, GioNode};
use crate::transfer::{Cancellation, Node, TransferError};

impl GioNode {
    pub(super) fn move_item(
        &self,
        target: &dyn Node,
        cancel: Option<&Cancellation>,
        replace: bool,
    ) -> Result<(), TransferError> {
        check(cancel)?;
        self.require_item()?;
        let target_file = gio::File::for_uri(&target.uri());
        if self.is_mtp() {
            if replace {
                return Err(TransferError::ReplaceUnsupported(
                    "This device cannot replace an item in one step.".into(),
                ));
            }
            let same_parent = self
                .file
                .parent()
                .zip(target_file.parent())
                .is_some_and(|(source, destination)| source.equal(&destination));
            if same_parent {
                return self.rename_mtp(target, cancel);
            }
            if self.name() != target.name() {
                return Err(TransferError::failed("This device can move an item to another folder or rename it, but not both in one step. Nothing was changed."));
            }
        }
        let mut flags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE;
        if replace {
            flags |= gio::FileCopyFlags::OVERWRITE;
        }
        self.file
            .move_(&target_file, flags, raw(cancel), None)
            .map_err(|error| move_error(error, replace))
    }

    fn rename_mtp(&self, target: &dyn Node, cancel: Option<&Cancellation>) -> Result<(), TransferError> {
        check(cancel)?;
        if target.exists(cancel) {
            return Err(name_taken(target));
        }
        match self.file.set_display_name(&target.name(), raw(cancel)) {
            Ok(_) => Ok(()),
            Err(error) => {
                // A device may finish a rename after the client timed out.
                // Relist both parents and require definite source absence;
                // a disconnected source must never be counted as success.
                if crate::transfer::verify_installation(self, target).is_ok() {
                    return Ok(());
                }
                if self.info(None).is_ok() && target.info(None).is_ok() {
                    Err(name_taken(target))
                } else {
                    Err(error.into())
                }
            }
        }
    }
}

fn name_taken(target: &dyn Node) -> TransferError {
    TransferError::Exists(format!(
        "An item named “{}” already exists. Nothing was overwritten.",
        target.name()
    ))
}

fn move_error(error: glib::Error, replace: bool) -> TransferError {
    let code = error.kind::<gio::IOErrorEnum>();
    let unsupported = matches!(
        code,
        Some(gio::IOErrorEnum::NotSupported | gio::IOErrorEnum::WouldRecurse)
    );
    if replace && (unsupported || code == Some(gio::IOErrorEnum::Exists)) {
        return TransferError::ReplaceUnsupported("The backend does not support direct overwrite.".into());
    }
    if unsupported {
        return TransferError::NotSupported("A native move is not supported here. Cross-filesystem/cross-share moves are deliberately disabled. Copy, verify, then trash the source separately.".into());
    }
    error.into()
}
