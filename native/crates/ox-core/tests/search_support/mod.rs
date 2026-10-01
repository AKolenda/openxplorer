// SPDX-License-Identifier: AGPL-3.0-only
//! Helpers of the search service integration tests: a local folder indexed
//! by a service with the real GIO reader (the set-up of `LiveTests` in
//! `v2.0.0:desktop/tests/test_v05.py`), an SMB share held in memory, and waiting
//! for the service's ticks. Include it with `mod search_support;`.
//!
//! `src/search/fixtures.rs` has crate-private counterparts of
//! [`found_names`], [`root_state`], the wait loop of [`tick_until`] and
//! [`IndexedFolder`] (`LocalRoot` there). Both copies are needed: these
//! helpers use only the public API, as the app does, and cannot reach that
//! module, while the unit tests there need crate-private items, such as
//! lowered limits and the service state, that this crate cannot use.
#![allow(
    dead_code,
    reason = "each test crate that includes this module uses a different part of it"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use gio::prelude::*;
use ox_core::entry::{EntryError, EntryKind};
use ox_core::location::file_uri;
use ox_core::search::{
    Caching, FolderReader, GioFolderReader, HiddenItems, IndexRoot, IndexService, IndexSettings, ListedItem,
    RootStatus, SearchError, SearchIndex, SearchQuery,
};
use tempfile::TempDir;

/// How long [`tick_until`] waits; Python's `until` waited five seconds.
const TICK_TIMEOUT: Duration = Duration::from_secs(10);

/// The pause between two ticks in [`tick_until`], as in Python's `until`.
const TICK_PAUSE: Duration = Duration::from_millis(80);

/// Ticks `service` with `settings` until `done` holds (`until` in
/// `LiveTests`). The test fails, naming `what`, after ten seconds.
pub fn tick_until(service: &IndexService, settings: &IndexSettings, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + TICK_TIMEOUT;
    while Instant::now() < deadline {
        service.tick(settings).expect("the tick runs");
        if done() {
            return;
        }
        thread::sleep(TICK_PAUSE);
    }
    panic!("timed out waiting until {what}");
}

/// Ticks `service` for `duration`, for tests that show that something does
/// not happen.
pub fn tick_for(service: &IndexService, settings: &IndexSettings, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        service.tick(settings).expect("the tick runs");
        thread::sleep(TICK_PAUSE);
    }
}

/// The names a search of every cached folder for `text` finds.
pub fn found_names(index: &SearchIndex, text: &str) -> Vec<String> {
    let results = index
        .search(&SearchQuery::new(text), None)
        .expect("the search runs");
    results.hits.into_iter().map(|hit| hit.name).collect()
}

/// The root `uri` as the cache status lists it; `None` when it has none.
pub fn find_root(index: &SearchIndex, uri: &str) -> Option<IndexRoot> {
    let roots = index.roots().expect("the roots can be read");
    roots.into_iter().find(|root| root.uri == uri)
}

/// The root `uri`; the test fails when it has none.
pub fn root_state(index: &SearchIndex, uri: &str) -> IndexRoot {
    find_root(index, uri).unwrap_or_else(|| panic!("{uri} is a root"))
}

/// Ticks `service` with the default settings until the root `uri` reaches
/// `status`.
pub fn wait_for_status(service: &IndexService, uri: &str, status: RootStatus) {
    let what = format!("{uri} is {}", status.as_str());
    let reached = || find_root(service.index(), uri).is_some_and(|root| root.status == status);
    tick_until(service, &IndexSettings::default(), &what, reached);
}

/// A cache in a new temporary directory, kept until the test ends.
pub struct TemporaryCache {
    /// Keeps the cache until the test ends.
    _directory: TempDir,
    /// The cache.
    pub index: SearchIndex,
}

