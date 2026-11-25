// SPDX-License-Identifier: AGPL-3.0-only
//! The undo journal: what the last file operations did, so Undo can
//! reverse them one step at a time (OPS-029).
//!
//! New in the native app; the Python app has no Undo (`v2.0.0:desktop/MANUAL.md`,
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
//! | Copy or move that replaced files | Undoing the copy or move, then restoring the replaced files from the Recycle Bin |
//! | Folders a merging move emptied and removed | Making them again, never over an item |
//!
//! A Replace answered in the name-conflict questions moves the replaced
//! file to the Recycle Bin before the incoming file takes its name
//! (`resolved_transfer`), and the record lists every replaced file, so
//! Undo can bring it back. Folders merged by a paste are never moved to
//! the Trash by its Undo: the record lists the items that went into them.
//!
//! Not undoable: permanent delete and emptying the Recycle Bin (nothing is
//! left to bring back), and a file a Replace overwrote, as Windows 11 does
//! (the setting off) or where nothing could go to the Recycle Bin: the record
//! names it, Undo says it can't bring it back, and its replacement stays
//! when the rest is undone. Every undo step runs with the same safety
//! rules as the operation itself; `undo_apply` carries them out.
//!
//! Reversing a record has an effect of its own, which [`UndoRecord::inverse`]
//! describes as another record: Redo reverses an Undo with it, and Undo a
//! Redo. The journal that keeps both directions is `journal`.

use std::path::PathBuf;

use super::journal::JournalDirection;
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
    /// Folders a move merged into folders of their names, emptied and
    /// removed: undone by making each again where nothing has its name.
    /// A Replace record lists them first, so the moved items have their
    /// folders to go back into.
    EmptiedFolders {
        /// The folders, each after the folder it is in.
        folders: Vec<String>,
    },
    /// Folders an Undo made again: undone (Redo) by removing each again
    /// while it is still empty.
    RemadeFolders {
        /// The folders, each after the folder it is in.
        folders: Vec<String>,
    },
    /// A copy, move or restore that replaced files, which went to the
    /// Recycle Bin or were overwritten, or that removed the folders it
    /// emptied: undone by undoing `steps` in order, then restoring each
    /// replaced file. The Recycle Bin items are looked up before the
    /// steps run, since the steps may move other items from the same
    /// paths to the Trash. A file it overwrote, as Windows overwrites one,
    /// is gone: Undo says so first ([`UndoRecord::overwritten`]) and
    /// leaves its replacement in place. Its Redo is a record of the same
    /// kind.
    Replace {
        /// The operation it belongs to, which names the step.
        operation: Operation,
        /// What undoes the rest of the operation.
        steps: Vec<UndoRecord>,
        /// The local paths the replaced files had.
        replaced: Vec<PathBuf>,
        /// When they went to the Recycle Bin, at the earliest, in seconds
        /// since the Unix epoch.
        replaced_since: u64,
        /// The files the operation overwrote, by the URI their
        /// replacements have: nothing can bring the old ones back.
        overwritten: Vec<String>,
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

/// The operation a step of the journal belongs to, in Windows Explorer's
/// words: "Undo Delete" after a Move to Trash, "Undo Rename" after any
/// rename and "Undo New" after a new folder or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// A rename or a batch rename.
    Rename,
    /// New links.
    Link,
    /// A new folder or file.
    New,
    /// A copy.
    Copy,
    /// Duplicate.
    Duplicate,
    /// A move.
    Move,
    /// Move to Trash.
    Delete,
    /// Restore from the Recycle Bin.
    Restore,
}

