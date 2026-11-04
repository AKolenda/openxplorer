// SPDX-License-Identifier: AGPL-3.0-only
//! What the app does around the helper: which locations go to it, which
//! path it is given, and what the user is told when the walk comes back
//! with items still in place.
//!
//! The step that elevates is replaced here, so these tests never start a
//! process and never ask for an administrator.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::super::{admin_folder, bound_identity, delete_tree, local_path, set_elevation, Record, Totals};
use crate::test_support::elevation_lock;
use crate::transfer::{Cancellation, ItemIdentity, NodeKind, TransferError};

/// The identity a test says the selected folder has.
const SELECTED: ItemIdentity = ItemIdentity {
    device: 2049,
    inode: 131_074,
};

/// Installs an elevation that reports `records` for whatever path it is
/// given, and remembers that path. The replacement is removed again when
/// the returned guard is dropped.
struct FakeElevation {
    asked_for: Arc<Mutex<Vec<(PathBuf, ItemIdentity)>>>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl FakeElevation {
    fn reporting(records: Vec<Record>) -> Self {
        let lock = elevation_lock();
        let asked_for = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked_for);
        set_elevation(Some(Arc::new(
            move |path: &Path,
                  expected: ItemIdentity,
                  _cancel: &Cancellation,
                  report: &mut dyn FnMut(Record)| {
                seen.lock()
                    .expect("the lock is not poisoned")
                    .push((path.to_path_buf(), expected));
                let mut totals = Totals::default();
                for record in &records {
                    totals.count(record);
                    report(record.clone());
                }
                Ok(totals)
            },
        )));
        Self {
            asked_for,
            _lock: lock,
        }
    }

    /// The paths the app asked to have deleted with administrator rights.
    fn paths(&self) -> Vec<PathBuf> {
        self.requests().into_iter().map(|(path, _)| path).collect()
    }

    /// The paths, with the identity each had to still have.
    fn requests(&self) -> Vec<(PathBuf, ItemIdentity)> {
        self.asked_for.lock().expect("the lock is not poisoned").clone()
    }
}

impl Drop for FakeElevation {
    fn drop(&mut self) {
        set_elevation(None);
    }
}

/// parity: XFER-015
#[test]
fn an_admin_location_is_deleted_by_its_own_path_with_its_escapes_read() {
    let elevation = FakeElevation::reporting(vec![Record::Deleted(PathBuf::from("/etc/a folder"))]);
    let folder = gio::File::for_uri("admin:///etc/a%20folder");

    let result = delete_tree(&folder, SELECTED, &Cancellation::new(), None);

    assert_eq!(result, Ok(()));
    assert_eq!(
        elevation.requests(),
        vec![(PathBuf::from("/etc/a folder"), SELECTED)]
    );
}

/// The folder found when the deletion reaches it has to be the one the
/// engine selected when the deletion started; a folder that took its
/// name meanwhile is refused before the helper is started.
///
/// parity: XFER-015
#[test]
fn a_folder_that_is_not_the_selected_one_is_refused_before_anything_is_elevated() {
    let other = ItemIdentity {
        device: SELECTED.device,
        inode: SELECTED.inode + 1,
    };

    assert_eq!(bound_identity(Some(SELECTED), SELECTED), Ok(SELECTED));
    assert_eq!(bound_identity(None, other), Ok(other));
    assert_eq!(
        bound_identity(Some(SELECTED), other),
        Err(crate::safe_walk::folder_changed())
    );
}

/// A wide tree that could not be removed is summed up, not kept item by
/// item: the message names the first few and counts the rest.
///
/// parity: XFER-015
#[test]
fn a_long_list_of_items_left_behind_is_named_in_part_and_counted() {
    let records = (0..8)
        .map(|index| Record::Failed {
            path: PathBuf::from(format!("/etc/tree/{index}")),
            message: "Read-only file system".to_owned(),
        })
        .collect();
    let _elevation = FakeElevation::reporting(records);

    let result = delete_tree(
        &gio::File::for_uri("admin:///etc/tree"),
        SELECTED,
        &Cancellation::new(),
        None,
    );

    let listed: Vec<String> = (0..5)
        .map(|index| format!("/etc/tree/{index}: Read-only file system"))
        .collect();
    assert_eq!(
        result,
        Err(TransferError::failed(super::super::incomplete_message(
            &listed, 3
        )))
    );
    assert!(result
        .expect_err("the deletion is incomplete")
        .to_string()
        .ends_with("… and 3 more."));
}

