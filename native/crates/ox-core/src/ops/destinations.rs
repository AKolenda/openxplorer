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
//!   `(copy N)` name. The folder's names are read before the run, and the
//!   finished items are replayed in order against them, each claiming its
//!   name. Only a program creating that same `(copy N)` name during the
//!   run could make the answer wrong; the undo of a copy moves copies to
//!   the Trash, never deleting them, so even then nothing is lost.
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
    /// whose place is not known exactly, or that are no longer there, are
    /// left out.
    pub(crate) fn landed(&self, done: &[String]) -> Vec<Landed> {
        match &self.rule {
            NamingRule::SameName => done.iter().map(|source| self.same_name(source)).collect(),
            NamingRule::KeepBoth { names_before } => self.replay_keep_both(done, names_before.clone()),
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

    /// Replays the engine's Keep-both choices for `done`, in order, each
    /// claiming the first name that was still free.
    fn replay_keep_both(&self, done: &[String], mut taken: HashSet<OsString>) -> Vec<Landed> {
        let mut landed = Vec::new();
        for source in done {
            let Some(name) = keep_both_name(&GioNode::new(source), &taken) else {
                continue;
            };
            let destination = self.folder.child(&name);
            taken.insert(name);
            if destination.exists(None) {
                landed.push(Landed {
                    source: source.clone(),
                    destination: destination.uri(),
                });
            }
        }
        landed
    }
}

/// The name Keep both gave `source` when `taken` were the names in use:
/// its own if free, otherwise the first free `(copy N)` name.
fn keep_both_name(source: &GioNode, taken: &HashSet<OsString>) -> Option<OsString> {
    let name = source.name();
    if !taken.contains(&name) {
        return Some(name);
    }
    let text = name.to_str()?;
    let kind = match source.info(None).ok()?.kind {
        NodeKind::Directory => ItemKind::Folder,
        NodeKind::File | NodeKind::Symlink | NodeKind::Special => ItemKind::File,
    };
    (2..COPY_NUMBER_LIMIT)
        .filter_map(|number| new_copy_name(text, number, kind).ok())
        .map(OsString::from)
        .find(|candidate| !taken.contains(candidate))
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

    use gio::prelude::*;

    use super::*;

    fn uri_of(path: &std::path::Path) -> String {
        gio::File::for_path(path).uri().to_string()
    }

    #[test]
    fn keep_both_claims_the_first_free_copy_name_in_order() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let first = temp.path().join("one").join("a.txt");
        let second = temp.path().join("two").join("a.txt");
        for path in [&first, &second] {
            fs::create_dir_all(path.parent().expect("a parent")).expect("a source folder");
            fs::write(path, b"x").expect("a source file");
        }
        let mut taken = HashSet::new();
        taken.insert(OsString::from("a.txt"));

        let first_name = keep_both_name(&GioNode::new(&uri_of(&first)), &taken).expect("a free name");
        taken.insert(first_name.clone());
        let second_name = keep_both_name(&GioNode::new(&uri_of(&second)), &taken).expect("a free name");

        assert_eq!(first_name, OsString::from("a (copy 2).txt"));
        assert_eq!(second_name, OsString::from("a (copy 3).txt"));
    }

    #[test]
    fn a_folder_keeps_its_whole_name_before_the_copy_marker() {
        let temp = tempfile::tempdir().expect("a temporary folder");
        let folder = temp.path().join("Folder.v1");
        fs::create_dir(&folder).expect("a source folder");
        let mut taken = HashSet::new();
        taken.insert(OsString::from("Folder.v1"));

        let name = keep_both_name(&GioNode::new(&uri_of(&folder)), &taken);

        assert_eq!(name, Some(OsString::from("Folder.v1 (copy 2)")));
    }
}
