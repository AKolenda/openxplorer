// SPDX-License-Identifier: AGPL-3.0-only
//! Finding the previous versions of an item (PROP-032) through GIO, in
//! real local snapshot folders, blocking and on a worker thread.
//!
//! The lookup rules themselves are compared with the Python app over
//! simulated snapshot folders in `versions_lookup.rs`. Every file is
//! inside a temporary directory.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ox_core::location::{file_uri, ItemKind};
use ox_core::transfer::Cancellation;
use ox_core::versions::{GioSnapshotProvider, PreviousVersions, SnapshotLayout};

/// A project folder with a document in two dated snapshots. The newest
/// snapshot is also reachable through a `latest` link, and in a third
/// snapshot the document is a link.
fn project_with_snapshots(root: &Path) -> PathBuf {
    let project = root.join("project");
    for (day, contents) in [("2026-09-01", "first"), ("2026-09-02", "second")] {
        let snapshot = project.join(".snapshot").join(day);
        fs::create_dir_all(&snapshot).unwrap();
        fs::write(snapshot.join("doc.txt"), contents).unwrap();
    }
    let linked = project.join(".snapshot/2026-09-03");
    fs::create_dir_all(&linked).unwrap();
    symlink("/etc/hostname", linked.join("doc.txt")).unwrap();
    symlink("2026-09-02", project.join(".snapshot/latest")).unwrap();
    fs::write(project.join("doc.txt"), "live").unwrap();
    project
}

/// parity: PROP-032
#[test]
fn gio_finds_versions_in_local_snapshot_folders_without_following_links() {
    let temporary = tempfile::tempdir().unwrap();
    let project = project_with_snapshots(temporary.path());
    let versions = PreviousVersions::new(&temporary.path().join("settings"));

    let list = versions
        .find_versions(
            &GioSnapshotProvider,
            &file_uri(&project.join("doc.txt")),
            ItemKind::File,
            &Cancellation::new(),
        )
        .expect("the lookup completes");

    let labels: Vec<&str> = list
        .versions
        .iter()
        .map(|version| version.label.as_str())
        .collect();
    assert_eq!(labels, ["2026-09-02", "2026-09-01"]);
    let newest = &list.versions[0];
    assert_eq!(
        newest.entry.uri,
        file_uri(&project.join(".snapshot/2026-09-02/doc.txt"))
    );
    assert_eq!(newest.entry.size, Some("second".len() as u64));
    assert_eq!(
        newest.snapshot_root,
        file_uri(&project.join(".snapshot/2026-09-02"))
    );
    assert_eq!(list.collections, [file_uri(&project.join(".snapshot"))]);
    assert_eq!(list.warnings.len(), 2, "{:?}", list.warnings);
    assert!(!list.is_truncated);
}

/// parity: PROP-032
#[test]
fn a_background_lookup_finds_the_same_versions() {
    let temporary = tempfile::tempdir().unwrap();
    let project = project_with_snapshots(temporary.path());
    let versions = Arc::new(PreviousVersions::new(&temporary.path().join("settings")));
    let uri = file_uri(&project.join("doc.txt"));
    let expected = versions
        .find_versions(&GioSnapshotProvider, &uri, ItemKind::File, &Cancellation::new())
        .unwrap();

    let lookup = Arc::clone(&versions).find_versions_in_background(uri, ItemKind::File, Cancellation::new());
    let list = glib::MainContext::new().block_on(lookup);

    assert_eq!(list.expect("the lookup completes"), expected);
}

/// Only real directories may connect a snapshot to a nested version.
/// This also checks encoded components and Snapper's extra layout folder.
#[test]
fn linked_intermediate_directories_are_not_previous_versions() {
    for layout in [SnapshotLayout::Direct, SnapshotLayout::Snapper] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let live = root.join("live");
        let history = root.join("history");
        let outside = root.join("other-example-folder");
        fs::create_dir_all(live.join("Docs #1/nested")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("doc.txt"), "outside content").unwrap();
        for day in ["2026-09-01", "2026-09-02"] {
            let mut files = history.join(day);
            if layout == SnapshotLayout::Snapper {
                files = files.join("snapshot");
            }
            fs::create_dir_all(files.join("Docs #1")).unwrap();
            let nested = files.join("Docs #1/nested");
            if day == "2026-09-01" {
                fs::create_dir(&nested).unwrap();
                fs::write(nested.join("doc.txt"), "historical content").unwrap();
            } else {
                symlink(&outside, nested).unwrap();
            }
        }
        if layout == SnapshotLayout::Snapper {
            fs::create_dir_all(history.join("2026-09-03")).unwrap();
            symlink(&live, history.join("2026-09-03/snapshot")).unwrap();
            fs::write(live.join("Docs #1/nested/doc.txt"), "live content").unwrap();
        }
        let versions = PreviousVersions::new(&root.join("settings"));
        versions
            .configure(&file_uri(&live), &file_uri(&history), layout)
            .unwrap();

        let list = versions
            .find_versions(
                &GioSnapshotProvider,
                &file_uri(&live.join("Docs #1/nested/doc.txt")),
                ItemKind::File,
                &Cancellation::new(),
            )
            .unwrap();

        assert_eq!(list.versions.len(), 1, "{layout:?}: {:?}", list.versions);
        assert_eq!(list.versions[0].label, "2026-09-01");
        assert_eq!(
            list.versions[0].entry.size,
            Some("historical content".len() as u64)
        );
        assert!(list.warnings.is_empty(), "{layout:?}: {:?}", list.warnings);
    }
}
