// SPDX-License-Identifier: AGPL-3.0-only
//! The index service's periodic work.
//!
//! Ports `refresh_due` and `overflow` in `desktop/index_service.py`
//! (SRCH-026 to SRCH-030): requests from other processes, live changes,
//! the rescan at start-up, the Auto-index switch and the timed checks of
//! roots that cannot be watched.

use std::time::{Duration, Instant};

use super::error::SearchError;
use super::policy::RootStorage;
use super::requests::IndexRequest;
use super::root::IndexRoot;
use super::service::{AutoIndex, IndexService, IndexSettings, ScanTrigger};
use super::state::{FolderKey, NetworkPoll, PollCursor};
use super::watch::{LocalWatch, WatchEvent};

/// How long a folder must stay unchanged before it is re-read, so a burst
/// of events causes one update.
const CHANGE_DEBOUNCE: Duration = Duration::from_millis(350);

/// Most changed folders handed to the worker per tick.
const MAX_UPDATES_PER_TICK: usize = 64;

/// Folders checked per tick across all polled roots.
const CHECKS_PER_TICK: usize = 4;

/// The pause between two batches of timed checks of one root.
const CHECK_BATCH_PAUSE: Duration = Duration::from_secs(2);

impl IndexService {
    /// Does the periodic work with the user's current `settings`; the app
    /// calls it about once a second, off the main thread (`refresh_due` in
    /// Python). A process that is not the owner tries to become it.
    ///
    /// # Errors
    ///
    /// Database errors; the next tick tries again.
    pub fn tick(&self, settings: &IndexSettings) -> Result<(), SearchError> {
        self.shared.state().settings = *settings;
        if self.shared.is_closed() || !self.elect()? {
            return Ok(());
        }
        self.handle_requests()?;
        self.handle_watch_events()?;
        let roots: Vec<IndexRoot> = self
            .index()
            .roots()?
            .into_iter()
            .filter(IndexRoot::is_enabled)
            .collect();
        self.stop_removed_roots(&roots);
        self.start_unscanned_roots(&roots)?;
        self.apply_auto_index_change(&roots);
        if settings.auto_index == AutoIndex::Paused {
            return Ok(());
        }
        self.rescan_forced_roots()?;
        self.update_changed_folders(&roots);
        self.check_unwatched_roots(&roots, settings.network_interval)
    }

    /// Does what other processes asked for, oldest request first.
    fn handle_requests(&self) -> Result<(), SearchError> {
        for request in self.index().drain_requests()? {
            self.handle_request(request)?;
        }
        Ok(())
    }

    /// Does what another process asked for, as that process would have
    /// done it had it owned the index.
    fn handle_request(&self, request: IndexRequest) -> Result<(), SearchError> {
        match request {
            IndexRequest::Refresh { root } => {
                self.start_scan(&root, ScanTrigger::User)?;
            }
            IndexRequest::Cancel { root } => self.stop(&root)?,
            IndexRequest::Changed { root, folder } => {
                self.record_change(&root, &folder, Instant::now())?;
            }
            IndexRequest::PauseServer { host } => self.pause_server(&host)?,
            IndexRequest::ResumeServer { host } => self.resume_server(&host)?,
        }
        Ok(())
    }

    /// Takes the inotify events that arrived since the last tick.
    fn handle_watch_events(&self) -> Result<(), SearchError> {
        let events = {
            let state = self.shared.state();
            state
                .watch
                .as_ref()
                .map(LocalWatch::take_events)
                .unwrap_or_default()
        };
        for event in events {
            match event {
                WatchEvent::Changed {
                    root,
                    folder,
                    changed_at,
                } => self.record_change(&root, &folder, changed_at)?,
                WatchEvent::Overflowed => self.rescan_after_overflow()?,
            }
        }
        Ok(())
    }

