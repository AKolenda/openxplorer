// SPDX-License-Identifier: AGPL-3.0-only
//! Batch rename: F2 with several items selected (OPS-014).
//!
//! New in the native app, from Dolphin's Rename Items dialog
//! (`KIO::RenameFileDialog` and `KIO::BatchRenameJob`). Every item gets
//! the typed name with its first run of `#` replaced by an ascending
//! number, as wide as the run (`###` gives `001`), and keeps its own
//! extension. Without `#` the name is allowed only when the items' types
//! differ, so each keeps a name of its own ("Holiday.jpg", "Holiday.png").
//!
//! Each item is renamed with the rules of a single rename (OPS-008): the
//! write protection, a same-folder move that never overwrites. An item
//! whose new name is taken is reported and left as it is; the others go
//! on. Undo renames the whole batch back as one step (OPS-029).

use super::context::{on_worker, OperationContext};
use super::error::OpsError;
use super::rename::rename_item_blocking;
use super::results::record_failure;
use super::run_transfer::TransferOutcome;
use super::undo::{RenamedPair, UndoRecord};
use crate::location::validate_name;

/// The character the number replaces.
pub const NUMBER_PLACEHOLDER: char = '#';

/// The name the batch rename dialog starts with (Dolphin's "New name #").
pub const DEFAULT_BATCH_NAME: &str = crate::i18n::message_id("New name #");

/// Extensions of two parts, kept whole: "backup.tar.gz" keeps ".tar.gz".
const COMPOUND_EXTENSIONS: [&str; 5] = [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tar.lz"];

/// Why the typed name would give two items the same name.
const NEEDS_NUMBER: &str = crate::i18n::message_id("Add # to the name, so each item gets its own number.");

/// One item of a batch rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    /// Where it is.
    pub uri: String,
    /// Its name now.
    pub name: String,
    /// Whether it is a folder, whose name has no extension.
    pub is_dir: bool,
}

/// A batch rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRename {
    /// The items, in the order they are numbered.
    pub items: Vec<BatchItem>,
    /// The name, with `#` where each item's number goes.
    pub pattern: String,
    /// The first item's number.
    pub first_number: u32,
}

impl BatchRename {
    /// Every item's new name, in order.
    ///
    /// # Errors
    ///
    /// An empty name, a name without `#` when two items would get the same
    /// name, or a new name that is not valid (such as one with a slash).
    pub fn new_names(&self) -> Result<Vec<String>, OpsError> {
        let pattern = self.pattern.trim();
        if pattern.is_empty() {
            return Err(OpsError::failed(crate::i18n::gettext(
                "Enter a name for the items.",
            )));
        }
        let names: Vec<String> = self
            .items
            .iter()
            .zip(self.first_number..)
            .map(|(item, number)| {
                let stem = numbered(pattern, number);
                format!("{stem}{}", extension_of(item))
            })
            .collect();
        for name in &names {
            validate_name(name)?;
        }
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != names.len() {
            return Err(OpsError::failed(crate::i18n::gettext(NEEDS_NUMBER)));
        }
        Ok(names)
    }
}

/// `pattern` with its first run of `#` replaced by `number`, padded with
/// zeros to the run's length, as Dolphin does.
fn numbered(pattern: &str, number: u32) -> String {
    let Some(start) = pattern.find(NUMBER_PLACEHOLDER) else {
        return pattern.to_owned();
    };
    let run = pattern[start..]
        .chars()
        .take_while(|character| *character == NUMBER_PLACEHOLDER)
        .count();
    let end = start + run * NUMBER_PLACEHOLDER.len_utf8();
    format!("{}{number:0run$}{}", &pattern[..start], &pattern[end..])
}

/// The extension `item` keeps, with its dot: none for a folder, a name
/// that starts with its only dot (".bashrc"), or a name without one.
fn extension_of(item: &BatchItem) -> &str {
    if item.is_dir {
        return "";
    }
    let name = item.name.as_str();
    let lower = name.to_lowercase();
    if let Some(compound) = COMPOUND_EXTENSIONS
        .iter()
        .find(|extension| lower.ends_with(*extension) && lower.len() > extension.len())
    {
        return &name[name.len() - compound.len()..];
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[dot..],
        _ => "",
    }
}

