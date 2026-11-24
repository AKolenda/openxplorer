// SPDX-License-Identifier: AGPL-3.0-only
//! Rename (F2).
//!
//! Ports `rename_item` in `v2.0.0:desktop/gio_backend.py` and the check the
//! `rename` branch of `dispatch` in `v2.0.0:desktop/winspace.py` runs first. The
//! rules (OPS-008): the new name is one valid path component; a whole
//! share, a device or a filesystem root is never renamed; the same name is
//! a no-op; the whole tree is checked against the write protection first
//! (XFER-020); and the item is renamed by a native move within its folder
//! that never overwrites (`set_display_name` on a phone, XFER-024).

use std::ffi::{OsStr, OsString};

use gio::prelude::*;

use super::context::{on_worker, OperationContext};
use super::create::name_taken_or;
use super::error::OpsError;
use super::undo::UndoRecord;
use crate::gio_node::GioNode;
use crate::location::{require_item_uri, validate_name};
use crate::transfer::{staging_name, Cancellation, Node, SourceChange, TransferError};

/// A finished rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedItem {
    /// The item's URI before the rename.
    pub original_uri: String,
    /// The item's URI now, to select it.
    pub uri: String,
    /// The item's name now.
    pub name: String,
}

impl RenamedItem {
    /// True when the new name was the old one, so nothing changed.
    pub fn is_unchanged(&self) -> bool {
        self.original_uri == self.uri
    }

    /// How Undo renames the item back, or `None` when nothing changed.
    pub fn undo_record(&self) -> Option<UndoRecord> {
        if self.is_unchanged() {
            return None;
        }
        Some(UndoRecord::Rename {
            original_uri: self.original_uri.clone(),
            renamed_uri: self.uri.clone(),
        })
    }
}

/// Renames the item at `uri` to `new_name` within its folder.
///
/// # Errors
///
/// An invalid name, a share, device or filesystem root, a protected
/// location anywhere in the item's tree, a taken name
/// ([`OpsError::Exists`]; nothing is overwritten), cancellation, or the
/// backend's failure.
pub async fn rename_item(
    uri: &str,
    new_name: &str,
    context: &OperationContext,
) -> Result<RenamedItem, OpsError> {
    let uri = uri.to_owned();
    let new_name = new_name.to_owned();
    let context = context.clone();
    on_worker(move || rename_item_blocking(&uri, &new_name, &context)).await
}

/// [`rename_item`] on the calling thread.
pub(super) fn rename_item_blocking(
    uri: &str,
    new_name: &str,
    context: &OperationContext,
) -> Result<RenamedItem, OpsError> {
    validate_name(new_name)?;
    // OPS-035: a whole share or device is not an item that can be renamed.
    let uri = require_item_uri(uri)?;
    context.protection.check(&uri)?;
    let source = GioNode::new(&uri);
    let Some(folder) = source.parent() else {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Cannot rename a filesystem root.",
        )));
    };
    let destination = folder.child(new_name.as_ref());
    let renamed = RenamedItem {
        original_uri: source.uri(),
        uri: destination.uri(),
        name: new_name.to_owned(),
    };
    if renamed.is_unchanged() {
        return Ok(renamed);
    }
    move_within_folder(&source, destination.as_ref(), context)
        .map_err(|error| rename_refusal(error, &source.display_name(), new_name))?;
    Ok(renamed)
}

/// The message for a rename the backend refused: a taken `new_name`
/// says which name is taken, and a denied permission names the item,
/// `old_name`, in place of GIO's "Error moving file" with the item's
/// whole path, as Windows says it needs permission to rename it. Other
/// refusals keep their messages.
fn rename_refusal(error: OpsError, old_name: &str, new_name: &str) -> OpsError {
    match name_taken_or(error, new_name) {
        OpsError::PermissionDenied(_) => OpsError::PermissionDenied(crate::i18n::format_message(
            "You don't have permission to rename “{name}” here.",
            &[("name", old_name)],
        )),
        other => other,
    }
}

