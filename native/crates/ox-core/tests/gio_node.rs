// SPDX-License-Identifier: AGPL-3.0-only
//! The production GIO adapter ([`GioNode`]) on isolated temporary local
//! files: listing, copying, moving and publishing, each without following
//! links and without overwriting. Trash and permanent deletion are in
//! `gio_node_cases/removal.rs`, and the device capabilities, inspected
//! without contacting device backends, in
//! `gio_node_cases/device_capabilities.rs`; the adapter on a simulated
//! phone is tested in `transfer_cases/mtp_adapter.rs`.

#[path = "gio_node_cases/device_capabilities.rs"]
mod device_capabilities;
#[path = "gio_node_cases/removal.rs"]
mod removal;
#[path = "transfer_support/shared.rs"]
mod shared;

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::{Path, PathBuf};

use ox_core::gio_node::GioNode;
use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, Operation, TransferError};

use shared::{gio_engine, mode_of, set_mode, RestoreOwnerAccess};

/// The adapter for the local item at `path`.
fn node(path: &Path) -> GioNode {
    GioNode::from_file(gio::File::for_path(path))
}

/// A folder `kept` holding `data`, and a symbolic link `link` to it.
struct LinkedFolder {
    kept: PathBuf,
    link: PathBuf,
}

impl LinkedFolder {
    /// Creates the folder and the link in `root`.
    fn create(root: &Path) -> Self {
        let kept = root.join("kept");
        fs::create_dir(&kept).unwrap();
        fs::write(kept.join("data"), b"retained").unwrap();
        let link = root.join("link");
        symlink(&kept, &link).unwrap();
        Self { kept, link }
    }

    /// Asserts the folder kept its data and the link is still a link.
    fn assert_untouched(&self) {
        assert_eq!(fs::read(self.kept.join("data")).unwrap(), b"retained");
        assert!(fs::symlink_metadata(&self.link).unwrap().file_type().is_symlink());
    }
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
    let link = node(&folder.join("dangling"));
    let alias = node(&temp.path().join("alias"));

    let folder_info = node(&folder).info(None).unwrap();
    let children = node(&folder).children(None).unwrap();
    let link_exists = link.exists(None);
    let link_info = link.info(None).unwrap();
    let alias_is_directory = alias.is_directory(None).unwrap();
    let alias_listing = alias.children(None);

    assert_eq!(folder_info.kind, NodeKind::Directory);
    assert_eq!(folder_info.mode, Some(0o750));
    let mut names: Vec<String> = children.iter().map(|child| child.display_name()).collect();
    names.sort();
    assert_eq!(names, [".hidden", "dangling"]);
    assert!(link_exists);
    assert_eq!(link_info.kind, NodeKind::Symlink);
    assert!(alias_is_directory);
    assert!(alias_listing.is_err());
}

/// The adapter's file copy, which the engine uses to fill its staging.
#[test]
fn copy_file_writes_the_complete_content_and_keeps_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("copy");
    fs::write(&source, b"complete content").unwrap();
    let mut progress = Vec::new();

    let copied = node(&source).copy_file(&node(&target), &Cancellation::new(), &mut |current, total| {
        progress.push((current, total));
    });

    assert_eq!(copied, Ok(()));
    assert_eq!(fs::read(&target).unwrap(), b"complete content");
    assert_eq!(fs::read(&source).unwrap(), b"complete content");
    assert!(progress.iter().all(|(current, total)| current <= total));
}

/// parity: XFER-015, XFER-017
#[test]
fn copying_a_link_copies_the_link_not_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let linked = LinkedFolder::create(temp.path());
    let copied = temp.path().join("copied");

    let result = node(&linked.link).copy_file(&node(&copied), &Cancellation::new(), &mut |_, _| {});

    assert_eq!(result, Ok(()));
    assert_eq!(fs::read_link(&copied).unwrap(), linked.kept);
    linked.assert_untouched();
}

/// The adapter's native move on one filesystem.
#[test]
fn move_native_leaves_nothing_under_the_old_name() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("copy");
    let final_path = temp.path().join("final");
    fs::write(&source, b"complete content").unwrap();

    let moved = node(&source).move_native(&node(&final_path), Some(&Cancellation::new()));

    assert_eq!(moved, Ok(()));
    assert!(!source.exists());
    assert_eq!(fs::read(&final_path).unwrap(), b"complete content");
}