/// Renames every item of `batch` on a worker thread. The outcome lists
/// the renamed items where they are now, to select them, and how Undo
/// renames them back.
///
/// # Errors
///
/// As [`BatchRename::new_names`], before anything is renamed. Items that
/// cannot be renamed, and the user's cancellation, are reported in
/// [`TransferOutcome::result`].
pub async fn rename_batch(
    batch: &BatchRename,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let batch = batch.clone();
    let context = context.clone();
    on_worker(move || rename_batch_blocking(&batch, &context)).await
}

/// [`rename_batch`] on the calling thread.
fn rename_batch_blocking(
    batch: &BatchRename,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let names = batch.new_names()?;
    let mut outcome = TransferOutcome::default();
    let mut renamed = Vec::new();
    for (item, name) in batch.items.iter().zip(&names) {
        if context.cancel.is_cancelled() {
            outcome.result.cancelled = true;
            break;
        }
        match rename_item_blocking(&item.uri, name, context) {
            Ok(done) => {
                outcome.result.done.push(item.uri.clone());
                outcome.created.push(done.uri.clone());
                if !done.is_unchanged() {
                    renamed.push(RenamedPair {
                        original_uri: done.original_uri,
                        renamed_uri: done.uri,
                    });
                }
            }
            Err(error) => record_failure(&mut outcome.result, &item.name, &error),
        }
    }
    if !renamed.is_empty() {
        outcome.undo = Some(UndoRecord::BatchRename { items: renamed });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::file_uri;
    use crate::ops::WriteProtection;
    use crate::test_support::temporary_folder;

    fn item(name: &str, is_dir: bool) -> BatchItem {
        BatchItem {
            uri: format!("file:///tmp/{name}"),
            name: name.to_owned(),
            is_dir,
        }
    }

    fn batch(items: Vec<BatchItem>, pattern: &str, first_number: u32) -> BatchRename {
        BatchRename {
            items,
            pattern: pattern.to_owned(),
            first_number,
        }
    }

    /// parity: OPS-014
    #[test]
    fn items_are_numbered_from_the_first_number_and_keep_their_extensions() {
        let items = vec![
            item("IMG_2031.JPG", false),
            item("backup.tar.gz", false),
            item("Trip", true),
            item(".bashrc", false),
        ];

        let numbered = batch(items.clone(), "Holiday ###", 9).new_names();
        let without_number = batch(items[..2].to_vec(), "Holiday", 1).new_names();
        let same_type = batch(vec![item("a.txt", false), item("b.txt", false)], "Notes", 1).new_names();
        let slash = batch(items, "a/# b", 1).new_names();

        assert_eq!(
            numbered.expect("valid names"),
            [
                "Holiday 009.JPG",
                "Holiday 010.tar.gz",
                "Holiday 011",
                "Holiday 012"
            ]
        );
        assert_eq!(
            without_number.expect("the extensions differ"),
            ["Holiday.JPG", "Holiday.tar.gz"]
        );
        assert!(matches!(same_type, Err(OpsError::Failed(message)) if message == NEEDS_NUMBER));
        assert!(slash.is_err());
    }

    /// parity: OPS-014
    #[test]
    fn a_taken_name_is_reported_and_the_rest_are_renamed() {
        let folder = temporary_folder();
        for name in ["one.txt", "two.txt", "Photo 2.txt"] {
            std::fs::write(folder.path().join(name), name).expect("fixture file");
        }
        let items = ["one.txt", "two.txt"]
            .map(|name| BatchItem {
                uri: file_uri(&folder.path().join(name)),
                name: name.to_owned(),
                is_dir: false,
            })
            .to_vec();
        let context = OperationContext::new(WriteProtection::unrestricted());

        let outcome = rename_batch_blocking(&batch(items, "Photo #", 1), &context).expect("valid names");

        assert!(folder.path().join("Photo 1.txt").is_file());
        assert_eq!(
            std::fs::read(folder.path().join("Photo 2.txt")).expect("kept"),
            b"Photo 2.txt"
        );
        assert!(folder.path().join("two.txt").is_file(), "its new name was taken");
        assert_eq!(outcome.result.done.len(), 1);
        assert_eq!(outcome.result.errors.len(), 1, "{:?}", outcome.result.errors);
        assert!(matches!(outcome.undo, Some(UndoRecord::BatchRename { items }) if items.len() == 1));
    }
}
