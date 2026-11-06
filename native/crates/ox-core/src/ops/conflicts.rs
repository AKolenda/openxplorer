// SPDX-License-Identifier: AGPL-3.0-only
//! The name-conflict check before a paste or drop (OPS-026, OPS-027), as
//! Windows Explorer makes it.
//!
//! The Python app asked once about the top-level items whose names were
//! taken (`transferConflicts` in `dispatch` of `v2.0.0:desktop/winspace.py`).
//! Windows instead merges a folder into the folder of its name without
//! asking (Folder Options' "Hide folder merge conflicts" is on by
//! default) and asks only about the files with taken names, inside merged
//! folders too: "The destination has 3 files with the same names". So the
//! check looks inside each folder that meets a folder of its name, and
//! the plan it returns says where every item goes: free items into their
//! folder with the Skip policy, so a name that appears after the check is
//! still never overwritten, a copy into the item's own folder as `name -
//! Copy` without asking, and each conflict as the user answers it
//! ([`super::resolved_transfer`]). A file meeting a folder's name, or a
//! folder a file's, is a conflict, never a merge.

use super::context::{on_worker, unless_cancelled};
use super::error::OpsError;
use crate::gio_node::GioNode;
use crate::location::{normalise, parent_location, require_item_uri, same_location};
use crate::transfer::{Cancellation, Node, NodeKind, TransferMode, MAX_DEPTH, MAX_ITEMS};

/// What the File Conflict window shows of one side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemFacts {
    /// A folder rather than a file.
    pub is_folder: bool,
    /// A file's size in bytes.
    pub size: Option<u64>,
    /// Last modified, in seconds since the Unix epoch.
    pub modified: Option<u64>,
}

/// One incoming item whose name is taken in the folder it goes into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameConflict {
    /// The incoming item.
    pub incoming_uri: String,
    /// The item of that name already in the destination.
    pub existing_uri: String,
    /// The incoming item's type, size and date.
    pub incoming: ItemFacts,
    /// The existing item's.
    pub existing: ItemFacts,
}

impl NameConflict {
    /// True when both are files of the same size and modification time,
    /// the ones "Skip N files with the same date and size" skips.
    pub fn same_date_and_size(&self) -> bool {
        let (incoming, existing) = (self.incoming, self.existing);
        !incoming.is_folder
            && !existing.is_folder
            && incoming.size.is_some()
            && incoming.modified.is_some()
            && incoming.size == existing.size
            && incoming.modified == existing.modified
    }

    /// The name both items have, as the destination has it: a Recycle
    /// Bin item's own URI need not end in the name it is restored under.
    pub fn name(&self) -> String {
        GioNode::new(&self.existing_uri).display_name()
    }
}

/// How one item of the plan goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedKind {
    /// Its name is free: it goes in with Skip.
    Free,
    /// A copy into the folder it is in: `name - Copy`, without asking.
    OwnFolderCopy,
    /// Its name is taken: the conflict of this index decides.
    Conflict(usize),
}

/// One item of the plan and the folder it goes into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    /// The item.
    pub uri: String,
    /// The folder it goes into: the destination, or a merged folder in it.
    pub destination_folder: String,
    /// How it goes in.
    pub kind: PlannedKind,
}

/// The plan of a copy or move and its conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictScan {
    /// Copy or move.
    pub mode: TransferMode,
    /// The folder the user chose.
    pub destination_folder: String,
    /// The items the user chose, in order.
    pub selection: Vec<String>,
    /// Every item that goes somewhere, in the order of the selection,
    /// each merged folder's contents by name in its place.
    pub items: Vec<PlannedItem>,
    /// The items whose names are taken, in the same order.
    pub conflicts: Vec<NameConflict>,
    /// The incoming folders merged into a folder of their name, the
    /// deepest first. A move removes each once it is empty.
    pub merged_folders: Vec<String>,
}

impl ConflictScan {
    /// The conflicts the user is asked about.
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// The plan of copying or moving (`mode`) `uris` into the folder at
/// `destination_folder`, with every name conflict, inside merged folders
/// too. Hidden items and dangling links count: their names are taken too.
///
/// # Errors
///
/// No items or more than [`MAX_ITEMS`], a share or device root among them,
/// a destination that is not a folder ("Open a destination folder before
/// pasting."), or the backend's failure. A cancelled check is always
/// [`OpsError::Cancelled`], never a shorter plan the interface would read
/// as "no conflicts" and start copying.
pub async fn scan_name_conflicts(
    mode: TransferMode,
    uris: &[String],
    destination_folder: &str,
    cancel: &Cancellation,
) -> Result<ConflictScan, OpsError> {
    let uris = uris.to_vec();
    let destination_folder = destination_folder.to_owned();
    let cancel = cancel.clone();
    on_worker(move || scan_blocking(mode, &uris, &destination_folder, &cancel)).await
}

/// [`scan_name_conflicts`] on the calling thread.
fn scan_blocking(
    mode: TransferMode,
    uris: &[String],
    destination_folder: &str,
    cancel: &Cancellation,
) -> Result<ConflictScan, OpsError> {
    if uris.is_empty() || uris.len() > MAX_ITEMS {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Select between 1 and 100,000 items.",
        )));
    }
    let items = uris
        .iter()
        .map(|uri| require_item_uri(uri))
        .collect::<Result<Vec<String>, _>>()?;
    let destination = normalise(destination_folder)?;
    let folder = GioNode::new(&destination);
    // Inspected without following a link, as the Python bridge does.
    if unless_cancelled(cancel, || folder.info(Some(cancel)))??.kind != NodeKind::Directory {
        return Err(OpsError::failed(crate::i18n::gettext(
            "Open a destination folder before pasting.",
        )));
    }
    let mut scan = ConflictScan {
        mode,
        destination_folder: destination.clone(),
        selection: items.clone(),
        items: Vec::new(),
        conflicts: Vec::new(),
        merged_folders: Vec::new(),
    };
    for uri in items {
        let own_folder = parent_location(&uri).is_some_and(|parent| same_location(&parent, &destination));
        if mode == TransferMode::Copy && own_folder {
            scan.items.push(PlannedItem {
                uri,
                destination_folder: destination.clone(),
                kind: PlannedKind::OwnFolderCopy,
            });
            continue;
        }
        let source = GioNode::new(&uri);
        Scanner {
            scan: &mut scan,
            cancel,
        }
        .place(&source, &folder, 0)?;
    }
    Ok(scan)
}

