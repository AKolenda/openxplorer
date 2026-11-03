// SPDX-License-Identifier: AGPL-3.0-only
//! Permanent deletion of a whole folder under `admin://`.
//!
//! Ports no Python module: the Python app deleted `admin://` folders by
//! path through `gvfsd-admin`, exactly as `gio_node::remote_delete` still
//! does for shares and phones. That is safe enough for a remote server,
//! which resolves its own paths, and unsafe for a local path resolved
//! again and again by a process running as root: a folder inside the tree
//! that is swapped for a symbolic link between two steps of the walk sends
//! the next step wherever the link points, with root's rights.
//!
//! So an `admin://` folder is not deleted by path. The app starts
//! `openxplorer-admin-delete` through `pkexec` ([`elevation`]), and that
//! helper walks the tree through pinned folder descriptors ([`walk`]), the
//! way [`crate::gio_node`] already deletes `file:` items. Single files and
//! symbolic links under `admin://` still go through `gvfsd-admin`: one
//! `unlink` of one name resolves nothing a second time, so there is no
//! walk to redirect, and no administrator prompt for the user to answer.
//!
//! 2.0.12 closed the same hole the other way, by refusing up front every
//! `admin://` folder another account could change, or that was on a drive
//! whose owners and modes could not be believed, because the walk by path
//! could not be made safe for those. This walk replaces that refusal: it
//! deletes those folders, and `tests::others_can_change` holds each shape
//! 2.0.12 refused, with the advisory's race against it. A Flatpak is still
//! refused ([`elevation::helper_path`]), because a helper there would walk
//! the sandbox's view of the filesystem rather than the host's.
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`walk`] | The privileged walk, the rules root needs on top, and the trusted resolution of the item's own folder |
//! | [`report`] | The line protocol the helper writes and the app reads |
//! | [`command`] | The `openxplorer-admin-delete` program itself |
//! | [`elevation`] | Starting the helper, and the replaceable step that elevates |
//!
//! **The write guard does not run inside the helper.** XFER-020's check for
//! protected locations (previous versions, `.snapshot` folders) depends on
//! app state that a short-lived root program has no business reading. It
//! does not need to: [`crate::transfer`]'s `check_write_tree` preflight
//! already asks the guard about every URI in the tree, from the app and as
//! the user, before any deletion starts, and a refusal anywhere stops the
//! whole item before the helper is started at all.

mod command;
mod elevation;
mod report;
mod walk;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use gio::prelude::*;

pub use command::admin_delete_command;
pub use elevation::{helper_path, set_elevation, Elevation, HELPER_COMMAND};
pub use report::{Line, Record, Totals};

use crate::i18n::format_message;
use crate::transfer::{Cancellation, NodeKind, TransferError, WriteGuard};

/// How many items a refusal names before it stops listing them.
const LISTED_ITEMS: usize = 5;

/// True for an `admin://` folder, which is deleted through the privileged
/// helper rather than by path through `gvfsd-admin`.
///
/// `kind` is what the caller found the item to be, without following
/// links. Anything but a folder is one `unlink` of one name: there is no
/// walk to redirect, so it stays on `gvfsd-admin` and the user is not
/// asked to authenticate. A folder is asked about once more, as
/// [`super::gio_node`]'s remote walk does, because a link may have taken
/// its place since the folder was listed.
///
/// # Errors
///
/// The backend's error when the item cannot be inspected, and
/// [`TransferError::Cancelled`].
pub(crate) fn is_admin_folder(
    file: &gio::File,
    kind: Option<NodeKind>,
    cancel: &Cancellation,
) -> Result<bool, TransferError> {
    if !file.has_uri_scheme("admin") {
        return Ok(false);
    }
    if matches!(kind, Some(kind) if kind != NodeKind::Directory) {
        return Ok(false);
    }
    let info = file.query_info(
        "standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        Some(cancel.cancellable()),
    )?;
    Ok(info.file_type() == gio::FileType::Directory)
}

/// Permanently deletes the `admin://` folder `file` and everything inside
/// it, through the privileged helper.
///
/// # Errors
///
/// The guard's refusal for the folder itself, [`TransferError::Cancelled`],
/// the helper's refusal when it will not touch the path at all, and a
/// summary of what is still there when the walk finished incomplete. What
/// the walk deleted before that stays deleted, as it does everywhere else.
pub(crate) fn delete_tree(
    file: &gio::File,
    cancel: &Cancellation,
    guard: Option<&WriteGuard>,
) -> Result<(), TransferError> {
    cancel.check()?;
    let path = local_path(file).ok_or_else(|| {
        TransferError::failed(format_message(
            "{uri} is not an administrator location of this computer.",
            &[("uri", file.uri().as_ref())],
        ))
    })?;
    // The whole tree was checked by the engine's XFER-020 preflight before
    // anything started; this is the same last look every other backend
    // takes at the item it is about to remove.
    if let Some(guard) = guard {
        guard(&file.uri())?;
        cancel.check()?;
    }
    let mut left_behind = Vec::new();
    let totals = elevation::elevated_delete(&path, cancel, &mut |record| match record {
        Record::Deleted(_) => {}
        Record::Skipped { path, reason } => {
            left_behind.push(format!("{}: {reason}", path.display()));
        }
        Record::Failed { path, message } => {
            left_behind.push(format!("{}: {message}", path.display()));
        }
    })?;
    if totals.is_complete() {
        return Ok(());
    }
    Err(TransferError::failed(incomplete_message(&left_behind)))
}

/// The path of an `admin://` location on this computer.
///
/// `admin://` addresses local paths with administrator rights, so its URIs
/// carry no host. One that does belongs to another backend and is left
/// alone.
fn local_path(file: &gio::File) -> Option<PathBuf> {
    let parts = crate::location::split_location(&file.uri()).ok()?;
    if parts.scheme != "admin" || !parts.authority.is_empty() {
        return None;
    }
    // A path is bytes, not text: the escaped form is decoded to the bytes
    // it stands for, so a name that is not valid UTF-8 survives.
    let decoded = glib::uri_unescape_string(&parts.path, None::<&str>)?;
    let path = PathBuf::from(OsString::from_vec(decoded.as_bytes().to_vec()));
    path.is_absolute().then_some(path)
}

/// The refusal that names what the deletion left behind.
fn incomplete_message(left_behind: &[String]) -> String {
    let listed = left_behind
        .iter()
        .take(LISTED_ITEMS)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if left_behind.len() <= LISTED_ITEMS {
        return format_message("Some items were not deleted:\n{items}", &[("items", &listed)]);
    }
    format_message(
        "Some items were not deleted:\n{items}\n… and {more} more.",
        &[
            ("items", &listed),
            ("more", &(left_behind.len() - LISTED_ITEMS).to_string()),
        ],
    )
}

#[cfg(test)]
mod tests;
