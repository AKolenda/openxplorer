// SPDX-License-Identifier: AGPL-3.0-only
//! The production GIO adapter ([`GioNode`]) on isolated temporary local
//! files: listing, copying, moving and publishing, each without following
//! links and without overwriting. Trash and permanent deletion are in
//! `gio_node_cases/removal.rs`. Device capabilities are inspected without
//! contacting device backends; the adapter on a simulated phone is tested in
//! `transfer_cases/mtp_adapter.rs`.

#[path = "gio_node_cases/removal.rs"]
mod removal;

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use ox_core::gio_node::GioNode;
use ox_core::transfer::{
    Cancellation, ConflictPolicy, Node, NodeKind, TransferEngine, TransferError, TransferMode,
};

/// The adapter for the local item at `path`.
fn node(path: &Path) -> GioNode {
    GioNode::from_file(gio::File::for_path(path))
}

/// An engine resolving every URI with [`GioNode`], without a write guard.
fn gio_engine() -> TransferEngine {
    TransferEngine::new(Arc::new(|uri: &str| {
        Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>)
    }))
}

/// The permission bits of `path`, following a link.
fn mode_of(path: &Path) -> u32 {
    let metadata = fs::metadata(path).unwrap();
    metadata.permissions().mode() & 0o7777
}

/// Sets the permission bits of `path`.
fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Port of the listing half of `test_enumeration_and_creation` in
/// `desktop/tests/gio_integration.py`.
///
/// parity: XFER-017
#[test]
fn metadata_and_listing_preserve_hidden_files_and_do_not_follow_links() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    set_mode(&folder, 0o750);
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
        .map(|child| child.display_name())
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

/// parity: XFER-009
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
            progress.push((current, total));
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

/// Port of the creation half of `test_enumeration_and_creation`: copies,
/// moves and new folders never take a name that exists.
///
/// parity: OPS-008, XFER-002
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

/// Publishing a staged copy never replaces an item that took the final name
/// first. On local disks the kernel's no-replace rename refuses it in the
/// same step that would rename (GIO's move checks the name first and renames
/// after, which leaves a window), so the refusal is the adapter's own
/// "Nothing was overwritten" message rather than GIO's.
///
/// parity: XFER-007
#[test]
fn publishing_refuses_a_taken_name_atomically_and_overwrites_nothing() {
    for kind in [NodeKind::File, NodeKind::Directory] {
        let temp = tempfile::tempdir().unwrap();
        let staged = temp.path().join("staged");
        if kind == NodeKind::Directory {
            fs::create_dir(&staged).unwrap();
        } else {
            fs::write(&staged, b"incoming").unwrap();
        }
        let taken = temp.path().join("report");
        fs::write(&taken, b"another program").unwrap();

        let published = node(&staged).publish(&node(&taken), Some(&Cancellation::new()));

        assert_eq!(
            published,
            Err(TransferError::Exists(
                "An item named “report” already exists. Nothing was overwritten.".into()
            )),
            "{kind:?}"
        );
        assert_eq!(fs::read(&taken).unwrap(), b"another program");
        assert!(staged.exists(), "{kind:?}");
    }
}

/// parity: XFER-001
#[test]
fn publishing_installs_a_staged_file_or_folder_under_a_free_name() {
    let temp = tempfile::tempdir().unwrap();
    let staged_file = temp.path().join("staged-file");
    let staged_folder = temp.path().join("staged-folder");
    fs::write(&staged_file, b"complete").unwrap();
    fs::create_dir(&staged_folder).unwrap();
    fs::write(staged_folder.join("inside"), b"inside").unwrap();
    let cancel = Cancellation::new();

    node(&staged_file)
        .publish(&node(&temp.path().join("file")), Some(&cancel))
        .unwrap();
    node(&staged_folder)
        .publish(&node(&temp.path().join("folder")), Some(&cancel))
        .unwrap();

    assert_eq!(fs::read(temp.path().join("file")).unwrap(), b"complete");
    assert_eq!(fs::read(temp.path().join("folder/inside")).unwrap(), b"inside");
    assert!(!staged_file.exists() && !staged_folder.exists());
}

/// parity: OPS-022
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

/// Linux names need not be UTF-8. The adapter lists, inspects and copies
/// them byte for byte; only labels replace invalid bytes.
#[test]
fn names_that_are_not_utf8_are_listed_and_copied_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let name = OsString::from_vec(b"caf\xe9.mp3".to_vec());
    let source = temp.path().join(&name);
    fs::write(&source, b"song").unwrap();
    let listed = node(temp.path()).children(None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name(), name);
    assert_eq!(listed[0].display_name(), "caf\u{fffd}.mp3");
    assert_eq!(listed[0].info(None).unwrap().kind, NodeKind::File);
    let copies = temp.path().join("copies");
    fs::create_dir(&copies).unwrap();
    let target = node(&copies).child(&name);
    listed[0]
        .copy_file(target.as_ref(), &Cancellation::new(), &mut |_, _| {})
        .unwrap();
    assert_eq!(fs::read(copies.join(&name)).unwrap(), b"song");
}

/// Port of `test_recursive_copy_preserves_link_and_source` in
/// `desktop/tests/gio_integration.py`.
///
/// parity: XFER-001, XFER-005, XFER-017
#[test]
fn complete_engine_stages_and_publishes_a_recursive_local_copy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("nested/data"), b"complete").unwrap();
    symlink("../missing", source.join("nested/link")).unwrap();
    set_mode(&source.join("nested"), 0o500);
    let result = gio_engine()
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
    assert_eq!(mode_of(&target.join("source/nested")), 0o500);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    // Restore owner access so the temporary folder can be removed.
    set_mode(&source.join("nested"), 0o700);
    set_mode(&target.join("source/nested"), 0o700);
}

/// parity: XFER-011
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

/// Ports `test_same_device_copies_are_detected` and
/// `test_device_schemes_request_sibling_staging` in
/// `desktop/tests/test_device_staging.py`: only MTP destinations stage
/// beside the final name (cameras on gphoto2 keep folder staging), and only
/// a copy within one MTP device keeps the source's name.
///
/// parity: XFER-021, XFER-023
#[test]
fn device_capabilities_and_unsupported_renames_are_resolved_without_device_io() {
    let source = GioNode::new("mtp://test-device/Internal/source/photo.jpg");
    let same_device = GioNode::new("mtp://test-device/Internal/destination");
    let other_device = GioNode::new("mtp://other-device/Internal/destination");
    let local = GioNode::new("file:///tmp/x");
    assert!(source.stage_as_sibling());
    for folder in ["gphoto2://cam/DCIM/x", "smb://host/share/x", "file:///tmp/x"] {
        assert!(!GioNode::new(folder).stage_as_sibling(), "{folder}");
    }
    assert!(source.native_copy_keeps_name(&same_device));
    assert!(!source.native_copy_keeps_name(&other_device));
    assert!(!source.native_copy_keeps_name(&local));
    assert!(!local.native_copy_keeps_name(&same_device));
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
