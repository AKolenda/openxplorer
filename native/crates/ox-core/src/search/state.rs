// SPDX-License-Identifier: AGPL-3.0-only
//! The state the index service shares with its worker thread.
//!
//! Ports the attributes of `IndexService` in `desktop/index_service.py`,
//! its helpers `monitor` and `update_monitoring`, and the job bookkeeping
//! of `_run`, `_update` and `_release`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use gio::prelude::*;

use super::error::SearchError;
use super::index::SearchIndex;
use super::limits::ServiceLimits;
use super::ownership::Ownership;
use super::policy::{IndexScope, RootStorage};
use super::reader::FolderReader;
use super::root::{Monitoring, UpdateMode};
use super::service::{AutoIndex, IndexSettings};
use super::text::host_of;
use super::watch::LocalWatch;

/// Called after anything the Search settings or results show changed
/// (the `cacheChanged` event of the Python app).
pub(crate) type ChangeListener = Box<dyn Fn() + Send + Sync>;

/// Shown for a local root when inotify cannot be used at all.
const NO_INOTIFY_MESSAGE: &str = "inotify unavailable; using incremental checks.";

/// Most changed folders waiting for live updates.
const MAX_DIRTY_FOLDERS: usize = 8192;

/// A folder below a root, the unit of a live update.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct FolderKey {
    pub(super) root: String,
    pub(super) folder: String,
}

/// Where the timed checks of one root continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PollCursor {
    /// Check the root folder itself first.
    RootFolder,
    /// Check the cached folders that sort after this URI.
    After(String),
}

/// The timed checks of one network or partly watched root (SRCH-030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NetworkPoll {
    pub(super) cursor: PollCursor,
    pub(super) next_at: Instant,
}

/// Everything the service and its worker change, behind one lock.
#[derive(Debug)]
pub(super) struct ServiceState {
    pub(super) closed: bool,
    pub(super) ownership: Ownership,
    pub(super) watch: Option<LocalWatch>,
    pub(super) settings: IndexSettings,
    /// The Auto-index setting at the previous tick, to notice a change.
    pub(super) auto_index_before: AutoIndex,
    /// Running or queued full scans, by root.
    pub(super) scans: HashMap<String, gio::Cancellable>,
    /// Running or queued live updates.
    pub(super) updates: HashMap<FolderKey, gio::Cancellable>,
    /// Folders that changed, with when they last changed.
    pub(super) dirty: HashMap<FolderKey, Instant>,
    /// Roots that need a full rescan, after an overflow.
    pub(super) forced_rescans: HashSet<String>,
    /// SMB hosts the user is signing out of.
    pub(super) paused_hosts: HashSet<String>,
    /// Why some directories of a root could not be watched.
    pub(super) failed_watches: HashMap<String, String>,
    /// Where the timed checks of each polled root are.
    pub(super) network_polls: HashMap<String, NetworkPoll>,
    /// Roots scanned at least once since start-up or since Auto-index was
    /// switched back on.
    pub(super) started: HashSet<String>,
}

impl ServiceState {
    /// A service that owns nothing yet.
    fn new(ownership: Ownership) -> Self {
        Self {
            closed: false,
            ownership,
            watch: None,
            settings: IndexSettings::default(),
            auto_index_before: AutoIndex::On,
            scans: HashMap::new(),
            updates: HashMap::new(),
            dirty: HashMap::new(),
            forced_rescans: HashSet::new(),
            paused_hosts: HashSet::new(),
            failed_watches: HashMap::new(),
            network_polls: HashMap::new(),
            started: HashSet::new(),
        }
    }

    /// Whether the SMB host of `uri` is paused for a sign-out.
    pub(super) fn is_paused(&self, uri: &str) -> bool {
        host_of(uri).is_some_and(|host| self.paused_hosts.contains(&host))
    }

    /// Ends the sign-out pause of the host of `uri`.
    pub(super) fn resume_host_of(&mut self, uri: &str) {
        if let Some(host) = host_of(uri) {
            self.paused_hosts.remove(&host);
        }
    }

    /// Cancels the scan and the live updates of `root`.
    pub(super) fn cancel_root(&self, root: &str) {
        if let Some(scan) = self.scans.get(root) {
            scan.cancel();
        }
        let updates = self.updates.iter().filter(|(key, _)| key.root == root);
        for (_, update) in updates {
            update.cancel();
        }
    }

    /// Cancels the full scans of roots on `host`. Live updates stop by
    /// themselves once they see the host paused.
    pub(super) fn cancel_scans_on_host(&self, host: &str) {
        let on_host = self
            .scans
            .iter()
            .filter(|(root, _)| host_of(root).as_deref() == Some(host));
        for (_, scan) in on_host {
            scan.cancel();
        }
    }

    /// Records that `folder` below `root` changed at `changed_at`.
    ///
    /// Beyond [`MAX_DIRTY_FOLDERS`] waiting folders, a full rescan of the
    /// root replaces its live updates (SRCH-029).
    pub(super) fn record_dirty_folder(&mut self, root: &str, folder: &str, changed_at: Instant) {
        if self.dirty.len() > MAX_DIRTY_FOLDERS {
            self.forced_rescans.insert(root.to_owned());
            self.dirty.retain(|key, _| key.root != root);
        }
        let key = FolderKey {
            root: root.to_owned(),
            folder: folder.to_owned(),
        };
        self.dirty.insert(key, changed_at);
    }

