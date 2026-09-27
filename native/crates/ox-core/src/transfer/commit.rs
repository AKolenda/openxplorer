// SPDX-License-Identifier: AGPL-3.0-only
//! Installing a completed item under its final name.
//!
//! Ports `_publish_staged`, `_commit_replace` and `_replace_via_backup` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - Publishing ([`Node::publish`]) is a native rename that never
//!   overwrites. If another program took the name meanwhile, publishing
//!   fails and nothing is overwritten. On local disks the kernel checks the
//!   name and renames in one step; filesystems without that fall back to
//!   GIO's check-then-rename, which leaves a window of microseconds, as in
//!   the Python app (see `gio_node::move_item`).
//! - Replace (an explicit user choice) merges same-name folders, keeping
//!   destination-only items, and overwrites files only through the
//!   backend's explicit overwrite move. A file/folder type mismatch is left
//!   untouched rather than deleting a tree as a side effect of a batch
//!   choice.
//! - Where one-step overwrite is unsupported (MTP), the old file is renamed
//!   aside to an unguessable backup name, the new one installed, and the old
//!   name restored if installing fails. These renames are not interruptible:
//!   a device can finish a rename after the client stopped waiting, and
//!   stopping midway would leave the public name empty.

use std::fs::File;

use super::error::TransferError;
use super::guard::{nesting_error, MAX_DEPTH};
use super::modes::{
    open_directory_nofollow, restore_directory_modes, set_mode, DirectoryModes, PRIVATE_DIRECTORY_MODE,
};
use super::names::{backup_name, child_node};
use super::node::{Cancellation, Node, NodeKind, WriteGuard};

/// Attempts to find a free backup name before giving up.
const BACKUP_NAME_ATTEMPTS: usize = 100;

/// A staged local folder, opened before publishing, and its final mode.
struct StagedFolder {
    /// Opened without following links, so the mode lands on the folder the
    /// engine built even after it moved.
    directory: File,
    /// The source folder's permission bits, applied once it is published.
    mode: u32,
}

/// Moves a completed staged item to `destination` without overwriting,
/// then gives a staged local folder tree its final permissions.
///
/// Linux requires owner write access when moving a directory between
/// parents, so the root keeps it just for the rename and gets its exact
/// mode afterwards through the already opened descriptor. Descendants get
/// their exact modes before the rename. Group and other access only appears
/// once the staging parent stops being the only way in (it is `0700`), so
/// private contents are never exposed.
pub(crate) fn publish_staged(
    source: &dyn Node,
    destination: &dyn Node,
    modes: &mut DirectoryModes,
    cancel: &Cancellation,
) -> Result<(), TransferError> {
    // Only local staged folders have a recorded mode; everything else (files,
    // links, device and network items) is published by a plain rename.
    let Some(root) = modes.take(&source.uri()) else {
        return source.publish(destination, Some(cancel));
    };
    let staged = StagedFolder {
        directory: open_directory_nofollow(&root.path)?,
        mode: root.mode,
    };
    let moved = move_with_owner_access(source, destination, modes, cancel, &staged);
    // Always leave the exact final mode, whether or not the rename worked.
    match set_mode(&staged.directory, staged.mode) {
        Ok(()) => moved,
        Err(error) if moved.is_ok() => Err(TransferError::RecoveryRequired(format!(
            "The copied folder exists at {}, but its final permissions could not be restored. {error}",
            destination.uri()
        ))),
        // Like the Python engine, a failure to restore the mode after a
        // failed rename is what gets reported.
        Err(error) => Err(error.into()),
    }
}

/// Restores the descendants' final modes, gives the staged root owner write
/// access for the rename, then publishes it.
fn move_with_owner_access(
    source: &dyn Node,
    destination: &dyn Node,
    modes: &mut DirectoryModes,
    cancel: &Cancellation,
    staged: &StagedFolder,
) -> Result<(), TransferError> {
    if !modes.is_empty() {
        restore_directory_modes(source, modes, cancel)?;
    }
    set_mode(&staged.directory, staged.mode | PRIVATE_DIRECTORY_MODE)?;
    // Never overwrites: another program that took the name meanwhile keeps
    // it, and the staged copy stays private for cleanup.
    source.publish(destination, Some(cancel))
}

