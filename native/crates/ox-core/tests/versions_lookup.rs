// SPDX-License-Identifier: AGPL-3.0-only
//! Finding the previous versions of an item (PROP-032) in simulated
//! snapshot folders.
//!
//! `desktop/tests` has no backend test of the lookup, so each scenario
//! here runs twice: through [`PreviousVersions::find_versions`] and
//! through `PreviousVersions.list` in `desktop/previous_versions.py`, over
//! the same simulated snapshot folders, and the two outcomes must match
//! (`versions_support`). The GIO provider is checked on real local
//! snapshot folders in `versions_lookup_gio.rs`. Every file is inside a
//! temporary directory.

mod python_support;
mod versions_support;

use ox_core::entry::{Entry, EntryError};
use ox_core::location::ItemKind;
use ox_core::transfer::Cancellation;
use ox_core::versions::{
    CollectionListing, PreviousVersions, SnapshotLayout, SnapshotProvider, VersionsError, NO_VERSIONS_FOUND,
};
use serde_json::{json, Value};
use versions_support::{labels, lookup_in_both_apps, Lookup, TreeProvider};

/// A share whose `work/a.txt` is in two snapshots, missing from one and
/// a link in another. The `.snapshot` collection also holds a file, a
/// link and a snapshot with an unusable name; `#snapshot` cannot be read
/// and `.zfs/snapshot` is missing.
fn share_with_snapshots() -> Value {
    json!({
        "collections": {
            "smb://nas/share/.snapshot": {"entries": [
                {"name": "2026-09-01_1200", "isDir": true, "modified": 1_756_728_000},
                {"name": "2026-09-05_1800", "isDir": true, "modified": 1_757_095_200},
                {"name": "2026-09-03_0900", "isDir": true},
                {"name": "2026-09-04_0900", "isDir": true},
                {"name": "latest", "symlink": true},
                {"name": "readme.txt", "isDir": false},
                {"name": "bad\u{1}name", "isDir": true},
            ]},
            "smb://nas/share/%23snapshot": {"error": "Permission denied"},
            "smb://nas/share/.snapshots": {"entries": [{"name": "5", "isDir": true}]},
        },
        "items": {
            "smb://nas/share/.snapshot/2026-09-01_1200/work/a.txt": {"name": "a.txt"},
            "smb://nas/share/.snapshot/2026-09-05_1800/work/a.txt": {"name": "a.txt"},
            "smb://nas/share/.snapshot/2026-09-04_0900/work/a.txt": {"name": "a.txt", "symlink": true},
            "smb://nas/share/.snapshots/5/snapshot/work/a.txt": {"error": "Input/output error"},
        },
    })
}

/// A share item found in two snapshots, missing from one, a link in
/// another; a file, a link and an unusable name in the collection are
/// not snapshots, and unreadable collections and snapshots are warnings.
///
/// parity: PROP-032
#[test]
fn share_items_are_found_at_the_share_root_and_failures_become_warnings() {
    let lookup = Lookup {
        uri: "smb://nas/share/work/a.txt",
        kind: ItemKind::File,
        tree: share_with_snapshots(),
    };

    let outcome = lookup_in_both_apps(&lookup, |_| {});

    assert_eq!(labels(&outcome), ["2026-09-05_1800", "2026-09-01_1200"]);
    assert_eq!(
        outcome["versions"][0]["uri"],
        "smb://nas/share/.snapshot/2026-09-05_1800/work/a.txt"
    );
    assert_eq!(outcome["versions"][0]["snapshotModified"], 1_757_095_200);
    assert_eq!(
        outcome["warnings"],
        json!([
            "bad\u{1}name: A name cannot contain slashes or control characters.",
            "smb://nas/share/%23snapshot: Permission denied",
            "smb://nas/share/.zfs/snapshot: No such folder",
            "5: Input/output error",
        ])
    );
    assert_eq!(outcome["message"], "");
}