impl TemporaryCache {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary cache directory");
        let index = SearchIndex::open(directory.path()).expect("a new cache opens");
        Self {
            _directory: directory,
            index,
        }
    }

    /// Starts a service that reads folders with `reader`.
    pub fn start_service(&self, reader: impl FolderReader + 'static) -> IndexService {
        IndexService::start(self.index.clone(), reader, || {}).expect("the service starts")
    }

    /// Starts a service for `share` and indexes the whole share, ticking
    /// with the default settings.
    pub fn index_share(&self, share: &MemoryShare) -> IndexService {
        self.index_share_ticking(share, &IndexSettings::default())
    }

    /// [`TemporaryCache::index_share`], ticking with `settings`. A root's
    /// first network check is timed by the interval of the tick that first
    /// sees the root, so tests of network checks tick with their settings
    /// from the start.
    pub fn index_share_ticking(&self, share: &MemoryShare, settings: &IndexSettings) -> IndexService {
        let service = self.start_service(share.clone());
        service
            .configure(MemoryShare::URI, Caching::Enabled, "", HiddenItems::Skip)
            .expect("the share can be indexed");
        let is_ready = || root_state(&self.index, MemoryShare::URI).status == RootStatus::Ready;
        tick_until(&service, settings, "the share is indexed", is_ready);
        service
    }
}

/// A local folder chosen for indexing, the cache next to it, and a service
/// that scanned it once (`LiveTests.setUp`).
pub struct IndexedFolder {
    /// Keeps the folder and the cache until the test ends.
    pub base: TempDir,
    /// The indexed folder.
    pub tree: PathBuf,
    /// Its URI, the root.
    pub root: String,
    /// The service that keeps the root up to date.
    pub service: IndexService,
}

impl IndexedFolder {
    /// Hidden items are left out, as in `LiveTests`.
    pub fn new() -> Self {
        Self::with_hidden_items(HiddenItems::Skip)
    }

    pub fn with_hidden_items(hidden_items: HiddenItems) -> Self {
        let base = tempfile::tempdir().expect("temporary folder");
        let tree = base.path().join("files");
        fs::create_dir(&tree).expect("the indexed folder is created");
        let index = SearchIndex::open(&base.path().join("index")).expect("the cache opens");
        let service = IndexService::start(index, GioFolderReader, || {}).expect("the service starts");
        let root = file_uri(&tree);
        service
            .configure(&root, Caching::Enabled, "", hidden_items)
            .expect("a local folder can be indexed");
        let folder = Self {
            base,
            tree,
            root,
            service,
        };
        folder.tick_until("the first scan is ready", || {
            folder.state().status == RootStatus::Ready
        });
        folder
    }

    /// The root as the cache status lists it.
    pub fn state(&self) -> IndexRoot {
        root_state(self.service.index(), &self.root)
    }

    /// The names a search of every cached folder for `text` finds.
    pub fn found_names(&self, text: &str) -> Vec<String> {
        found_names(self.service.index(), text)
    }

    /// The names a search of every cached folder for `text` finds when
    /// hidden items are shown.
    pub fn found_names_including_hidden(&self, text: &str) -> Vec<String> {
        let query = SearchQuery {
            hidden_items: HiddenItems::Include,
            ..SearchQuery::new(text)
        };
        let results = self.service.index().search(&query, None);
        let hits = results.expect("the search runs").hits;
        hits.into_iter().map(|hit| hit.name).collect()
    }

    /// Ticks the service with the default settings until `done` holds.
    pub fn tick_until(&self, what: &str, done: impl Fn() -> bool) {
        tick_until(&self.service, &IndexSettings::default(), what, done);
    }
}

/// How the folders of a [`MemoryShare`] answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShareAnswer {
    /// With their items.
    Items,
    /// With "not mounted", as a share that needs sign-in does.
    NotMounted,
    /// Not until the read is cancelled, as a share that stopped answering.
    Stalled,
}

/// The folders of a [`MemoryShare`] and how they answer.
#[derive(Debug)]
struct ShareContents {
    answer: ShareAnswer,
    folders: BTreeMap<String, Vec<ListedItem>>,
}

/// An SMB share held in memory, so the tests never reach a network, with
/// the folders below [`MemoryShare::URI`].
#[derive(Debug, Clone)]
pub struct MemoryShare {
    contents: Arc<Mutex<ShareContents>>,
}

impl MemoryShare {
    /// The share's root folder.
    pub const URI: &'static str = "smb://nas/share";

    /// An empty share that answers.
    pub fn new() -> Self {
        let folders = BTreeMap::from([(Self::URI.to_owned(), Vec::new())]);
        let contents = ShareContents {
            answer: ShareAnswer::Items,
            folders,
        };
        Self {
            contents: Arc::new(Mutex::new(contents)),
        }
    }

