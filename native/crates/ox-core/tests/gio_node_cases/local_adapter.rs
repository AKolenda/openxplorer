// SPDX-License-Identifier: AGPL-3.0-only
//! The production GIO adapter on local files: listing, copying, moving and
//! publishing, each without following links and without overwriting.
//! Ports `desktop/tests/gio_integration.py`.

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{symlink, MetadataExt};
use std::path::Path;

use ox_core::transfer::{Cancellation, ConflictPolicy, Node, NodeKind, Operation, TransferError};

use super::shared::{gio_engine, mode_of, set_mode, temporary_folder, RestoreOwnerAccess};
use super::{node, LinkedFolder};

/// Ported from `desktop/tests/gio_integration.py::GioLocalIntegration::test_enumeration_and_creation`
/// (the listing half).
///
/// parity: XFER-017
#[test]
fn metadata_and_listing_preserve_hidden_files_and_do_not_follow_links() {
    let root = temporary_folder();
    let folder = root.path().join("folder");
    fs::create_dir(&folder).expect("the fixture folder is created");
    set_mode(&folder, 0o750);
    fs::write(folder.join(".hidden"), b"hello").expect("the fixture file is written");
    symlink("missing", folder.join("dangling")).expect("the fixture link is created");
    symlink(&folder, root.path().join("alias")).expect("the fixture link is created");
    let link = node(&folder.join("dangling"));
    let alias = node(&root.path().join("alias"));

    let folder_info = node(&folder).info(None).expect("the adapter inspects the item");
    let children = node(&folder)
        .children(None)
        .expect("the adapter lists the folder");
    let link_exists = link.exists(None);
    let link_info = link.info(None).expect("the adapter inspects the item");
    let alias_is_directory = alias.is_directory(None).expect("the adapter inspects the item");
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
    let root = temporary_folder();
    let source = root.path().join("source");
    let target = root.path().join("copy");
    fs::write(&source, b"complete content").expect("the fixture file is written");
    let mut progress = Vec::new();

    let copied = node(&source).copy_file(&node(&target), &Cancellation::new(), &mut |current, total| {
        progress.push((current, total));
    });

    assert_eq!(copied, Ok(()));
    assert_eq!(
        fs::read(&target).expect("the file can be read"),
        b"complete content"
    );
    assert_eq!(
        fs::read(&source).expect("the file can be read"),
        b"complete content"
    );
    assert!(progress.iter().all(|(current, total)| current <= total));
}

/// parity: XFER-015, XFER-017
#[test]
fn copying_a_link_copies_the_link_not_its_target() {
    let root = temporary_folder();
    let linked = LinkedFolder::create(root.path());
    let copied = root.path().join("copied");

    let result = node(&linked.link).copy_file(&node(&copied), &Cancellation::new(), &mut |_, _| {});

    assert_eq!(result, Ok(()));
    assert_eq!(fs::read_link(&copied).expect("the item is a link"), linked.kept);
    linked.assert_untouched();
}

/// The adapter's native move on one filesystem.
#[test]
fn move_native_leaves_nothing_under_the_old_name() {
    let root = temporary_folder();
    let source = root.path().join("copy");
    let final_path = root.path().join("final");
    fs::write(&source, b"complete content").expect("the fixture file is written");

    let moved = node(&source).move_native(&node(&final_path), Some(&Cancellation::new()));

    assert_eq!(moved, Ok(()));
    assert!(!source.exists());
    assert_eq!(
        fs::read(&final_path).expect("the file can be read"),
        b"complete content"
    );
}

/// parity: XFER-009
#[test]
fn replace_native_overwrites_the_existing_file() {
    let root = temporary_folder();
    let replacement = root.path().join("copy");
    let final_path = root.path().join("final");
    fs::write(&replacement, b"replacement").expect("the fixture file is written");
    fs::write(&final_path, b"complete content").expect("the fixture file is written");

    let replaced = node(&replacement).replace_native(&node(&final_path), Some(&Cancellation::new()));

    assert_eq!(replaced, Ok(()));
    assert!(!replacement.exists());
    assert_eq!(
        fs::read(&final_path).expect("the file can be read"),
        b"replacement"
    );
}

