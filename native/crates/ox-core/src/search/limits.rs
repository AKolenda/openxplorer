// SPDX-License-Identifier: AGPL-3.0-only
//! The safety limits of scans, live updates and watches (SRCH-029,
//! SRCH-032).
//!
//! Ports the limits of `_run` and `_update` in `desktop/index_service.py`
//! and `max_watches` in `desktop/local_watch.py`. The service keeps them in
//! one [`ServiceLimits`], so that tests can lower each limit to reach it.

use super::watch::WATCH_LIMIT;

/// The limits one index service enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceLimits {
    /// Most directories watched with inotify; the rest are checked on a
    /// timer (SRCH-029).
    pub(crate) watched_folders: usize,
    /// Most entries one root may hold (SRCH-032).
    pub(crate) entries_per_root: usize,
    /// Most entries one folder may hold when a live update reads it
    /// (SRCH-032).
    pub(crate) entries_per_folder: usize,
    /// Most folders one live update may read, the changed folder included
    /// (SRCH-032).
    pub(crate) folders_per_update: usize,
}

impl Default for ServiceLimits {
    /// The limits of the Python app.
    fn default() -> Self {
        Self {
            watched_folders: WATCH_LIMIT,
            entries_per_root: 1_000_000,
            entries_per_folder: 1_000_000,
            folders_per_update: 10_000,
        }
    }
}
