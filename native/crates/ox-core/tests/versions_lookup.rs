// SPDX-License-Identifier: AGPL-3.0-only
//! Finding the previous versions of an item (PROP-032).
//!
//! `desktop/tests` has no backend test of the lookup, so each scenario
//! here runs twice: through [`PreviousVersions::find_versions`] and
//! through `PreviousVersions.list` in `desktop/previous_versions.py`, over
//! the same simulated snapshot folders, and the two outcomes must match.
//! The GIO provider is then checked on real local snapshot folders. Every
//! file is inside a temporary directory.

mod python_support;

use std::cell::RefCell;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;

use ox_core::entry::{entry_from_info, Entry, EntryError};
use ox_core::location::{file_uri, ItemKind};
use ox_core::transfer::Cancellation;
use ox_core::versions::{
    CollectionListing, GioSnapshotProvider, PreviousVersions, SnapshotLayout, SnapshotProvider, VersionList,
    VersionsError, NO_VERSIONS_FOUND,
};
use python_support::run_python;
use serde_json::{json, Value};

/// Runs `PreviousVersions.list` over a simulated tree and prints the
/// outcome as JSON. `sys.argv[1]` is the settings directory and
/// `sys.argv[2]` a file holding the lookup: `uri`, `isDir` and `tree`.
const PYTHON_LISTS_VERSIONS: &str = r"
import json, sys
from pathlib import Path
from previous_versions import PreviousVersions

class Cancel:
    def check(self):
        pass

class Missing(Exception):
    code = 'not-found'

class TreeProvider:
    def __init__(self, tree):
        self.tree, self.listed = tree, []
    def children(self, uri, cancel, limit=100):
        self.listed.append(uri)
        collection = self.tree['collections'].get(uri)
        if collection is None:
            raise Missing('No such folder')
        if 'error' in collection:
            raise OSError(collection['error'])
        return collection['entries'][:limit], len(collection['entries']) > limit
    def inspect(self, uri, cancel):
        item = self.tree['items'].get(uri)
        if item is None:
            raise Missing('Not present in this snapshot.')
        if 'error' in item:
            raise OSError(item['error'])
        return dict(item, uri=uri)

lookup = json.loads(Path(sys.argv[2]).read_text())
provider = TreeProvider(lookup['tree'])
result = PreviousVersions(Path(sys.argv[1]), provider).list(lookup['uri'], lookup['isDir'], Cancel())
versions = [{key: version[key] for key in ('uri', 'label', 'snapshotRoot', 'source', 'snapshotModified')}
            for version in result['versions']]
print(json.dumps({'listed': provider.listed, 'versions': versions, 'sources': result['sources'],
                  'warnings': result['warnings'], 'truncated': result['truncated'],
                  'message': result['message'], 'configured': result['configured']}))
";

/// One lookup: the item and the simulated snapshot folders.
struct Lookup {
    uri: &'static str,
    kind: ItemKind,
    /// `collections`: collection URI to `{"entries": [...]}` or
    /// `{"error": message}`; `items`: item URI to its metadata or
    /// `{"error": message}`. Anything not listed is missing.
    tree: Value,
}

/// A [`SnapshotProvider`] over a simulated tree, in the format the Python
/// script reads, that records which collections were listed.
struct TreeProvider {
    tree: Value,
    listed: RefCell<Vec<String>>,
}

impl SnapshotProvider for TreeProvider {
    fn list_collection(
        &self,
        uri: &str,
        limit: usize,
        _: &Cancellation,
    ) -> Result<CollectionListing, EntryError> {
        self.listed.borrow_mut().push(uri.to_owned());
        let Some(collection) = self.tree["collections"].get(uri) else {
            return Err(EntryError::NotFound("No such folder".into()));
        };
        if let Some(message) = collection["error"].as_str() {
            return Err(EntryError::Other(message.into()));
        }
        let entries = collection["entries"].as_array().expect("a list of entries");
        Ok(CollectionListing {
            snapshots: entries.iter().take(limit).map(fixture_entry).collect(),
            has_more: entries.len() > limit,
        })
    }

    fn inspect(&self, uri: &str, _: &Cancellation) -> Result<Entry, EntryError> {
        let Some(item) = self.tree["items"].get(uri) else {
            return Err(EntryError::NotFound("Not present in this snapshot.".into()));
        };
        if let Some(message) = item["error"].as_str() {
            return Err(EntryError::Other(message.into()));
        }
        Ok(fixture_entry(item))
    }
}

