// SPDX-License-Identifier: AGPL-3.0-only
//! New folder and New file (an empty file with any name).
//!
//! Ports `create_item` in `v2.0.0:desktop/gio_backend.py` and the checks the
//! `create` branch of `dispatch` in `v2.0.0:desktop/winspace.py` runs first.
//! Creating never overwrites (OPS-008): a folder is made with GIO's
//! exclusive `make_directory` and a file with an exclusive `create`, so a
//! taken name fails and the existing item stays as it was.

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::undo::UndoRecord;
use crate::location::{is_smb_server, normalise, validate_name, ItemKind};

/// An item an operation created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedItem {
    /// The new item's URI, to select it.
    pub uri: String,
    /// Whether it is a folder or a file.
    pub kind: ItemKind,
}

impl CreatedItem {
    /// How Undo removes the item again: it goes to the Trash.
    pub fn undo_record(&self) -> UndoRecord {
        UndoRecord::Create {
            uri: self.uri.clone(),
            kind: self.kind,
        }
    }
}

/// Creates an empty folder or file called `name` in the folder at
/// `folder_uri`.
///
/// # Errors
///
/// An invalid name, a server listing ("Open a network share before
/// creating files or folders."), a protected folder, a taken name
/// ([`OpsError::Exists`]; nothing is overwritten), cancellation, or the
/// backend's failure.
pub async fn create_item(
    folder_uri: &str,
    name: &str,
    kind: ItemKind,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let name = name.to_owned();
    let context = context.clone();
    on_worker(move || create_item_blocking(&folder_uri, &name, kind, &context)).await
}

/// What [`create_folder_path`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedFolderPath {
    /// The outermost folder it created, which Undo moves to the Trash.
    pub created: CreatedItem,
    /// The item of the first name in the folder it started from, new or
    /// not, to select there.
    pub first: String,
}

/// Creates the folders `names` name, each inside the one before, in the
/// folder at `folder_uri`, as Dolphin's New folder does with a name that
/// holds slashes (`a/b/c`, OPS-007). A folder of an earlier name that
/// exists already is entered; the last name must be free, so nothing is
/// overwritten (OPS-008).
///
/// # Errors
///
/// As [`create_item`]: no names, an invalid name, a taken last name, or an
/// earlier name taken by something that is not a folder.
pub async fn create_folder_path(
    folder_uri: &str,
    names: &[&str],
    context: &OperationContext,
) -> Result<CreatedFolderPath, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let context = context.clone();
    on_worker(move || create_folder_path_blocking(&folder_uri, &names, &context)).await
}

/// [`create_folder_path`] on the calling thread.
fn create_folder_path_blocking(
    folder_uri: &str,
    names: &[String],
    context: &OperationContext,
) -> Result<CreatedFolderPath, OpsError> {
    let (last, earlier) = names
        .split_last()
        .ok_or_else(|| OpsError::failed(crate::i18n::gettext("Type a name for the new folder.")))?;
    let mut parent = folder_uri.to_owned();
    let mut first = None;
    let mut created = None;
    for name in earlier {
        let entered = match create_item_blocking(&parent, name, ItemKind::Folder, context) {
            Ok(item) => {
                let uri = item.uri.clone();
                created.get_or_insert(item);
                uri
            }
            Err(OpsError::Exists(message)) => {
                existing_folder(&parent, name).ok_or(OpsError::Exists(message))?
            }
            Err(error) => return Err(error),
        };
        first.get_or_insert_with(|| entered.clone());
        parent = entered;
    }
    let item = create_item_blocking(&parent, last, ItemKind::Folder, context)?;
    let first = first.unwrap_or_else(|| item.uri.clone());
    Ok(CreatedFolderPath {
        created: created.unwrap_or(item),
        first,
    })
}

/// The URI of the folder `name` in the folder at `parent_uri`, when it is
/// a folder and not a link to one.
fn existing_folder(parent_uri: &str, name: &str) -> Option<String> {
    let item = gio::File::for_uri(parent_uri).child(name);
    let file_type = item.query_file_type(gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE);
    (file_type == gio::FileType::Directory).then(|| item.uri().to_string())
}

/// The most numbered names New folder tries before it gives up.
const MAX_NUMBERED_NAMES: u32 = 10_000;

/// Creates an empty folder in the folder at `folder_uri` under the first
/// free name of `base_name`, `base_name (2)`, `base_name (3)`, ..., as
/// Windows names new folders. It is the first half of Explorer's New
/// folder, which then renames the folder in place; the window asks for the
/// name first (OPS-001) until it can rename in place (OPS-010).
///
/// # Errors
///
/// As [`create_item`], except that a taken name tries the next number;
/// after [`MAX_NUMBERED_NAMES`] taken names the refusal says to rename
/// some of them.
pub async fn create_numbered_folder(
    folder_uri: &str,
    base_name: &str,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    let folder_uri = folder_uri.to_owned();
    let base_name = base_name.to_owned();
    let context = context.clone();
    on_worker(move || create_numbered_folder_blocking(&folder_uri, &base_name, &context)).await
}

