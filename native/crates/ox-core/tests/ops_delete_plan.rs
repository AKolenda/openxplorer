// SPDX-License-Identifier: AGPL-3.0-only
//! Trash support and the Delete plan (`trashSupport` in
//! `v2.0.0:desktop/winspace.py` and `trash` in `v2.0.0:desktop/ui/app.js`) on temporary
//! local folders and a share that is not mounted. The Python suite has no
//! test of them, so none is ported here; the confirmation texts are unit
//! tests of `ops::delete_plan`.

mod ops_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use gio::prelude::*;

use ox_core::ops::{plan_delete, trash_support, DeleteItem, OpsError};
use ox_core::transfer::Cancellation;

use ops_support::{block_on, file_uri};

/// Sets the permission bits of the folder at `path`.
fn set_folder_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod a test folder");
}

/// A GIO query that fails for any other reason than an unmounted share
/// answers "no Trash", as `trash_support` in `v2.0.0:desktop/gio_backend.py`
/// does, so the confirmation says the item is deleted permanently.
#[test]
fn an_item_whose_folder_cannot_be_queried_is_planned_for_permanent_delete_like_python() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("a.txt");
    fs::write(&local, b"a").unwrap();
    let vanished = temp.path().join("gone").join("b.txt");
    let items = [
        DeleteItem {
            uri: file_uri(&local),
            name: "a.txt".into(),
        },
        DeleteItem {
            uri: file_uri(&vanished),
            name: "b.txt".into(),
        },
    ];
    let cancel = Cancellation::new();

    let local_trash = block_on(trash_support(&file_uri(temp.path()), &cancel));
    let plan = block_on(plan_delete(&items, &cancel)).unwrap();

    assert_eq!(local_trash, Ok(Some(true)));
    assert_eq!(plan.to_trash, [file_uri(&local)]);
    assert_eq!(plan.to_delete, [file_uri(&vanished)]);
    assert_eq!(plan.confirmation().title, "Delete items?");
}

/// An item goes to the Trash when GIO says the item itself can, even
/// where its folder could not be trashed: a file directly in the home
/// folder, whose own folder `/home` only root may change, is moved to the
/// Recycle Bin, as `gio trash` and Windows Explorer do.
#[test]
fn an_item_in_a_folder_inside_a_read_only_folder_goes_to_the_trash() {
    let temp = tempfile::tempdir().unwrap();
    let locked = temp.path().join("locked");
    let home = locked.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("a.txt"), b"a").unwrap();
    let item = DeleteItem {
        uri: file_uri(&home.join("a.txt")),
        name: "a.txt".into(),
    };
    let cancel = Cancellation::new();

    set_folder_mode(&locked, 0o555);
    let support = block_on(trash_support(&file_uri(&home), &cancel));
    let plan = block_on(plan_delete(std::slice::from_ref(&item), &cancel));
    set_folder_mode(&locked, 0o755);

    assert_eq!(support, Ok(Some(true)));
    let plan = plan.unwrap();
    assert_eq!(plan.to_trash, [item.uri]);
    assert!(plan.to_delete.is_empty());
    assert_eq!(plan.confirmation().title, "Move to Trash?");
}

/// GIO reports `access::can-trash` false for an item in a folder the user
/// may not change, so the confirmation says it is deleted permanently.
#[test]
fn an_item_gio_reports_without_a_trash_is_planned_for_permanent_delete() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("locked");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("a.txt"), b"a").unwrap();
    let item = DeleteItem {
        uri: file_uri(&folder.join("a.txt")),
        name: "a.txt".into(),
    };
    let cancel = Cancellation::new();

    set_folder_mode(&folder, 0o555);
    let support = block_on(trash_support(&file_uri(&folder), &cancel));
    let plan = block_on(plan_delete(std::slice::from_ref(&item), &cancel));
    set_folder_mode(&folder, 0o755);

    assert_eq!(support, Ok(Some(false)));
    let plan = plan.unwrap();
    assert!(plan.to_trash.is_empty());
    assert_eq!(plan.to_delete, [item.uri]);
    assert_eq!(plan.confirmation().title, "Delete permanently?");
}

/// One item that cannot go to the Trash does not decide for the others
/// in its folder: a file that vanished after the listing is planned for
/// permanent delete (which then reports it missing), the file beside it
/// still goes to the Trash.
#[test]
fn an_item_gio_cannot_answer_for_does_not_decide_for_its_folder() {
    let temp = tempfile::tempdir().unwrap();
    let kept = temp.path().join("b.txt");
    fs::write(&kept, b"b").unwrap();
    let items = [
        DeleteItem {
            uri: file_uri(&temp.path().join("vanished.txt")),
            name: "vanished.txt".into(),
        },
        DeleteItem {
            uri: file_uri(&kept),
            name: "b.txt".into(),
        },
    ];

    let plan = block_on(plan_delete(&items, &Cancellation::new())).unwrap();

    assert_eq!(plan.to_trash, [file_uri(&kept)]);
    assert_eq!(plan.to_delete, [items[0].uri.clone()]);
}

/// parity: OPS-037, OPS-017
#[test]
fn a_share_that_cannot_answer_counts_as_having_a_trash() {
    let schemes = gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "smb"),
        "this check needs GVfs with its SMB backend (gvfs-backends); found {schemes:?}"
    );
    let item = DeleteItem {
        uri: "smb://example.invalid/share/report.pdf".into(),
        name: "report.pdf".into(),
    };
    let cancel = Cancellation::new();

    let support = block_on(trash_support("smb://example.invalid/share", &cancel));
    let plan = block_on(plan_delete(std::slice::from_ref(&item), &cancel)).unwrap();

    assert!(matches!(support, Err(OpsError::NotMounted(_))), "{support:?}");
    assert!(support.unwrap_err().needs_mount());
    assert_eq!(plan.to_trash, [item.uri]);
    assert!(plan.to_delete.is_empty());
    assert_eq!(plan.confirmation().title, "Move to Trash?");
}

#[test]
fn a_cancelled_trash_check_reports_the_cancellation_and_plans_nothing() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), b"a").unwrap();
    let item = DeleteItem {
        uri: file_uri(&temp.path().join("a.txt")),
        name: "a.txt".into(),
    };
    let cancelled = Cancellation::new();
    cancelled.cancel();

    let support = block_on(trash_support(&file_uri(temp.path()), &cancelled));
    let plan = block_on(plan_delete(std::slice::from_ref(&item), &cancelled));

    assert_eq!(support, Err(OpsError::Cancelled));
    assert_eq!(plan, Err(OpsError::Cancelled));
    assert!(temp.path().join("a.txt").exists());
}

/// An empty folder has no item for GIO to answer about, so its Trash
/// support stays unknown and is asked again once it holds items.
#[test]
fn an_empty_folder_leaves_its_trash_support_unknown() {
    let temp = tempfile::tempdir().unwrap();

    let support = block_on(trash_support(&file_uri(temp.path()), &Cancellation::new()));

    assert_eq!(support, Ok(None));
}