/// parity: PROP-032, PROP-023
#[test]
fn a_configured_source_is_used_and_an_empty_result_explains_itself() {
    let lookup = Lookup {
        uri: "file:///srv/data/reports",
        kind: ItemKind::Folder,
        tree: json!({
            "collections": {
                "file:///srv/history": {"entries": [
                    {"name": "monday", "isDir": true},
                    {"name": "tuesday", "isDir": true},
                ]},
            },
            "items": {},
        }),
    };

    let outcome = lookup_in_both_apps(&lookup, |versions| {
        versions
            .configure("/srv", "/backup/srv", SnapshotLayout::Direct)
            .unwrap();
        versions
            .configure("/srv/data", "/srv/history", SnapshotLayout::Direct)
            .unwrap();
    });

    assert_eq!(outcome["listed"], json!(["file:///srv/history"]));
    assert_eq!(outcome["versions"], json!([]));
    assert_eq!(outcome["message"], NO_VERSIONS_FOUND);
    assert_eq!(outcome["configured"].as_array().map(Vec::len), Some(2));
}

/// A folder is looked up inside itself; in a Snapper collection its
/// files are in `<id>/snapshot`.
///
/// parity: PROP-032
#[test]
fn a_folder_is_found_in_snapper_snapshots_next_to_it() {
    let lookup = Lookup {
        uri: "file:///home/demo",
        kind: ItemKind::Folder,
        tree: json!({
            "collections": {
                "file:///home/demo/.snapshots": {"entries": [
                    {"name": "1", "isDir": true},
                    {"name": "2", "isDir": true},
                ]},
            },
            "items": {
                "file:///home/demo/.snapshots/1/snapshot": {"name": "snapshot", "isDir": true},
                "file:///home/demo/.snapshots/2/snapshot": {"name": "snapshot", "isDir": true},
            },
        }),
    };

    let outcome = lookup_in_both_apps(&lookup, |_| {});

    assert_eq!(
        outcome["listed"],
        json!([
            "file:///home/demo/.snapshot",
            "file:///home/demo/.zfs/snapshot",
            "file:///home/demo/.snapshots",
        ])
    );
    assert_eq!(labels(&outcome), ["2", "1"]);
    assert_eq!(
        outcome["versions"][0]["snapshotRoot"],
        "file:///home/demo/.snapshots/2/snapshot"
    );
}

/// parity: PROP-032
#[test]
fn at_most_one_hundred_versions_are_returned_and_marked_truncated() {
    let snapshots: Vec<Value> = (0..=100)
        .map(|number| json!({"name": format!("s{number:03}"), "isDir": true}))
        .collect();
    let items: serde_json::Map<String, Value> = (0..=100)
        .map(|number| {
            let uri = format!("file:///srv/data/.snapshot/s{number:03}/a.txt");
            (uri, json!({"name": "a.txt"}))
        })
        .collect();
    let lookup = Lookup {
        uri: "file:///srv/data/a.txt",
        kind: ItemKind::File,
        tree: json!({
            "collections": {"file:///srv/data/.snapshot": {"entries": snapshots}},
            "items": items,
        }),
    };

    let outcome = lookup_in_both_apps(&lookup, |_| {});

    assert_eq!(outcome["versions"].as_array().map(Vec::len), Some(100));
    assert_eq!(outcome["truncated"], true);
    assert_eq!(outcome["listed"], json!(["file:///srv/data/.snapshot"]));
    assert_eq!(labels(&outcome)[0], "s099");
}

/// parity: PROP-032
#[test]
fn at_most_eight_warnings_are_kept() {
    let snapshots: Vec<Value> = (0..10)
        .map(|number| json!({"name": format!("day{number}"), "isDir": true}))
        .collect();
    let items: serde_json::Map<String, Value> = (0..10)
        .map(|number| {
            let uri = format!("file:///srv/data/.snapshot/day{number}/a.txt");
            (uri, json!({"error": "Permission denied"}))
        })
        .collect();
    let lookup = Lookup {
        uri: "file:///srv/data/a.txt",
        kind: ItemKind::File,
        tree: json!({
            "collections": {"file:///srv/data/.snapshot": {"entries": snapshots}},
            "items": items,
        }),
    };

    let outcome = lookup_in_both_apps(&lookup, |_| {});

    assert_eq!(outcome["warnings"].as_array().map(Vec::len), Some(8));
    assert_eq!(outcome["warnings"][0], "day0: Permission denied");
    assert_eq!(outcome["message"], NO_VERSIONS_FOUND);
}