/// Ported from `desktop/tests/gio_integration.py::GioLocalIntegration::test_enumeration_and_creation`
/// (the creation half): copies, moves and new folders never take a name
/// that exists.
///
/// parity: OPS-008, XFER-002
#[test]
fn exclusive_copy_move_and_folder_creation_preserve_existing_destinations() {
    let root = temporary_folder();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"incoming").expect("the fixture file is written");
    fs::write(&target, b"retained").expect("the fixture file is written");
    let cancel = Cancellation::new();
    let source_node = node(&source);
    let target_node = node(&target);

    let copied = source_node.copy_file(&target_node, &cancel, &mut |_, _| {});
    let moved = source_node.move_native(&target_node, Some(&cancel));
    let created = target_node.create_directory(None);

    assert!(matches!(copied, Err(TransferError::Exists(_))), "{copied:?}");
    assert!(matches!(moved, Err(TransferError::Exists(_))), "{moved:?}");
    assert!(matches!(created, Err(TransferError::Exists(_))), "{created:?}");
    assert_eq!(fs::read(source).expect("the file can be read"), b"incoming");
    assert_eq!(fs::read(target).expect("the file can be read"), b"retained");
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
        let root = temporary_folder();
        let staged = root.path().join("staged");
        if kind == NodeKind::Directory {
            fs::create_dir(&staged).expect("the fixture folder is created");
        } else {
            fs::write(&staged, b"incoming").expect("the fixture file is written");
        }
        let taken = root.path().join("report");
        fs::write(&taken, b"another program").expect("the fixture file is written");

        let published = node(&staged).publish(&node(&taken), Some(&Cancellation::new()));

        assert_eq!(
            published,
            Err(TransferError::Exists(
                "An item named “report” already exists. Nothing was overwritten.".into()
            )),
            "{kind:?}"
        );
        assert_eq!(
            fs::read(&taken).expect("the file can be read"),
            b"another program"
        );
        assert!(staged.exists(), "{kind:?}");
    }
}

/// parity: XFER-001
#[test]
fn publishing_installs_a_staged_file_or_folder_under_a_free_name() {
    let root = temporary_folder();
    let staged_file = root.path().join("staged-file");
    let staged_folder = root.path().join("staged-folder");
    fs::write(&staged_file, b"complete").expect("the fixture file is written");
    fs::create_dir(&staged_folder).expect("the fixture folder is created");
    fs::write(staged_folder.join("inside"), b"inside").expect("the fixture file is written");
    let cancel = Cancellation::new();

    let file_published = node(&staged_file).publish(&node(&root.path().join("file")), Some(&cancel));
    let folder_published = node(&staged_folder).publish(&node(&root.path().join("folder")), Some(&cancel));

    assert_eq!(file_published, Ok(()));
    assert_eq!(folder_published, Ok(()));
    assert_eq!(
        fs::read(root.path().join("file")).expect("the file can be read"),
        b"complete"
    );
    assert_eq!(
        fs::read(root.path().join("folder/inside")).expect("the file can be read"),
        b"inside"
    );
    assert!(!staged_file.exists() && !staged_folder.exists());
}

/// parity: OPS-022
#[test]
fn cancellation_prevents_copy_move_and_recursive_delete() {
    let root = temporary_folder();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"untouched").expect("the fixture file is written");
    let cancel = Cancellation::new();
    cancel.cancel();

    let copied = node(&source).copy_file(&node(&target), &cancel, &mut |_, _| {});
    let moved = node(&source).move_native(&node(&target), Some(&cancel));
    let deleted = node(&source).delete_tree(&cancel, None);

    assert_eq!(copied, Err(TransferError::Cancelled));
    assert_eq!(moved, Err(TransferError::Cancelled));
    assert_eq!(deleted, Err(TransferError::Cancelled));
    assert_eq!(fs::read(source).expect("the file can be read"), b"untouched");
    assert!(!target.exists());
}

