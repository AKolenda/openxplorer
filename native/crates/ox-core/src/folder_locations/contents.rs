// SPDX-License-Identifier: AGPL-3.0-only
//! Which items of a relocated standard folder may follow it.
//!
//! Relocating a folder never moves files ([`FolderRelocation::apply`]).
//! Afterwards the Location tab offers, as Windows 11 does, to move the
//! old folder's contents into the new one; the move runs through the
//! transfer engine with its name-conflict check, so nothing is ever
//! overwritten without an answer. This module only decides what that move
//! may take. Safety rule "never take more than the folder's own files":
//!
//! - nothing when the old folder is the new one (through symlinks), does
//!   not exist, is the home folder, `/` or a folder holding home, or is
//!   also another standard folder's location;
//! - never the new folder itself or an item holding it;
//! - never an item that is or holds another standard folder.
//!
//! [`FolderRelocation::apply`]: super::FolderRelocation::apply

use std::fs;
use std::path::{Path, PathBuf};

/// The folders a move of a folder's contents must leave alone.
#[derive(Debug, Clone, Copy)]
pub(super) struct KeptFolders<'a> {
    /// The user's home folder.
    pub home: &'a Path,
    /// Where the other standard folders are.
    pub others: &'a [PathBuf],
}

/// The items of `previous` that may move into `destination`, sorted by
/// path; empty when nothing may move.
pub(super) fn movable_contents(previous: &Path, destination: &Path, kept: KeptFolders<'_>) -> Vec<PathBuf> {
    let Ok(previous) = fs::canonicalize(previous) else {
        return Vec::new();
    };
    let destination = resolved(destination);
    let home = resolved(kept.home);
    let others: Vec<PathBuf> = kept.others.iter().map(|path| resolved(path)).collect();
    let is_whole_tree = previous == Path::new("/") || home.starts_with(&previous);
    let is_shared = others.contains(&previous);
    if previous == destination || is_whole_tree || is_shared || !previous.is_dir() {
        return Vec::new();
    }
    let Ok(children) = fs::read_dir(&previous) else {
        return Vec::new();
    };
    let holds = |item: &Path, folder: &Path| folder.starts_with(item);
    let mut items: Vec<PathBuf> = children
        .filter_map(Result::ok)
        .map(|child| previous.join(child.file_name()))
        .filter(|item| !holds(item, &destination) && !others.iter().any(|other| holds(item, other)))
        .collect();
    items.sort();
    items
}

/// `path` with its symlinks resolved, or as given when it does not exist.
fn resolved(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    /// A home folder with a Documents folder holding two items.
    struct Fixture {
        _root: tempfile::TempDir,
        home: PathBuf,
        documents: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("a temporary folder");
            let home = fs::canonicalize(root.path()).expect("it resolves").join("home");
            let documents = home.join("Documents");
            fs::create_dir_all(documents.join("Taxes")).expect("the folders are created");
            fs::write(documents.join("plan.txt"), b"plan").expect("the file is written");
            Self {
                _root: root,
                home,
                documents,
            }
        }

        fn kept<'a>(&'a self, others: &'a [PathBuf]) -> KeptFolders<'a> {
            KeptFolders {
                home: &self.home,
                others,
            }
        }

        fn folder(&self, name: &str) -> PathBuf {
            let path = self.home.join(name);
            fs::create_dir_all(&path).expect("the folder is created");
            path
        }
    }

    /// parity: PROP-017
    #[test]
    fn the_old_folders_items_may_follow_it_but_never_the_new_folder_or_another_standard_folder() {
        let fixture = Fixture::new();
        let destination = fixture.folder("Documents/Archive/New");
        let pictures = fixture.folder("Documents/Photos");
        let elsewhere = fixture.folder("Elsewhere");

        let inside = movable_contents(&fixture.documents, &destination, fixture.kept(&[pictures]));
        let beside = movable_contents(&fixture.documents, &elsewhere, fixture.kept(&[]));

        assert_eq!(
            inside,
            [
                fixture.documents.join("Taxes"),
                fixture.documents.join("plan.txt")
            ]
        );
        assert_eq!(beside.len(), 4, "every item may follow to a separate folder");
    }

    /// parity: PROP-017
    #[test]
    fn nothing_follows_from_home_a_shared_folder_or_the_same_folder_through_a_link() {
        let fixture = Fixture::new();
        let destination = fixture.folder("New");
        let link = fixture.home.join("Linked");
        symlink(&destination, &link).expect("the link is created");

        let from_home = movable_contents(&fixture.home, &destination, fixture.kept(&[]));
        let from_root = movable_contents(Path::new("/"), &destination, fixture.kept(&[]));
        let shared = movable_contents(
            &fixture.documents,
            &destination,
            fixture.kept(std::slice::from_ref(&fixture.documents)),
        );
        let same = movable_contents(&link, &destination, fixture.kept(&[]));
        let missing = movable_contents(&fixture.home.join("Gone"), &destination, fixture.kept(&[]));

        for items in [from_home, from_root, shared, same, missing] {
            assert!(items.is_empty(), "{items:?}");
        }
    }
}
