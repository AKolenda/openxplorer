// SPDX-License-Identifier: AGPL-3.0-only
//! The undo journal: what the last file operations did, so Undo can
//! reverse them one step at a time (OPS-029).
//!
//! New in the native app; the Python app has no Undo (`desktop/MANUAL.md`,
//! "Important limitations"). The journal follows Dolphin's and Nautilus's
//! undo managers, restricted to steps that can be reversed without ever
//! overwriting or permanently deleting anything:
//!
//! | Operation | Undone by |
//! |---|---|
//! | Rename | Renaming the item back |
//! | Batch rename | Renaming each item back, newest first |
//! | New folder, New file, New from template | Moving the new item to the Trash |
//! | Links | Moving the links to the Trash; what they point to stays |
//! | Copy (Skip or Keep both) and Duplicate | Moving the copies to the Trash, as Nautilus does |
//! | Move (Skip) | Moving each item back to its folder |
//! | Move to Trash | Restoring the items from the Recycle Bin |
//! | Restore from the Recycle Bin | Moving the items to the Trash again |
//!
//! Not undoable: permanent delete and emptying the Recycle Bin (nothing is
//! left to bring back), Replace (the replaced items are gone and merged
//! folders cannot be separated again), and a move with Keep both (the new
//! names are not known exactly). Every undo step runs with the same safety
//! rules as the operation itself; `undo_apply` carries them out.
//!
//! Reversing a record has an effect of its own, which [`UndoRecord::inverse`]
//! describes as another record: Redo reverses an Undo with it, and Undo a
//! Redo. The journal that keeps both directions is `journal`.

use std::path::PathBuf;

use crate::location::ItemKind;
use crate::transfer::TransferResult;

/// One operation Undo can reverse, with exactly what it needs to do so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoRecord {
    /// A rename, undone by renaming the item back. Never overwrites.
    Rename {
        /// The item's URI before the rename.
        original_uri: String,
        /// The item's URI after the rename.
        renamed_uri: String,
    },
    /// New folder, New file or New from template, undone by moving the
    /// new item to the Trash.
    Create {
        /// The new item.
        uri: String,
        /// Whether it is a folder or a file, which names the step.
        kind: ItemKind,
    },
    /// A batch rename (OPS-014), undone by renaming each item back, the
    /// last renamed first. Never overwrites.
    BatchRename {
        /// Each renamed item, in the order it was renamed.
        items: Vec<RenamedPair>,
    },
    /// Links made by a drop or New ▸ Link (DND-019, OPS-004), undone by
    /// moving the links, never what they point to, to the Trash.
    Link {
        /// The new links.
        links: Vec<String>,
    },
    /// A copy, undone by moving the copies to the Trash.
    Copy {
        /// The copies the operation created.
        copies: Vec<String>,
    },
    /// Duplicate, undone by moving the duplicates to the Trash.
    Duplicate {
        /// The duplicates the operation created.
        copies: Vec<String>,
    },
    /// A move, undone by moving each item back into the folder it came
    /// from. Never overwrites.
    Move {
        /// Where each moved item was and is now.
        items: Vec<MovedItem>,
    },
    /// Move to Trash, undone by restoring the items from the Recycle Bin.
    Trash {
        /// The local paths the items had.
        original_paths: Vec<PathBuf>,
        /// When the operation started, in seconds since the Unix epoch.
        /// Only Recycle Bin items deleted since then are restored, so an
        /// older item with the same original path stays where it is.
        trashed_since: u64,
    },
    /// Restore from the Recycle Bin, undone by moving the items to the
    /// Trash again.
    Restore {
        /// Where the restored items are now.
        restored: Vec<String>,
    },
}

/// One item of a batch rename: its URI before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedPair {
    /// The item's URI before the rename.
    pub original_uri: String,
    /// The item's URI after the rename.
    pub renamed_uri: String,
}

impl RenamedPair {
    /// The pair that renames the item back.
    fn swapped(&self) -> Self {
        Self {
            original_uri: self.renamed_uri.clone(),
            renamed_uri: self.original_uri.clone(),
        }
    }
}

