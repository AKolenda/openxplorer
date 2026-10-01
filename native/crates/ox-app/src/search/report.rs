// SPDX-License-Identifier: AGPL-3.0-only
//! What the window says about a search: the strip's caption and note, the
//! status bar's count and the empty page's message.
//!
//! Ports `renderSearchInfo`, the search branch of `updateStatus` and the
//! empty-state message of `renderRows` in `v2.0.0:desktop/ui/app.js` (SRCH-012,
//! SRCH-013, VIEW-050), with their wording.

use super::source::SearchSource;

/// The results a cached search shows at most (`slice(0,500)` in
/// `runSearch`, PERF-005).
pub(crate) const RESULT_LIMIT: usize = 500;

/// The tooltip of the strip's note on cached results.
pub(crate) const FRESHNESS_TOOLTIP: &str = "Names and paths are stored locally. Refresh the cache to \
                                            pick up changes on a disconnected or unmonitored share.";

/// How far a search has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchProgress {
    /// Typed, and running or about to run after the pause.
    Searching,
    /// The results are shown.
    Shown {
        /// More matched than the [`RESULT_LIMIT`] shown.
        is_truncated: bool,
    },
    /// The cache could not be searched; the message says why.
    Failed(String),
}

/// A search as the window reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchReport {
    /// Where it looks.
    pub source: SearchSource,
    /// Where the cache covers the folder; differs from `source` while a
    /// search of contents walks a cached folder.
    pub coverage: SearchSource,
    /// How far it has got.
    pub progress: SearchProgress,
}

impl SearchReport {
    /// The strip's caption: the error, "Searching…", or where it looked.
    pub(crate) fn caption(&self) -> &str {
        match &self.progress {
            SearchProgress::Failed(message) => message,
            SearchProgress::Searching => "Searching…",
            SearchProgress::Shown { .. } => self.source.caption(),
        }
    }

    /// The strip's note: that results were cut off, or how fresh cached
    /// ones are; `None` for a complete live search.
    pub(crate) fn freshness_note(&self) -> Option<&'static str> {
        if self.is_truncated() {
            return Some("First 500 results · narrow your search");
        }
        if !self.source.uses_cache() {
            return None;
        }
        let note = if self.source == SearchSource::CurrentFolderAndCachedSubfolders {
            "Other subfolders are not indexed."
        } else {
            "Cached metadata · see update coverage in Settings"
        };
        Some(note)
    }

    /// Whether the strip offers "Cache this folder": the folder is not
    /// indexed, or only some of its subfolders are. A search of contents
    /// in a cached folder does not offer it, because the button would
    /// switch the folder's caching off.
    pub(crate) fn offers_to_cache_folder(&self) -> bool {
        self.coverage != SearchSource::Cache
    }

    /// What the status bar counts while `shown` results are shown.
    pub(crate) fn count(&self, shown: u32) -> SearchCount {
        let progress = match self.progress {
            SearchProgress::Searching => CountProgress::Searching,
            SearchProgress::Shown { is_truncated } => CountProgress::Shown {
                is_truncated,
                is_cached: self.source.uses_cache(),
            },
            SearchProgress::Failed(_) => CountProgress::Shown {
                is_truncated: false,
                is_cached: true,
            },
        };
        SearchCount { shown, progress }
    }

    /// Why nothing is shown, under "No matching items".
    pub(crate) fn empty_message(&self) -> &str {
        if let SearchProgress::Failed(message) = &self.progress {
            return message;
        }
        match self.source {
            SearchSource::CurrentFolder => "No items found in this folder or its subfolders.",
            SearchSource::CurrentFolderOnly => {
                "Only this folder is being filtered. Enable its search cache to include subfolders."
            }
            SearchSource::Cache | SearchSource::CurrentFolderAndCachedSubfolders => {
                "No cached matches. Refresh the cache if this folder changed, or try another search."
            }
        }
    }

    fn is_truncated(&self) -> bool {
        matches!(self.progress, SearchProgress::Shown { is_truncated: true })
    }
}

/// The status bar's count while searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SearchCount {
    /// Results shown.
    pub shown: u32,
    /// How far the search has got.
    pub progress: CountProgress,
}

