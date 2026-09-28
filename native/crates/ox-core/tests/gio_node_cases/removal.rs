// SPDX-License-Identifier: AGPL-3.0-only
//! Trash and permanent deletion through the production GIO adapter on
//! temporary local files: links are removed as links, protected items and
//! swapped folders stop the deletion, and roots and whole shares are never
//! changed.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gio::prelude::VfsExt;
use ox_core::gio_node::GioNode;
use ox_core::transfer::{Cancellation, Node, Operation, TransferError};

use super::shared::{gio_engine, temporary_folder};
use super::{node, LinkedFolder};

/// parity: XFER-015, XFER-017
#[test]
fn deleting_a_copied_link_keeps_its_target() {
    let root = temporary_folder();
    let linked = LinkedFolder::create(root.path());
    let copied = root.path().join("copied");
    let cancel = Cancellation::new();
    node(&linked.link)
        .copy_file(&node(&copied), &cancel, &mut |_, _| {})
        .expect("the adapter copies the file");

    let result = node(&copied).delete_tree(&cancel, None);

    assert_eq!(result, Ok(()));
    assert!(fs::symlink_metadata(&copied).is_err());
    linked.assert_untouched();
}

/// parity: XFER-015, XFER-017
#[test]
fn local_recursive_delete_removes_selected_tree_without_following_its_links() {
    let root = temporary_folder();
    let selected = root.path().join("selected");
    fs::create_dir_all(selected.join("nested")).expect("the fixture folders are created");
    fs::write(selected.join("nested/data"), b"delete me").expect("the fixture file is written");
    let outside = root.path().join("outside");
    fs::write(&outside, b"retained").expect("the fixture file is written");
    symlink(&outside, selected.join("link")).expect("the fixture link is created");

    let result = node(&selected).delete_tree(&Cancellation::new(), None);

    assert_eq!(result, Ok(()));
    assert!(!selected.exists());
    assert_eq!(fs::read(outside).expect("the file can be read"), b"retained");
}

/// A folder opened through a symbolic link (`~/Music` pointing at a data
/// drive) is where the user deletes: the items inside the real folder go,
/// the link and the real folder's other items stay. The Python app deletes
/// these items too.
///
/// parity: XFER-015
#[test]
fn local_delete_works_inside_a_folder_reached_through_a_symbolic_link() {
    let root = temporary_folder();
    let actual = root.path().join("actual");
    fs::create_dir_all(actual.join("sub/nested")).expect("the fixture folders are created");
    fs::write(actual.join("data"), b"delete me").expect("the fixture file is written");
    fs::write(actual.join("sub/nested/deep"), b"delete me too").expect("the fixture file is written");
    fs::write(actual.join("kept"), b"retained").expect("the fixture file is written");
    let alias = root.path().join("alias");
    symlink(&actual, &alias).expect("the fixture link is created");
    let cancel = Cancellation::new();

    let file_deleted = node(&alias.join("data")).delete_tree(&cancel, None);
    let folder_deleted = node(&alias.join("sub")).delete_tree(&cancel, None);

    assert_eq!(file_deleted, Ok(()));
    assert_eq!(folder_deleted, Ok(()));
    assert!(!actual.join("data").exists());
    assert!(!actual.join("sub").exists());
    assert_eq!(
        fs::read(actual.join("kept")).expect("the file can be read"),
        b"retained"
    );
    assert!(fs::symlink_metadata(&alias)
        .expect("the link still exists")
        .file_type()
        .is_symlink());
}

/// Which folder another program swaps for a symbolic link mid-deletion.
#[derive(Debug, Clone, Copy)]
enum SwappedFolder {
    /// The folder the user selected.
    Selected,
    /// A folder inside it.
    Nested,
}

/// A selected folder holding `nested/victim`, and an outside folder with
/// the same layout whose files must survive the deletion.
struct SwapTree {
    selected: PathBuf,
    nested: PathBuf,
    outside: PathBuf,
}

impl SwapTree {
    /// Creates the tree in `root`.
    fn create(root: &Path) -> Self {
        let selected = root.join("selected");
        let nested = selected.join("nested");
        let outside = root.join("outside");
        fs::create_dir_all(&nested).expect("the fixture folders are created");
        fs::write(nested.join("victim"), b"selected content").expect("the fixture file is written");
        fs::create_dir_all(outside.join("nested")).expect("the fixture folders are created");
        fs::write(outside.join("victim"), b"must survive").expect("the fixture file is written");
        fs::write(outside.join("nested/victim"), b"must also survive").expect("the fixture file is written");
        Self {
            selected,
            nested,
            outside,
        }
    }