/// Undoes a rename: gives the item at `renamed_uri` its name from
/// `original_uri` back, under the same rules as a rename. The original
/// name is used byte for byte, so a name that is not valid UTF-8 comes
/// back exactly.
///
/// # Errors
///
/// A location that is no longer in the same folder, a protected location,
/// a taken original name (nothing is overwritten), cancellation, or the
/// backend's failure.
pub(crate) fn rename_back(
    original_uri: &str,
    renamed_uri: &str,
    context: &OperationContext,
) -> Result<(), OpsError> {
    let source = GioNode::new(renamed_uri);
    let original = GioNode::new(original_uri);
    if !are_in_same_folder(&source, &original) {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Undo can only rename an item back within its own folder.",
        )));
    }
    context.protection.check(renamed_uri)?;
    move_within_folder(&source, &original, context)
        .map_err(|error| rename_refusal(error, &source.display_name(), &original.display_name()))
}

/// True when `first` and `second` have the same parent folder.
fn are_in_same_folder(first: &GioNode, second: &GioNode) -> bool {
    let (Some(first_folder), Some(second_folder)) = (first.parent(), second.parent()) else {
        return false;
    };
    let first_folder = gio::File::for_uri(&first_folder.uri());
    first_folder.equal(&gio::File::for_uri(&second_folder.uri()))
}

/// The whole-tree write check, then the native same-folder move that never
/// overwrites.
fn move_within_folder(
    source: &GioNode,
    destination: &dyn Node,
    context: &OperationContext,
) -> Result<(), OpsError> {
    // XFER-020: every location in the renamed tree and its new location,
    // as `_check_write_tree` with `source_writable=True`, which
    // `rename_item` in `v2.0.0:desktop/gio_backend.py` runs.
    let protection = &context.protection;
    protection.check_tree(source, destination, &context.cancel, SourceChange::Changed)?;
    let (Some(folder), old_name, new_name) = (source.parent(), source.name(), destination.name()) else {
        source.move_native(destination, Some(&context.cancel))?;
        return Ok(());
    };
    let cancel = &context.cancel;
    rename_keeping_case(
        &old_name,
        &new_name,
        |from, to| {
            folder
                .child(from)
                .move_native(folder.child(to).as_ref(), Some(cancel))
        },
        || names_in(folder.as_ref(), cancel),
        || staging_name().map(OsString::from),
    )?;
    Ok(())
}

/// Renames `old` to `new` in one folder with `move_named`, as
/// [`Node::move_native`] does.
///
/// A folder on a FAT32 or exFAT drive ignores case, so it says `new` is
/// taken when it differs from `old` only in case: the only item by that
/// name is the one being renamed. Windows 11 then leaves the name as it
/// was, without a word. Here, a case-only change that no other item in
/// the folder's `names` matches goes through the temporary name
/// `temporary` gives (a private staging name, which OPS-038 reports and
/// never deletes) and then to `new`; when that second step fails, the
/// item is given `old` back. It never tries `new` first: a FUSE driver
/// (exfat-fuse) keeps the answer that `new` exists for a second, and the
/// second step would be refused too. When the names cannot be read, the
/// item is renamed directly.
fn rename_keeping_case(
    old: &OsStr,
    new: &OsStr,
    mut move_named: impl FnMut(&OsStr, &OsStr) -> Result<(), TransferError>,
    names: impl FnOnce() -> Result<Vec<OsString>, TransferError>,
    temporary: impl FnOnce() -> Result<OsString, TransferError>,
) -> Result<(), TransferError> {
    let is_own_case_change =
        differs_only_in_case(old, new) && names().is_ok_and(|names| is_taken_only_by(old, new, &names));
    if !is_own_case_change {
        return move_named(old, new);
    }
    let temporary = temporary()?;
    move_named(old, &temporary)?;
    move_named(&temporary, new).inspect_err(|_| {
        let _ = move_named(&temporary, old);
    })
}

/// The name `name` as a folder that ignores case compares it, or `None`
/// for a name that is not text.
fn folded(name: &OsStr) -> Option<String> {
    name.to_str().map(str::to_lowercase)
}

/// Whether `new` is `old` in other case.
fn differs_only_in_case(old: &OsStr, new: &OsStr) -> bool {
    old != new && folded(old).is_some_and(|old| folded(new) == Some(old))
}

