// SPDX-License-Identifier: AGPL-3.0-only
//! The name-conflict check before a paste or drop (OPS-026, OPS-027).
//!
//! Ports the `transferConflicts` branch of `dispatch` in
//! `v2.0.0:desktop/winspace.py`. The interface asks "Items already exist" only
//! when this finds a taken name, and otherwise copies at once with the
//! Skip policy, so a name that appears after the check is still never
//! overwritten.

use super::context::{on_worker, unless_cancelled};
use super::error::OpsError;
use crate::gio_node::GioNode;
use crate::location::{normalise, require_item_uri};
use crate::transfer::{Cancellation, Node, NodeKind, MAX_ITEMS};

/// The items of `uris` whose names are already taken in the folder at
/// `destination_folder`, in the order given. Hidden items and dangling
/// links count: their names are taken too.
///
/// # Errors
///
/// No items or more than [`MAX_ITEMS`], a share or device root among them,
/// a destination that is not a folder ("Open a destination folder before
/// pasting."), or the backend's failure. A cancelled check is always
/// [`OpsError::Cancelled`], never a shorter list the interface would read
/// as "no conflicts" and start copying.
pub async fn find_conflicts(
    uris: &[String],
    destination_folder: &str,
    cancel: &Cancellation,
) -> Result<Vec<String>, OpsError> {
    let uris = uris.to_vec();
    let destination_folder = destination_folder.to_owned();
    let cancel = cancel.clone();
    on_worker(move || find_conflicts_blocking(&uris, &destination_folder, &cancel)).await
}

/// [`find_conflicts`] on the calling thread.
fn find_conflicts_blocking(
    uris: &[String],
    destination_folder: &str,
    cancel: &Cancellation,
) -> Result<Vec<String>, OpsError> {
    if uris.is_empty() || uris.len() > MAX_ITEMS {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Select between 1 and 100,000 items.",
        )));
    }
    let items = uris
        .iter()
        .map(|uri| require_item_uri(uri))
        .collect::<Result<Vec<String>, _>>()?;
    let folder = GioNode::new(&normalise(destination_folder)?);
    // Inspected without following a link, as the Python bridge does.
    if folder.info(Some(cancel))?.kind != NodeKind::Directory {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Open a destination folder before pasting.",
        )));
    }
    let mut conflicts = Vec::new();
    for uri in items {
        let name = GioNode::new(&uri).name();
        let destination = folder.child(&name);
        // OPS-027: a query the user cancelled answers "free", and "no
        // conflicts" starts the copy at once, so it must not count.
        if unless_cancelled(cancel, || destination.exists(Some(cancel)))? {
            conflicts.push(uri);
        }
    }
    Ok(conflicts)
}
