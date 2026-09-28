// SPDX-License-Identifier: AGPL-3.0-only
//! Create links: symbolic links in a folder to dropped items (DND-019).
//!
//! New in the native app, from the Dolphin baseline ("Link Here") and
//! Windows Explorer's "Create shortcuts here". A link is named after its
//! item and points to the item's absolute path.
//!
//! Safety rule "a link never replaces anything": a name that is taken in
//! the folder, by anything including a broken link, is reported for that
//! item and nothing is written there; the other items go on. The folder
//! must pass the write protection, like the destination of a copy
//! (XFER-020). Only local items can be linked into a local folder: GIO's
//! other backends make no symbolic links, and a link to an `smb://`
//! address would point nowhere for other apps.

use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::run_transfer::TransferOutcome;
use crate::location::{file_uri, normalise};
use crate::transfer::{TransferResult, MAX_ITEMS};

/// Why the folder takes no links.
const NOT_LOCAL_FOLDER: &str = "Links can only be created in folders on this computer.";

/// Why one item gets no link.
const NOT_LOCAL_ITEM: &str = "Links can only point to items on this computer.";

/// Why one item's link was not made.
const NAME_TAKEN: &str = "An item with this name already exists. Nothing was replaced.";

/// Links to some items, made in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRequest {
    /// The items to link to, in order.
    pub uris: Vec<String>,
    /// The folder the links go into.
    pub destination_folder: String,
}

/// Makes a symbolic link in the request's folder to each of its items, on
/// a worker thread. The outcome lists the new links, to select them.
///
/// # Errors
///
/// No items or more than [`MAX_ITEMS`], a folder that is not local or is
/// protected. Items that cannot be linked, and the user's cancellation,
/// are reported in [`TransferOutcome::result`].
pub async fn create_links(
    request: &LinkRequest,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let request = request.clone();
    let context = context.clone();
    on_worker(move || create_links_blocking(&request, &context)).await
}

/// [`create_links`] on the calling thread.
fn create_links_blocking(
    request: &LinkRequest,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    if request.uris.is_empty() || request.uris.len() > MAX_ITEMS {
        return Err(OpsError::failed("Select between 1 and 100,000 items."));
    }
    let folder_uri = normalise(&request.destination_folder)?;
    context.protection.check(&folder_uri)?;
    let folder = gio::File::for_uri(&folder_uri)
        .path()
        .ok_or_else(|| OpsError::failed(NOT_LOCAL_FOLDER))?;
    let mut outcome = TransferOutcome::default();
    for uri in &request.uris {
        if context.cancel.is_cancelled() {
            outcome.result.cancelled = true;
            break;
        }
        match link_item(uri, &folder) {
            Ok(link) => {
                outcome.result.done.push(uri.clone());
                outcome.created.push(file_uri(&link));
            }
            Err(reason) => record_refusal(&mut outcome.result, uri, &reason),
        }
    }
    Ok(outcome)
}

/// Makes the link to the item at `uri` in `folder`; the link's path, or
/// why there is none.
fn link_item(uri: &str, folder: &Path) -> Result<PathBuf, String> {
    let target = gio::File::for_uri(uri)
        .path()
        .ok_or_else(|| NOT_LOCAL_ITEM.to_owned())?;
    let name = target.file_name().ok_or_else(|| NOT_LOCAL_ITEM.to_owned())?;
    let link = folder.join(name);
    // `symlink` itself refuses a taken name, so a name that appears after
    // this check is never replaced either.
    if link.symlink_metadata().is_ok() {
        return Err(NAME_TAKEN.to_owned());
    }
    symlink(&target, &link).map_err(|error| describe(&error))?;
    Ok(link)
}

/// The message for a failed link.
fn describe(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AlreadyExists {
        return NAME_TAKEN.to_owned();
    }
    error.to_string()
}

/// Records that the item at `uri` got no link, as `name: reason`.
fn record_refusal(result: &mut TransferResult, uri: &str, reason: &str) {
    let file = gio::File::for_uri(uri);
    let name = file
        .basename()
        .map_or_else(|| uri.to_owned(), |name| name.to_string_lossy().into_owned());
    result.errors.push(format!("{name}: {reason}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::WriteProtection;
    use crate::test_support::temporary_folder;

    fn context() -> OperationContext {
        OperationContext::new(WriteProtection::unrestricted())
    }

    /// parity: DND-019
    #[test]
    fn a_link_points_to_the_item_and_is_named_after_it() {
        let folder = temporary_folder();
        let item = folder.path().join("Report (final).txt");
        std::fs::write(&item, b"report").expect("fixture file");
        let links = folder.path().join("Links");
        std::fs::create_dir(&links).expect("fixture folder");
        let request = LinkRequest {
            uris: vec![file_uri(&item)],
            destination_folder: file_uri(&links),
        };

        let outcome = create_links_blocking(&request, &context()).expect("the folder is local");

        let link = links.join("Report (final).txt");
        assert_eq!(std::fs::read_link(&link).expect("a symbolic link"), item);
        assert_eq!(outcome.result.done, [file_uri(&item)]);
        assert_eq!(outcome.created, [file_uri(&link)]);
    }

    /// parity: DND-019
    #[test]
    fn a_taken_name_is_reported_and_never_replaced() {
        let folder = temporary_folder();
        let item = folder.path().join("notes.txt");
        std::fs::write(&item, b"notes").expect("fixture file");
        let links = folder.path().join("Links");
        std::fs::create_dir(&links).expect("fixture folder");
        std::fs::write(links.join("notes.txt"), b"keep me").expect("fixture file");
        let request = LinkRequest {
            uris: vec![file_uri(&item)],
            destination_folder: file_uri(&links),
        };

        let outcome = create_links_blocking(&request, &context()).expect("the folder is local");

        assert!(outcome.result.done.is_empty());
        assert_eq!(outcome.result.errors, [format!("notes.txt: {NAME_TAKEN}")]);
        assert_eq!(std::fs::read(links.join("notes.txt")).expect("kept"), b"keep me");
    }

    /// parity: DND-019
    #[test]
    fn items_and_folders_that_are_not_local_get_no_links() {
        let folder = temporary_folder();
        let to_share = LinkRequest {
            uris: vec![file_uri(&folder.path().join("a.txt"))],
            destination_folder: "smb://nas/share/folder".to_owned(),
        };
        let from_share = LinkRequest {
            uris: vec!["smb://nas/share/a.txt".to_owned()],
            destination_folder: file_uri(folder.path()),
        };

        let refused = create_links_blocking(&to_share, &context());
        let outcome = create_links_blocking(&from_share, &context()).expect("the folder is local");

        assert!(matches!(refused, Err(OpsError::Failed(message)) if message == NOT_LOCAL_FOLDER));
        assert_eq!(outcome.result.errors, [format!("a.txt: {NOT_LOCAL_ITEM}")]);
    }
}
