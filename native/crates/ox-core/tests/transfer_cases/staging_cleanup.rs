// SPDX-License-Identifier: AGPL-3.0-only
//! Cleanup of local staging through the production GIO adapter: after a
//! cancelled copy, with read-only folders inside, through a destination
//! reached by a symbolic link, and after someone else moved another folder
//! in under the staging name.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ox_core::gio_node::GioNode;
use ox_core::transfer::{ConflictPolicy, Node, TransferEngine, TransferMode};

use crate::transfer_support::*;

/// An engine resolving every URI with [`GioNode`].
fn gio_engine() -> TransferEngine {
    TransferEngine::new(Arc::new(|uri: &str| {
        Ok(Box::new(GioNode::new(uri)) as Box<dyn Node>)
    }))
}

/// Gives the folders below `root` owner access again, so the temporary
/// folder can be removed even after a test failed midway.
struct RestoreOwnerAccess(PathBuf);

impl Drop for RestoreOwnerAccess {
    fn drop(&mut self) {
        restore_owner_access(&self.0);
    }
}

fn restore_owner_access(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            set_mode(&path, 0o700);
            restore_owner_access(&path);
        }
    }
}

/// The engine's staging folder in `folder`, if there is one.
fn staging_in(folder: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(folder).expect("list the destination");
    entries
        .map(|entry| entry.expect("read an entry").path())
        .find(|path| is_staging_path(path))
}

/// A cancelled copy removes its staging even though cleanup runs after the
/// cancellation.
///
/// parity: OPS-022, XFER-002
#[test]
fn a_cancelled_copy_leaves_no_staging() {
    let fixture = Fixture::new();
    let source = fixture.src.join("large");
    fs::write(&source, random_bytes(262_144)).expect("write the source");
    let cancel = fixture.cancel.clone();
    let mut engine = gio_engine().with_progress(move |progress| {
        if progress.label.starts_with("Copying ") {
            cancel.cancel();
        }
    });

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.cancelled, "{result:?}");
    assert!(result.errors.is_empty(), "{result:?}");
    assert!(list(&fixture.dst).is_empty());
}

/// Publishing fails after the staged folders got their read-only modes
/// back; cleanup still empties every one of them.
///
/// parity: XFER-002, XFER-005
#[test]
fn staging_with_read_only_folders_inside_is_removed() {
    let fixture = Fixture::new();
    let _access = RestoreOwnerAccess(fixture.root.clone());
    let source = fixture.src.join("project");
    fs::create_dir_all(source.join("sub")).expect("create the source folders");
    write(&source.join("sub/data"), "contents");
    set_mode(&source.join("sub"), 0o555);
    set_mode(&source, 0o500);
    let racer = fixture.dst.join("project");
    let racer_path = racer.clone();
    let mut engine = gio_engine().with_progress(move |progress| {
        if progress.label.starts_with("Copying ") && !lexists(&racer_path) {
            write(&racer_path, "another program");
        }
    });

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(list(&fixture.dst), ["project"]);
    assert_eq!(read(&racer), "another program");
}

/// A destination opened through a symbolic link is where the staging
/// lives, and cleanup finds it there.
///
/// parity: XFER-002
#[test]
fn staging_in_a_destination_reached_through_a_link_is_removed() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("alias");
    symlink(&fixture.dst, &alias).expect("create the link");
    let source = fixture.src.join("project");
    fs::create_dir(&source).expect("create the source folder");
    write(&source.join("data"), "contents");
    let racer_path = fixture.dst.join("project");
    let mut engine = gio_engine().with_progress(move |progress| {
        if progress.label.starts_with("Copying ") && !lexists(&racer_path) {
            write(&racer_path, "another program");
        }
    });

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        Some(&alias),
    );

    assert_eq!(result.errors.len(), 1, "{result:?}");
    assert_eq!(list(&fixture.dst), ["project"]);
}

/// Someone who can write to a shared destination moves the engine's
/// staging folder away while it is filled and moves one of the user's
/// folders in under its name. Cleanup refuses that folder instead of
/// emptying it, and reports the leftover.
///
/// parity: XFER-002, XFER-003
#[test]
fn a_folder_moved_in_under_the_staging_name_is_never_emptied() {
    let fixture = Fixture::new();
    let victim = fixture.dst.join("victim");
    fs::create_dir(&victim).expect("create the user's folder");
    write(&victim.join("precious"), "precious");
    let source = fixture.src.join("project");
    fs::create_dir(&source).expect("create the source folder");
    write(&source.join("a"), "a");
    write(&source.join("b"), "b");
    let destination = fixture.dst.clone();
    let mut engine = gio_engine().with_progress(move |progress| {
        let victim = destination.join("victim");
        if !progress.label.starts_with("Copying ") || !lexists(&victim) {
            return;
        }
        let stage = staging_in(&destination).expect("the copy is staged");
        fs::rename(&stage, destination.join("moved-away")).expect("move the staging away");
        fs::rename(&victim, &stage).expect("move the user's folder in");
    });

    let result = fixture.run(
        &mut engine,
        &[&source],
        TransferMode::Copy,
        ConflictPolicy::Skip,
        None,
    );

    assert!(result.done.is_empty(), "{result:?}");
    let leftover = result.errors.last().expect("the leftover is reported");
    assert!(
        leftover.contains("Another item now has the staging folder's name"),
        "{leftover}"
    );
    let swapped_in = staging_in(&fixture.dst).expect("the user's folder keeps the name");
    assert_eq!(read(&swapped_in.join("precious")), "precious");
}
