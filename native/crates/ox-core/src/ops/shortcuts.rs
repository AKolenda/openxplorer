// SPDX-License-Identifier: AGPL-3.0-only
//! Shortcuts as Windows Explorer names them: Send to ▸ Desktop (create
//! shortcut) (CMD-038), and the right-drag menu's "Create shortcuts here".
//!
//! On Linux a shortcut is a symbolic link. It gets the name Windows gives
//! a new shortcut, the shell's naming template `%s - Shortcut` filled with
//! the item's whole name, its extension included ("report.txt -
//! Shortcut", "Reports - Shortcut"). When that name is taken the shell
//! numbers it as `PathYetAnotherMakeUniqueName` does: "report.txt -
//! Shortcut (2)", then (3) and so on. A link has no hidden `.lnk`
//! extension, so the name is exactly what Explorer shows. A name too long
//! for that within 255 bytes loses whole characters before its extension,
//! as copies' names do, so the extension and the number are kept.
//!
//! Safety rule "a shortcut never replaces anything": each name is claimed
//! by creating the link itself, which fails on any existing item, a broken
//! link included, so a name that appears meanwhile is never overwritten;
//! the next number is tried instead. The link points to the item's path
//! on this computer: its own path, or the mount path of a share (a kernel
//! CIFS mount or the `GVfs` FUSE export), so other apps can follow it.
//! Where the folder's file system has no symbolic links (an SMB share
//! through `GVfs`, a FAT-formatted USB stick), nothing is written and
//! [`NO_LINKS_HERE`] says why.

use std::path::PathBuf;

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::links::LinkRequest;
use super::results::{AskedItem, ItemQuestions};
use super::run_transfer::TransferOutcome;
use super::undo::UndoRecord;
use crate::location::{file_uri, fitted_name, normalise, ItemKind};
use crate::network::local_path;
use crate::transfer::{TransferMode, MAX_ITEMS};

/// Why a folder takes no shortcuts.
pub const NO_LINKS_HERE: &str = crate::i18n::message_id(
    "Shortcuts cannot be created here: this location does not support symbolic links.",
);

/// Why an item gets no shortcut.
const NOT_LINKABLE: &str =
    crate::i18n::message_id("A shortcut can only point to an item on this computer or on a mounted share.");

/// How many numbered names are tried before giving up.
const MAX_NUMBER: u32 = 10_000;

/// The name of the shortcut to an item named `name`, as Windows names it:
/// "`name` - Shortcut" for `number` 1, "`name` - Shortcut (`number`)"
/// from 2 on.
pub fn shortcut_name(name: &str, number: u32) -> String {
    if number <= 1 {
        crate::i18n::format_message("{name} - Shortcut", &[("name", name)])
    } else {
        crate::i18n::format_message(
            "{name} - Shortcut ({number})",
            &[("name", name), ("number", &number.to_string())],
        )
    }
}

/// Makes a shortcut in the folder at `folder_uri` to the item at
/// `target_uri`, under the first free name of [`shortcut_name`], and
/// returns the shortcut's URI. Blocking: call it on a worker thread. It
/// never replaces anything; the caller checks the folder's write
/// protection first, as [`create_shortcuts`] does.
///
/// # Errors
///
/// The item does not exist ([`OpsError::NotFound`]) or has no path on
/// this computer, the folder has no symbolic links
/// ([`OpsError::NotSupported`] with [`NO_LINKS_HERE`]), every numbered
/// name is taken ([`OpsError::Exists`]), or the system's failure.
pub fn create_shortcut(target_uri: &str, folder_uri: &str) -> Result<String, OpsError> {
    let target_uri = normalise(target_uri)?;
    let folder = gio::File::for_uri(&normalise(folder_uri)?);
    let target = gio::File::for_uri(&target_uri);
    let info = target.query_info(
        "standard::display-name,standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        gio::Cancellable::NONE,
    )?;
    let name = info.display_name().to_string();
    let kind = if info.file_type() == gio::FileType::Directory {
        ItemKind::Folder
    } else {
        ItemKind::File
    };
    let path = target_path(&target, &target_uri)?;
    for number in 1..=MAX_NUMBER {
        // A long name loses whole characters before its extension, so the
        // extension and " - Shortcut (2)" are kept within 255 bytes.
        let candidate = fitted_name(&name, kind, |name| shortcut_name(name, number))?;
        let link = folder.child(&candidate);
        match link.make_symbolic_link(&path, gio::Cancellable::NONE) {
            Ok(()) => return Ok(link_uri(&link)),
            Err(error) if error.matches(gio::IOErrorEnum::Exists) => {}
            Err(error) => return Err(describe(error, &folder)),
        }
    }
    Err(OpsError::Exists(crate::i18n::gettext(
        "Every name for this shortcut is taken. Nothing was replaced.",
    )))
}

