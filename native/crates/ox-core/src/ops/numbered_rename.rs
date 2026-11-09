// SPDX-License-Identifier: AGPL-3.0-only
//! Renaming several items at once with F2 (OPS-050), as Windows Explorer
//! does. The "Rename items" dialog's `#` template is
//! [`super::batch_rename`] (OPS-014).
//!
//! With several items selected, Windows Explorer's F2 edits one name in
//! place; the name typed there goes to every selected item with a number
//! in brackets: "Holiday (1).jpg", "Holiday (2).jpg" and so on, from (1),
//! in the order the items are shown. Each file keeps its own extension, a
//! folder or a file without one gets none, and a number whose name is
//! taken is skipped, so nothing is overwritten.
//!
//! Each item is renamed with the rules of a single rename (OPS-008): the
//! write protection and a same-folder move that never overwrites, through
//! the item's own location (a share, a phone). An item that fails for
//! another reason is reported and left as it is; the others go on. Undo
//! renames the whole batch back as one step (OPS-029).

use super::batch_rename::BatchItem;
use super::context::{on_worker, OperationContext};
use super::create::name_taken_or;
use super::error::OpsError;
use super::rename::rename_item_blocking;
use super::results::{AskedItem, ItemQuestions};
use super::run_transfer::TransferOutcome;
use super::undo::{RenamedPair, UndoRecord};
use crate::gio_node::GioNode;
use crate::location::validate_name;
use crate::transfer::{Node, TransferMode};

/// How many taken numbers are skipped for one item before it is reported
/// instead: a folder full of "Holiday (n)" items must not keep the rename
/// looking for a free number for ever.
const MAX_SKIPPED_NUMBERS: u32 = 10_000;

/// A Windows-style rename of several items: one typed name for all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberedRename {
    /// The items, in the order they are numbered.
    pub items: Vec<BatchItem>,
    /// The name typed for the item that was edited.
    pub name: String,
    /// The edited item's name before the rename, whose extension a typed
    /// name may keep.
    pub edited_name: String,
    /// Whether the edited item is a folder, whose name has no extension:
    /// what is typed for it is numbered whole.
    pub edited_is_dir: bool,
}

impl NumberedRename {
    /// The name the numbers follow: the typed name without the edited
    /// item's extension when it still ends with it, so "Holiday.jpg"
    /// typed for "Photo.jpg" numbers "Holiday", and each item adds its
    /// own extension back. A folder has no extension, so what is typed for
    /// one is numbered whole ("Archive.v1" for "Project.v1").
    ///
    /// # Errors
    ///
    /// An empty name, or one that is not valid (such as one with a slash),
    /// also once an item's number and extension are added: a name too long
    /// for one item with the numbers the items take when none is taken is
    /// refused before any item is renamed. A taken number that makes a
    /// name longer still fails that item alone, which is reported.
    pub fn base_name(&self) -> Result<String, OpsError> {
        let typed = self.name.trim_end();
        let kept = if self.edited_is_dir {
            None
        } else {
            extension(&self.edited_name)
        };
        let base = match (kept, extension(typed)) {
            (Some(kept), Some(ending)) if same_ignoring_case(kept, ending) => {
                &typed[..typed.len() - ending.len()]
            }
            _ => typed,
        };
        if base.trim().is_empty() {
            return Err(OpsError::failed(crate::i18n::gettext(
                "Enter a name for the items.",
            )));
        }
        validate_name(base)?;
        // The largest number the items take when none is skipped.
        let last = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        for item in &self.items {
            validate_name(&numbered_name(base, last, item))?;
        }
        Ok(base.to_owned())
    }
}

impl NumberedRename {
    /// Each item with its new name for `base`, numbered from (1) as when
    /// no number is taken: what the extension question looks at.
    pub fn numbered_names<'a>(&'a self, base: &'a str) -> impl Iterator<Item = (&'a BatchItem, String)> + 'a {
        self.items
            .iter()
            .zip(1..)
            .map(move |(item, number)| (item, numbered_name(base, number, item)))
    }
}

