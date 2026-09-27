// SPDX-License-Identifier: AGPL-3.0-only
//! Explicit Trash and permanent deletion, kept as separate operations.

use gio::prelude::*;

use super::GioNode;
use crate::transfer::{Cancellation, Node, NodeKind, TransferError, WriteGuard};

impl GioNode {
    pub(super) fn trash_item(&self, cancel: &Cancellation) -> Result<(), TransferError> {
        cancel.check()?;
        self.require_item()?;
        self.file.trash(Some(cancel.cancellable())).map_err(|error| {
            if error.kind::<gio::IOErrorEnum>() == Some(gio::IOErrorEnum::NotSupported) {
                TransferError::NotSupported("Trash is not supported at this location. The original item was not permanently deleted. Delete it permanently instead, or use the server’s recycle-bin policy.".into())
            } else {
                error.into()
            }
        })
    }

    pub(super) fn delete_remote_item(
        &self,
        cancel: &Cancellation,
        guard: Option<&WriteGuard>,
    ) -> Result<(), TransferError> {
        cancel.check()?;
        if let Some(guard) = guard {
            guard(&self.uri())?;
        }
        if self.query_info(Some(cancel))?.kind == NodeKind::Directory {
            // GIO offers path-based recursion, not handles that pin each
            // ancestor. Refuse until the remote adapter can guarantee that
            // a concurrent rename cannot redirect deletion outside the tree.
            return Err(TransferError::NotSupported(
                "Safe permanent deletion of remote folders is not available in this native preview.".into(),
            ));
        }
        cancel.check()?;
        self.file.delete(Some(cancel.cancellable())).map_err(Into::into)
    }
}