    /// The folder that `swapped_folder` names.
    fn folder(&self, swapped_folder: SwappedFolder) -> PathBuf {
        match swapped_folder {
            SwappedFolder::Selected => self.selected.clone(),
            SwappedFolder::Nested => self.nested.clone(),
        }
    }
}

/// parity: XFER-015
#[test]
fn an_ancestor_swapped_for_a_symlink_cannot_redirect_recursive_deletion() {
    for swapped_folder in [SwappedFolder::Nested, SwappedFolder::Selected] {
        let root = temporary_folder();
        let tree = SwapTree::create(root.path());
        let swapped = Arc::new(AtomicBool::new(false));
        let did_swap = swapped.clone();
        let ancestor = tree.folder(swapped_folder);
        let detached = root.path().join("detached");
        let outside_for_guard = tree.outside.clone();
        let guard = move |uri: &str| {
            let first_victim = uri.ends_with("/victim") && !did_swap.swap(true, Ordering::SeqCst);
            if first_victim {
                fs::rename(&ancestor, &detached)?;
                symlink(&outside_for_guard, &ancestor)?;
            }
            Ok(())
        };

        let result = node(&tree.selected).delete_tree(&Cancellation::new(), Some(&guard));

        assert!(swapped.load(Ordering::SeqCst));
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("changed during deletion"));
        assert_eq!(
            fs::read(tree.outside.join("victim")).expect("the file can be read"),
            b"must survive"
        );
        assert_eq!(
            fs::read(tree.outside.join("nested/victim")).expect("the file can be read"),
            b"must also survive"
        );
    }
}

/// parity: XFER-015, XFER-020
#[test]
fn protected_descendants_are_preflighted_before_gio_permanent_deletion() {
    let root = temporary_folder();
    let source = root.path().join("source");
    fs::create_dir_all(source.join("protected")).expect("the fixture folders are created");
    fs::write(source.join("a"), b"live").expect("the fixture file is written");
    fs::write(source.join("protected/data"), b"snapshot").expect("the fixture file is written");
    let protected = node(&source.join("protected")).uri();
    let mut engine = gio_engine().with_write_guard(move |uri| {
        if uri.starts_with(&protected) {
            Err(TransferError::failed("Protected snapshot."))
        } else {
            Ok(())
        }
    });

    let result = engine
        .run(Operation::Delete, &[node(&source).uri()], &Cancellation::new())
        .expect("the engine accepts the request");

    assert!(result.done.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(fs::read(source.join("a")).expect("the file can be read"), b"live");
    assert_eq!(
        fs::read(source.join("protected/data")).expect("the file can be read"),
        b"snapshot"
    );
}

/// parity: OPS-022, XFER-015
#[test]
fn cancellation_during_local_delete_preflight_preserves_the_file() {
    let root = temporary_folder();
    let source = root.path().join("source");
    fs::write(&source, b"retained").expect("the fixture file is written");
    let cancel = Cancellation::new();
    let requested = cancel.clone();
    let guard = move |_uri: &str| {
        requested.cancel();
        Ok(())
    };

    let result = node(&source).delete_tree(&cancel, Some(&guard));

    assert_eq!(result, Err(TransferError::Cancelled));
    assert_eq!(fs::read(source).expect("the file can be read"), b"retained");
}

/// Like `can_trash` in `desktop/gio_backend.py`: a share unmounted in the
/// background is reported, so the caller mounts it and asks again instead
/// of offering a permanent delete. Any other failure means "no Trash".
#[test]
fn trash_support_reports_an_unmounted_share_instead_of_denying_trash() {
    let schemes = gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "smb"),
        "this check needs GVfs with its SMB backend (gvfs-backends); found {schemes:?}"
    );
    let root = temporary_folder();
    let missing = root.path().join("missing");

    let unmounted = GioNode::new("smb://example.invalid/share/folder").can_trash(None);
    let local_missing = node(&missing).can_trash(None);

    assert!(
        matches!(unmounted, Err(TransferError::NotMounted(_))),
        "{unmounted:?}"
    );
    assert_eq!(local_missing, Ok(false));
}

/// parity: OPS-035, XFER-019
#[test]
fn roots_and_whole_network_shares_are_refused_before_mutation() {
    let cancel = Cancellation::new();
    let unused_target = GioNode::new("file:///tmp/unused");
    for uri in ["file:///", "smb://example.invalid/Shared", "mtp://test-device/"] {
        let root = GioNode::new(uri);

        let deleted = root.delete_tree(&cancel, None);
        let trashed = root.trash(&cancel);
        let moved = root.move_native(&unused_target, Some(&cancel));

        assert!(deleted.is_err(), "{uri}");
        assert!(trashed.is_err(), "{uri}");
        assert!(moved.is_err(), "{uri}");
    }
}