/// `item`'s new name with `number`: "Holiday (2).jpg".
fn numbered_name(base: &str, number: u32, item: &BatchItem) -> String {
    let extension = if item.is_dir {
        ""
    } else {
        extension(&item.name).unwrap_or_default()
    };
    format!("{base} ({number}){extension}")
}

/// The extension of the file name `name`, with its dot, as Windows reads
/// it: from the last dot, unless that dot starts the name (".bashrc" has
/// none) or ends it.
fn extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    (dot > 0 && dot + 1 < name.len()).then(|| &name[dot..])
}

/// True when `first` and `second` differ at most in case, by Unicode case
/// folding (OPS-049): "ß" and "SS", the Kelvin sign and "k".
fn same_ignoring_case(first: &str, second: &str) -> bool {
    glib::casefold(first) == glib::casefold(second)
}

/// Renames every item of `batch` on a worker thread. The outcome lists
/// the renamed items where they are now, to select them, and how Undo
/// renames them back.
///
/// # Errors
///
/// As [`NumberedRename::base_name`], before anything is renamed. Items that
/// cannot be renamed, and the user's cancellation, are reported in
/// [`TransferOutcome::result`].
pub async fn rename_numbered_items(
    batch: &NumberedRename,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let batch = batch.clone();
    let context = context.clone();
    on_worker(move || rename_numbered_blocking(&batch, &context)).await
}

/// [`rename_numbered_items`] on the calling thread.
fn rename_numbered_blocking(
    batch: &NumberedRename,
    context: &OperationContext,
) -> Result<TransferOutcome, OpsError> {
    let base = batch.base_name()?;
    let mut outcome = TransferOutcome::default();
    let mut renamed = Vec::new();
    let mut number = 1;
    let mut questions = ItemQuestions::new(context, TransferMode::Move);
    let count = batch.items.len();
    for (index, item) in batch.items.iter().enumerate() {
        if context.cancel.is_cancelled() {
            outcome.result.cancelled = true;
            break;
        }
        let asked = AskedItem {
            name: &item.name,
            uri: &item.uri,
            more_items: index + 1 < count,
        };
        // A failure is asked about in the progress window (OPS-047).
        let attempt = || rename_numbered(item, &base, &mut number, context);
        match questions.run(&mut outcome.result, asked, attempt) {
            Some(done) => {
                outcome.result.done.push(item.uri.clone());
                outcome.created.push(done.uri.clone());
                if !done.is_unchanged() {
                    renamed.push(RenamedPair {
                        original_uri: done.original_uri,
                        renamed_uri: done.uri,
                    });
                }
            }
            None if outcome.result.cancelled => break,
            None => {}
        }
    }
    if !renamed.is_empty() {
        outcome.undo = Some(UndoRecord::BatchRename { items: renamed });
    }
    Ok(outcome)
}

/// Renames `item` to `base` with the first free number from `number` on,
/// and moves `number` past the one it took. A taken number is skipped; an
/// item that fails for another reason keeps its number for the next item.
fn rename_numbered(
    item: &BatchItem,
    base: &str,
    number: &mut u32,
    context: &OperationContext,
) -> Result<super::RenamedItem, OpsError> {
    let folder = GioNode::new(&item.uri).parent();
    let is_taken = |name: &str| {
        !is_own_name(name, item)
            && folder
                .as_ref()
                .is_some_and(|folder| folder.child(name.as_ref()).exists(Some(&context.cancel)))
    };
    rename_numbered_with(item, base, number, context, is_taken)
}

/// True when `name` is `item`'s own name, or differs from it only in case,
/// which a share that ignores case reports as the item itself: it is not
/// taken. A different item with that name, where case counts, is still
/// found by the rename, which never overwrites.
fn is_own_name(name: &str, item: &BatchItem) -> bool {
    same_ignoring_case(name, &item.name)
}