/// One item of a move and where it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedItem {
    /// The item's URI before the move.
    pub original_uri: String,
    /// The item's URI after the move, under the same name.
    pub moved_uri: String,
}

impl UndoRecord {
    /// The operation's name as the Undo command shows it (`Rename`, `Move
    /// to Trash`, ...).
    pub fn title(&self) -> &'static str {
        match self {
            UndoRecord::Rename { .. } => "Rename",
            UndoRecord::BatchRename { .. } => "Batch rename",
            UndoRecord::Link { .. } => "Link",
            UndoRecord::Create {
                kind: ItemKind::Folder,
                ..
            } => "New folder",
            UndoRecord::Create {
                kind: ItemKind::File, ..
            } => "New file",
            UndoRecord::Copy { .. } => "Copy",
            UndoRecord::Duplicate { .. } => "Duplicate",
            UndoRecord::Move { .. } => "Move",
            UndoRecord::Trash { .. } => "Move to Trash",
            UndoRecord::Restore { .. } => "Restore",
        }
    }

    /// The label of the Undo command for this operation, for example
    /// `Undo: Rename`.
    pub fn undo_label(&self) -> String {
        format!("Undo: {}", self.title())
    }

    /// The record that reverses what reversing this record did, given the
    /// `result` of reversing it, which started at `reversed_since` (seconds
    /// since the Unix epoch). `None` when nothing was reversed.
    ///
    /// Only the items the reversal finished count, so a Redo never touches
    /// an item its Undo left alone:
    ///
    /// | Reversing | did | so the inverse |
    /// |---|---|---|
    /// | Rename | renamed the item back | renames it forward again |
    /// | Batch rename | renamed items back, newest first | renames them forward again, oldest first |
    /// | New item, Link, Copy, Duplicate, Restore | moved items to the Trash | restores them from the Recycle Bin |
    /// | Move | moved items back | moves them forward again |
    /// | Move to Trash | restored items | moves them to the Trash again |
    pub fn inverse(&self, result: &TransferResult, reversed_since: u64) -> Option<UndoRecord> {
        let finished = &result.done;
        if finished.is_empty() {
            return None;
        }
        let inverse = match self {
            UndoRecord::Rename {
                original_uri,
                renamed_uri,
            } => UndoRecord::Rename {
                original_uri: renamed_uri.clone(),
                renamed_uri: original_uri.clone(),
            },
            UndoRecord::BatchRename { items } => UndoRecord::BatchRename {
                items: renamed_forward_again(items, finished),
            },
            UndoRecord::Create { .. }
            | UndoRecord::Link { .. }
            | UndoRecord::Copy { .. }
            | UndoRecord::Duplicate { .. }
            | UndoRecord::Restore { .. } => trashed_again(finished, reversed_since)?,
            UndoRecord::Move { items } => UndoRecord::Move {
                items: moved_forward_again(items, finished),
            },
            UndoRecord::Trash { .. } => UndoRecord::Restore {
                restored: finished.clone(),
            },
        };
        Some(inverse)
    }
}

/// The record that restores `trashed` (the URIs a reversal moved to the
/// Trash since `since`) from the Recycle Bin, by their local paths, which
/// the Recycle Bin records as their original locations.
fn trashed_again(trashed: &[String], since: u64) -> Option<UndoRecord> {
    let original_paths: Vec<PathBuf> = trashed
        .iter()
        .filter_map(|uri| gio::prelude::FileExt::path(&gio::File::for_uri(uri)))
        .collect();
    if original_paths.is_empty() {
        return None;
    }
    Some(UndoRecord::Trash {
        original_paths,
        trashed_since: since,
    })
}

/// The renames that take the `items` a reversal renamed back (their
/// original URIs are in `renamed_back`) forward again. The reversal ran
/// from the last item to the first, so its own reversal runs the other
/// way: the list is kept in reverse, as the reversal walks it backwards.
fn renamed_forward_again(items: &[RenamedPair], renamed_back: &[String]) -> Vec<RenamedPair> {
    items
        .iter()
        .rev()
        .filter(|item| renamed_back.contains(&item.original_uri))
        .map(RenamedPair::swapped)
        .collect()
}

