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
use ox_core::transfer::{Cancellation, ConflictPolicy, Node, TransferError, TransferMode};

use super::{gio_engine, node};

/// parity: XFER-015, XFER-017
#[test]
fn copying_and_deleting_symbolic_links_never_traverses_the_target() {
    let temp = tempfile::tempdir().unwrap();
    let kept = temp.path().join("kept");
    fs::create_dir(&kept).unwrap();
    fs::write(kept.join("data"), b"retained").unwrap();
    let link = temp.path().join("link");
    let copied = temp.path().join("copied");
    symlink(&kept, &link).unwrap();
    let cancel = Cancellation::new();
    node(&link)
        .copy_file(&node(&copied), &cancel, &mut |_, _| {})
        .unwrap();
    assert_eq!(fs::read_link(&copied).unwrap(), kept);
    node(&copied).delete_tree(&cancel, None).unwrap();
    assert!(fs::symlink_metadata(&copied).is_err());
    assert_eq!(fs::read(kept.join("data")).unwrap(), b"retained");
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

/// parity: XFER-015, XFER-017
#[test]
fn local_recursive_delete_removes_selected_tree_without_following_its_links() {
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("selected");
    fs::create_dir_all(selected.join("nested")).unwrap();
    fs::write(selected.join("nested/data"), b"delete me").unwrap();
    let outside = temp.path().join("outside");
    fs::write(&outside, b"retained").unwrap();
    symlink(&outside, selected.join("link")).unwrap();
    node(&selected).delete_tree(&Cancellation::new(), None).unwrap();
    assert!(!selected.exists());
    assert_eq!(fs::read(outside).unwrap(), b"retained");
}

/// A folder opened through a symbolic link (`~/Music` pointing at a data
/// drive) is where the user deletes: the items inside the real folder go,
/// the link and the real folder's other items stay. The Python app deletes
/// these items too.
///
/// parity: XFER-015
#[test]
fn local_delete_works_inside_a_folder_reached_through_a_symbolic_link() {
    let temp = tempfile::tempdir().unwrap();
    let actual = temp.path().join("actual");
    fs::create_dir_all(actual.join("sub/nested")).unwrap();
    fs::write(actual.join("data"), b"delete me").unwrap();
    fs::write(actual.join("sub/nested/deep"), b"delete me too").unwrap();
    fs::write(actual.join("kept"), b"retained").unwrap();
    let alias = temp.path().join("alias");
    symlink(&actual, &alias).unwrap();
    let cancel = Cancellation::new();
    node(&alias.join("data")).delete_tree(&cancel, None).unwrap();
    node(&alias.join("sub")).delete_tree(&cancel, None).unwrap();
    assert!(!actual.join("data").exists());
    assert!(!actual.join("sub").exists());
    assert_eq!(fs::read(actual.join("kept")).unwrap(), b"retained");
    assert!(fs::symlink_metadata(&alias).unwrap().file_type().is_symlink());
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
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("victim"), b"selected content").unwrap();
        fs::create_dir_all(outside.join("nested")).unwrap();
        fs::write(outside.join("victim"), b"must survive").unwrap();
        fs::write(outside.join("nested/victim"), b"must also survive").unwrap();
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
        let temp = tempfile::tempdir().unwrap();
        let tree = SwapTree::create(temp.path());
        let swapped = Arc::new(AtomicBool::new(false));
        let did_swap = swapped.clone();
        let ancestor = tree.folder(swapped_folder);
        let detached = temp.path().join("detached");
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
        assert_eq!(fs::read(tree.outside.join("victim")).unwrap(), b"must survive");
        assert_eq!(
            fs::read(tree.outside.join("nested/victim")).unwrap(),
            b"must also survive"
        );
    }
}

/// parity: XFER-015, XFER-020
#[test]
fn protected_descendants_are_preflighted_before_gio_permanent_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("protected")).unwrap();
    fs::write(source.join("a"), b"live").unwrap();
    fs::write(source.join("protected/data"), b"snapshot").unwrap();
    let protected = node(&source.join("protected")).uri();
    let mut engine = gio_engine().with_write_guard(move |uri| {
        if uri.starts_with(&protected) {
            Err(TransferError::failed("Protected snapshot."))
        } else {
            Ok(())
        }
    });
    let result = engine
        .run(
            TransferMode::Delete,
            &[node(&source).uri()],
            None,
            ConflictPolicy::Skip,
            &Cancellation::new(),
        )
        .unwrap();
    assert!(result.done.is_empty());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(fs::read(source.join("a")).unwrap(), b"live");
    assert_eq!(fs::read(source.join("protected/data")).unwrap(), b"snapshot");
}

/// parity: OPS-022, XFER-015
#[test]
fn cancellation_during_local_delete_preflight_preserves_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::write(&source, b"retained").unwrap();
    let cancel = Cancellation::new();
    let requested = cancel.clone();
    let guard = move |_uri: &str| {
        requested.cancel();
        Ok(())
    };
    assert_eq!(
        node(&source).delete_tree(&cancel, Some(&guard)),
        Err(TransferError::Cancelled)
    );
    assert_eq!(fs::read(source).unwrap(), b"retained");
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
    let unmounted = GioNode::new("smb://example.invalid/share/folder").can_trash(None);
    assert!(
        matches!(unmounted, Err(TransferError::NotMounted(_))),
        "{unmounted:?}"
    );
    let missing = tempfile::tempdir().unwrap().path().join("missing");
    assert_eq!(node(&missing).can_trash(None), Ok(false));
}

/// parity: OPS-035, XFER-019
#[test]
fn roots_and_whole_network_shares_are_refused_before_mutation() {
    let cancel = Cancellation::new();
    for uri in ["file:///", "smb://example.invalid/Shared", "mtp://test-device/"] {
        let root = GioNode::new(uri);
        assert!(root.delete_tree(&cancel, None).is_err());
        assert!(root.trash(&cancel).is_err());
        assert!(root
            .move_native(&GioNode::new("file:///tmp/unused"), Some(&cancel))
            .is_err());
    }
}
