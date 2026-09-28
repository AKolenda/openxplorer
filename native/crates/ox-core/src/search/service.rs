// SPDX-License-Identifier: AGPL-3.0-only
//! The index coordinator: its worker thread and the one index owner.
//!
//! Ports the life cycle of `IndexService` in `desktop/index_service.py`:
//! `__init__`, `elect`, `refresh`, `update`, `changed` and `close`
//! (SRCH-025, SRCH-027). One elected process owns the crawler and the
//! watchers for every `OpenXplorer` window; the database is shared, and
//! other processes leave their requests in it for the owner. Neither the
//! crawler nor the event handling opens file contents or mounts anything.
//!
//! Scans and updates run one at a time on a worker thread, each with its
//! own `gio::Cancellable`. The operations the app calls are in
//! `commands.rs`, the periodic work in `tick.rs`, and indexing pinned
//! folders in `pins.rs`.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gio::prelude::*;

use super::crawl::{run_scan, ScanJob};
use super::error::SearchError;
use super::index::SearchIndex;
use super::ownership::Ownership;
use super::reader::FolderReader;
use super::requests::IndexRequest;
use super::root::IndexRoot;
use super::state::{ChangeListener, FolderKey, Shared};
use super::text::is_at_or_below;
use super::update::{run_update, UpdateJob};
use super::watch::{LocalWatch, WATCH_LIMIT};
use crate::settings::Preferences;

/// Whether automatic index updates run (the Auto-index setting, SRCH-026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoIndex {
    /// Roots are rescanned at start-up and kept up to date.
    On,
    /// Only scans the user asks for run; nothing is watched.
    Paused,
}

/// The settings each [`IndexService::tick`] applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexSettings {
    /// Whether automatic updates run.
    pub auto_index: AutoIndex,
    /// How often network roots are checked (SRCH-030).
    pub network_interval: Duration,
}

impl Default for IndexSettings {
    /// Auto-index on and a one-minute network check interval, the
    /// defaults of the Search & indexing settings.
    fn default() -> Self {
        Self {
            auto_index: AutoIndex::On,
            network_interval: Duration::from_secs(60),
        }
    }
}

impl IndexSettings {
    /// The settings the user chose in the Search & indexing settings.
    pub fn from_preferences(preferences: &Preferences) -> Self {
        let auto_index = if preferences.auto_index {
            AutoIndex::On
        } else {
            AutoIndex::Paused
        };
        Self {
            auto_index,
            network_interval: Duration::from_secs(preferences.network_interval.into()),
        }
    }
}

/// Who asked for a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScanTrigger {
    /// The user: another process's request is queued for the owner, and a
    /// server paused for sign-out is resumed (`explicit=True` in Python).
    User,
    /// The service itself: nothing is queued, and paused servers stay
    /// paused.
    Automatic,
}

/// Work for the worker thread.
#[derive(Debug)]
enum Job {
    /// A full scan of one root.
    Scan(ScanJob),
    /// A live update of one changed folder.
    Update(UpdateJob),
}

/// Keeps the search cache up to date (`IndexService` in Python).
///
/// Every method blocks on SQLite, so the app calls them off the main
/// thread, and [`IndexService::tick`] about once a second.
#[derive(Debug)]
pub struct IndexService {
    pub(super) shared: Arc<Shared>,
    jobs: Sender<Job>,
    worker: Option<JoinHandle<()>>,
}