/// [`rename_numbered`], asking `is_taken` about each numbered name first:
/// a taken one is skipped without trying the rename, whose write check
/// walks the item's whole tree. A name taken after the question, by
/// another program, is skipped when the rename finds it taken.
fn rename_numbered_with(
    item: &BatchItem,
    base: &str,
    number: &mut u32,
    context: &OperationContext,
    is_taken: impl Fn(&str) -> bool,
) -> Result<super::RenamedItem, OpsError> {
    let mut skipped = 0;
    loop {
        let name = numbered_name(base, *number, item);
        if is_taken(&name) {
            skipped += 1;
            if skipped > MAX_SKIPPED_NUMBERS {
                return Err(name_taken_or(OpsError::Exists(String::new()), &name));
            }
            *number += 1;
            continue;
        }
        match rename_item_blocking(&item.uri, &name, context) {
            Ok(done) => {
                *number += 1;
                return Ok(done);
            }
            Err(OpsError::Exists(message)) => {
                skipped += 1;
                if skipped > MAX_SKIPPED_NUMBERS {
                    return Err(OpsError::Exists(message));
                }
                *number += 1;
            }
            Err(error) => return Err(error),
        }
    }
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

    fn batch(items: Vec<BatchItem>, name: &str, edited_name: &str) -> NumberedRename {
        NumberedRename {
            items,
            name: name.to_owned(),
            edited_name: edited_name.to_owned(),
            edited_is_dir: false,
        }
    }

    /// parity: OPS-050
    #[test]
    fn each_item_keeps_its_own_extension_and_folders_get_none() {
        let numbered = [
            item("IMG_2031.JPG", false),
            item("backup.tar.gz", false),
            item("Trip", true),
            item(".bashrc", false),
            item("README", false),
            item("draft.", false),
        ]
        .iter()
        .zip(1..)
        .map(|(item, number)| numbered_name("Holiday", number, item))
        .collect::<Vec<_>>();

        assert_eq!(
            numbered,
            [
                "Holiday (1).JPG",
                "Holiday (2).gz",
                "Holiday (3)",
                "Holiday (4)",
                "Holiday (5)",
                "Holiday (6)",
            ]
        );
    }

    /// The edited item's extension at the end of the typed name is not
    /// part of the name the numbers follow; any other ending is.
    ///
    /// parity: OPS-050
    #[test]
    fn a_typed_extension_of_the_edited_item_is_not_numbered() {
        let base = |typed: &str, edited: &str| batch(Vec::new(), typed, edited).base_name();

        assert_eq!(base("Holiday.jpg", "IMG_2031.jpg").expect("valid"), "Holiday");
        assert_eq!(base("Holiday.JPG", "IMG_2031.jpg").expect("valid"), "Holiday");
        assert_eq!(base("Holiday", "IMG_2031.jpg").expect("valid"), "Holiday");
        assert_eq!(base("Holiday.png", "IMG_2031.jpg").expect("valid"), "Holiday.png");
        assert_eq!(base("Plan v1.2", "Trip").expect("valid"), "Plan v1.2");
        assert!(base(".jpg", "IMG_2031.jpg").is_ok_and(|name| name == ".jpg"));
        assert!(base("   ", "IMG_2031.jpg").is_err());
        assert!(base("a/b", "IMG_2031.jpg").is_err());
    }

    /// The ending is compared with Unicode case folding, as the extension
    /// question does (OPS-049), and cut where the typed name has it.
    ///
    /// parity: OPS-050
    #[test]
    fn a_typed_extension_differing_only_by_case_folding_is_not_numbered() {
        let base = |typed: &str, edited: &str| batch(Vec::new(), typed, edited).base_name();

        assert_eq!(base("Holiday.SS", "report.\u{df}").expect("valid"), "Holiday");
        assert_eq!(
            base("Holiday.\u{1e9e}", "report.\u{df}").expect("valid"),
            "Holiday"
        );
        assert_eq!(base("Holiday.k", "photo.\u{212a}").expect("valid"), "Holiday");
    }

    /// A name that fits on its own but not with an item's number and
    /// extension is refused before anything is renamed.
    ///
    /// parity: OPS-050
    #[test]
    fn a_name_too_long_for_one_item_is_refused_for_all() {
        let long = "a".repeat(248);
        let items = vec![item("Trip", true), item("notes.txt", false)];

        assert!(batch(items, &long, "Trip").base_name().is_err());
        assert!(batch(vec![item("Trip", true)], &long, "Trip").base_name().is_ok());
    }

    /// A numbered name that differs from the item's own only in case is
    /// the item's own: a share that ignores case says it exists, but it is
    /// not taken, so the item is not pushed to the next number.
    ///
    /// parity: OPS-050
    #[test]
    fn a_case_only_change_of_the_items_own_name_is_not_taken() {
        let photo = item("Photo (1).JPG", false);

        assert!(is_own_name("Photo (1).JPG", &photo));
        assert!(is_own_name("photo (1).JPG", &photo));
        assert!(!is_own_name("photo (2).JPG", &photo));
    }

    /// parity: OPS-050
    #[test]
    fn a_taken_number_is_skipped_and_nothing_is_overwritten() {
        let folder = temporary_folder();
        for name in ["one.txt", "two.txt", "three.txt", "Photo (2).txt"] {
            std::fs::write(folder.path().join(name), name).expect("fixture file");
        }
        let items = ["one.txt", "two.txt", "three.txt"]
            .map(|name| BatchItem {
                uri: file_uri(&folder.path().join(name)),
                name: name.to_owned(),
                is_dir: false,
            })
            .to_vec();
        let context = OperationContext::new(WriteProtection::unrestricted());

        let outcome =
            rename_numbered_blocking(&batch(items, "Photo", "one.txt"), &context).expect("a valid name");

        let read = |name: &str| std::fs::read_to_string(folder.path().join(name)).expect(name);
        assert_eq!(read("Photo (1).txt"), "one.txt");
        assert_eq!(read("Photo (2).txt"), "Photo (2).txt", "kept as it was");
        assert_eq!(read("Photo (3).txt"), "two.txt");
        assert_eq!(read("Photo (4).txt"), "three.txt");
        assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
        assert_eq!(outcome.result.done.len(), 3);
        assert!(matches!(outcome.undo, Some(UndoRecord::BatchRename { items }) if items.len() == 3));
    }

    /// Taken numbers are found by asking the item's location whether the
    /// name exists, not by trying the rename: the write protection walks
    /// the renamed item's whole tree once, not once per taken number.
    ///
    /// parity: OPS-050
    #[test]
    fn taken_numbers_do_not_walk_the_item_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let folder = temporary_folder();
        let tree = folder.path().join("Trip");
        std::fs::create_dir(&tree).expect("fixture folder");
        for number in 0..20 {
            std::fs::write(tree.join(format!("photo {number}.jpg")), "data").expect("fixture file");
        }
        for number in 1..=30 {
            std::fs::create_dir(folder.path().join(format!("Plan ({number})"))).expect("fixture folder");
        }
        let checks = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&checks);
        let protection = WriteProtection::new(move |_uri| {
            counted.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });
        let items = vec![BatchItem {
            uri: file_uri(&tree),
            name: "Trip".to_owned(),
            is_dir: true,
        }];
        let context = OperationContext::new(protection);

        let outcome =
            rename_numbered_blocking(&batch(items, "Plan", "Trip"), &context).expect("a valid name");

        assert!(outcome.result.errors.is_empty(), "{:?}", outcome.result.errors);
        assert_eq!(
            std::fs::read_dir(folder.path().join("Plan (31)"))
                .expect("renamed")
                .count(),
            20
        );
        let walks = checks.load(Ordering::Relaxed);
        assert!(walks < 3 * 21, "the tree was checked {walks} times");
    }

    /// A folder has no extension: a dotted name typed for one is numbered
    /// whole, even when it ends like the folder's own name.
    ///
    /// parity: OPS-050
    #[test]
    fn a_dotted_name_typed_for_a_folder_is_not_split() {
        let folder = NumberedRename {
            edited_is_dir: true,
            ..batch(Vec::new(), "Archive.v1", "Project.v1")
        };
        let file = batch(Vec::new(), "Archive.v1", "Project.v1");

        assert_eq!(folder.base_name().expect("valid"), "Archive.v1");
        assert_eq!(file.base_name().expect("valid"), "Archive");
    }

    /// A name taken after the question, as another program may take it,
    /// is found by the rename itself, which never overwrites, and the
    /// next number is used.
    ///
    /// parity: OPS-050
    #[test]
    fn a_name_taken_after_the_question_is_skipped_too() {
        let folder = temporary_folder();
        for name in ["one.txt", "Plan (1).txt"] {
            std::fs::write(folder.path().join(name), name).expect("fixture file");
        }
        let item = BatchItem {
            uri: file_uri(&folder.path().join("one.txt")),
            name: "one.txt".to_owned(),
            is_dir: false,
        };
        let context = OperationContext::new(WriteProtection::unrestricted());
        let mut number = 1;

        let done = rename_numbered_with(&item, "Plan", &mut number, &context, |_| false).expect("renamed");

        assert_eq!(done.name, "Plan (2).txt");
        assert_eq!(number, 3);
        assert_eq!(
            std::fs::read_to_string(folder.path().join("Plan (1).txt")).expect("kept"),
            "Plan (1).txt"
        );
    }

    /// An item that already has its numbered name stays as it is and
    /// takes its number.
    ///
    /// parity: OPS-050
    #[test]
    fn an_item_that_has_its_numbered_name_already_keeps_it() {
        let folder = temporary_folder();
        for name in ["Photo (1).txt", "two.txt"] {
            std::fs::write(folder.path().join(name), name).expect("fixture file");
        }
        let items = ["Photo (1).txt", "two.txt"]
            .map(|name| BatchItem {
                uri: file_uri(&folder.path().join(name)),
                name: name.to_owned(),
                is_dir: false,
            })
            .to_vec();
        let context = OperationContext::new(WriteProtection::unrestricted());

        let outcome =
            rename_numbered_blocking(&batch(items, "Photo", "two.txt"), &context).expect("a valid name");

        assert!(folder.path().join("Photo (1).txt").is_file());
        assert_eq!(
            std::fs::read_to_string(folder.path().join("Photo (2).txt")).expect("renamed"),
            "two.txt"
        );
        assert!(matches!(outcome.undo, Some(UndoRecord::BatchRename { items }) if items.len() == 1));
    }

    /// An item that cannot be renamed, here in a folder the user may not
    /// change, is reported by name and left as it is; the items before and
    /// after it are renamed, and the failed one's number goes to the next.
    ///
    /// parity: OPS-050
    #[test]
    fn an_item_that_cannot_be_renamed_is_reported_and_the_rest_go_on() {
        use std::os::unix::fs::PermissionsExt;

        let folder = temporary_folder();
        let locked = folder.path().join("Locked");
        std::fs::create_dir(&locked).expect("fixture folder");
        for path in [
            folder.path().join("a.txt"),
            locked.join("b.txt"),
            folder.path().join("c.txt"),
        ] {
            std::fs::write(&path, "data").expect("fixture file");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("read-only");
        let probe = locked.join("probe");
        if std::fs::write(&probe, "").is_ok() {
            // Running as root, which a read-only folder does not stop.
            std::fs::remove_file(probe).ok();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).ok();
            return;
        }
        let items = vec![
            BatchItem {
                uri: file_uri(&folder.path().join("a.txt")),
                name: "a.txt".to_owned(),
                is_dir: false,
            },
            BatchItem {
                uri: file_uri(&locked.join("b.txt")),
                name: "b.txt".to_owned(),
                is_dir: false,
            },
            BatchItem {
                uri: file_uri(&folder.path().join("c.txt")),
                name: "c.txt".to_owned(),
                is_dir: false,
            },
        ];
        let context = OperationContext::new(WriteProtection::unrestricted());

        let outcome = rename_numbered_blocking(&batch(items, "Plan", "a.txt"), &context);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("writable again");
        let outcome = outcome.expect("a valid name");

        assert!(folder.path().join("Plan (1).txt").is_file());
        assert!(
            folder.path().join("Plan (2).txt").is_file(),
            "c.txt took the number"
        );
        assert!(locked.join("b.txt").is_file(), "left as it was");
        assert_eq!(outcome.result.done.len(), 2);
        assert_eq!(outcome.result.errors.len(), 1, "{:?}", outcome.result.errors);
        assert!(
            outcome.result.errors[0].starts_with("b.txt: "),
            "{:?}",
            outcome.result.errors
        );
        assert!(matches!(outcome.undo, Some(UndoRecord::BatchRename { items }) if items.len() == 2));
    }
}
