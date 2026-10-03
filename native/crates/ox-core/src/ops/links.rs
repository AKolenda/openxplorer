// SPDX-License-Identifier: AGPL-3.0-only
//! Create links: symbolic links in a folder to dropped items (DND-019),
//! and New ▸ Link to file or folder (OPS-004).
//!
//! New in the native app, from the Dolphin baseline ("Link Here" and
//! Create New ▸ "Basic Link to File or Directory…") and Windows Explorer's
//! "Create shortcuts here". A dropped item's link is named after it and
//! points to its absolute path; New ▸ Link asks for the path, and the name
//! defaults to the name of what it points to. Undo moves the links, never
//! what they point to, to the Trash.
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
use super::create::name_taken_or;
use super::error::OpsError;
use super::run_transfer::TransferOutcome;
use super::undo::UndoRecord;
use crate::location::{file_uri, normalise, validate_name};
use crate::transfer::{TransferResult, MAX_ITEMS};

/// Why the folder takes no links.
const NOT_LOCAL_FOLDER: &str =
    crate::i18n::message_id("Links can only be created in folders on this computer.");

/// Why one item gets no link.
const NOT_LOCAL_ITEM: &str = crate::i18n::message_id("Links can only point to items on this computer.");

/// Why one item's link was not made.
const NAME_TAKEN: &str =
    crate::i18n::message_id("An item with this name already exists. Nothing was replaced.");

/// Why New ▸ Link has nothing to point to.
const NO_TARGET: &str = crate::i18n::message_id("Enter the path of the file or folder to link to.");

/// A link New ▸ Link to file or folder makes (OPS-004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLink {
    /// The folder the link goes into.
    pub folder: String,
    /// The link's name; empty for the name of what it points to.
    pub name: String,
    /// What it points to, as typed: a path, which may start with `~/` or
    /// be relative to the folder, or a `file:` URI.
    pub target: String,
}

/// A link New ▸ Link made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedLink {
    /// The link's URI, to select it.
    pub uri: String,
}

impl CreatedLink {
    /// How Undo removes the link again: it goes to the Trash.
    pub fn undo_record(&self) -> UndoRecord {
        UndoRecord::Link {
            links: vec![self.uri.clone()],
        }
    }
}

/// Makes the symbolic link `request` describes, on a worker thread.
///
/// # Errors
///
/// No target, a target that does not exist, an invalid name, a folder
/// that is not local or is protected, a taken name ([`OpsError::Exists`];
/// nothing is replaced), or the system's failure.
pub async fn create_link(request: &NewLink, context: &OperationContext) -> Result<CreatedLink, OpsError> {
    let request = request.clone();
    let context = context.clone();
    on_worker(move || create_link_blocking(&request, &context)).await
}

/// [`create_link`] on the calling thread.
fn create_link_blocking(request: &NewLink, context: &OperationContext) -> Result<CreatedLink, OpsError> {
    let folder_uri = normalise(&request.folder)?;
    context.protection.check(&folder_uri)?;
    let folder = gio::File::for_uri(&folder_uri)
        .path()
        .ok_or_else(|| OpsError::failed(crate::i18n::gettext(NOT_LOCAL_FOLDER)))?;
    let target = link_target(&request.target)?;
    // A relative target is relative to the link's folder, as the system
    // resolves it.
    if folder.join(&target).symlink_metadata().is_err() {
        return Err(OpsError::NotFound(crate::i18n::format_message(
            "Nothing exists at “{display}”. Check the path.",
            &[("display", &(target.display()).to_string())],
        )));
    }
    let name = match request.name.trim() {
        "" => default_link_name(&target)?,
        typed => typed.to_owned(),
    };
    validate_name(&name)?;
    let link = folder.join(&name);
    symlink(&target, &link).map_err(|error| name_taken_or(error.into(), &name))?;
    Ok(CreatedLink { uri: file_uri(&link) })
}