/// How far a counted search has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CountProgress {
    /// Still searching.
    Searching,
    /// Its results are shown.
    Shown {
        /// More matched than are shown.
        is_truncated: bool,
        /// They came from the cache.
        is_cached: bool,
    },
}

impl SearchCount {
    /// "Searching…", or "12 results (first 500) · Cached".
    pub(crate) fn text(self) -> String {
        let CountProgress::Shown {
            is_truncated,
            is_cached,
        } = self.progress
        else {
            return "Searching…".to_owned();
        };
        let mut text = if self.shown == 1 {
            "1 result".to_owned()
        } else {
            format!("{} results", self.shown)
        };
        if is_truncated {
            text.push_str(" (first 500)");
        }
        if is_cached {
            text.push_str(" · Cached");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(source: SearchSource, progress: SearchProgress) -> SearchReport {
        SearchReport {
            source,
            coverage: source,
            progress,
        }
    }

    fn shown(is_truncated: bool) -> SearchProgress {
        SearchProgress::Shown { is_truncated }
    }

    /// Ported from `renderSearchInfo` in `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: SRCH-012
    #[test]
    fn the_strip_says_where_the_search_looked() {
        let folder = report(SearchSource::CurrentFolder, shown(false));
        let partial = report(SearchSource::CurrentFolderAndCachedSubfolders, shown(false));
        let cached = report(SearchSource::Cache, shown(false));
        let truncated = report(SearchSource::Cache, shown(true));
        let running = report(SearchSource::Cache, SearchProgress::Searching);
        let failed = report(
            SearchSource::Cache,
            SearchProgress::Failed("Search must be at most 512 characters.".into()),
        );

        assert_eq!(folder.caption(), "Current folder + subfolders");
        assert_eq!(folder.freshness_note(), None);
        assert_eq!(
            report(SearchSource::CurrentFolder, shown(true)).freshness_note(),
            Some("First 500 results · narrow your search")
        );
        assert!(folder.offers_to_cache_folder());
        assert_eq!(partial.caption(), "Current folder + cached subfolders");
        assert_eq!(
            partial.freshness_note(),
            Some("Other subfolders are not indexed.")
        );
        assert!(partial.offers_to_cache_folder());
        assert_eq!(cached.caption(), "Cached names & paths");
        assert_eq!(
            cached.freshness_note(),
            Some("Cached metadata · see update coverage in Settings")
        );
        assert!(!cached.offers_to_cache_folder());
        assert_eq!(
            truncated.freshness_note(),
            Some("First 500 results · narrow your search")
        );
        assert_eq!(running.caption(), "Searching…");
        assert_eq!(failed.caption(), "Search must be at most 512 characters.");
    }

    /// Ported from the search branch of `updateStatus` in
    /// `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: VIEW-050
    #[test]
    fn the_status_bar_counts_results() {
        let running = report(SearchSource::Cache, SearchProgress::Searching);
        let one_filtered = report(SearchSource::CurrentFolder, shown(false));
        let many_cached = report(SearchSource::Cache, shown(true));

        assert_eq!(running.count(3).text(), "Searching…");
        assert_eq!(one_filtered.count(1).text(), "1 result");
        assert_eq!(one_filtered.count(0).text(), "0 results");
        assert_eq!(many_cached.count(500).text(), "500 results (first 500) · Cached");
    }

    /// Ported from the empty state of `renderRows` in `v2.0.0:desktop/ui/app.js`.
    ///
    /// parity: SRCH-013
    #[test]
    fn an_empty_result_says_why() {
        let folder = report(SearchSource::CurrentFolder, shown(false));
        let cached = report(SearchSource::CurrentFolderAndCachedSubfolders, shown(false));
        let failed = report(
            SearchSource::Cache,
            SearchProgress::Failed("The search cache is not running.".into()),
        );

        assert_eq!(
            folder.empty_message(),
            "No items found in this folder or its subfolders."
        );
        assert_eq!(
            cached.empty_message(),
            "No cached matches. Refresh the cache if this folder changed, or try another search."
        );
        assert_eq!(failed.empty_message(), "The search cache is not running.");
    }
}