/// Ported from `desktop/tests/snapshot_meta.test.cjs` ("Never disguise
/// mtime as snapshot date"): a version's date comes from its snapshot's
/// name, and a name without one has no date even though the snapshot
/// folder's modification time is known.
///
/// parity: PROP-020, PROP-032
#[test]
fn a_version_is_dated_by_its_snapshot_name_never_by_its_modification_time() {
    let temporary = tempfile::tempdir().unwrap();
    let versions = PreviousVersions::new(temporary.path());
    let provider = TreeProvider::new(json!({
        "collections": {"file:///srv/data/.snapshot": {"entries": [
            {"name": "manual", "isDir": true, "modified": 1_788_658_800},
            {"name": "auto-2026-09-04_16-30", "isDir": true, "modified": 1_788_658_800},
        ]}},
        "items": {
            "file:///srv/data/.snapshot/manual/a.txt": {"name": "a.txt"},
            "file:///srv/data/.snapshot/auto-2026-09-04_16-30/a.txt": {"name": "a.txt"},
        },
    }));

    let list = versions
        .find_versions(&provider, "/srv/data/a.txt", ItemKind::File, &Cancellation::new())
        .unwrap();

    let manual = &list.versions[0];
    let dated = &list.versions[1];
    assert_eq!(manual.label, "manual");
    assert_eq!(manual.snapshot_modified, Some(1_788_658_800));
    assert_eq!(manual.date(), None);
    assert_eq!(
        dated.date().map(|date| date.iso_8601()).as_deref(),
        Some("2026-09-04T16:30")
    );
}

/// A provider whose every query cancels the lookup, as the user closing
/// the Previous versions tab would.
struct CancellingProvider;

impl SnapshotProvider for CancellingProvider {
    fn list_collection(
        &self,
        _: &str,
        _: usize,
        cancel: &Cancellation,
    ) -> Result<CollectionListing, EntryError> {
        cancel.cancel();
        Err(EntryError::Cancelled)
    }

    fn inspect(&self, _: &str, cancel: &Cancellation) -> Result<Entry, EntryError> {
        cancel.cancel();
        Err(EntryError::Cancelled)
    }
}

/// parity: PROP-032
#[test]
fn a_cancelled_lookup_stops_instead_of_reporting_warnings() {
    let temporary = tempfile::tempdir().unwrap();
    let versions = PreviousVersions::new(temporary.path());

    let outcome = versions.find_versions(
        &CancellingProvider,
        "smb://nas/share/a.txt",
        ItemKind::File,
        &Cancellation::new(),
    );

    assert!(matches!(outcome, Err(VersionsError::Cancelled)), "{outcome:?}");
}

/// A collection a lookup could read stays read-only for the session, even
/// once it is no longer configured.
///
/// parity: PROP-024, PROP-032
#[test]
fn a_collection_read_by_a_lookup_becomes_read_only() {
    let temporary = tempfile::tempdir().unwrap();
    let versions = PreviousVersions::new(temporary.path());
    versions
        .configure("/srv/data", "/srv/history", SnapshotLayout::Direct)
        .unwrap();
    let provider = TreeProvider::new(json!({
        "collections": {"file:///srv/history": {"entries": []}},
        "items": {},
    }));
    versions
        .find_versions(&provider, "/srv/data/a.txt", ItemKind::File, &Cancellation::new())
        .unwrap();

    versions.remove_source("/srv/data").unwrap();

    assert!(versions.sources().is_empty());
    assert!(matches!(
        versions.check_writable("/srv/history/monday/a.txt"),
        Err(VersionsError::ReadOnly)
    ));
    assert_eq!(versions.snapshot_roots(), ["file:///srv/history"]);
}