    /// The kernel dropped events: only a full rescan of every root is
    /// reliable now (`overflow` in Python, SRCH-029).
    fn rescan_after_overflow(&self) -> Result<(), SearchError> {
        let roots = self.index().roots()?;
        let enabled = roots
            .into_iter()
            .filter(IndexRoot::is_enabled)
            .map(|root| root.uri);
        self.shared.state().forced_rescans.extend(enabled);
        self.shared.notify();
        Ok(())
    }

    /// Stops scans and watches of roots that were disabled or removed.
    fn stop_removed_roots(&self, roots: &[IndexRoot]) {
        let mut state = self.shared.state();
        let removed: Vec<String> = state
            .started
            .iter()
            .filter(|started| !roots.iter().any(|root| &root.uri == *started))
            .cloned()
            .collect();
        for root in removed {
            state.cancel_root(&root);
            if let Some(watch) = &state.watch {
                watch.remove_root(&root);
            }
            state.started.remove(&root);
        }
    }

    /// Scans each root once after start-up, to catch changes made while no
    /// `OpenXplorer` was running (SRCH-026).
    fn start_unscanned_roots(&self, roots: &[IndexRoot]) -> Result<(), SearchError> {
        for root in roots {
            let is_due = {
                let state = self.shared.state();
                let is_waiting = !state.started.contains(&root.uri) && !state.is_paused(&root.uri);
                is_waiting && state.settings.auto_index == AutoIndex::On
            };
            if is_due {
                self.start_scan(&root.uri, ScanTrigger::Automatic)?;
            }
        }
        Ok(())
    }

    /// Follows a change of the Auto-index setting: pausing removes every
    /// watch, and switching back on rescans every root (SRCH-026).
    fn apply_auto_index_change(&self, roots: &[IndexRoot]) {
        let auto_index = {
            let state = self.shared.state();
            if state.settings.auto_index == state.auto_index_before {
                return;
            }
            state.settings.auto_index
        };
        for root in roots {
            if auto_index == AutoIndex::Paused {
                if let Some(watch) = &self.shared.state().watch {
                    watch.remove_root(&root.uri);
                }
            }
            self.shared
                .report_monitoring(&root.uri, RootStorage::current(&root.uri));
        }
        let mut state = self.shared.state();
        if auto_index == AutoIndex::On {
            state.started.clear();
        }
        state.auto_index_before = auto_index;
    }

    /// Starts the full rescans an overflow asked for.
    fn rescan_forced_roots(&self) -> Result<(), SearchError> {
        let forced: Vec<String> = self.shared.state().forced_rescans.iter().cloned().collect();
        for root in forced {
            if self.start_scan(&root, ScanTrigger::Automatic)? {
                self.shared.state().forced_rescans.remove(&root);
            }
        }
        Ok(())
    }

    /// Re-reads folders that changed and then stayed unchanged for
    /// [`CHANGE_DEBOUNCE`], oldest first (SRCH-028).
    fn update_changed_folders(&self, roots: &[IndexRoot]) {
        for (key, changed_at) in self.settled_changes() {
            let Some(root) = roots.iter().find(|root| root.uri == key.root) else {
                continue;
            };
            // A separate statement, so the state lock is released before
            // `start_update` takes it again.
            let is_paused = self.shared.state().is_paused(&root.uri);
            if is_paused || !self.start_update(root, &key.folder) {
                continue;
            }
            let mut state = self.shared.state();
            // A newer change of the same folder waits for its own update.
            if state.dirty.get(&key) == Some(&changed_at) {
                state.dirty.remove(&key);
            }
        }
    }

    /// Up to [`MAX_UPDATES_PER_TICK`] changed folders that have settled.
    fn settled_changes(&self) -> Vec<(FolderKey, Instant)> {
        let state = self.shared.state();
        let mut settled: Vec<(FolderKey, Instant)> = state
            .dirty
            .iter()
            .filter(|(_, changed_at)| changed_at.elapsed() > CHANGE_DEBOUNCE)
            .map(|(key, changed_at)| (key.clone(), *changed_at))
            .collect();
        settled.sort_by_key(|(_, changed_at)| *changed_at);
        settled.truncate(MAX_UPDATES_PER_TICK);
        settled
    }