/// The path a link points to, from what was typed: `~` is the home
/// folder, and a `file:` URI its local path.
fn link_target(typed: &str) -> Result<PathBuf, OpsError> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(OpsError::failed(crate::i18n::gettext(NO_TARGET)));
    }
    if typed.starts_with("file:") {
        return gio::File::for_uri(typed)
            .path()
            .ok_or_else(|| OpsError::failed(crate::i18n::gettext(NOT_LOCAL_ITEM)));
    }
    if typed.contains("://") {
        return Err(OpsError::failed(crate::i18n::gettext(NOT_LOCAL_ITEM)));
    }
    if typed == "~" {
        return Ok(glib::home_dir());
    }
    if let Some(below_home) = typed.strip_prefix("~/") {
        return Ok(glib::home_dir().join(below_home));
    }
    Ok(PathBuf::from(typed))
}

/// The name a link gets when none was typed: the name of what it points
/// to.
fn default_link_name(target: &Path) -> Result<String, OpsError> {
    target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| OpsError::failed(crate::i18n::gettext("Enter a name for the link.")))
}

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
        return Err(OpsError::failed(crate::i18n::gettext(
            "Select between 1 and 100,000 items.",
        )));
    }
    let folder_uri = normalise(&request.destination_folder)?;
    context.protection.check(&folder_uri)?;
    let folder = gio::File::for_uri(&folder_uri)
        .path()
        .ok_or_else(|| OpsError::failed(crate::i18n::gettext(NOT_LOCAL_FOLDER)))?;
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
    if !outcome.created.is_empty() {
        outcome.undo = Some(UndoRecord::Link {
            links: outcome.created.clone(),
        });
    }
    Ok(outcome)
}

/// Makes the link to the item at `uri` in `folder`; the link's path, or
/// why there is none.
fn link_item(uri: &str, folder: &Path) -> Result<PathBuf, String> {
    let target = gio::File::for_uri(uri)
        .path()
        .ok_or_else(|| crate::i18n::gettext(NOT_LOCAL_ITEM))?;
    let name = target
        .file_name()
        .ok_or_else(|| crate::i18n::gettext(NOT_LOCAL_ITEM))?;
    let link = folder.join(name);
    // `symlink` itself refuses a taken name, so a name that appears after
    // this check is never replaced either.
    if link.symlink_metadata().is_ok() {
        return Err(crate::i18n::gettext(NAME_TAKEN));
    }
    symlink(&target, &link).map_err(|error| describe(&error))?;
    Ok(link)
}

/// The message for a failed link.
fn describe(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AlreadyExists {
        return crate::i18n::gettext(NAME_TAKEN);
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

    /// parity: OPS-004
    #[test]
    fn new_link_points_to_the_typed_path_and_is_named_after_it_by_default() {
        let folder = temporary_folder();
        let item = folder.path().join("Sources/Reports");
        std::fs::create_dir_all(&item).expect("fixture folder");
        let links = folder.path().join("Links");
        std::fs::create_dir(&links).expect("fixture folder");
        let request = |name: &str, target: &str| NewLink {
            folder: file_uri(&links),
            name: name.to_owned(),
            target: target.to_owned(),
        };

        let by_uri = create_link_blocking(&request("", &file_uri(&item)), &context());
        let relative = create_link_blocking(&request("Latest", "../Sources/Reports"), &context());
        let taken = create_link_blocking(&request("Latest", "../Sources"), &context());
        let missing = create_link_blocking(&request("", "/no/such/place"), &context());

        assert_eq!(by_uri.map(|link| link.uri), Ok(file_uri(&links.join("Reports"))));
        assert_eq!(std::fs::read_link(links.join("Reports")).expect("a link"), item);
        assert!(relative.is_ok(), "{relative:?}");
        let relative_target = std::fs::read_link(links.join("Latest")).expect("a link");
        assert_eq!(relative_target, PathBuf::from("../Sources/Reports"));
        assert!(matches!(taken, Err(OpsError::Exists(_))), "{taken:?}");
        assert!(matches!(missing, Err(OpsError::NotFound(_))), "{missing:?}");
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
