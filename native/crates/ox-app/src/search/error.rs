// SPDX-License-Identifier: AGPL-3.0-only
//! Why the app could not use the search cache.
//!
//! Wraps ox-core's [`SearchError`], whose messages are the Python app's
//! (`v2.0.0:desktop/search_index.py`, `v2.0.0:desktop/winspace.py`), with the ways the
//! app's own side can fail: the cache never started, or its worker
//! stopped.

use ox_core::search::SearchError;

/// Why an operation on the search cache failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CacheError {
    /// The app did not start the search cache, as in tests that do not
    /// use it.
    #[error("The search cache is not running.")]
    NotStarted,
    /// Opening the cache or starting the index service failed; the
    /// message says why.
    #[error("The search cache could not start: {0}")]
    StartFailed(String),
    /// The cache refused or failed the operation.
    #[error(transparent)]
    Search(#[from] SearchError),
    /// The worker thread running the operation ended without an answer.
    #[error("The search cache stopped unexpectedly.")]
    WorkerLost,
}

impl CacheError {
    /// Whether the operation stopped because a newer one cancelled it,
    /// which is never shown as an error.
    pub(crate) fn is_cancelled(&self) -> bool {
        matches!(self, CacheError::Search(error) if error.is_cancelled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_cancelled_search_counts_as_cancelled() {
        assert!(CacheError::Search(SearchError::Cancelled).is_cancelled());
        assert!(!CacheError::Search(SearchError::QueryTooLong).is_cancelled());
        assert!(!CacheError::NotStarted.is_cancelled());
    }

    #[test]
    fn the_caches_own_messages_are_shown_as_they_are() {
        let too_long = CacheError::Search(SearchError::QueryTooLong);
        assert_eq!(too_long.to_string(), "Search must be at most 512 characters.");
    }
}