    /// Gives up ownership once the service closed and no job runs
    /// (`_release` in Python), so another process can take over.
    pub(super) fn release_if_idle(&mut self) {
        if self.closed && self.scans.is_empty() && self.updates.is_empty() {
            self.ownership.release();
        }
    }

    /// How many directories `root` watches.
    fn watch_count(&self, root: &str) -> usize {
        self.watch.as_ref().map_or(0, |watch| watch.count(root))
    }
}

/// What the service and its worker thread share.
pub(super) struct Shared {
    pub(super) index: SearchIndex,
    pub(super) reader: Box<dyn FolderReader>,
    listener: ChangeListener,
    pub(super) limits: ServiceLimits,
    state: Mutex<ServiceState>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Shared")
            .field("index", &self.index)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Shared {
    /// The shared parts of a new service.
    pub(super) fn new(
        index: SearchIndex,
        reader: Box<dyn FolderReader>,
        listener: ChangeListener,
        limits: ServiceLimits,
        ownership: Ownership,
    ) -> Self {
        Self {
            index,
            reader,
            listener,
            limits,
            state: Mutex::new(ServiceState::new(ownership)),
        }
    }

    /// Locks the state.
    ///
    /// Every change under the lock is a few map operations that leave the
    /// maps consistent, so the state of a thread that panicked is still
    /// usable and a poisoned lock is used as it is.
    pub(super) fn state(&self) -> MutexGuard<'_, ServiceState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Tells the app that the cache status or results may have changed.
    pub(super) fn notify(&self) {
        (self.listener)();
    }

    /// Whether the service closed.
    pub(super) fn is_closed(&self) -> bool {
        self.state().closed
    }

    /// What may be indexed below `root`.
    ///
    /// # Errors
    ///
    /// As [`IndexScope::current`]: a scan or update fails rather than
    /// index beyond the root's filesystem.
    pub(super) fn scope_of(&self, root: &str) -> Result<IndexScope, SearchError> {
        IndexScope::current(root, self.index.directory())
    }

    /// Watches `folder` of a local root (`monitor` in Python). Called
    /// before the folder is read, so a change during the read is not lost.
    /// Network roots are never push-watched, and nothing is watched while
    /// Auto-index is paused.
    pub(super) fn watch_folder(&self, root: &str, folder: &str, storage: RootStorage) {
        let mut state = self.state();
        if state.settings.auto_index == AutoIndex::Paused || storage == RootStorage::Network {
            return;
        }
        let failure = match &state.watch {
            Some(watch) => watch.add(root, folder).err().map(|error| error.to_string()),
            None => Some(NO_INOTIFY_MESSAGE.to_owned()),
        };
        if let Some(failure) = failure {
            state.failed_watches.insert(root.to_owned(), failure);
        }
    }

    /// Forgets the watches of `root` before a full scan sets them up again.
    pub(super) fn forget_watches(&self, root: &str) {
        let mut state = self.state();
        state.failed_watches.remove(root);
        if let Some(watch) = &state.watch {
            watch.remove_root(root);
        }
    }

    /// Records how changes below `root` reach the cache
    /// (`update_monitoring` in Python).
    pub(super) fn report_monitoring(&self, root: &str, storage: RootStorage) {
        let monitoring = {
            let state = self.state();
            let error = state.failed_watches.get(root).cloned().unwrap_or_default();
            let mode = match (state.settings.auto_index, storage) {
                (AutoIndex::Paused, _) => UpdateMode::Paused,
                (AutoIndex::On, RootStorage::Network) => UpdateMode::IncrementalNetworkChecks,
                (AutoIndex::On, RootStorage::Local) if !error.is_empty() => UpdateMode::LiveWithTimedFallback,
                (AutoIndex::On, RootStorage::Local) => UpdateMode::LiveLocalEvents,
            };
            Monitoring {
                mode,
                watch_count: state.watch_count(root),
                error,
            }
        };
        self.record_monitoring(root, &monitoring);
    }

    /// Records that a live update or timed check of `root` failed.
    ///
    /// Safety rule "a failed check keeps the last good data" (`_update` in
    /// Python, SRCH-030): only the status changes; an offline share must
    /// not erase what was found before.
    pub(super) fn report_failed_check(&self, root: &str, error: &str) {
        let watch_count = self.state().watch_count(root);
        let monitoring = Monitoring {
            mode: UpdateMode::OfflineChecks,
            watch_count,
            error: error.to_owned(),
        };
        self.record_monitoring(root, &monitoring);
    }

    /// Ends the bookkeeping of a full scan of `root`.
    pub(super) fn finish_scan_job(&self, root: &str, cancellable: &gio::Cancellable) {
        {
            let mut state = self.state();
            // A newer scan of the same root may already be queued.
            if state.scans.get(root) == Some(cancellable) {
                state.scans.remove(root);
            }
            state.release_if_idle();
        }
        self.notify();
    }

    /// Ends the bookkeeping of a live update.
    pub(super) fn finish_update_job(&self, key: &FolderKey) {
        {
            let mut state = self.state();
            state.updates.remove(key);
            state.release_if_idle();
        }
        self.notify();
    }

    /// Stores `monitoring` for `root`. A status that cannot be stored is
    /// shown stale until the next report; the cached entries are
    /// unaffected, so the failure is not worth stopping a scan for.
    fn record_monitoring(&self, root: &str, monitoring: &Monitoring) {
        let _ = self.index.set_monitoring(root, monitoring);
    }
}