    /// An empty share that is not mounted until the user signs in.
    pub fn needing_sign_in() -> Self {
        let share = Self::new();
        share.sign_out();
        share
    }

    /// The user signed in: the share answers.
    pub fn sign_in(&self) {
        self.contents().answer = ShareAnswer::Items;
    }

    /// The user signed out: every folder is "not mounted".
    pub fn sign_out(&self) {
        self.contents().answer = ShareAnswer::NotMounted;
    }

    /// The share stops answering until a read is cancelled.
    pub fn stall(&self) {
        self.contents().answer = ShareAnswer::Stalled;
    }

    /// Adds a file `name` to `folder` and returns its URI.
    pub fn add_file(&self, folder: &str, name: &str) -> String {
        self.add(folder, name, EntryKind::File)
    }

    /// Adds an empty folder `name` to `folder` and returns its URI.
    pub fn add_folder(&self, folder: &str, name: &str) -> String {
        self.add(folder, name, EntryKind::Directory)
    }

    /// Adds a folder `name` to `folder` whose own listing fails, as a
    /// NAS's `@eaDir` or `#recycle` folder can, and returns its URI.
    pub fn add_unreadable_folder(&self, folder: &str, name: &str) -> String {
        let uri = self.add_folder(folder, name);
        self.contents().folders.remove(&uri);
        uri
    }

    /// Deletes the item `uri` from `folder`, and its contents if it is a
    /// folder.
    pub fn remove(&self, folder: &str, uri: &str) {
        let mut contents = self.contents();
        contents.folders.remove(uri);
        let parent = contents
            .folders
            .get_mut(folder)
            .expect("the parent folder exists");
        parent.retain(|item| item.uri != uri);
    }

    fn add(&self, folder: &str, name: &str, kind: EntryKind) -> String {
        let item = listed_item(folder, name, kind);
        let uri = item.uri.clone();
        let mut contents = self.contents();
        if kind == EntryKind::Directory {
            contents.folders.insert(uri.clone(), Vec::new());
        }
        let parent = contents
            .folders
            .get_mut(folder)
            .expect("the parent folder exists");
        parent.push(item);
        uri
    }

    fn contents(&self) -> MutexGuard<'_, ShareContents> {
        self.contents
            .lock()
            .expect("no test thread panicked while holding the share")
    }

    /// Waits until `cancellable` is cancelled, as a read of a share that
    /// stopped answering does.
    fn wait_for_cancel(cancellable: &gio::Cancellable) -> Result<(), SearchError> {
        while !cancellable.is_cancelled() {
            thread::sleep(Duration::from_millis(10));
        }
        Err(SearchError::Cancelled)
    }
}

impl FolderReader for MemoryShare {
    fn read_folder(
        &self,
        folder: &str,
        _hidden_items: HiddenItems,
        cancellable: &gio::Cancellable,
        receive: &mut dyn FnMut(Vec<ListedItem>) -> Result<(), SearchError>,
    ) -> Result<(), SearchError> {
        let (answer, items) = {
            let contents = self.contents();
            (contents.answer, contents.folders.get(folder).cloned())
        };
        match answer {
            ShareAnswer::Items => {
                let items = items.ok_or_else(|| EntryError::NotFound(format!("No such folder: {folder}")))?;
                receive(items)
            }
            ShareAnswer::NotMounted => {
                Err(EntryError::NotMounted("Location is not mounted".to_owned()).into())
            }
            ShareAnswer::Stalled => Self::wait_for_cancel(cancellable),
        }
    }
}

/// An item of `kind` named `name` in `folder`.
fn listed_item(folder: &str, name: &str, kind: EntryKind) -> ListedItem {
    let is_dir = kind == EntryKind::Directory;
    ListedItem {
        uri: format!("{folder}/{name}"),
        name: name.to_owned(),
        kind,
        is_dir,
        is_hidden: false,
        is_symlink: false,
        is_virtual: false,
        size: (!is_dir).then_some(12),
        modified: Some(1),
        type_label: if is_dir { "Folder" } else { "File" }.to_owned(),
    }
}