/// Commits one completed item using Windows-like Replace semantics.
///
/// `modes` is `Some` for staged copies (their folders get final modes when
/// published) and `None` for moves of the user's own items. `guard` is
/// asked about every destination before it changes.
pub(crate) fn commit_replace(
    source: &dyn Node,
    destination: &dyn Node,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
    modes: Option<&mut DirectoryModes>,
) -> Result<(), TransferError> {
    Replacement { cancel, guard }.replace(source, destination, modes, 0)
}

/// One Replace commit of a top-level item.
struct Replacement<'a> {
    cancel: &'a Cancellation,
    guard: Option<&'a WriteGuard>,
}

impl Replacement<'_> {
    /// Commits `source` over `destination`, at nesting `depth`.
    fn replace(
        &self,
        source: &dyn Node,
        destination: &dyn Node,
        modes: Option<&mut DirectoryModes>,
        depth: usize,
    ) -> Result<(), TransferError> {
        self.cancel.check()?;
        if depth > MAX_DEPTH {
            return Err(nesting_error());
        }
        if let Some(guard) = self.guard {
            guard(&destination.uri())?;
        }
        if !destination.exists(Some(self.cancel)) {
            return match modes {
                Some(modes) => publish_staged(source, destination, modes, self.cancel),
                None => source.move_native(destination, Some(self.cancel)),
            };
        }
        let incoming = source.info(Some(self.cancel))?.kind;
        let existing = destination.info(Some(self.cancel))?.kind;
        match (incoming, existing) {
            (NodeKind::Directory, NodeKind::Directory) => self.merge(source, destination, modes, depth),
            (NodeKind::Directory, _) | (_, NodeKind::Directory) => Err(TransferError::failed(
                "A file and folder have the same name. Rename or remove one of them, then try again.",
            )),
            (NodeKind::File | NodeKind::Symlink, NodeKind::File | NodeKind::Symlink) => {
                self.overwrite(source, destination)
            }
            _ => Err(TransferError::failed(
                "This item type cannot be replaced automatically.",
            )),
        }
    }

    /// Merges the folder `source` into the existing folder `destination`:
    /// each child is committed, destination-only items stay, and the then
    /// empty incoming folder is removed (a plain delete cannot remove a
    /// folder that still has contents).
    fn merge(
        &self,
        source: &dyn Node,
        destination: &dyn Node,
        mut modes: Option<&mut DirectoryModes>,
        depth: usize,
    ) -> Result<(), TransferError> {
        for child in source.children(Some(self.cancel))? {
            let target = child_node(destination, child.name())?;
            self.replace(child.as_ref(), target.as_ref(), modes.as_deref_mut(), depth + 1)?;
        }
        source.delete()?;
        if let Some(modes) = modes {
            modes.take(&source.uri());
        }
        Ok(())
    }

    /// Overwrites one file or link, reversibly where the backend cannot do
    /// it in one step.
    fn overwrite(&self, source: &dyn Node, destination: &dyn Node) -> Result<(), TransferError> {
        match source.replace_native(destination, Some(self.cancel)) {
            Err(TransferError::ReplaceUnsupported(_)) => replace_via_backup(source, destination, self.cancel),
            other => other,
        }
    }
}