/// Walks the incoming items and the folders they merge into.
struct Scanner<'a> {
    scan: &'a mut ConflictScan,
    cancel: &'a Cancellation,
}

impl Scanner<'_> {
    /// Plans `source` going into `folder`, at merge `depth`.
    fn place(&mut self, source: &GioNode, folder: &GioNode, depth: usize) -> Result<(), OpsError> {
        let cancel = self.cancel;
        let existing = GioNode::new(&folder.child(&source.name()).uri());
        // OPS-027: a query the user cancelled answers "free", and "no
        // conflicts" starts the copy at once, so it must not count.
        let taken = unless_cancelled(cancel, || existing.exists(Some(cancel)))?;
        if !taken {
            self.free(source, folder);
            return Ok(());
        }
        let incoming = facts(source, cancel)?;
        let present = facts(&existing, cancel)?;
        // The folder copied into itself or a folder inside it is left to
        // the engine, which refuses it with its own message.
        let inside_itself = existing.uri() == source.uri() || is_inside(&existing.uri(), &source.uri());
        if incoming.is_folder && present.is_folder && depth < MAX_DEPTH && !inside_itself {
            let mut children = unless_cancelled(cancel, || source.children(Some(cancel)))??;
            children.sort_by_key(|child| child.display_name().to_lowercase());
            for child in children {
                self.place(&GioNode::new(&child.uri()), &existing, depth + 1)?;
            }
            self.scan.merged_folders.push(source.uri());
            return Ok(());
        }
        if inside_itself {
            self.free(source, folder);
            return Ok(());
        }
        let index = self.scan.conflicts.len();
        self.scan.conflicts.push(NameConflict {
            incoming_uri: source.uri(),
            existing_uri: existing.uri(),
            incoming,
            existing: present,
        });
        self.scan.items.push(PlannedItem {
            uri: source.uri(),
            destination_folder: folder.uri(),
            kind: PlannedKind::Conflict(index),
        });
        Ok(())
    }

    /// Plans `source` going into `folder` under its free name.
    fn free(&mut self, source: &GioNode, folder: &GioNode) {
        self.scan.items.push(PlannedItem {
            uri: source.uri(),
            destination_folder: folder.uri(),
            kind: PlannedKind::Free,
        });
    }
}

/// True when `uri` is inside the folder at `folder`.
fn is_inside(uri: &str, folder: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    uri.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// The type, size and date of `node`, read without following a link.
pub(super) fn facts(node: &GioNode, cancel: &Cancellation) -> Result<ItemFacts, OpsError> {
    let info = unless_cancelled(cancel, || node.info(Some(cancel)))??;
    let is_folder = info.kind == NodeKind::Directory;
    let modified = info
        .modified
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs());
    Ok(ItemFacts {
        is_folder,
        size: (!is_folder).then_some(info.size),
        modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64, modified: u64) -> ItemFacts {
        ItemFacts {
            is_folder: false,
            size: Some(size),
            modified: Some(modified),
        }
    }

    fn conflict(incoming: ItemFacts, existing: ItemFacts) -> NameConflict {
        NameConflict {
            incoming_uri: "file:///tmp/in/report.png".into(),
            existing_uri: "file:///tmp/out/report.png".into(),
            incoming,
            existing,
        }
    }

    /// parity: OPS-028
    #[test]
    fn only_two_files_alike_in_date_and_size_count_as_the_same() {
        let folder = ItemFacts {
            is_folder: true,
            size: None,
            modified: Some(5),
        };
        assert!(conflict(file(4, 5), file(4, 5)).same_date_and_size());
        assert!(!conflict(file(4, 5), file(4, 6)).same_date_and_size());
        assert!(!conflict(file(4, 5), file(3, 5)).same_date_and_size());
        assert!(!conflict(folder, folder).same_date_and_size());
        assert!(!conflict(ItemFacts::default(), ItemFacts::default()).same_date_and_size());
        assert_eq!(conflict(file(1, 1), file(1, 1)).name(), "report.png");
    }

    #[test]
    fn inside_means_below_the_folder_not_beside_it() {
        assert!(is_inside("file:///a/Photos/Photos", "file:///a/Photos"));
        assert!(!is_inside("file:///a/Photos 2", "file:///a/Photos"));
        assert!(!is_inside("file:///a/Photos", "file:///a/Photos"));
    }
}