    /// Checks a few folders of network roots, and of local roots with
    /// unwatched directories, every `interval` (SRCH-030). Small batches
    /// avoid hammering a NAS; a large tree takes longer than the interval,
    /// which the status reports as timed checks, never as live coverage.
    fn check_unwatched_roots(&self, roots: &[IndexRoot], interval: Duration) -> Result<(), SearchError> {
        let mut budget = CHECKS_PER_TICK;
        for root in roots {
            if budget == 0 {
                break;
            }
            let Some(cursor) = self.due_check(root, interval) else {
                continue;
            };
            let folders = match &cursor {
                PollCursor::RootFolder => vec![root.uri.clone()],
                PollCursor::After(after) => self.index().cached_folders_after(&root.uri, after, budget)?,
            };
            self.advance_checks(root, &cursor, folders.last().map(String::as_str), interval);
            for folder in &folders {
                self.start_update(root, folder);
                budget = budget.saturating_sub(1);
            }
        }
        Ok(())
    }

    /// Where the timed checks of `root` continue, when they are due now.
    fn due_check(&self, root: &IndexRoot, interval: Duration) -> Option<PollCursor> {
        let storage = RootStorage::current(&root.uri);
        let mut state = self.shared.state();
        let is_busy = state.scans.contains_key(&root.uri) || state.is_paused(&root.uri);
        let is_watched = storage == RootStorage::Local && !state.failed_watches.contains_key(&root.uri);
        if is_busy || is_watched {
            return None;
        }
        let now = Instant::now();
        let poll = state
            .network_polls
            .entry(root.uri.clone())
            .or_insert_with(|| NetworkPoll {
                cursor: PollCursor::RootFolder,
                next_at: now + interval,
            });
        (now >= poll.next_at).then(|| poll.cursor.clone())
    }

    /// Moves the checks of `root` on after the batch that ended at `last`:
    /// to the next batch shortly, or to a new round after `interval` once
    /// every cached folder was checked.
    fn advance_checks(&self, root: &IndexRoot, cursor: &PollCursor, last: Option<&str>, interval: Duration) {
        let now = Instant::now();
        let (cursor, pause) = match (cursor, last) {
            (PollCursor::RootFolder, _) => (PollCursor::After(String::new()), CHECK_BATCH_PAUSE),
            (PollCursor::After(_), Some(last)) => (PollCursor::After(last.to_owned()), CHECK_BATCH_PAUSE),
            (PollCursor::After(_), None) => (PollCursor::RootFolder, interval),
        };
        let poll = NetworkPoll {
            cursor,
            next_at: now + pause,
        };
        self.shared.state().network_polls.insert(root.uri.clone(), poll);
    }
}

#[cfg(test)]
mod tests {
    use crate::search::fixtures::LocalRoot;
    use crate::search::root::RootStatus;
    use crate::search::watch::WATCH_LIMIT;

    /// Events were lost when the inotify queue overflowed, so every root
    /// is scanned again in full (`overflow` in Python).
    ///
    /// parity: SRCH-029
    #[test]
    fn a_queue_overflow_rescans_every_root() {
        let local = LocalRoot::new();
        let service = local.start_service(WATCH_LIMIT);
        service.refresh(&local.root).unwrap();
        local.tick_until(&service, "the first scan is ready", |root| {
            root.status == RootStatus::Ready
        });
        let generation = local.state().generation;

        service.rescan_after_overflow().unwrap();

        assert!(service.shared.state().forced_rescans.contains(&local.root));
        local.tick_until(&service, "the forced rescan is done", |root| {
            root.status == RootStatus::Ready && root.generation != generation
        });
        assert!(service.shared.state().forced_rescans.is_empty());
    }
}