/// Linux names need not be UTF-8. The adapter lists, inspects and copies
/// them byte for byte; only labels replace invalid bytes.
#[test]
fn names_that_are_not_utf8_are_listed_and_copied_byte_for_byte() {
    let root = temporary_folder();
    let name = OsString::from_vec(b"caf\xe9.mp3".to_vec());
    let music = root.path().join("music");
    fs::create_dir(&music).expect("the fixture folder is created");
    fs::write(music.join(&name), b"song").expect("the fixture file is written");
    let copies = root.path().join("copies");
    fs::create_dir(&copies).expect("the fixture folder is created");
    let target = node(&copies).child(&name);

    let listed = node(&music).children(None).expect("the adapter lists the folder");
    let song = listed.first().expect("the song is listed");
    let copied = song.copy_file(target.as_ref(), &Cancellation::new(), &mut |_, _| {});

    assert_eq!(listed.len(), 1);
    assert_eq!(song.name(), name);
    assert_eq!(song.display_name(), "caf\u{fffd}.mp3");
    assert_eq!(
        song.info(None).expect("the adapter inspects the item").kind,
        NodeKind::File
    );
    assert_eq!(copied, Ok(()));
    assert_eq!(
        fs::read(copies.join(&name)).expect("the file can be read"),
        b"song"
    );
}

/// Ported from `desktop/tests/gio_integration.py::GioLocalIntegration::test_recursive_copy_preserves_link_and_source`
///
/// parity: XFER-001, XFER-005, XFER-017
#[test]
fn complete_engine_stages_and_publishes_a_recursive_local_copy() {
    let root = temporary_folder();
    let _access = RestoreOwnerAccess::new(root.path());
    let source = root.path().join("source");
    let target = root.path().join("destination");
    fs::create_dir_all(source.join("nested")).expect("the fixture folders are created");
    fs::create_dir(&target).expect("the fixture folder is created");
    fs::write(source.join("nested/data"), b"complete").expect("the fixture file is written");
    symlink("../missing", source.join("nested/link")).expect("the fixture link is created");
    set_mode(&source.join("nested"), 0o500);
    let target_uri = node(&target).uri();
    let copy = Operation::Copy {
        destination_folder: &target_uri,
        policy: ConflictPolicy::Skip,
    };

    let result = gio_engine()
        .run(copy, &[node(&source).uri()], &Cancellation::new())
        .expect("the engine accepts the request");

    assert!(result.errors.is_empty(), "{result:?}");
    assert_eq!(result.done, [node(&source).uri()]);
    assert_eq!(
        fs::read(target.join("source/nested/data")).expect("the file can be read"),
        b"complete"
    );
    assert_eq!(
        fs::read_link(target.join("source/nested/link")).expect("the item is a link"),
        Path::new("../missing")
    );
    assert_eq!(mode_of(&target.join("source/nested")), 0o500);
    assert_eq!(
        fs::read_dir(&target).expect("the folder can be listed").count(),
        1
    );
}

/// parity: XFER-011
#[test]
fn cross_filesystem_move_is_refused_without_copying_or_removing_the_source() {
    let source_root = tempfile::tempdir_in("/tmp").expect("the test may create folders there");
    let target_root = tempfile::tempdir_in("/dev/shm").expect("the test may create folders there");
    assert_ne!(
        fs::metadata(source_root.path()).expect("the item exists").dev(),
        fs::metadata(target_root.path()).expect("the item exists").dev(),
        "this Linux integration check needs separate tmp and shm filesystems"
    );
    let source = source_root.path().join("source");
    let target = target_root.path().join("source");
    fs::write(&source, b"original").expect("the fixture file is written");

    let result = node(&source).move_native(&node(&target), Some(&Cancellation::new()));

    assert!(
        matches!(result, Err(TransferError::NotSupported(_))),
        "{result:?}"
    );
    assert_eq!(fs::read(source).expect("the file can be read"), b"original");
    assert!(!target.exists());
}