/// The entry GIO would report for a simulated item: `name`, `isDir`,
/// `symlink` and `modified` as in the Python fixtures.
fn fixture_entry(item: &Value) -> Entry {
    let is_symlink = item["symlink"].as_bool().unwrap_or(false);
    let file_type = if is_symlink {
        gio::FileType::SymbolicLink
    } else if item["isDir"].as_bool().unwrap_or(false) {
        gio::FileType::Directory
    } else {
        gio::FileType::Regular
    };
    let info = gio::FileInfo::new();
    info.set_file_type(file_type);
    info.set_is_symlink(is_symlink);
    info.set_display_name(item["name"].as_str().unwrap_or("item"));
    if let Some(modified) = item["modified"].as_u64() {
        info.set_attribute_uint64("time::modified", modified);
    }
    entry_from_info(&gio::File::for_path("/simulated"), &info)
}

/// The outcome of a lookup in the JSON shape the Python script prints.
fn outcome_as_json(list: &VersionList, listed: &[String]) -> Value {
    let versions: Vec<Value> = list
        .versions
        .iter()
        .map(|version| {
            json!({
                "uri": version.entry.uri,
                "label": version.label,
                "snapshotRoot": version.snapshot_root,
                "source": version.collection,
                "snapshotModified": version.snapshot_modified,
            })
        })
        .collect();
    json!({
        "listed": listed,
        "versions": versions,
        "sources": list.collections,
        "warnings": list.warnings,
        "truncated": list.is_truncated,
        "message": list.message().unwrap_or_default(),
        "configured": list.configured,
    })
}

/// Runs `lookup` with the Rust service in `settings`.
fn rust_lookup(versions: &PreviousVersions, lookup: &Lookup) -> Value {
    let provider = TreeProvider {
        tree: lookup.tree.clone(),
        listed: RefCell::default(),
    };
    let list = versions
        .find_versions(&provider, lookup.uri, lookup.kind, &Cancellation::new())
        .expect("the lookup completes");
    outcome_as_json(&list, &provider.listed.into_inner())
}

/// Runs `lookup` with the Python service in `settings`.
fn python_lookup(settings: &Path, lookup: &Lookup) -> Value {
    let file = settings.with_extension("lookup.json");
    let request = json!({
        "uri": lookup.uri,
        "isDir": lookup.kind == ItemKind::Folder,
        "tree": lookup.tree,
    });
    fs::write(&file, request.to_string()).expect("write the lookup");
    let printed = run_python(PYTHON_LISTS_VERSIONS, &[settings, &file]);
    serde_json::from_str(&printed).expect("Python printed JSON")
}

/// Runs `lookup` in both apps, checks that they agree, and returns the
/// outcome. `configure` prepares the snapshot sources first.
fn lookup_in_both_apps(lookup: &Lookup, configure: impl FnOnce(&PreviousVersions)) -> Value {
    let temporary = tempfile::tempdir().unwrap();
    let settings = temporary.path().join("winspace");
    let versions = PreviousVersions::new(&settings);
    configure(&versions);

    let from_rust = rust_lookup(&versions, lookup);
    let from_python = python_lookup(&settings, lookup);

    assert_eq!(from_rust, from_python);
    from_rust
}

/// The labels of the versions in an outcome.
fn labels(outcome: &Value) -> Vec<&str> {
    let versions = outcome["versions"].as_array().expect("a list of versions");
    versions
        .iter()
        .map(|version| version["label"].as_str().expect("a label"))
        .collect()
}

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
    let provider = TreeProvider {
        tree: json!({
            "collections": {"file:///srv/data/.snapshot": {"entries": [
                {"name": "manual", "isDir": true, "modified": 1_788_658_800},
                {"name": "auto-2026-09-04_16-30", "isDir": true, "modified": 1_788_658_800},
            ]}},
            "items": {
                "file:///srv/data/.snapshot/manual/a.txt": {"name": "a.txt"},
                "file:///srv/data/.snapshot/auto-2026-09-04_16-30/a.txt": {"name": "a.txt"},
            },
        }),
        listed: RefCell::default(),
    };

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
    let provider = TreeProvider {
        tree: json!({"collections": {"file:///srv/history": {"entries": []}}, "items": {}}),
        listed: RefCell::default(),
    };
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

/// A project folder with a document in two dated snapshots. The newest
/// snapshot is also reachable through a `latest` link, and in a third
/// snapshot the document is a link.
fn project_with_snapshots(root: &Path) -> std::path::PathBuf {
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