/// What the walk could not remove is named, so a deletion that left part
/// of the tree behind never looks like a clean sweep.
///
/// parity: XFER-015
#[test]
fn items_the_walk_left_behind_are_named_in_the_failure() {
    let _elevation = FakeElevation::reporting(vec![
        Record::Deleted(PathBuf::from("/etc/tree/gone")),
        Record::Skipped {
            path: PathBuf::from("/etc/tree/mounted"),
            reason: "another drive".to_owned(),
        },
        Record::Failed {
            path: PathBuf::from("/etc/tree"),
            message: "Directory not empty".to_owned(),
        },
    ]);

    let result = delete_tree(
        &gio::File::for_uri("admin:///etc/tree"),
        SELECTED,
        &Cancellation::new(),
        None,
    );

    // The whole message, not a substring of it: one that quietly lost
    // its heading, or named the item that did go, would still hold both
    // of these lines.
    let error = result.expect_err("the deletion is incomplete");
    assert_eq!(
        error,
        TransferError::failed(super::super::incomplete_message(
            &[
                "/etc/tree/mounted: another drive".to_owned(),
                "/etc/tree: Directory not empty".to_owned(),
            ],
            0
        ))
    );
    assert!(
        !error.to_string().contains("gone"),
        "what was deleted is named as left behind: {error}"
    );
}

/// XFER-020: the engine's preflight has already asked the guard about
/// every URI in the tree, and the backend asks once more about the item
/// itself before it does anything. A refusal must stop the helper from
/// being started at all.
///
/// parity: XFER-015, XFER-020
#[test]
fn a_protected_item_is_refused_before_anything_is_elevated() {
    let elevation = FakeElevation::reporting(vec![]);
    let guard = |_uri: &str| Err(TransferError::failed("Protected snapshot."));

    let result = delete_tree(
        &gio::File::for_uri("admin:///etc/tree"),
        SELECTED,
        &Cancellation::new(),
        Some(&guard),
    );

    assert_eq!(result, Err(TransferError::failed("Protected snapshot.")));
    assert!(elevation.paths().is_empty(), "{:?}", elevation.paths());
}

/// parity: XFER-015
#[test]
fn only_an_admin_location_of_this_computer_has_a_path_to_delete() {
    let path = |uri: &str| local_path(&gio::File::for_uri(uri));

    assert_eq!(path("admin:///etc/hosts"), Some(PathBuf::from("/etc/hosts")));
    // A host means another backend; admin:// addresses this computer.
    assert_eq!(path("admin://server/etc"), None);
    assert_eq!(path("file:///etc/hosts"), None);
    assert_eq!(path("smb://server/share/folder"), None);
}

/// A location that is not `admin://` never reaches the helper, whatever
/// it is, and neither does an `admin://` item the caller already found
/// not to be a folder: both questions are answered before anything is
/// asked of `GVfs`, so neither costs a round trip or a prompt.
///
/// parity: XFER-015
#[test]
fn only_an_admin_folder_reaches_the_helper() {
    let cancel = Cancellation::new();
    let folder = Some(NodeKind::Directory);

    let local = admin_folder(&gio::File::for_uri("file:///tmp"), folder, &cancel);
    let share = admin_folder(&gio::File::for_uri("smb://server/share"), folder, &cancel);
    let admin_file = admin_folder(
        &gio::File::for_uri("admin:///etc/hosts"),
        Some(NodeKind::File),
        &cancel,
    );
    let admin_link = admin_folder(
        &gio::File::for_uri("admin:///etc/link"),
        Some(NodeKind::Symlink),
        &cancel,
    );

    assert_eq!(local, Ok(None));
    assert_eq!(share, Ok(None));
    assert_eq!(admin_file, Ok(None));
    assert_eq!(admin_link, Ok(None));
}
