// SPDX-License-Identifier: AGPL-3.0-only
//! Installing a completed item under its final name.
//!
//! Ports `_publish_staged`, `_commit_replace` and `_replace_via_backup` in
//! `desktop/operations.py`.
//!
//! Rules enforced here:
//! - Publishing is a native, non-overwriting rename. If another program
//!   took the name meanwhile, publishing fails and nothing is overwritten.
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

use super::error::TransferError;
use super::guard::{nesting_error, MAX_DEPTH};
use super::modes::{
    open_directory_nofollow, restore_directory_modes, set_mode, DirectoryModes, PRIVATE_DIRECTORY_MODE,
};
use super::names::{backup_name, child_node};
use super::node::{Cancellation, Node, NodeKind, WriteGuard};

/// Attempts to find a free backup name before giving up.
const BACKUP_NAME_ATTEMPTS: usize = 100;

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
        return source.move_native(destination, Some(cancel));
    };
    // Opened before the rename and without following links, so the mode
    // lands on the folder the engine built even after it moved.
    let directory = open_directory_nofollow(&root.path)?;
    let moved = move_with_owner_access(source, destination, modes, cancel, &directory, root.mode);
    // Always leave the exact final mode, whether or not the rename worked.
    match set_mode(&directory, root.mode) {
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

fn move_with_owner_access(
    source: &dyn Node,
    destination: &dyn Node,
    modes: &mut DirectoryModes,
    cancel: &Cancellation,
    directory: &std::fs::File,
    final_mode: u32,
) -> Result<(), TransferError> {
    if !modes.is_empty() {
        restore_directory_modes(source, modes, cancel)?;
    }
    set_mode(directory, final_mode | PRIVATE_DIRECTORY_MODE)?;
    // Never overwrites: another program that took the name meanwhile keeps
    // it, and the staged copy stays private for cleanup.
    source.move_native(destination, Some(cancel))
}

/// Commits one completed item using Windows-like Replace semantics.
///
/// `modes` is `Some` for staged copies (their folders get final modes when
/// published) and `None` for moves of the user's own items.
pub(crate) fn commit_replace(
    source: &dyn Node,
    destination: &dyn Node,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
    modes: Option<&mut DirectoryModes>,
) -> Result<(), TransferError> {
    replace_at_depth(source, destination, cancel, guard, modes, 0)
}

fn replace_at_depth(
    source: &dyn Node,
    destination: &dyn Node,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
    mut modes: Option<&mut DirectoryModes>,
    depth: usize,
) -> Result<(), TransferError> {
    cancel.check()?;
    if depth > MAX_DEPTH {
        return Err(nesting_error());
    }
    if let Some(guard) = guard {
        guard(&destination.uri())?;
    }
    if !destination.exists(Some(cancel)) {
        return match modes {
            Some(modes) => publish_staged(source, destination, modes, cancel),
            None => source.move_native(destination, Some(cancel)),
        };
    }
    let incoming = source.info(Some(cancel))?.kind;
    let existing = destination.info(Some(cancel))?.kind;
    if incoming == NodeKind::Directory && existing == NodeKind::Directory {
        // Merge: commit each child, keep destination-only items, then remove
        // the now empty incoming folder (a plain delete cannot remove a
        // folder that still has contents).
        for child in source.children(Some(cancel))? {
            let target = child_node(destination, child.name())?;
            replace_at_depth(
                child.as_ref(),
                target.as_ref(),
                cancel,
                guard,
                modes.as_deref_mut(),
                depth + 1,
            )?;
        }
        source.delete()?;
        if let Some(modes) = modes {
            modes.take(&source.uri());
        }
        return Ok(());
    }
    if incoming == NodeKind::Directory || existing == NodeKind::Directory {
        return Err(TransferError::failed(
            "A file and folder have the same name. Rename or remove one of them, then try again.",
        ));
    }
    let replaceable = |kind: NodeKind| matches!(kind, NodeKind::File | NodeKind::Symlink);
    if !replaceable(incoming) || !replaceable(existing) {
        return Err(TransferError::failed(
            "This item type cannot be replaced automatically.",
        ));
    }
    match source.replace_native(destination, Some(cancel)) {
        Err(TransferError::ReplaceUnsupported(_)) => replace_via_backup(source, destination, cancel),
        other => other,
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
    // Last chance to stop: from here on, the renames run to completion. The
    // move-aside gets no cancellation because a device can finish a rename
    // after the client stopped waiting for it.
    cancel.check()?;
    let moved_aside = destination
        .move_native(backup.as_ref(), None)
        .and_then(|()| verify_installation(destination, backup.as_ref()));
    if let Err(aside_error) = moved_aside {
        let backup_info = backup.info(None);
        let destination_info = destination.info(None);
        match (backup_info, destination_info) {
            (Ok(_), Err(error)) if error.is_not_found() => {
                restore_backup(backup.as_ref(), destination)?;
            }
            (Err(error), Ok(_)) if error.is_not_found() => {}
            (_, _) if matches!(aside_error, TransferError::Exists(_)) => {}
            _ => return Err(TransferError::RecoveryRequired(format!(
                "Replacement stopped before installation. Check {} and the possible recovery file at {} before retrying. {aside_error}",
                destination.uri(), backup.uri()
            ))),
        }
        return Err(aside_error);
    }
    // Once the old file is aside, finish the tiny install step even if the
    // user cancels meanwhile: stopping here would leave the public name empty.
    let installed = source
        .move_native(destination, None)
        .and_then(|()| verify_installation(source, destination));
    if let Err(install_error) = installed {
        restore_backup(backup.as_ref(), destination)?;
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

/// Rollback uses the same non-overwriting move and explicit verification as
/// installation. Never claim restoration solely from a backend success code.
fn restore_backup(backup: &dyn Node, destination: &dyn Node) -> Result<(), TransferError> {
    backup.move_native(destination, None)
        .and_then(|()| verify_installation(backup, destination))
        .map_err(|error| TransferError::RecoveryRequired(format!(
            "Replacement failed and restoration could not be verified. Check the original at {} and restore it manually before retrying. {error}",
            backup.uri()
        )))
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
            "The backend reported success, but the incoming item was not moved. The prior file was retained.",
        )),
    }
}

/// Finds a free `.winspace-replaced-<32 hex>.backup` name in `parent`.
fn reserve_backup_name(parent: &dyn Node, cancel: &Cancellation) -> Result<Box<dyn Node>, TransferError> {
    for _ in 0..BACKUP_NAME_ATTEMPTS {
        let candidate = child_node(parent, &backup_name()?)?;
        if !candidate.exists(Some(cancel)) {
            return Ok(candidate);
        }
    }
    Err(TransferError::failed(
        "Could not reserve a temporary replacement name.",
    ))
}