/// [`create_numbered_folder`] on the calling thread. Each attempt is an
/// exclusive creation, so a folder that appears meanwhile is never taken
/// over; it only moves the new folder to the next number.
fn create_numbered_folder_blocking(
    folder_uri: &str,
    base_name: &str,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    for number in 1..=MAX_NUMBERED_NAMES {
        let name = numbered_name(base_name, number);
        match create_item_blocking(folder_uri, &name, ItemKind::Folder, context) {
            Err(OpsError::Exists(_)) => {}
            outcome => return outcome,
        }
    }
    Err(OpsError::failed(crate::i18n::format_message(
        "Too many folders are called “{base_name}”. Rename some of them, then try again.",
        &[("base_name", base_name)],
    )))
}

/// `base_name` for the first attempt, then `base_name (number)`.
fn numbered_name(base_name: &str, number: u32) -> String {
    if number == 1 {
        base_name.to_owned()
    } else {
        format!("{base_name} ({number})")
    }
}

/// [`create_item`] on the calling thread.
fn create_item_blocking(
    folder_uri: &str,
    name: &str,
    kind: ItemKind,
    context: &OperationContext,
) -> Result<CreatedItem, OpsError> {
    validate_name(name)?;
    // A server listing holds shares, not files; nothing can be created there.
    if is_smb_server(folder_uri) {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Open a network share before creating files or folders.",
        )));
    }
    let folder_uri = normalise(folder_uri)?;
    context.protection.check(&folder_uri)?;
    context.cancel.check()?;
    let item = gio::File::for_uri(&folder_uri).child(name);
    create_exclusively(&item, kind, context).map_err(|error| name_taken_or(error, name))?;
    Ok(CreatedItem {
        uri: item.uri().to_string(),
        kind,
    })
}

/// OPS-008: creates `item` only when its name is free. Both GIO calls are
/// exclusive, so an item that appears meanwhile is never replaced.
fn create_exclusively(item: &gio::File, kind: ItemKind, context: &OperationContext) -> Result<(), OpsError> {
    let cancellable = Some(context.cancellable());
    match kind {
        ItemKind::Folder => item.make_directory(cancellable)?,
        ItemKind::File => {
            let stream = item.create(gio::FileCreateFlags::NONE, cancellable)?;
            stream.close(cancellable)?;
        }
    }
    Ok(())
}

/// A taken name reported in the app's wording (OPS-008); other failures
/// keep their message.
pub(crate) fn name_taken_or(error: OpsError, name: &str) -> OpsError {
    match error {
        OpsError::Exists(_) => OpsError::Exists(crate::i18n::format_message(
            "An item named “{name}” already exists. Nothing was overwritten.",
            &[("name", name)],
        )),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_uri(path: &std::path::Path) -> String {
        gio::File::for_path(path).uri().to_string()
    }

    #[test]
    fn numbered_names_follow_windows() {
        assert_eq!(numbered_name("New folder", 1), "New folder");
        assert_eq!(numbered_name("New folder", 2), "New folder (2)");
    }

    /// A name with slashes makes folders inside folders, entering one that
    /// exists; Undo takes the outermost new one.
    ///
    /// parity: OPS-007
    #[test]
    fn a_folder_path_creates_each_folder_inside_the_one_before() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir(temp.path().join("Photos")).expect("an existing folder");
        let context = OperationContext::default();

        let created = glib::MainContext::new().block_on(create_folder_path(
            &folder_uri(temp.path()),
            &["Photos", "2026", "Summer"],
            &context,
        ));

        let created = created.expect("the folders are made");
        assert!(temp.path().join("Photos/2026/Summer").is_dir());
        assert_eq!(created.created.uri, folder_uri(&temp.path().join("Photos/2026")));
        assert_eq!(created.first, folder_uri(&temp.path().join("Photos")));
        let again = glib::MainContext::new().block_on(create_folder_path(
            &folder_uri(temp.path()),
            &["Photos", "2026", "Summer"],
            &context,
        ));
        assert!(matches!(again, Err(OpsError::Exists(_))), "{again:?}");
    }

    /// parity: OPS-001, OPS-008
    #[test]
    fn a_new_folder_takes_the_next_free_number_and_leaves_the_others_alone() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir(temp.path().join("New folder")).expect("a taken name");
        std::fs::write(temp.path().join("New folder (2)"), b"a file").expect("a taken name");
        let context = OperationContext::default();

        let created = glib::MainContext::new().block_on(create_numbered_folder(
            &folder_uri(temp.path()),
            "New folder",
            &context,
        ));

        let created = created.expect("a free name");
        assert_eq!(created.uri, folder_uri(&temp.path().join("New folder (3)")));
        assert!(temp.path().join("New folder (3)").is_dir());
        let taken = std::fs::read(temp.path().join("New folder (2)")).expect("the file stays");
        assert_eq!(taken, b"a file");
    }
}
