// SPDX-License-Identifier: AGPL-3.0-only
//! Where the items of a finished copy or move are now, so the interface
//! can select them (SEL-016) and Undo can find them (OPS-029).
//!
//! The transfer engine reports which sources it handled, not the names it
//! gave them. They follow from its naming rules (`conflicts.rs` in the
//! transfer module):
//!
//! - Skip and Replace keep the source's name: a finished item is exactly
//!   `folder/name`.
//! - Keep both uses the name if it is free, and otherwise the first free
//!   `(copy N)` name, where the filesystem decides what is free. On a
//!   case-insensitive drive (FAT, exFAT, most SMB shares) `A (copy 2).txt`
//!   takes `a (copy 2).txt` too, so the engine's choice cannot be worked
//!   out from the names alone without risking the user's own item.
//!   Instead the folder is listed before and after the run, and only a
//!   name that appeared during the run can be a copy: each finished item,
//!   in order, claims the first of its own name and its `(copy N)` names
//!   that appeared and that no earlier item claimed. Only another program
//!   creating one of those names during the run could make the answer
//!   wrong, and even then it names an item that did not exist before the
//!   run; the undo of a copy moves copies to the Trash, never deleting
//!   them, so nothing is lost.
//! - A Keep-both move is not tracked: its source is gone, so the source's
//!   kind, which decides where `(copy N)` goes, cannot be read afterwards.

use std::collections::HashSet;
use std::ffi::OsString;

use crate::gio_node::GioNode;
use crate::location::{new_copy_name, ItemKind};
use crate::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, TransferMode};

/// The transfer engine's Keep-both limit: it tries `(copy 2)` up to
/// `(copy 9999)` (XFER-008).
const COPY_NUMBER_LIMIT: u32 = 10_000;

/// One finished item and where it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Landed {
    /// The item's URI before the run.
    pub(crate) source: String,
    /// The item's URI now.
    pub(crate) destination: String,
}

/// How the engine named the items of one run.
#[derive(Debug)]
enum NamingRule {
    /// Every finished item kept its name (Skip, Replace).
    SameName,
    /// Keep both, with the names the folder had before the run.
    KeepBoth { names_before: HashSet<OsString> },
    /// The names cannot be known exactly.
    Unknown,
}

/// Finds where the finished items of one copy or move into a folder are.
#[derive(Debug)]
pub(crate) struct DestinationTracker {
    folder: GioNode,
    rule: NamingRule,
}

impl DestinationTracker {
    /// Prepares to track a `mode` run into the folder at `folder_uri` under
    /// `policy`, reading the folder's names first when Keep both needs
    /// them. A folder that cannot be read leaves the names unknown; the
    /// run itself is not affected.
    pub(crate) fn before_run(
        mode: TransferMode,
        folder_uri: &str,
        policy: ConflictPolicy,
        cancel: &Cancellation,
    ) -> Self {
        let folder = GioNode::new(folder_uri);
        let rule = match (policy, mode) {
            (ConflictPolicy::Skip | ConflictPolicy::Replace, _) => NamingRule::SameName,
            (ConflictPolicy::KeepBoth, TransferMode::Copy) => match names_in(&folder, cancel) {
                Some(names_before) => NamingRule::KeepBoth { names_before },
                None => NamingRule::Unknown,
            },
            (ConflictPolicy::KeepBoth, _) => NamingRule::Unknown,
        };
        Self { folder, rule }
    }

    /// Where each of the finished `done` items is now, in order. Items
    /// whose place is not known exactly are left out.
    pub(crate) fn landed(&self, done: &[String]) -> Vec<Landed> {
        match &self.rule {
            NamingRule::SameName => done.iter().map(|source| self.same_name(source)).collect(),
            NamingRule::KeepBoth { names_before } => self.keep_both(done, names_before),
            NamingRule::Unknown => Vec::new(),
        }
    }

    /// `folder/name` for the finished item `source`.
    fn same_name(&self, source: &str) -> Landed {
        let name = GioNode::new(source).name();
        let destination = self.folder.child(&name);
        Landed {
            source: source.to_owned(),
            destination: destination.uri(),
        }
    }

    /// Finds the Keep-both copies of `done` among the names that appeared
    /// in the folder since `names_before` were read.
    fn keep_both(&self, done: &[String], names_before: &HashSet<OsString>) -> Vec<Landed> {
        // Listed even after a cancellation: the items that finished stay in
        // place, and Undo must still find them.
        let Some(names_after) = names_in(&self.folder, &Cancellation::new()) else {
            return Vec::new();
        };
        let mut appeared = AppearedNames::between(names_before, names_after);
        let mut landed = Vec::new();
        for source in done {
            let Some(name) = appeared.claim_for(&GioNode::new(source)) else {
                continue;
            };
            landed.push(Landed {
                source: source.clone(),
                destination: self.folder.child(&name).uri(),
            });
        }
        landed
    }
}

/// The names that appeared in a folder during a run, each to be claimed by
/// the finished item the engine gave it.
#[derive(Debug)]
struct AppearedNames {
    unclaimed: HashSet<OsString>,
}