/// Whether, among the folder's `names`, the only item that matches `new`
/// in any case is `old` itself.
fn is_taken_only_by(old: &OsStr, new: &OsStr, names: &[OsString]) -> bool {
    let matching: Vec<&OsString> = names
        .iter()
        .filter(|name| folded(name).is_some_and(|name| Some(name) == folded(new)))
        .collect();
    matching == [old]
}

/// The names of the items directly in `folder`.
fn names_in(folder: &dyn Node, cancel: &Cancellation) -> Result<Vec<OsString>, TransferError> {
    Ok(folder
        .children(Some(cancel))?
        .iter()
        .map(|child| child.name())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder that ignores case, as on a FAT32 or exFAT drive: a name is
    /// taken when any item's name matches it in any case.
    struct CaseBlindFolder {
        names: Vec<OsString>,
        /// The rename that fails, by its destination name.
        failing: Option<OsString>,
        /// Names it said were taken, which it keeps saying for a while, as
        /// exfat-fuse does.
        remembered: Vec<OsString>,
    }

    impl CaseBlindFolder {
        fn new(names: &[&str]) -> Self {
            Self {
                names: names.iter().map(OsString::from).collect(),
                failing: None,
                remembered: Vec::new(),
            }
        }

        fn rename(&mut self, from: &OsStr, to: &OsStr) -> Result<(), TransferError> {
            let folded = |name: &OsStr| name.to_string_lossy().to_lowercase();
            if self.remembered.iter().any(|name| name == to)
                || self.names.iter().any(|name| folded(name) == folded(to))
            {
                self.remembered.push(to.to_owned());
                return Err(TransferError::Exists(String::new()));
            }
            if self.failing.as_deref() == Some(to) {
                return Err(TransferError::failed("Input/output error"));
            }
            let item = self.names.iter_mut().find(|name| name.as_os_str() == from);
            *item.ok_or_else(|| TransferError::NotFound(String::new()))? = to.to_owned();
            Ok(())
        }

        fn rename_keeping_case(&mut self, old: &str, new: &str) -> Result<(), TransferError> {
            let names = self.names.clone();
            let folder = std::cell::RefCell::new(self);
            rename_keeping_case(
                OsStr::new(old),
                OsStr::new(new),
                |from, to| folder.borrow_mut().rename(from, to),
                || Ok(names),
                || Ok(OsString::from(".winspace-transfer-0123.part")),
            )
        }
    }

    /// A case-only rename on a drive that ignores case (photo.txt to
    /// Photo.txt on FAT32 or exFAT) goes through a temporary name, where
    /// Windows 11 keeps the old name without a word. The new name is never
    /// tried first, which exfat-fuse would remember as taken. Another item
    /// by the new name, in any case, still refuses it, and an item that
    /// cannot take its new name gets its old one back.
    ///
    /// parity: OPS-008
    #[test]
    fn a_case_only_rename_works_on_a_drive_that_ignores_case() {
        let mut folder = CaseBlindFolder::new(&["photo.txt", "notes.txt"]);
        folder
            .rename_keeping_case("photo.txt", "Photo.txt")
            .expect("renamed");
        assert_eq!(folder.names, [OsString::from("Photo.txt"), "notes.txt".into()]);

        let mut taken = CaseBlindFolder::new(&["photo.txt", "PHOTO.TXT"]);
        assert_eq!(
            taken.rename_keeping_case("photo.txt", "Photo.txt"),
            Err(TransferError::Exists(String::new()))
        );
        assert_eq!(taken.names, [OsString::from("photo.txt"), "PHOTO.TXT".into()]);

        let mut other = CaseBlindFolder::new(&["photo.txt", "notes.txt"]);
        assert_eq!(
            other.rename_keeping_case("photo.txt", "Notes.txt"),
            Err(TransferError::Exists(String::new())),
            "not a case-only change"
        );

        let mut failing = CaseBlindFolder::new(&["photo.txt"]);
        failing.failing = Some("Photo.txt".into());
        assert_eq!(
            failing.rename_keeping_case("photo.txt", "Photo.txt"),
            Err(TransferError::failed("Input/output error"))
        );
        assert_eq!(failing.names, [OsString::from("photo.txt")]);
    }
}