impl Operation {
    /// The operation's name on its own, such as `Rename`.
    pub fn title(self) -> &'static str {
        match self {
            Operation::Rename => crate::i18n::gettext_static("Rename"),
            Operation::Link => crate::i18n::gettext_static("Link"),
            Operation::New => crate::i18n::gettext_static("New"),
            Operation::Copy => crate::i18n::gettext_static("Copy"),
            Operation::Duplicate => crate::i18n::gettext_static("Duplicate"),
            Operation::Move => crate::i18n::gettext_static("Move"),
            Operation::Delete => crate::i18n::gettext_static("Delete"),
            Operation::Restore => crate::i18n::gettext_static("Restore"),
        }
    }

    /// The label of the command walking `direction` through this
    /// operation, such as `Undo Rename`. Each label is one message, so a
    /// translation can inflect the operation's name inside it.
    pub fn label(self, direction: JournalDirection) -> &'static str {
        use JournalDirection::{Redo, Undo};
        match (direction, self) {
            (Undo, Operation::Rename) => crate::i18n::gettext_static("Undo Rename"),
            (Undo, Operation::Link) => crate::i18n::gettext_static("Undo Link"),
            (Undo, Operation::New) => crate::i18n::gettext_static("Undo New"),
            (Undo, Operation::Copy) => crate::i18n::gettext_static("Undo Copy"),
            (Undo, Operation::Duplicate) => crate::i18n::gettext_static("Undo Duplicate"),
            (Undo, Operation::Move) => crate::i18n::gettext_static("Undo Move"),
            (Undo, Operation::Delete) => crate::i18n::gettext_static("Undo Delete"),
            (Undo, Operation::Restore) => crate::i18n::gettext_static("Undo Restore"),
            (Redo, Operation::Rename) => crate::i18n::gettext_static("Redo Rename"),
            (Redo, Operation::Link) => crate::i18n::gettext_static("Redo Link"),
            (Redo, Operation::New) => crate::i18n::gettext_static("Redo New"),
            (Redo, Operation::Copy) => crate::i18n::gettext_static("Redo Copy"),
            (Redo, Operation::Duplicate) => crate::i18n::gettext_static("Redo Duplicate"),
            (Redo, Operation::Move) => crate::i18n::gettext_static("Redo Move"),
            (Redo, Operation::Delete) => crate::i18n::gettext_static("Redo Delete"),
            (Redo, Operation::Restore) => crate::i18n::gettext_static("Redo Restore"),
        }
    }
}

impl UndoRecord {
    /// This move's record once `folder`, whose items it moved into a
    /// folder of another name, was emptied and removed: a folder renamed
    /// onto another folder's name is merged into it, as Windows offers.
    /// One Undo, named Rename, makes `folder` again first, so the items
    /// have it to go back into, then undoes the move and brings back what
    /// it replaced; Redo moves them in again and removes the folder while
    /// it is empty.
    #[must_use]
    pub fn with_emptied_folder(self, folder: String) -> UndoRecord {
        let emptied = UndoRecord::EmptiedFolders {
            folders: vec![folder],
        };
        match self {
            UndoRecord::Replace {
                steps,
                replaced,
                replaced_since,
                overwritten,
                ..
            } => UndoRecord::Replace {
                operation: Operation::Rename,
                steps: std::iter::once(emptied).chain(steps).collect(),
                replaced,
                replaced_since,
                overwritten,
            },
            other => UndoRecord::Replace {
                operation: Operation::Rename,
                steps: vec![emptied, other],
                replaced: Vec::new(),
                replaced_since: 0,
                overwritten: Vec::new(),
            },
        }
    }

    /// The operation the record reverses.
    pub fn operation(&self) -> Operation {
        match self {
            UndoRecord::Rename { .. } | UndoRecord::BatchRename { .. } => Operation::Rename,
            UndoRecord::Link { .. } => Operation::Link,
            UndoRecord::Create { .. } => Operation::New,
            UndoRecord::Copy { .. } => Operation::Copy,
            UndoRecord::Duplicate { .. } => Operation::Duplicate,
            UndoRecord::Move { .. }
            | UndoRecord::EmptiedFolders { .. }
            | UndoRecord::RemadeFolders { .. } => Operation::Move,
            UndoRecord::Trash { .. } => Operation::Delete,
            UndoRecord::Restore { .. } => Operation::Restore,
            UndoRecord::Replace { operation, .. } => *operation,
        }
    }

