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
use ox_core::versions::{GioSnapshotProvider, PreviousVersions};

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