/// The moves that take the `items` a reversal put back (their moved URIs
/// are in `moved_back`) to where the operation had moved them.
fn moved_forward_again(items: &[MovedItem], moved_back: &[String]) -> Vec<MovedItem> {
    items
        .iter()
        .filter(|item| moved_back.contains(&item.moved_uri))
        .map(|item| MovedItem {
            original_uri: item.moved_uri.clone(),
            moved_uri: item.original_uri.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(number: usize) -> UndoRecord {
        UndoRecord::Rename {
            original_uri: format!("file:///tmp/{number}"),
            renamed_uri: format!("file:///tmp/{number}-renamed"),
        }
    }

    /// One record and the label of its Undo command.
    struct LabelCase {
        record: UndoRecord,
        label: &'static str,
    }

    #[test]
    fn undo_commands_name_their_operation() {
        let cases = [
            LabelCase {
                record: rename(1),
                label: "Undo: Rename",
            },
            LabelCase {
                record: UndoRecord::Create {
                    uri: "file:///tmp/a".into(),
                    kind: ItemKind::Folder,
                },
                label: "Undo: New folder",
            },
            LabelCase {
                record: UndoRecord::Create {
                    uri: "file:///tmp/a.txt".into(),
                    kind: ItemKind::File,
                },
                label: "Undo: New file",
            },
            LabelCase {
                record: UndoRecord::Duplicate { copies: Vec::new() },
                label: "Undo: Duplicate",
            },
            LabelCase {
                record: UndoRecord::Trash {
                    original_paths: Vec::new(),
                    trashed_since: 0,
                },
                label: "Undo: Move to Trash",
            },
        ];
        for case in cases {
            assert_eq!(case.record.undo_label(), case.label);
        }
    }

    /// A reversal that finished `done`.
    fn finished(done: &[&str]) -> TransferResult {
        TransferResult {
            done: done.iter().map(ToString::to_string).collect(),
            ..TransferResult::default()
        }
    }

    /// One record, what reversing it finished, and the record that
    /// reverses that in turn.
    struct InverseCase {
        record: UndoRecord,
        reversal: TransferResult,
        inverse: Option<UndoRecord>,
    }

    /// parity: OPS-031
    #[test]
    fn reversing_a_reversal_redoes_exactly_the_finished_items() {
        let moved = |from: &str, to: &str| MovedItem {
            original_uri: from.to_owned(),
            moved_uri: to.to_owned(),
        };
        let cases = [
            InverseCase {
                record: rename(1),
                reversal: finished(&["file:///tmp/1"]),
                inverse: Some(UndoRecord::Rename {
                    original_uri: "file:///tmp/1-renamed".into(),
                    renamed_uri: "file:///tmp/1".into(),
                }),
            },
            InverseCase {
                record: UndoRecord::Copy {
                    copies: vec!["file:///tmp/a".into(), "file:///tmp/b".into()],
                },
                reversal: finished(&["file:///tmp/b"]),
                inverse: Some(UndoRecord::Trash {
                    original_paths: vec![PathBuf::from("/tmp/b")],
                    trashed_since: 7,
                }),
            },
            InverseCase {
                record: UndoRecord::Move {
                    items: vec![
                        moved("file:///a/1", "file:///b/1"),
                        moved("file:///a/2", "file:///b/2"),
                    ],
                },
                reversal: finished(&["file:///b/2"]),
                inverse: Some(UndoRecord::Move {
                    items: vec![moved("file:///b/2", "file:///a/2")],
                }),
            },
            InverseCase {
                record: UndoRecord::Trash {
                    original_paths: vec![PathBuf::from("/tmp/a")],
                    trashed_since: 1,
                },
                reversal: finished(&["file:///tmp/a"]),
                inverse: Some(UndoRecord::Restore {
                    restored: vec!["file:///tmp/a".into()],
                }),
            },
            InverseCase {
                record: rename(2),
                reversal: TransferResult::default(),
                inverse: None,
            },
        ];
        for case in cases {
            assert_eq!(
                case.record.inverse(&case.reversal, 7),
                case.inverse,
                "{:?}",
                case.record
            );
        }
    }
}