impl AppearedNames {
    /// The names in `after` that are not in `before`, compared byte for
    /// byte: on a case-insensitive drive, `A (copy 2).txt` before the run
    /// and `a (copy 3).txt` after it are different items.
    fn between(before: &HashSet<OsString>, after: HashSet<OsString>) -> Self {
        let unclaimed = after.into_iter().filter(|name| !before.contains(name)).collect();
        Self { unclaimed }
    }

    /// Claims the name Keep both gave `source`: the first of its own name
    /// and its `(copy N)` names that appeared and is not claimed yet.
    /// `None` when none did, so the item's place is not known.
    fn claim_for(&mut self, source: &GioNode) -> Option<OsString> {
        if self.unclaimed.is_empty() {
            return None;
        }
        let own_name = source.name();
        if self.unclaimed.remove(&own_name) {
            return Some(own_name);
        }
        // The engine gives no `(copy N)` name to a name that is not text.
        let text = own_name.to_str()?;
        let kind = item_kind(source)?;
        let copy_name = (2..COPY_NUMBER_LIMIT)
            .filter_map(|number| new_copy_name(text, number, kind).ok())
            .map(OsString::from)
            .find(|candidate| self.unclaimed.contains(candidate))?;
        self.unclaimed.remove(&copy_name);
        Some(copy_name)
    }
}

/// Whether `(copy N)` names `source` as a folder or as a file, as the
/// engine decides it; `None` when `source` cannot be inspected.
fn item_kind(source: &GioNode) -> Option<ItemKind> {
    let kind = match source.info(None).ok()?.kind {
        NodeKind::Directory => ItemKind::Folder,
        NodeKind::File | NodeKind::Symlink | NodeKind::Special => ItemKind::File,
    };
    Some(kind)
}

/// The names in `folder`, hidden ones included, or `None` when it cannot
/// be listed.
fn names_in(folder: &GioNode, cancel: &Cancellation) -> Option<HashSet<OsString>> {
    let children = folder.children(Some(cancel)).ok()?;
    let names = children.iter().map(|child| child.name()).collect();
    Some(names)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use gio::prelude::*;

    use super::*;

    /// A new source file at `path`, in a folder created for it.
    fn source_file(path: &Path) -> GioNode {
        fs::create_dir_all(path.parent().expect("a parent")).expect("a source folder");
        fs::write(path, b"x").expect("a source file");
        GioNode::new(&gio::File::for_path(path).uri())
    }

    /// The names that appeared when a folder holding `before` came to hold
    /// `before` and `added`.
    fn appeared(before: &[&str], added: &[&str]) -> AppearedNames {
        let before: HashSet<OsString> = before.iter().map(OsString::from).collect();
        let mut after = before.clone();
        after.extend(added.iter().map(OsString::from));
        AppearedNames::between(&before, after)
    }

    #[test]
    fn keep_both_claims_the_first_free_copy_name_in_order() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let first = source_file(&temp.path().join("one").join("a.txt"));
        let second = source_file(&temp.path().join("two").join("a.txt"));
        let mut names = appeared(&["a.txt"], &["a (copy 2).txt", "a (copy 3).txt"]);

        let first_name = names.claim_for(&first).expect("a copy name");
        let second_name = names.claim_for(&second).expect("a copy name");

        assert_eq!(first_name, OsString::from("a (copy 2).txt"));
        assert_eq!(second_name, OsString::from("a (copy 3).txt"));
    }

    #[test]
    fn a_folder_keeps_its_whole_name_before_the_copy_marker() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let folder = temp.path().join("Folder.v1");
        fs::create_dir(&folder).expect("a source folder");
        let source = GioNode::new(&gio::File::for_path(&folder).uri());
        let mut names = appeared(&["Folder.v1"], &["Folder.v1 (copy 2)"]);

        let name = names.claim_for(&source);

        assert_eq!(name, Some(OsString::from("Folder.v1 (copy 2)")));
    }

    #[test]
    fn a_free_name_is_claimed_as_it_is() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let source = source_file(&temp.path().join("one").join("b.txt"));
        let mut names = appeared(&["a.txt"], &["b.txt"]);

        let name = names.claim_for(&source);

        assert_eq!(name, Some(OsString::from("b.txt")));
    }

    /// On FAT, exFAT and most SMB shares a name differing only in case is
    /// taken, so the engine skips it; the copy must never be recorded
    /// under the name of the user's own item, which Undo would trash.
    #[test]
    fn on_a_case_insensitive_drive_a_copy_never_names_the_users_own_item() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let report = source_file(&temp.path().join("one").join("a.txt"));
        let notes = source_file(&temp.path().join("one").join("b.txt"));
        let mut names = appeared(
            &["a.txt", "A (copy 2).txt", "B.txt"],
            &["a (copy 3).txt", "b (copy 2).txt"],
        );

        let report_copy = names.claim_for(&report);
        let notes_copy = names.claim_for(&notes);

        assert_eq!(report_copy, Some(OsString::from("a (copy 3).txt")));
        assert_eq!(notes_copy, Some(OsString::from("b (copy 2).txt")));
    }

    #[test]
    fn an_item_without_a_new_name_in_the_folder_is_left_out() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let source = source_file(&temp.path().join("one").join("a.txt"));
        let mut names = appeared(&["a.txt", "a (copy 2).txt"], &[]);

        let name = names.claim_for(&source);

        assert_eq!(name, None);
    }
}