/// Replaces a file on backends such as MTP using reversible renames.
///
/// The old destination is kept under an unguessable sibling name until the
/// completed incoming file is installed. If installing fails, the old name
/// is restored. No copy/delete fallback is used.
fn replace_via_backup(
    source: &dyn Node,
    destination: &dyn Node,
    cancel: &Cancellation,
) -> Result<(), TransferError> {
    let parent = destination
        .parent()
        .ok_or_else(|| TransferError::failed("Filesystem roots cannot be replaced."))?;
    let backup = reserve_backup_name(parent.as_ref(), cancel)?;
    let backup = backup.as_ref();
    // Last chance to stop: from here on, the renames run to completion.
    cancel.check()?;
    move_aside(destination, backup)?;
    // Once the old file is aside, finish the tiny install step even if the
    // user cancels meanwhile: stopping here would leave the public name empty.
    let installed = source
        .move_native(destination, None)
        .and_then(|()| verify_installation(source, destination));
    if let Err(install_error) = installed {
        restore_backup(backup, destination)?;
        return Err(install_error);
    }
    // Only now, with the new file installed, is the old one discarded, as the
    // user's Replace asked. A failure leaves it, and the message says where.
    backup.delete().map_err(|cleanup_error| {
        TransferError::RecoveryRequired(format!(
            "Replacement completed, but the prior file remains at {}. \
             Remove that backup after checking the new file. {cleanup_error}",
            backup.uri()
        ))
    })
}

/// Renames the old file to its backup name. The rename gets no
/// cancellation because a device can finish a rename after the client
/// stopped waiting for it; a failure is checked against what actually
/// happened, so the original never stays under the hidden backup name.
fn move_aside(destination: &dyn Node, backup: &dyn Node) -> Result<(), TransferError> {
    let moved = destination
        .move_native(backup, None)
        .and_then(|()| verify_installation(destination, backup));
    let Err(aside_error) = moved else {
        return Ok(());
    };
    match (backup.info(None), destination.info(None)) {
        // The device finished the rename after all: put the original back.
        (Ok(_), Err(error)) if error.is_not_found() => restore_backup(backup, destination)?,
        // Nothing moved.
        (Err(error), Ok(_)) if error.is_not_found() => {}
        // The backup name was taken meanwhile; the original was not moved.
        _ if matches!(aside_error, TransferError::Exists(_)) => {}
        _ => {
            return Err(TransferError::RecoveryRequired(format!(
                "Replacement stopped before installation. Check {} and the possible \
                 recovery file at {} before retrying. {aside_error}",
                destination.uri(),
                backup.uri()
            )))
        }
    }
    Err(aside_error)
}

/// Rollback uses the same non-overwriting move and explicit verification as
/// installation. Never claim restoration solely from a backend success code.
fn restore_backup(backup: &dyn Node, destination: &dyn Node) -> Result<(), TransferError> {
    backup
        .move_native(destination, None)
        .and_then(|()| verify_installation(backup, destination))
        .map_err(|error| {
            TransferError::RecoveryRequired(format!(
                "Replacement failed and restoration could not be verified. Check the original \
                 at {} and restore it manually before retrying. {error}",
                backup.uri()
            ))
        })
}

/// A device may report success without moving the requested item. Retain
/// the old file until both names have been checked after rebuilding caches.
pub(crate) fn verify_installation(source: &dyn Node, destination: &dyn Node) -> Result<(), TransferError> {
    let source_parent = source.parent();
    if let Some(parent) = &source_parent {
        parent.refresh_listing(None)?;
    }
    if let Some(parent) = destination.parent() {
        let already_refreshed = source_parent
            .as_ref()
            .is_some_and(|source| source.uri() == parent.uri());
        if !already_refreshed {
            parent.refresh_listing(None)?;
        }
    }
    destination.info(None)?;
    match source.info(None) {
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(TransferError::failed(
            "The backend reported success, but the incoming item was not moved. \
             The prior file was retained.",
        )),
    }
}

/// Finds a free `.winspace-replaced-<32 hex>.backup` name in `parent`.
fn reserve_backup_name(parent: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn Node>, TransferError> {
    for _ in 0..BACKUP_NAME_ATTEMPTS {
        let candidate = child_node(parent, backup_name()?)?;
        if !candidate.exists(Some(cancel)) {
            return Ok(candidate);
        }
    }
    Err(TransferError::failed(
        "Could not reserve a temporary replacement name.",
    ))
}
