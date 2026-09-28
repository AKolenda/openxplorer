// SPDX-License-Identifier: AGPL-3.0-only
//! Shared by the `versions_lookup` tests: simulated snapshot folders in
//! the JSON format a Python script reads too, a [`TreeProvider`] that
//! serves them to the Rust lookup, and [`lookup_in_both_apps`], which runs
//! one lookup through [`PreviousVersions::find_versions`] and through
//! `PreviousVersions.list` in `desktop/previous_versions.py` and checks
//! that the outcomes match.
//!
//! A test crate that includes this module also declares `mod
//! python_support;`, which runs the Python side.
#![allow(
    dead_code,
    reason = "each test crate that includes this module uses a different part of it"
)]

use std::cell::RefCell;
use std::fs;
use std::path::Path;

use ox_core::entry::{entry_from_info, Entry, EntryError};
use ox_core::location::ItemKind;
use ox_core::transfer::Cancellation;
use ox_core::versions::{CollectionListing, PreviousVersions, SnapshotProvider, VersionList};
use serde_json::{json, Value};

use crate::python_support::run_python;

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
pub struct Lookup {
    /// The item whose versions are wanted.
    pub uri: &'static str,
    /// Whether the item is a file or a folder.
    pub kind: ItemKind,
    /// `collections`: collection URI to `{"entries": [...]}` or
    /// `{"error": message}`; `items`: item URI to its metadata or
    /// `{"error": message}`. Anything not listed is missing.
    pub tree: Value,
}

/// A [`SnapshotProvider`] over a simulated tree, in the format the Python
/// script reads, that records which collections were listed.
pub struct TreeProvider {
    tree: Value,
    listed: RefCell<Vec<String>>,
}

impl TreeProvider {
    /// A provider serving `tree`, in the format of [`Lookup::tree`].
    pub fn new(tree: Value) -> Self {
        Self {
            tree,
            listed: RefCell::default(),
        }
    }

    /// The collections listed so far, in order.
    pub fn into_listed(self) -> Vec<String> {
        self.listed.into_inner()
    }
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
            return Err(EntryError::Failed(message.into()));
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
            return Err(EntryError::Failed(message.into()));
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

/// Runs `lookup` with the Rust service `versions`.
fn rust_lookup(versions: &PreviousVersions, lookup: &Lookup) -> Value {
    let provider = TreeProvider::new(lookup.tree.clone());
    let list = versions
        .find_versions(&provider, lookup.uri, lookup.kind, &Cancellation::new())
        .expect("the lookup completes");
    outcome_as_json(&list, &provider.into_listed())
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
pub fn lookup_in_both_apps(lookup: &Lookup, configure: impl FnOnce(&PreviousVersions)) -> Value {
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
pub fn labels(outcome: &Value) -> Vec<&str> {
    let versions = outcome["versions"].as_array().expect("a list of versions");
    versions
        .iter()
        .map(|version| version["label"].as_str().expect("a label"))
        .collect()
}