/// parity: XFER-009
#[test]
fn replace_native_overwrites_the_existing_file() {
    let temp = tempfile::tempdir().unwrap();
    let replacement = temp.path().join("copy");
    let final_path = temp.path().join("final");
    fs::write(&replacement, b"replacement").unwrap();
    fs::write(&final_path, b"complete content").unwrap();

    let replaced = node(&replacement).replace_native(&node(&final_path), Some(&Cancellation::new()));

    assert_eq!(replaced, Ok(()));
    assert!(!replacement.exists());
    assert_eq!(fs::read(&final_path).unwrap(), b"replacement");
}

/// Port of the creation half of `test_enumeration_and_creation`: copies,
/// moves and new folders never take a name that exists.
///
/// parity: OPS-008, XFER-002
#[test]
fn exclusive_copy_move_and_folder_creation_preserve_existing_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    fs::write(&source, b"incoming").unwrap();
    fs::write(&target, b"retained").unwrap();
    let cancel = Cancellation::new();
    let source_node = node(&source);
    let target_node = node(&target);

    let copied = source_node.copy_file(&target_node, &cancel, &mut |_, _| {});
    let moved = source_node.move_native(&target_node, Some(&cancel));
    let created = target_node.create_directory(None);

    assert!(matches!(copied, Err(TransferError::Exists(_))), "{copied:?}");
    assert!(matches!(moved, Err(TransferError::Exists(_))), "{moved:?}");
    assert!(matches!(created, Err(TransferError::Exists(_))), "{created:?}");
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

    let file_published = node(&staged_file).publish(&node(&temp.path().join("file")), Some(&cancel));
    let folder_published = node(&staged_folder).publish(&node(&temp.path().join("folder")), Some(&cancel));

    assert_eq!(file_published, Ok(()));
    assert_eq!(folder_published, Ok(()));
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

    let copied = node(&source).copy_file(&node(&target), &cancel, &mut |_, _| {});
    let moved = node(&source).move_native(&node(&target), Some(&cancel));
    let deleted = node(&source).delete_tree(&cancel, None);

    assert_eq!(copied, Err(TransferError::Cancelled));
    assert_eq!(moved, Err(TransferError::Cancelled));
    assert_eq!(deleted, Err(TransferError::Cancelled));
    assert_eq!(fs::read(source).unwrap(), b"untouched");
    assert!(!target.exists());
}

/// Linux names need not be UTF-8. The adapter lists, inspects and copies
/// them byte for byte; only labels replace invalid bytes.
#[test]
fn names_that_are_not_utf8_are_listed_and_copied_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let name = OsString::from_vec(b"caf\xe9.mp3".to_vec());
    let music = temp.path().join("music");
    fs::create_dir(&music).unwrap();
    fs::write(music.join(&name), b"song").unwrap();
    let copies = temp.path().join("copies");
    fs::create_dir(&copies).unwrap();
    let target = node(&copies).child(&name);

    let listed = node(&music).children(None).unwrap();
    let song = listed.first().expect("the song is listed");
    let copied = song.copy_file(target.as_ref(), &Cancellation::new(), &mut |_, _| {});

    assert_eq!(listed.len(), 1);
    assert_eq!(song.name(), name);
    assert_eq!(song.display_name(), "caf\u{fffd}.mp3");
    assert_eq!(song.info(None).unwrap().kind, NodeKind::File);
    assert_eq!(copied, Ok(()));
    assert_eq!(fs::read(copies.join(&name)).unwrap(), b"song");
}

/// Port of `test_recursive_copy_preserves_link_and_source` in
/// `desktop/tests/gio_integration.py`.
///
/// parity: XFER-001, XFER-005, XFER-017
#[test]
fn complete_engine_stages_and_publishes_a_recursive_local_copy() {
    let temp = tempfile::tempdir().unwrap();
    let _access = RestoreOwnerAccess::new(temp.path());
    let source = temp.path().join("source");
    let target = temp.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("nested/data"), b"complete").unwrap();
    symlink("../missing", source.join("nested/link")).unwrap();
    set_mode(&source.join("nested"), 0o500);
    let target_uri = node(&target).uri();
    let copy = Operation::Copy {
        destination_folder: &target_uri,
        policy: ConflictPolicy::Skip,
    };

    let result = gio_engine()
        .run(copy, &[node(&source).uri()], &Cancellation::new())
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
