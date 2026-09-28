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
//! | New folder, New file, New from template | Moving the new item to the Trash |
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

use std::collections::VecDeque;
use std::path::PathBuf;

use crate::location::ItemKind;

/// How many operations the journal remembers; older ones are dropped.
pub const UNDO_LIMIT: usize = 100;

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
}

/// The operations Undo can still reverse, newest last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoJournal {
    records: VecDeque<UndoRecord>,
}

impl UndoJournal {
    /// An empty journal: Undo is disabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remembers a finished operation. Beyond [`UNDO_LIMIT`] the oldest
    /// one is forgotten.
    pub fn record(&mut self, record: UndoRecord) {
        if self.records.len() == UNDO_LIMIT {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    /// The operation Undo would reverse next, which labels the command.
    pub fn last(&self) -> Option<&UndoRecord> {
        self.records.back()
    }

    /// Removes and returns the operation to reverse now.
    pub fn take_last(&mut self) -> Option<UndoRecord> {
        self.records.pop_back()
    }

    /// True when there is nothing to undo, so the command is disabled.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The number of operations that can be undone.
    pub fn len(&self) -> usize {
        self.records.len()
    }
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

    #[test]
    fn undo_reverses_the_newest_operation_first() {
        let mut journal = UndoJournal::new();
        assert!(journal.is_empty());

        journal.record(rename(1));
        journal.record(rename(2));

        assert_eq!(journal.last(), Some(&rename(2)));
        assert_eq!(journal.take_last(), Some(rename(2)));
        assert_eq!(journal.take_last(), Some(rename(1)));
        assert_eq!(journal.take_last(), None);
    }

    #[test]
    fn the_journal_forgets_the_oldest_operation_beyond_its_limit() {
        let mut journal = UndoJournal::new();

        for number in 0..=UNDO_LIMIT {
            journal.record(rename(number));
        }

        assert_eq!(journal.len(), UNDO_LIMIT);
        let oldest = std::iter::from_fn(|| journal.take_last()).last();
        assert_eq!(oldest, Some(rename(1)));
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
}