/// The path a shortcut to `target` points to: its own path, or the mount
/// path of the share it is on.
fn target_path(target: &gio::File, uri: &str) -> Result<PathBuf, OpsError> {
    target
        .path()
        .or_else(|| local_path(uri))
        .ok_or_else(|| OpsError::failed(crate::i18n::gettext(NOT_LINKABLE)))
}

/// The URI of the new `link`, spelled as listings spell it.
fn link_uri(link: &gio::File) -> String {
    match link.path() {
        Some(path) => file_uri(&path),
        None => link.uri().to_string(),
    }
}

/// The error for a link `folder` refused. A file system without symbolic
/// links says "not supported", or "not permitted" where the folder can be
/// written (FAT refuses links with `EPERM`): both are [`NO_LINKS_HERE`].
fn describe(error: glib::Error, folder: &gio::File) -> OpsError {
    let unsupported = error.matches(gio::IOErrorEnum::NotSupported)
        || (error.matches(gio::IOErrorEnum::PermissionDenied) && can_write(folder));
    if unsupported {
        return OpsError::NotSupported(crate::i18n::gettext(NO_LINKS_HERE));
    }
    error.into()
}

/// Whether `folder` says it can be written.
fn can_write(folder: &gio::File) -> bool {
    folder
        .query_info(
            "access::can-write",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .is_ok_and(|info| info.boolean("access::can-write"))
}

/// Makes a shortcut in the request's folder to each of its items, on a
/// worker thread: Send to ▸ Desktop (create shortcut). The outcome lists
/// the new shortcuts, and Undo moves them, never their items, to the
/// Trash.
///
/// # Errors
///
/// No items or more than [`MAX_ITEMS`], or a protected folder. Items that
/// get no shortcut, and the user's cancellation, are reported in
/// [`TransferOutcome::result`].
pub async fn create_shortcuts(
    request: &LinkRequest,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let request = request.clone();
    let context = context.clone();
    on_worker(move || create_shortcuts_blocking(&request, &context)).await
}

/// [`create_shortcuts`] on the calling thread.
fn create_shortcuts_blocking(
    request: &LinkRequest,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    if request.uris.is_empty() || request.uris.len() > MAX_ITEMS {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Select between 1 and 100,000 items.",
        )));
    }
    let folder = normalise(&request.destination_folder)?;
    context.protection.check(&folder)?;
    let mut outcome = TransferOutcome::default();
    let mut questions = ItemQuestions::new(context, TransferMode::Copy);
    for (index, uri) in request.uris.iter().enumerate() {
        if context.cancel.is_cancelled() {
            outcome.result.cancelled = true;
            break;
        }
        let name = gio::File::for_uri(uri)
            .basename()
            .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
        let asked = AskedItem {
            name: &name,
            uri,
            more_items: index + 1 < request.uris.len(),
        };
        // An item that gets no shortcut is asked about in the progress
        // window (OPS-047).
        if let Some(link) = questions.run(&mut outcome.result, asked, || create_shortcut(uri, &folder)) {
            outcome.result.done.push(uri.clone());
            outcome.created.push(link);
        }
        if outcome.result.cancelled {
            break;
        }
    }
    if !outcome.created.is_empty() {
        outcome.undo = Some(UndoRecord::Link {
            links: outcome.created.clone(),
        });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::ops::WriteProtection;
    use crate::test_support::temporary_folder;
    use crate::transfer::TransferError;

    /// parity: CMD-038
    #[test]
    fn a_shortcut_is_named_as_windows_names_it_extension_and_number_included() {
        assert_eq!(shortcut_name("report.txt", 1), "report.txt - Shortcut");
        assert_eq!(shortcut_name("Reports", 1), "Reports - Shortcut");
        assert_eq!(shortcut_name("report.txt", 2), "report.txt - Shortcut (2)");
        assert_eq!(shortcut_name("Reports", 3), "Reports - Shortcut (3)");
    }

    /// parity: CMD-038
    #[test]
    fn a_shortcut_points_to_the_item_and_takes_the_next_free_number() {
        let folder = temporary_folder();
        let item = folder.path().join("report.txt");
        std::fs::write(&item, b"quarterly figures").expect("fixture file");
        let desktop = folder.path().join("Desktop");
        std::fs::create_dir(&desktop).expect("fixture folder");
        // A broken link holds the first name, a file the second.
        symlink("/no/such/item", desktop.join("report.txt - Shortcut")).expect("fixture link");
        std::fs::write(desktop.join("report.txt - Shortcut (2)"), b"keep me").expect("fixture file");

        let link = create_shortcut(&file_uri(&item), &file_uri(&desktop)).expect("a shortcut");

        let made = desktop.join("report.txt - Shortcut (3)");
        assert_eq!(link, file_uri(&made));
        assert_eq!(std::fs::read_link(&made).expect("a symbolic link"), item);
        let first = std::fs::read_link(desktop.join("report.txt - Shortcut")).expect("kept");
        assert_eq!(first, PathBuf::from("/no/such/item"));
        let second = std::fs::read(desktop.join("report.txt - Shortcut (2)")).expect("kept");
        assert_eq!(second, b"keep me");
    }

    /// A long name that is valid still gets its shortcut, and the next
    /// one its number: the part before the extension is shortened by whole
    /// characters, never the extension or " - Shortcut (2)".
    ///
    /// parity: CMD-038
    #[test]
    fn a_long_name_is_shortened_to_keep_the_suffix_and_number() {
        let folder = temporary_folder();
        let name = format!("{}.txt", "a".repeat(241));
        assert_eq!(name.len(), 245);
        let item = folder.path().join(&name);
        std::fs::write(&item, b"Synthetic data").expect("fixture file");
        let desktop = folder.path().join("Desktop");
        std::fs::create_dir(&desktop).expect("fixture folder");

        let first = create_shortcut(&file_uri(&item), &file_uri(&desktop));
        let second = create_shortcut(&file_uri(&item), &file_uri(&desktop));

        let first_name = format!("{}.txt - Shortcut", "a".repeat(240));
        let second_name = format!("{}.txt - Shortcut (2)", "a".repeat(236));
        assert_eq!(first, Ok(file_uri(&desktop.join(&first_name))));
        assert_eq!(second, Ok(file_uri(&desktop.join(&second_name))));
        assert_eq!(first_name.len(), 255);
        assert_eq!(
            std::fs::read_link(desktop.join(&second_name)).expect("a link"),
            item
        );
    }

    /// parity: CMD-038
    #[test]
    fn a_folder_gets_a_shortcut_and_a_missing_item_none() {
        let folder = temporary_folder();
        let reports = folder.path().join("Reports");
        std::fs::create_dir(&reports).expect("fixture folder");

        let link = create_shortcut(&file_uri(&reports), &file_uri(folder.path()));
        let missing = create_shortcut(
            &file_uri(&folder.path().join("gone.txt")),
            &file_uri(folder.path()),
        );

        assert_eq!(link, Ok(file_uri(&folder.path().join("Reports - Shortcut"))));
        assert!(matches!(missing, Err(OpsError::NotFound(_))), "{missing:?}");
    }

    /// A file system without symbolic links says so clearly.
    ///
    /// parity: CMD-038
    #[test]
    fn a_location_without_symbolic_links_says_so() {
        let folder = temporary_folder();
        let unsupported = glib::Error::new(gio::IOErrorEnum::NotSupported, "Operation not supported");
        let denied = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "Operation not permitted");
        let gone = glib::Error::new(gio::IOErrorEnum::NotFound, "No such file or directory");
        let writable = gio::File::for_path(folder.path());

        let expected = OpsError::NotSupported(NO_LINKS_HERE.to_owned());
        assert_eq!(describe(unsupported, &writable), expected);
        assert_eq!(describe(denied, &writable), expected);
        assert!(matches!(describe(gone, &writable), OpsError::NotFound(_)));
    }

    /// parity: CMD-038
    #[test]
    fn shortcuts_for_several_items_are_undone_as_links_and_respect_protection() {
        let folder = temporary_folder();
        let items = ["a.txt", "b.txt"].map(|name| {
            let path = folder.path().join(name);
            std::fs::write(&path, b"Synthetic data").expect("fixture file");
            file_uri(&path)
        });
        let desktop = folder.path().join("Desktop");
        std::fs::create_dir(&desktop).expect("fixture folder");
        let request = LinkRequest {
            uris: items.to_vec(),
            destination_folder: file_uri(&desktop),
        };
        let open = OperationContext::new(WriteProtection::unrestricted());
        let guarded = OperationContext::new(WriteProtection::new(|_| {
            Err(TransferError::failed("This folder is read-only."))
        }));

        let outcome = create_shortcuts_blocking(&request, &open).expect("the folder is writable");
        let refused = create_shortcuts_blocking(&request, &guarded);

        let links = [
            file_uri(&desktop.join("a.txt - Shortcut")),
            file_uri(&desktop.join("b.txt - Shortcut")),
        ];
        assert_eq!(outcome.created, links);
        assert_eq!(outcome.result.done, items);
        assert_eq!(
            outcome.undo,
            Some(UndoRecord::Link {
                links: links.to_vec()
            })
        );
        assert!(refused.is_err(), "{refused:?}");
    }
}