    /// The operation's name as the Undo command shows it (`Rename`,
    /// `Delete`, ...).
    pub fn title(&self) -> &'static str {
        self.operation().title()
    }

    /// The files the operation overwrote, which Undo cannot bring back,
    /// by the URI their replacements have; empty when it overwrote none.
    pub fn overwritten(&self) -> &[String] {
        match self {
            UndoRecord::Replace { overwritten, .. } => overwritten,
            _ => &[],
        }
    }

    /// Each item the operation moved or renamed, from where it was to
    /// where it is now: what the thumbnail cache follows, so a moved file
    /// keeps its thumbnail (`crate::thumbnail_cache::follow_move`).
    pub fn moved_items(&self) -> Vec<(String, String)> {
        match self {
            UndoRecord::Rename {
                original_uri,
                renamed_uri,
            } => vec![(original_uri.clone(), renamed_uri.clone())],
            UndoRecord::BatchRename { items } => items
                .iter()
                .map(|pair| (pair.original_uri.clone(), pair.renamed_uri.clone()))
                .collect(),
            UndoRecord::Move { items } => items
                .iter()
                .map(|item| (item.original_uri.clone(), item.moved_uri.clone()))
                .collect(),
            UndoRecord::Replace { steps, .. } => steps.iter().flat_map(UndoRecord::moved_items).collect(),
            _ => Vec::new(),
        }
    }

    /// Every location (URI) reversing the record may rename, move, delete
    /// or create, in the record's order: each item as it is now and where
    /// it goes back to, and for a Replace, the files it put back and the
    /// ones it could not. The app stops any player of these first.
    pub fn locations(&self) -> Vec<String> {
        match self {
            UndoRecord::Rename {
                original_uri,
                renamed_uri,
            } => vec![original_uri.clone(), renamed_uri.clone()],
            UndoRecord::BatchRename { items } => items
                .iter()
                .flat_map(|pair| [pair.original_uri.clone(), pair.renamed_uri.clone()])
                .collect(),
            UndoRecord::Move { items } => items
                .iter()
                .flat_map(|item| [item.original_uri.clone(), item.moved_uri.clone()])
                .collect(),
            UndoRecord::Create { uri, .. } => vec![uri.clone()],
            UndoRecord::Link { links } => links.clone(),
            UndoRecord::Copy { copies } | UndoRecord::Duplicate { copies } => copies.clone(),
            UndoRecord::Restore { restored } => restored.clone(),
            UndoRecord::EmptiedFolders { folders } | UndoRecord::RemadeFolders { folders } => folders.clone(),
            UndoRecord::Trash { original_paths, .. } => original_paths
                .iter()
                .map(|path| crate::location::file_uri(path))
                .collect(),
            UndoRecord::Replace {
                steps,
                replaced,
                overwritten,
                ..
            } => steps
                .iter()
                .flat_map(UndoRecord::locations)
                .chain(replaced.iter().map(|path| crate::location::file_uri(path)))
                .chain(overwritten.iter().cloned())
                .collect(),
        }
    }

    /// Whether undoing the record does anything beyond saying which files
    /// it cannot bring back ([`Self::overwritten`]).
    pub fn undoes_more_than_overwrites(&self) -> bool {
        match self {
            UndoRecord::Replace { steps, replaced, .. } => !steps.is_empty() || !replaced.is_empty(),
            _ => true,
        }
    }

    /// How Undo takes back an extraction into an existing folder, given
    /// the record of the move that brought its items in from the
    /// extraction's private folder: that folder is gone, so the items are
    /// not moved back but taken away as copies are, to the Recycle Bin,
    /// under Undo Copy, as Windows names the Undo of Extract All. Files
    /// a Replace answered there still come back from the Recycle Bin;
    /// the private folder's removal is not undone. `None` when nothing is
    /// left to undo.
    pub fn as_extraction(self) -> Option<UndoRecord> {
        match self {
            UndoRecord::Move { items } => {
                let copies: Vec<String> = items.into_iter().map(|item| item.moved_uri).collect();
                (!copies.is_empty()).then_some(UndoRecord::Copy { copies })
            }
            UndoRecord::Replace {
                steps,
                replaced,
                replaced_since,
                overwritten,
                ..
            } => {
                let steps: Vec<UndoRecord> =
                    steps.into_iter().filter_map(UndoRecord::as_extraction).collect();
                if steps.is_empty() && replaced.is_empty() && overwritten.is_empty() {
                    return None;
                }
                Some(UndoRecord::Replace {
                    operation: Operation::Copy,
                    steps,
                    replaced,
                    replaced_since,
                    overwritten,
                })
            }
            // The private folder the move emptied is not made again.
            UndoRecord::EmptiedFolders { .. } | UndoRecord::RemadeFolders { .. } => None,
            other => Some(other),
        }
    }

    /// The label of the Undo command for this operation, for example
    /// `Undo Rename`.
    pub fn undo_label(&self) -> String {
        self.operation().label(JournalDirection::Undo).to_owned()
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
    ///
    /// A [`UndoRecord::Replace`] needs the result of each of its parts, so
    /// its inverse is [`UndoRecord::replace_inverse`]; this gives `None`.
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
            UndoRecord::Replace { .. } => return None,
            // Made again, they are removed again, the innermost first; and
            // the other way round.
            UndoRecord::EmptiedFolders { .. } => UndoRecord::RemadeFolders {
                folders: finished.iter().rev().cloned().collect(),
            },
            UndoRecord::RemadeFolders { .. } => UndoRecord::EmptiedFolders {
                folders: finished.iter().rev().cloned().collect(),
            },
        };
        Some(inverse)
    }

    /// The inverse of reversing a [`UndoRecord::Replace`] of `operation`
    /// that started at `reversed_since`: `steps` are its steps with each
    /// one's result, `restored` the replaced files it brought back from
    /// the Recycle Bin. Taking it back moves those files to the Trash
    /// again, then reverses each step's reversal, the last first; the
    /// items the steps moved to the Trash are its own replaced files.
    pub fn replace_inverse(
        operation: Operation,
        steps: &[(UndoRecord, TransferResult)],
        restored: &TransferResult,
        reversed_since: u64,
    ) -> Option<UndoRecord> {
        let mut inverse_steps = Vec::new();
        if !restored.done.is_empty() {
            inverse_steps.push(UndoRecord::Restore {
                restored: restored.done.clone(),
            });
        }
        let mut replaced = Vec::new();
        for (step, result) in steps.iter().rev() {
            match step.inverse(result, reversed_since) {
                Some(UndoRecord::Trash { original_paths, .. }) => replaced.extend(original_paths),
                Some(other) => inverse_steps.push(other),
                None => {}
            }
        }
        if inverse_steps.is_empty() && replaced.is_empty() {
            return None;
        }
        // Undo left the files it overwrote alone, so Redo has none.
        Some(UndoRecord::Replace {
            operation,
            steps: inverse_steps,
            replaced,
            replaced_since: reversed_since,
            overwritten: Vec::new(),
        })
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

    /// Renames and moves, also inside a Replace, name each item's old and
    /// new place; other operations move nothing.
    ///
    /// parity: VIEW-057
    #[test]
    fn renames_and_moves_name_what_they_moved() {
        let rename = UndoRecord::Rename {
            original_uri: "file:///work/a.jpg".to_owned(),
            renamed_uri: "file:///work/b.jpg".to_owned(),
        };
        assert_eq!(
            rename.moved_items(),
            [("file:///work/a.jpg".to_owned(), "file:///work/b.jpg".to_owned())]
        );
        let moved = UndoRecord::Move {
            items: vec![MovedItem {
                original_uri: "file:///work/c.jpg".to_owned(),
                moved_uri: "file:///work/Done/c.jpg".to_owned(),
            }],
        };
        let replace = UndoRecord::Replace {
            operation: Operation::Move,
            steps: vec![moved, rename],
            replaced: Vec::new(),
            replaced_since: 0,
            overwritten: Vec::new(),
        };
        assert_eq!(replace.moved_items().len(), 2);
        assert_eq!(replace.moved_items()[0].1, "file:///work/Done/c.jpg");
        let copy = UndoRecord::Copy {
            copies: vec!["file:///work/d.jpg".to_owned()],
        };
        assert!(copy.moved_items().is_empty());
    }

    #[test]
    fn a_reversal_names_every_file_it_may_change() {
        let moved = MovedItem {
            original_uri: "file:///work/Clip.ogv".to_owned(),
            moved_uri: "file:///work/Done/Clip.ogv".to_owned(),
        };
        let record = UndoRecord::Replace {
            operation: Operation::Move,
            steps: vec![
                UndoRecord::Move { items: vec![moved] },
                UndoRecord::EmptiedFolders {
                    folders: vec!["file:///work/Old".to_owned()],
                },
            ],
            replaced: vec![PathBuf::from("/work/Done/Song.ogg")],
            replaced_since: 0,
            overwritten: vec!["file:///work/Done/Tone.mp3".to_owned()],
        };
        assert_eq!(
            record.locations(),
            [
                "file:///work/Clip.ogv",
                "file:///work/Done/Clip.ogv",
                "file:///work/Old",
                "file:///work/Done/Song.ogg",
                "file:///work/Done/Tone.mp3",
            ]
        );
        assert_eq!(
            rename(1).locations(),
            ["file:///tmp/1", "file:///tmp/1-renamed"],
            "a rename back may land on its old name"
        );
        let trashed = UndoRecord::Trash {
            original_paths: vec![PathBuf::from("/work/Clip.ogv")],
            trashed_since: 0,
        };
        assert_eq!(trashed.locations(), ["file:///work/Clip.ogv"]);
        let copied = UndoRecord::Copy {
            copies: vec!["file:///work/Copy.ogv".to_owned()],
        };
        assert_eq!(copied.locations(), ["file:///work/Copy.ogv"]);
    }

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
                label: "Undo Rename",
            },
            LabelCase {
                record: UndoRecord::Create {
                    uri: "file:///tmp/a".into(),
                    kind: ItemKind::Folder,
                },
                label: "Undo New",
            },
            LabelCase {
                record: UndoRecord::Create {
                    uri: "file:///tmp/a.txt".into(),
                    kind: ItemKind::File,
                },
                label: "Undo New",
            },
            LabelCase {
                record: UndoRecord::Duplicate { copies: Vec::new() },
                label: "Undo Duplicate",
            },
            LabelCase {
                record: UndoRecord::Trash {
                    original_paths: Vec::new(),
                    trashed_since: 0,
                },
                label: "Undo Delete",
            },
        ];
        for case in cases {
            assert_eq!(case.record.undo_label(), case.label);
        }
    }

    /// An extraction into an existing folder moved its items in from a
    /// private folder that is gone: Undo takes them away as copies, and a
    /// Replace keeps bringing back the files it replaced.
    ///
    /// parity: OPS-029
    #[test]
    fn an_extraction_is_undone_as_a_copy() {
        let moved = |from: &str, to: &str| MovedItem {
            original_uri: from.to_owned(),
            moved_uri: to.to_owned(),
        };
        let private = "file:///tmp/Work/.openxplorer-extract-1";
        let plain = UndoRecord::Move {
            items: vec![moved(&format!("{private}/a.mp3"), "file:///tmp/Work/a.mp3")],
        };
        assert_eq!(
            plain.as_extraction(),
            Some(UndoRecord::Copy {
                copies: vec!["file:///tmp/Work/a.mp3".into()]
            })
        );
        let replacing = UndoRecord::Replace {
            operation: Operation::Move,
            steps: vec![
                UndoRecord::EmptiedFolders {
                    folders: vec![format!("{private}/Docs")],
                },
                UndoRecord::Move {
                    items: vec![moved(&format!("{private}/b.pdf"), "file:///tmp/Work/b.pdf")],
                },
            ],
            replaced: vec![PathBuf::from("/tmp/Work/b.pdf")],
            replaced_since: 3,
            overwritten: Vec::new(),
        };
        let undone = replacing.as_extraction().expect("a step to undo");
        assert_eq!(undone.undo_label(), "Undo Copy");
        assert_eq!(
            undone,
            UndoRecord::Replace {
                operation: Operation::Copy,
                steps: vec![UndoRecord::Copy {
                    copies: vec!["file:///tmp/Work/b.pdf".into()]
                }],
                replaced: vec![PathBuf::from("/tmp/Work/b.pdf")],
                replaced_since: 3,
                overwritten: Vec::new(),
            }
        );
        assert_eq!(UndoRecord::Move { items: Vec::new() }.as_extraction(), None);
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

#[cfg(test)]
mod merged_rename_tests {
    use super::*;

    fn moved(from: &str, to: &str) -> MovedItem {
        MovedItem {
            original_uri: from.to_owned(),
            moved_uri: to.to_owned(),
        }
    }

    /// A folder renamed onto another folder's name is merged into it: its
    /// items move in, and the emptied folder goes. One Undo, "Undo
    /// Rename", makes the folder again first, so the items have it to go
    /// back into, then undoes the move; what the move replaced comes back
    /// as after any Replace.
    ///
    /// parity: OPS-008
    #[test]
    fn a_merge_by_rename_is_undone_by_making_the_folder_then_moving_back() {
        let alpha = "file:///srv/example/Alpha";
        let movement = UndoRecord::Move {
            items: vec![moved(&format!("{alpha}/a.txt"), "file:///srv/example/Beta/a.txt")],
        };

        let merged = movement.clone().with_emptied_folder(alpha.to_owned());
        let replaced = UndoRecord::Replace {
            operation: Operation::Move,
            steps: vec![movement.clone()],
            replaced: vec![PathBuf::from("/srv/example/Beta/b.txt")],
            replaced_since: 42,
            overwritten: vec!["file:///srv/example/Beta/c.txt".to_owned()],
        }
        .with_emptied_folder(alpha.to_owned());

        let emptied = UndoRecord::EmptiedFolders {
            folders: vec![alpha.to_owned()],
        };
        assert_eq!(
            merged,
            UndoRecord::Replace {
                operation: Operation::Rename,
                steps: vec![emptied.clone(), movement.clone()],
                replaced: Vec::new(),
                replaced_since: 0,
                overwritten: Vec::new(),
            }
        );
        assert_eq!(merged.title(), Operation::Rename.title());
        assert_eq!(
            replaced,
            UndoRecord::Replace {
                operation: Operation::Rename,
                steps: vec![emptied, movement],
                replaced: vec![PathBuf::from("/srv/example/Beta/b.txt")],
                replaced_since: 42,
                overwritten: vec!["file:///srv/example/Beta/c.txt".to_owned()],
            }
        );
    }
}