impl IndexService {
    /// Starts the service for `index`, reading folders with `reader`, and
    /// tries at once to become the index owner.
    ///
    /// `listener` is called whenever the cache status or search results
    /// may have changed (the `cacheChanged` event of the Python app,
    /// SRCH-018). It runs on the service's worker thread or on the thread
    /// that calls [`IndexService::tick`], so the app passes the news on to
    /// its main loop.
    ///
    /// # Errors
    ///
    /// Private-storage errors for the owner lock file,
    /// [`SearchError::WorkerStart`] when the worker thread cannot start,
    /// and database errors while recovering interrupted scans.
    pub fn start(
        index: SearchIndex,
        reader: impl FolderReader + 'static,
        listener: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, SearchError> {
        Self::start_with_watch_limit(index, Box::new(reader), Box::new(listener), WATCH_LIMIT)
    }

    /// [`IndexService::start`] with at most `watch_limit` watched
    /// directories, so that tests can reach the limit.
    pub(crate) fn start_with_watch_limit(
        index: SearchIndex,
        reader: Box<dyn FolderReader>,
        listener: ChangeListener,
        watch_limit: usize,
    ) -> Result<Self, SearchError> {
        let ownership = Ownership::open(index.directory())?;
        let shared = Arc::new(Shared::new(index, reader, listener, watch_limit, ownership));
        let (jobs, queue) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("openxplorer-index".to_owned())
            .spawn(move || run_jobs(&worker_shared, &queue))
            .map_err(SearchError::WorkerStart)?;
        let service = Self {
            shared,
            jobs,
            worker: Some(worker),
        };
        service.elect()?;
        Ok(service)
    }

    /// The cache this service keeps up to date; search it and read its
    /// status through this.
    pub fn index(&self) -> &SearchIndex {
        &self.shared.index
    }

    /// Whether this process owns the index and runs the scans.
    pub fn is_owner(&self) -> bool {
        self.shared.state().ownership.is_owner()
    }

    /// Stops every scan and update and gives up ownership once the worker
    /// is idle (`close` in Python). Does not wait; see
    /// [`IndexService::shut_down`].
    pub fn close(&self) {
        let watch = {
            let mut state = self.shared.state();
            state.closed = true;
            for cancellable in state.scans.values().chain(state.updates.values()) {
                cancellable.cancel();
            }
            state.release_if_idle();
            state.watch.take()
        };
        // Dropped outside the lock: it waits for its reader thread.
        drop(watch);
    }

    /// Closes the service and waits for the worker thread to end, which
    /// happens as soon as the queued jobs notice their cancellation.
    pub fn shut_down(mut self) {
        let worker = self.worker.take();
        // Dropping closes the service and the job queue.
        drop(self);
        if let Some(worker) = worker {
            // A job that panicked has nothing left to clean up: the
            // database is transactional and ownership is already released.
            let _ = worker.join();
        }
    }

    /// Tries to become the index owner (`elect` in Python) and returns
    /// whether this process is the owner. A new owner starts watching and
    /// marks the scans a stopped owner left running as interrupted.
    pub(super) fn elect(&self) -> Result<bool, SearchError> {
        {
            let mut state = self.shared.state();
            if state.closed || state.ownership.is_owner() {
                return Ok(state.ownership.is_owner());
            }
            if !state.ownership.try_acquire() {
                return Ok(false);
            }
            // Without inotify, local roots fall back to timed checks.
            state.watch = LocalWatch::start(self.shared.watch_limit).ok();
        }
        self.shared.index.recover_interrupted()?;
        Ok(true)
    }

    /// Starts a full scan of the enabled root `root` (`refresh` in Python)
    /// and returns whether one was queued in this process. Another process
    /// passes a user's request on to the owner.
    pub(super) fn start_scan(&self, root: &str, trigger: ScanTrigger) -> Result<bool, SearchError> {
        if !self.is_owner() {
            if trigger == ScanTrigger::User {
                let request = IndexRequest::Refresh {
                    root: root.to_owned(),
                };
                self.shared.index.enqueue(&request)?;
            }
            return Ok(false);
        }
        let Some(root) = self.shared.index.enabled_root(root)? else {
            return Ok(false);
        };
        let cancellable = gio::Cancellable::new();
        {
            let mut state = self.shared.state();
            if state.closed || state.scans.contains_key(&root.uri) {
                return Ok(false);
            }
            if state.is_paused(&root.uri) {
                // Safety rule "no scan while signing out": only the user
                // resumes a server paused for sign-out.
                if trigger == ScanTrigger::Automatic {
                    return Ok(false);
                }
                state.resume_host_of(&root.uri);
            }
            state.scans.insert(root.uri.clone(), cancellable.clone());
            state.started.insert(root.uri.clone());
        }
        Ok(self.queue(Job::Scan(ScanJob { root, cancellable })))
    }

    /// Queues a live update of `folder` below `root` (`update` in Python)
    /// and returns whether it was queued: not when one is queued already,
    /// the root is being scanned or the service closed.
    pub(super) fn start_update(&self, root: &IndexRoot, folder: &str) -> bool {
        let key = FolderKey {
            root: root.uri.clone(),
            folder: folder.to_owned(),
        };
        let cancellable = gio::Cancellable::new();
        {
            let mut state = self.shared.state();
            let is_busy = state.updates.contains_key(&key) || state.scans.contains_key(&root.uri);
            if state.closed || is_busy {
                return false;
            }
            state.updates.insert(key, cancellable.clone());
        }
        let job = UpdateJob {
            root: root.clone(),
            folder: folder.to_owned(),
            cancellable,
        };
        self.queue(Job::Update(job))
    }

    /// Records that `folder` below `root` changed at `changed_at`
    /// (`changed` in Python). The owner re-reads it on a later tick;
    /// another process passes it on to the owner. Changes are ignored while
    /// Auto-index is paused.
    pub(super) fn record_change(
        &self,
        root: &str,
        folder: &str,
        changed_at: Instant,
    ) -> Result<(), SearchError> {
        let mut state = self.shared.state();
        let is_paused = state.settings.auto_index == AutoIndex::Paused;
        if state.closed || is_paused || !is_at_or_below(folder, root) {
            return Ok(());
        }
        if !state.ownership.is_owner() {
            drop(state);
            let request = IndexRequest::Changed {
                root: root.to_owned(),
                folder: folder.to_owned(),
            };
            return self.shared.index.enqueue(&request);
        }
        state.record_dirty_folder(root, folder, changed_at);
        Ok(())
    }

    /// Hands `job` to the worker; false when the worker has ended.
    fn queue(&self, job: Job) -> bool {
        self.jobs.send(job).is_ok()
    }
}

impl Drop for IndexService {
    fn drop(&mut self) {
        self.close();
    }
}

/// The worker thread: runs queued jobs until the service is dropped.
fn run_jobs(shared: &Shared, queue: &Receiver<Job>) {
    for job in queue {
        match job {
            Job::Scan(scan) => run_scan(shared, &scan),
            Job::Update(update) => run_update(shared, &update),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SRCH-026, SRCH-030
    #[test]
    fn the_settings_follow_the_users_preferences() {
        let preferences = Preferences {
            auto_index: false,
            network_interval: 300,
            ..Preferences::default()
        };

        let settings = IndexSettings::from_preferences(&preferences);
        let defaults = IndexSettings::from_preferences(&Preferences::default());

        assert_eq!(settings.auto_index, AutoIndex::Paused);
        assert_eq!(settings.network_interval, Duration::from_secs(300));
        assert_eq!(defaults, IndexSettings::default());
    }
}
