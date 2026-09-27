// SPDX-License-Identifier: AGPL-3.0-only
//! Production GIO adapter checks on isolated temporary local files.
//! Device capabilities are inspected without contacting device backends.

use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use ox_core::gio_node::GioNode;
use ox_core::transfer::{
    Cancellation, ConflictPolicy, Node, NodeKind, TransferEngine, TransferError, TransferMode,
};

fn node(path: &Path) -> GioNode {
    GioNode::from_file(gio::File::for_path(path))
}

fn engine() -> TransferEngine {
    TransferEngine::new(Arc::new(|uri| Ok(Box::new(GioNode::new(uri)))))
}

#[test]
fn metadata_and_listing_preserve_hidden_files_and_do_not_follow_links() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o750)).unwrap();
    fs::write(folder.join(".hidden"), b"hello").unwrap();
    symlink("missing", folder.join("dangling")).unwrap();
    symlink(&folder, temp.path().join("alias")).unwrap();
    let directory = node(&folder);
    assert_eq!(directory.info(None).unwrap().kind, NodeKind::Directory);
    assert_eq!(directory.info(None).unwrap().mode, Some(0o750));
    let mut names: Vec<String> = directory
        .children(None)
        .unwrap()
        .iter()
        .map(|child| child.name())
        .collect();
    names.sort();
    assert_eq!(names, [".hidden", "dangling"]);
    let link = node(&folder.join("dangling"));
    assert!(link.exists(None));
    assert_eq!(link.info(None).unwrap().kind, NodeKind::Symlink);
    let alias = node(&temp.path().join("alias"));
    assert!(alias.is_directory(None).unwrap());
    assert!(alias.children(None).is_err());
}

#[test]
fn local_copy_move_and_explicit_replace_use_the_gio_adapter() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("copy");
    let final_path = temp.path().join("final");
    fs::write(&source, b"complete content").unwrap();
    let cancel = Cancellation::new();
    let mut progress = Vec::new();
    node(&source)
        .copy_file(&node(&target), &cancel, &mut |current, total| {
            progress.push((current, total))
        })
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"complete content");
    assert_eq!(fs::read(&source).unwrap(), b"complete content");
    node(&target)
        .move_native(&node(&final_path), Some(&cancel))
        .unwrap();
    assert!(!target.exists());
    fs::write(&target, b"replacement").unwrap();
    node(&target)
        .replace_native(&node(&final_path), Some(&cancel))
        .unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(&final_path).unwrap(), b"replacement");
    assert!(progress.iter().all(|(current, total)| current <= total));
}

#[test]
fn exclusive_copy_move_and_mkdir_preserve_existing_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"incoming").unwrap();
    fs::write(&target, b"retained").unwrap();
    let cancel = Cancellation::new();
    let source_node = node(&source);
    let target_node = node(&target);
    assert!(matches!(
        source_node.copy_file(&target_node, &cancel, &mut |_, _| {}),
        Err(TransferError::Exists(_))
    ));
    assert!(matches!(
        source_node.move_native(&target_node, Some(&cancel)),
        Err(TransferError::Exists(_))
    ));
    assert!(matches!(target_node.mkdir(None), Err(TransferError::Exists(_))));
    assert_eq!(fs::read(source).unwrap(), b"incoming");
    assert_eq!(fs::read(target).unwrap(), b"retained");
}

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

#[test]
fn cancellation_prevents_copy_move_and_recursive_delete() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"untouched").unwrap();
    let cancel = Cancellation::new();
    cancel.cancel();
    assert_eq!(
        node(&source).copy_file(&node(&target), &cancel, &mut |_, _| {}),
        Err(TransferError::Cancelled)
    );
    assert_eq!(
        node(&source).move_native(&node(&target), Some(&cancel)),
        Err(TransferError::Cancelled)
    );
    assert_eq!(
        node(&source).delete_tree(&cancel, None),
        Err(TransferError::Cancelled)
    );
    assert_eq!(fs::read(source).unwrap(), b"untouched");
    assert!(!target.exists());
}

#[test]
fn names_with_invalid_utf8_are_refused_before_they_can_be_changed() {
    let temp = tempfile::tempdir().unwrap();
    let name = std::ffi::OsString::from_vec(vec![b'a', 0xff]);
    let source = temp.path().join(name);
    fs::write(&source, b"untouched").unwrap();
    assert!(node(&source)
        .info(None)
        .unwrap_err()
        .to_string()
        .contains("UTF-8"));
    assert!(node(temp.path()).children(None).is_err());
    assert_eq!(fs::read(source).unwrap(), b"untouched");
}

