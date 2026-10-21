// SPDX-License-Identifier: AGPL-3.0-only
//! A failed copy names the item the user asked for, never the engine's
//! private `.winspace-transfer-<hex>.part` staging, and keeps the system's
//! own reason. Windows Explorer names the file it could not create, and so
//! does this engine.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ox_core::transfer::{Cancellation, ConflictPolicy, FailedItem, FailureAnswer, Node, TransferError};

use crate::transfer_support::device::Device;
use crate::transfer_support::local::{local_path_of, LocalNode, Provider};
use crate::transfer_support::*;

/// The destination folder made read-only (`chmod 555`), with owner access
/// given back when dropped. `None` when the test runs as root, which
/// writes anywhere.
fn lock(folder: &Path) -> Option<RestoreOwnerAccess> {
    set_mode(folder, 0o555);
    let access = RestoreOwnerAccess::new(folder);
    let probe = folder.join("probe");
    if fs::write(&probe, "").is_ok() {
        let _ = fs::remove_file(probe);
        return None;
    }
    Some(access)
}

/// Asserts `error` names `real` and the system's reason, and nothing of
/// the staging.
fn assert_names(error: &str, real: &Path) {
    assert!(!error.contains(".winspace-transfer"), "{error}");
    assert!(!error.contains("payload"), "{error}");
    assert!(error.contains(&real.display().to_string()), "{error}");
    assert!(error.contains("Permission denied"), "{error}");
}

/// The case seen on a test build: pasting `one.txt` into a `chmod 555`
/// folder `Locked` said "Error creating directory
/// …/Locked/.winspace-transfer-<hex>.part: Permission denied".
///
/// parity: XFER-001
#[test]
fn a_copy_into_a_read_only_folder_names_the_real_file() {
    let fixture = Fixture::with_destination("Locked");
    let source = fixture.source_folder.join("one.txt");
    write(&source, "one");
    let Some(_access) = lock(&fixture.destination_folder) else {
        return;
    };

    let result = fixture.run(&mut gio_engine(), &[&source], Request::Copy(ConflictPolicy::Skip));

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 1, "{result:?}");
    let error = &result.errors[0];
    assert!(error.starts_with("one.txt: "), "{error}");
    assert_names(error, &fixture.destination_folder.join("one.txt"));
    assert!(list(&fixture.destination_folder).is_empty());
}

/// A folder names the folder it would have become, and "Keep both" names
/// the copy's new name, not the name that was taken.
///
/// parity: XFER-001, XFER-008
#[test]
fn folders_and_keep_both_name_the_item_they_would_have_become() {
    let fixture = Fixture::with_destination("Locked");
    let tree = fixture.source_folder.join("tree");
    fs::create_dir(&tree).expect("create the source folder");
    write(&tree.join("a.txt"), "a");
    let one = fixture.source_folder.join("one.txt");
    write(&one, "new");
    write(&fixture.destination_folder.join("one.txt"), "old");
    let Some(_access) = lock(&fixture.destination_folder) else {
        return;
    };

    let result = fixture.run(
        &mut gio_engine(),
        &[&tree, &one],
        Request::Copy(ConflictPolicy::KeepBoth),
    );

    assert!(result.done.is_empty(), "{result:?}");
    assert_eq!(result.errors.len(), 2, "{result:?}");
    assert_names(&result.errors[0], &fixture.destination_folder.join("tree"));
    assert_names(
        &result.errors[1],
        &fixture.destination_folder.join("one - Copy.txt"),
    );
    assert_eq!(read(&fixture.destination_folder.join("one.txt")), "old");
}

/// A backend that cannot write `b.txt` and says so with the full path and
/// URI it was given, as GIO and `GVfs` do.
struct RefusesB;

impl Provider for RefusesB {
    fn copy_file(
        &self,
        node: &LocalNode,
        target: &dyn Node,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        if node.display_name() != "b.txt" {
            return node.local_copy_file(target, cancel, progress);
        }
        Err(TransferError::failed(format!(
            "Error opening file {} ({}): Permission denied",
            local_path_of(target).display(),
            target.uri(),
        )))
    }
}

/// An entry inside a copied folder is asked about, and reported, by the
/// path it would have in the published folder.
///
/// parity: XFER-001, OPS-047
#[test]
fn an_entry_inside_a_folder_is_asked_about_by_its_real_path() {
    let fixture = Fixture::new();
    let tree = fixture.source_folder.join("tree");
    fs::create_dir_all(tree.join("sub")).expect("create the source folders");
    write(&tree.join("a.txt"), "a");
    write(&tree.join("sub").join("b.txt"), "b");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&asked);
    let mut engine = fixture
        .engine(Arc::new(RefusesB))
        .with_failure_question(move |item: &FailedItem| {
            log.lock().expect("question log").push(item.error.clone());
            FailureAnswer::Skip
        });

    let result = fixture.run(&mut engine, &[&tree], Request::Copy(ConflictPolicy::Skip));

    let real = fixture.destination_folder.join("tree").join("sub").join("b.txt");
    let asked = asked.lock().expect("question log").clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_names(&asked[0], &real);
    assert!(asked[0].contains(&file_uri(&real)), "{}", asked[0]);
    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_names(&result.errors[0], &real);
    assert_eq!(list(&fixture.destination_folder.join("tree")), ["a.txt", "sub"]);
    fixture.assert_no_staging();
}

/// A copy within one phone (MTP `CopyObject` keeps the source's name) that
/// fails, reporting the URI it was given, as `GVfs` does.
#[derive(Default)]
struct RefusesSameDeviceCopy {
    device: Device,
}

impl Provider for RefusesSameDeviceCopy {
    fn base(&self) -> Option<&dyn Provider> {
        Some(&self.device)
    }

    fn native_copy_keeps_name(&self, _node: &LocalNode, _target_folder: &dyn Node) -> bool {
        true
    }

    fn copy_file(
        &self,
        _node: &LocalNode,
        target: &dyn Node,
        _cancel: &Cancellation,
        _progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), TransferError> {
        Err(TransferError::failed(format!(
            "Error copying to {}: Device busy",
            target.uri()
        )))
    }
}

/// A name a URI must escape (`a b.txt` is `a%20b.txt` there) still becomes
/// the final name, here the "Keep both" copy name, never a path below it.
///
/// parity: XFER-001, XFER-023
#[test]
fn a_copy_within_one_device_names_the_final_name_in_an_escaped_uri() {
    let fixture = Fixture::with_destination("phone");
    let source = fixture.source_folder.join("a b.txt");
    write(&source, "new");
    write(&fixture.destination_folder.join("a b.txt"), "old");

    let result = fixture.copy(
        Arc::new(RefusesSameDeviceCopy::default()),
        &[&source],
        ConflictPolicy::KeepBoth,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    let error = &result.errors[0];
    let real = format!(
        "{}/a b - Copy.txt: Device busy",
        file_uri(&fixture.destination_folder)
    );
    assert!(error.ends_with(&real), "{error}");
    assert!(!error.contains(".winspace-transfer"), "{error}");
    assert_eq!(list(&fixture.destination_folder), ["a b.txt"]);
}
