// SPDX-License-Identifier: AGPL-3.0-only
//! Fixtures shared by the unit tests of the search module.
//!
//! [`listed_file`] and [`listed_folder`] build items as the `entry` helper
//! of `v2.0.0:desktop/tests/test_v05.py` does, [`ScannedShare`] is the set-up of
//! its `IndexTests` (a cache with one SMB root whose scan has begun), and
//! [`LocalRoot`] the set-up of its `LiveTests` without the service.
//!
//! `tests/search_support` has public-API counterparts of [`found_names`],
//! [`root_state`], the wait loop of [`wait_until`] and [`LocalRoot`]
//! (`IndexedFolder` there). Both copies are needed: the unit tests here
//! use crate-private items, such as `begin_scan`, `store_scanned`, lowered
//! [`ServiceLimits`] and the service state, which integration tests cannot
//! reach, and a unit test cannot include a module of `tests/`.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::index::SearchIndex;
use super::limits::ServiceLimits;
use super::query::{SearchHit, SearchQuery};
use super::reader::GioFolderReader;
use super::root::{Caching, HiddenItems, IndexRoot, ScanGeneration};
use super::scan::{ListedItem, ScanOutcome};
use super::service::{IndexService, IndexSettings};
use crate::entry::EntryKind;
use crate::location::file_uri;

/// The share `IndexTests` index.
pub(super) const SHARE: &str = "smb://nas/share";

/// How long [`wait_until`] waits.
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// The pause between two looks in [`wait_until`], as in Python's `until`.
const WAIT_PAUSE: Duration = Duration::from_millis(80);

/// A 12-byte file `name` in `folder` (`entry(folder, name)` in Python).
pub(super) fn listed_file(folder: &str, name: &str) -> ListedItem {
    ListedItem {
        uri: format!("{}/{name}", folder.trim_end_matches('/')),
        name: name.to_owned(),
        kind: EntryKind::File,
        is_dir: false,
        is_hidden: false,
        is_symlink: false,
        is_virtual: false,
        size: Some(12),
        modified: Some(1),
        type_label: "File".to_owned(),
    }
}

/// A folder `name` in `folder` (`entry(folder, name, True)` in Python).
pub(super) fn listed_folder(folder: &str, name: &str) -> ListedItem {
    ListedItem {
        kind: EntryKind::Directory,
        is_dir: true,
        size: None,
        type_label: "Folder".to_owned(),
        ..listed_file(folder, name)
    }
}

/// Opens a cache in `directory`.
pub(super) fn open_index(directory: &TempDir) -> SearchIndex {
    SearchIndex::open(directory.path()).expect("a cache in a new temporary directory opens")
}

/// The hits of a search of every cached folder for `text`.
pub(super) fn search(index: &SearchIndex, text: &str) -> Vec<SearchHit> {
    let results = index.search(&SearchQuery::new(text), None);
    results.expect("the search runs").hits
}

/// The names of the hits of [`search`].
pub(super) fn found_names(index: &SearchIndex, text: &str) -> Vec<String> {
    let hits = search(index, text);
    hits.into_iter().map(|hit| hit.name).collect()
}

/// The root `uri` as the cache status lists it.
pub(super) fn root_state(index: &SearchIndex, uri: &str) -> IndexRoot {
    let roots = index.roots().expect("the roots can be read");
    let root = roots.into_iter().find(|root| root.uri == uri);
    root.unwrap_or_else(|| panic!("{uri} is a root"))
}

/// Waits until `done` holds, calling `step` before each look (`until` in
/// `LiveTests`). The test fails, naming `what`, after ten seconds.
pub(super) fn wait_until(what: &str, mut step: impl FnMut(), done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        step();
        if done() {
            return;
        }
        thread::sleep(WAIT_PAUSE);
    }
    panic!("timed out waiting until {what}");
}

/// A cache in a temporary directory with [`SHARE`] enabled and a scan of it
/// begun (`IndexTests.setUp`).
pub(super) struct ScannedShare {
    /// Keeps the cache until the test ends.
    pub(super) directory: TempDir,
    pub(super) index: SearchIndex,
    pub(super) generation: ScanGeneration,
}

impl ScannedShare {
    pub(super) fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary cache directory");
        let index = open_index(&directory);
        index
            .configure(SHARE, Caching::Enabled, "", HiddenItems::Skip)
            .expect("the share can be indexed");
        let generation = index.begin_scan(SHARE).expect("an enabled root can be scanned");
        Self {
            directory,
            index,
            generation,
        }
    }

    /// Stores `items` and completes the scan (`put` in `IndexTests`).
    pub(super) fn store(&self, items: &[ListedItem]) {
        self.index
            .store_scanned(SHARE, &self.generation, items)
            .expect("the batch is stored");
        self.index
            .finish_scan(SHARE, &self.generation, &ScanOutcome::Complete)
            .expect("the scan finishes");
    }

    /// Makes the cached children of `folder` exactly `items` and returns
    /// the new folders among them (`replace_directory` in Python).
    pub(super) fn replace(&self, folder: &str, items: &[ListedItem]) -> Vec<String> {
        let new_folders = self.index.replace_folder_contents(SHARE, folder, items);
        new_folders.expect("the folder's children are replaced")
    }

    /// The names a search of every cached folder for `text` finds.
    pub(super) fn found_names(&self, text: &str) -> Vec<String> {
        found_names(&self.index, text)
    }
}

/// A local folder chosen for indexing, with the cache next to it
/// (`LiveTests.setUp` without the service).
pub(super) struct LocalRoot {
    /// Keeps the folder and the cache until the test ends.
    _base: TempDir,
    pub(super) tree: PathBuf,
    pub(super) root: String,
    pub(super) index: SearchIndex,
}

impl LocalRoot {
    pub(super) fn new() -> Self {
        let base = tempfile::tempdir().expect("temporary folder");
        let tree = base.path().join("files");
        fs::create_dir(&tree).expect("the indexed folder is created");
        let root = file_uri(&tree);
        let index = SearchIndex::open(&base.path().join("index")).expect("the cache opens");
        index
            .configure(&root, Caching::Enabled, "", HiddenItems::Skip)
            .expect("a local folder can be indexed");
        Self {
            _base: base,
            tree,
            root,
            index,
        }
    }

    /// Starts a service with `limits`.
    pub(super) fn start_service(&self, limits: ServiceLimits) -> IndexService {
        let reader = Box::new(GioFolderReader);
        let listener = Box::new(|| {});
        let service = IndexService::start_with_limits(self.index.clone(), reader, listener, limits);
        service.expect("the service starts")
    }

    /// The root as the cache status lists it.
    pub(super) fn state(&self) -> IndexRoot {
        root_state(&self.index, &self.root)
    }

    /// Ticks `service` with the default settings until `done` holds for
    /// the root.
    pub(super) fn tick_until(&self, service: &IndexService, what: &str, done: impl Fn(&IndexRoot) -> bool) {
        let tick = || service.tick(&IndexSettings::default()).expect("the tick runs");
        wait_until(what, tick, || done(&self.state()));
    }
}