#[test]
fn complete_engine_stages_and_publishes_a_recursive_local_copy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("nested/data"), b"complete").unwrap();
    symlink("../missing", source.join("nested/link")).unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o500)).unwrap();
    let result = engine()
        .run(
            TransferMode::Copy,
            &[node(&source).uri()],
            Some(&node(&target).uri()),
            ConflictPolicy::Skip,
            &Cancellation::new(),
        )
        .unwrap();
    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [node(&source).uri()]);
    assert_eq!(fs::read(target.join("source/nested/data")).unwrap(), b"complete");
    assert_eq!(
        fs::read_link(target.join("source/nested/link")).unwrap(),
        Path::new("../missing")
    );
    assert_eq!(
        fs::metadata(target.join("source/nested"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o500
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(target.join("source/nested"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn protected_descendants_are_preflighted_before_gio_permanent_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("protected")).unwrap();
    fs::write(source.join("a"), b"live").unwrap();
    fs::write(source.join("protected/data"), b"snapshot").unwrap();
    let protected = node(&source.join("protected")).uri();
    let mut engine = engine().with_write_guard(move |uri| {
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

#[test]
fn cross_filesystem_move_is_refused_without_copying_or_removing_the_source() {
    let source_temp = tempfile::tempdir_in("/tmp").unwrap();
    let target_temp = tempfile::tempdir_in("/dev/shm").unwrap();
    assert_ne!(
        fs::metadata(source_temp.path()).unwrap().dev(),
        fs::metadata(target_temp.path()).unwrap().dev(),
        "this Linux integration check needs separate tmp and shm filesystems"
    );
    let source = source_temp.path().join("source");
    let target = target_temp.path().join("source");
    fs::write(&source, b"original").unwrap();
    let result = node(&source).move_native(&node(&target), Some(&Cancellation::new()));
    assert!(
        matches!(result, Err(TransferError::NotSupported(_))),
        "{result:?}"
    );
    assert_eq!(fs::read(source).unwrap(), b"original");
    assert!(!target.exists());
}

#[test]
fn device_capabilities_and_unsupported_renames_are_resolved_without_device_io() {
    let source = GioNode::new("mtp://test-device/Internal/source/photo.jpg");
    let same_device = GioNode::new("mtp://test-device/Internal/destination");
    let other_device = GioNode::new("mtp://other-device/Internal/destination");
    assert!(source.stage_as_sibling());
    assert!(source.native_copy_keeps_name(&same_device));
    assert!(!source.native_copy_keeps_name(&other_device));
    assert!(!source.native_copy_keeps_name(&GioNode::new("file:///tmp")));
    let renamed = GioNode::new("mtp://test-device/Internal/destination/other.jpg");
    assert!(source
        .move_native(&renamed, None)
        .unwrap_err()
        .to_string()
        .contains("not both"));
    assert!(matches!(
        source.replace_native(&renamed, None),
        Err(TransferError::ReplaceUnsupported(_))
    ));
}

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

#[test]
fn an_ancestor_swapped_for_a_symlink_cannot_redirect_recursive_deletion() {
    for replace_root in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let selected = temp.path().join("selected");
        let nested = selected.join("nested");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(nested.join("victim"), b"selected content").unwrap();
        fs::write(outside.join("victim"), b"must survive").unwrap();
        fs::create_dir(outside.join("nested")).unwrap();
        fs::write(outside.join("nested/victim"), b"must also survive").unwrap();
        let swapped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let did_swap = swapped.clone();
        let ancestor = if replace_root { selected.clone() } else { nested };
        let detached = temp.path().join("detached");
        let outside_for_guard = outside.clone();
        let guard = move |uri: &str| {
            if uri.ends_with("/victim") && !did_swap.swap(true, std::sync::atomic::Ordering::SeqCst) {
                fs::rename(&ancestor, &detached)?;
                symlink(&outside_for_guard, &ancestor)?;
            }
            Ok(())
        };
        let result = node(&selected).delete_tree(&Cancellation::new(), Some(&guard));
        assert!(swapped.load(std::sync::atomic::Ordering::SeqCst));
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("changed during deletion"));
        assert_eq!(fs::read(outside.join("victim")).unwrap(), b"must survive");
        assert_eq!(
            fs::read(outside.join("nested/victim")).unwrap(),
            b"must also survive"
        );
    }
}

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

#[test]
fn local_delete_refuses_a_symbolic_link_in_the_selected_items_parent_path() {
    let temp = tempfile::tempdir().unwrap();
    let actual = temp.path().join("actual");
    fs::create_dir(&actual).unwrap();
    fs::write(actual.join("data"), b"retained").unwrap();
    let alias = temp.path().join("alias");
    symlink(&actual, &alias).unwrap();
    assert!(node(&alias.join("data"))
        .delete_tree(&Cancellation::new(), None)
        .is_err());
    assert_eq!(fs::read(actual.join("data")).unwrap(), b"retained");
}

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
