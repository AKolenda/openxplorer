// SPDX-License-Identifier: AGPL-3.0-only
//! The thread that opens the search cache, starts the index service and
//! ticks it once a second for as long as the app runs.
//!
//! Ports the start of `IndexService` in `desktop/winspace.py` and its
//! `refresh_due` timer (`winspace.py:148-163`): the index owner's periodic
//! work runs off the main thread, about once a second, and never overlaps
//! itself (PERF-006), because the next tick waits for the one before.
//! Opening the database, electing the owner and recovering interrupted
//! scans all block on SQLite, so they run on this thread too, and the
//! main thread never waits for them (PERF-003).

#[cfg(test)]
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use gtk::glib;
use ox_core::search::{GioFolderReader, IndexService, IndexSettings, SearchError, SearchIndex};
use ox_core::settings::Bookmark;
use ox_core::LOG_DOMAIN;

use super::error::CacheError;

/// How long the thread waits between two ticks (`refresh_due` ran every
/// second).
const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// Where the search cache lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CacheLocation {
    /// `$XDG_CACHE_HOME/winspace`, shared with the Python app.
    UserCache,
    /// Another directory, for tests.
    #[cfg(test)]
    Directory(PathBuf),
}

/// What the service starts with.
#[derive(Debug, Clone)]
pub(crate) struct IndexerStart {
    /// Where the cache lives.
    pub location: CacheLocation,
    /// The user's Search & indexing options at start-up, so the first tick
    /// already honours a paused Auto-index (SRCH-026).
    pub settings: IndexSettings,
    /// The Quick access pins at start-up, for the one-time indexing of
    /// folders pinned before pinned folders were indexed (SRCH-040).
    pub pins: Vec<Bookmark>,
}

/// The index service once its thread has started it, or why it could not
/// start. Filled once; operations wait for it on worker threads.
type StartedService = OnceLock<Result<Arc<IndexService>, String>>;

/// The running thread and the service it started.
#[derive(Debug)]
pub(crate) struct Indexer {
    /// Shared with the thread, which fills it, and with running operations.
    service: Arc<StartedService>,
    /// New options for the ticks; dropping it ends the thread.
    settings: Sender<IndexSettings>,
}

impl Indexer {
    /// Starts the thread. `on_change` is the service's change listener
    /// (see [`IndexService::start`]); it runs on a worker thread.
    ///
    /// # Errors
    ///
    /// The operating system could not start the thread.
    pub(crate) fn start(
        start: IndexerStart,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let service = Arc::new(StartedService::new());
        let (settings, updates) = mpsc::channel();
        let slot = Arc::clone(&service);
        thread::Builder::new()
            .name("openxplorer-index-tick".to_owned())
            .spawn(move || run(&start, on_change, &slot, &updates))?;
        Ok(Self { service, settings })
    }

    /// The service the thread started, for an operation on a worker
    /// thread; see [`started`].
    pub(crate) fn service(&self) -> Arc<StartedService> {
        Arc::clone(&self.service)
    }

    /// Applies the user's new Search & indexing options from the next tick.
    pub(crate) fn set_settings(&self, settings: IndexSettings) {
        // The thread only ends once this indexer is dropped.
        let _ = self.settings.send(settings);
    }
}

impl Drop for Indexer {
    /// Stops every scan and watch at once; the thread ends at its next
    /// wait, and ownership is released when the running jobs notice.
    fn drop(&mut self) {
        if let Some(Ok(service)) = self.service.get() {
            service.close();
        }
    }
}

/// Waits, on a worker thread, until the service has started, and returns
/// it.
///
/// # Errors
///
/// [`CacheError::StartFailed`] when it could not start.
pub(crate) fn started(service: &StartedService) -> Result<&IndexService, CacheError> {
    match service.wait() {
        Ok(service) => Ok(service),
        Err(message) => Err(CacheError::StartFailed(message.clone())),
    }
}

/// The thread: starts the service, indexes earlier pins once, then ticks
/// until the indexer is dropped.
fn run(
    start: &IndexerStart,
    on_change: impl Fn() + Send + Sync + 'static,
    slot: &StartedService,
    updates: &Receiver<IndexSettings>,
) {
    let service = match start_service(&start.location, on_change) {
        Ok(service) => Arc::new(service),
        Err(error) => {
            glib::g_warning!(LOG_DOMAIN, "The search cache could not start: {error}");
            let _ = slot.set(Err(error.to_string()));
            return;
        }
    };
    let _ = slot.set(Ok(Arc::clone(&service)));
    index_existing_pins(&service, &start.pins);
    tick_until_stopped(&service, start.settings, updates);
}

/// Opens the cache at `location` and starts its service.
fn start_service(
    location: &CacheLocation,
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<IndexService, SearchError> {
    let index = match location {
        CacheLocation::UserCache => SearchIndex::open_default()?,
        #[cfg(test)]
        CacheLocation::Directory(directory) => SearchIndex::open(directory)?,
    };
    IndexService::start(index, GioFolderReader, on_change)
}

/// Indexes the folders pinned before pinned folders were indexed, once per
/// cache (SRCH-040), as the switch says.
fn index_existing_pins(service: &IndexService, pins: &[Bookmark]) {
    let indexing = service.index().pin_indexing();
    let result = indexing.and_then(|indexing| service.index_existing_pins(pins, indexing));
    if let Err(error) = result {
        glib::g_warning!(LOG_DOMAIN, "Could not index the pinned folders: {error}");
    }
}

/// Ticks `service` about once a second with the latest `settings` until
/// the indexer is dropped. A failed tick is retried at the next one; its
/// error is logged once, not every second.
fn tick_until_stopped(
    service: &IndexService,
    mut settings: IndexSettings,
    updates: &Receiver<IndexSettings>,
) {
    let mut last_error = String::new();
    loop {
        match service.tick(&settings) {
            Ok(()) => last_error.clear(),
            Err(error) => {
                let message = error.to_string();
                if message != last_error {
                    glib::g_warning!(LOG_DOMAIN, "The search index could not update: {message}");
                    last_error = message;
                }
            }
        }
        match updates.recv_timeout(TICK_INTERVAL) {
            Ok(changed) => settings = changed,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Opening the cache and starting the service happen on the indexer's
    /// thread, which then ticks one tick after the other, so the caller
    /// never waits for SQLite.
    ///
    /// parity: PERF-006, PERF-003
    #[test]
    fn the_index_starts_and_ticks_off_the_callers_thread() {
        let directory = tempfile::tempdir().expect("the test home has room for a cache");
        let start = IndexerStart {
            location: CacheLocation::Directory(directory.path().join("cache")),
            settings: IndexSettings::default(),
            pins: Vec::new(),
        };
        let began = Instant::now();
        let indexer = Indexer::start(start, || {}).expect("the thread starts");
        assert!(began.elapsed() < TICK_INTERVAL, "starting does not wait for the cache");

        let service = indexer.service();
        let service = started(&service).expect("the thread opened the cache");
        assert!(service.is_owner(), "the thread elected this process the owner");
    }
}
