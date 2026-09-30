// SPDX-License-Identifier: AGPL-3.0-only
//! The search one window runs from its search box: what was typed, where
//! it looks, and which run's results may still be shown.
//!
//! Ports the search state of `desktop/ui/app.js` (`state.query`,
//! `searchScope`, `searchGeneration`, `searchToken`, `searchResults`,
//! `searchBusy`, `searchError`) and the bookkeeping of `queueSearch`,
//! `resetSearch` and `runSearch` (SRCH-002, SRCH-017, SAFE-013). Each edit
//! cancels the running search and starts a new generation; a result that
//! comes back for an older generation is dropped, so a slow search never
//! replaces a newer one.

use gtk::gio;
use gtk::prelude::*;
use ox_core::search::{walks_subfolders, IndexRoot, SearchFacets, SearchIn};

use super::report::{SearchProgress, SearchReport};
use super::source::{SearchScope, SearchSource};

/// One run of a search that asks the cache.
#[derive(Debug, Clone)]
pub(crate) struct SearchRun {
    /// Only the latest run may show its results.
    generation: u64,
    /// The folder searched.
    pub folder: String,
    /// The words searched for, without surrounding blanks.
    pub text: String,
    /// The folder's scope, or every cached folder.
    pub scope: SearchScope,
    /// Where it looks.
    pub source: SearchSource,
    /// Where the cache covers the folder, which a search of contents
    /// does not use but the strip's offer to cache it follows.
    pub coverage: SearchSource,
    /// Whether the text of files is searched too (SRCH-036).
    pub search_in: SearchIn,
    /// Cancelled when a newer edit replaces the run.
    pub cancellable: gio::Cancellable,
}

/// A window's search.
#[derive(Debug, Default)]
pub(crate) struct FolderSearch {
    /// The search box's text.
    query: String,
    /// The scope the strip shows.
    scope: SearchScope,
    /// Whether names or names and contents are searched (SRCH-036); kept
    /// when the search ends, as Dolphin remembers it.
    search_in: SearchIn,
    /// The search options that narrow what is shown (SRCH-037).
    facets: SearchFacets,
    /// Counts edits and runs; see [`SearchRun`].
    generation: u64,
    /// The run asking the cache now.
    running: Option<gio::Cancellable>,
    /// What the window reports; `None` while nothing is searched.
    report: Option<SearchReport>,
    /// The rows of the last cached search, shown in place of the listing.
    results: Option<gio::ListStore>,
}

impl FolderSearch {
    /// The search box's text changed to `text` in `folder`, whose indexed
    /// folders are `roots`: the running search stops, its results go, and
    /// the listing is filtered by `text` until the search runs after the
    /// typing pause (`queueSearch`).
    pub(crate) fn edit(&mut self, text: &str, folder: &str, roots: &[IndexRoot]) {
        self.cancel();
        self.results = None;
        text.clone_into(&mut self.query);
        if !self.is_active() {
            // Clearing the box ends the search: its kind and date go.
            self.facets = SearchFacets::default();
        }
        self.report = self.is_active().then(|| self.searching_report(roots, folder));
    }

    /// The report of a search of `folder` that has not answered yet.
    fn searching_report(&self, roots: &[IndexRoot], folder: &str) -> SearchReport {
        SearchReport {
            source: self.source(roots, folder),
            coverage: SearchSource::choose(roots, folder, self.scope),
            progress: SearchProgress::Searching,
        }
    }

    /// Stops the search and forgets it, as leaving the folder does
    /// (`resetSearch`, and `searchScope='folder'` in `navigate`).
    pub(crate) fn end(&mut self) {
        self.cancel();
        self.results = None;
        self.query.clear();
        self.scope = SearchScope::default();
        self.facets = SearchFacets::default();
        self.report = None;
    }

    /// Where a search of `folder` looks: a search of contents walks the
    /// folder, because the cache holds names only, and a folder on the
    /// network is only filtered.
    fn source(&self, roots: &[IndexRoot], folder: &str) -> SearchSource {
        let reads_contents = self.search_in == SearchIn::NamesAndContents;
        let source = if reads_contents && self.scope == SearchScope::ThisFolder {
            SearchSource::CurrentFolder
        } else {
            SearchSource::choose(roots, folder, self.scope)
        };
        if source == SearchSource::CurrentFolder && !walks_subfolders(folder) {
            SearchSource::CurrentFolderOnly
        } else {
            source
        }
    }

    /// Whether something is searched: the box holds more than blanks.
    pub(crate) fn is_active(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// Whether the search waits for the typing pause or for the cache.
    pub(crate) fn is_running(&self) -> bool {
        let progress = self.report.as_ref().map(|report| &report.progress);
        progress == Some(&SearchProgress::Searching)
    }

    /// The search box's text.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// The scope the strip shows.
    pub(crate) fn scope(&self) -> SearchScope {
        self.scope
    }

    /// Chooses the scope; the caller runs the search again.
    pub(crate) fn set_scope(&mut self, scope: SearchScope) {
        self.scope = scope;
    }

    /// Whether names or names and contents are searched.
    pub(crate) fn search_in(&self) -> SearchIn {
        self.search_in
    }

    /// Chooses what is searched; the caller runs the search again.
    pub(crate) fn set_search_in(&mut self, search_in: SearchIn) {
        self.search_in = search_in;
    }

    /// The search options that narrow what is shown.
    pub(crate) fn facets(&self) -> SearchFacets {
        self.facets
    }

    /// Chooses the search options; the caller filters the rows again.
    pub(crate) fn set_facets(&mut self, facets: SearchFacets) {
        self.facets = facets;
    }

    /// Starts the search of `folder`, with the indexed folders `roots`.
    /// Returns the run that asks the cache, or walks the folder's tree
    /// when no indexed folder is related ([`SearchSource::CurrentFolder`]);
    /// `None` while nothing is searched. Earlier cached results stay until
    /// the run replaces them, so a search that runs again after the cache
    /// changed does not flicker.
    pub(crate) fn begin(&mut self, folder: &str, roots: &[IndexRoot]) -> Option<SearchRun> {
        self.cancel();
        if !self.is_active() {
            self.results = None;
            self.report = None;
            return None;
        }
        let mut report = self.searching_report(roots, folder);
        let (source, coverage) = (report.source, report.coverage);
        if !source.uses_cache() {
            self.results = None;
        }
        if source == SearchSource::CurrentFolderOnly {
            // The filtered listing is the whole search.
            report.progress = SearchProgress::Shown { is_truncated: false };
            self.report = Some(report);
            return None;
        }
        self.report = Some(report);
        let cancellable = gio::Cancellable::new();
        self.running = Some(cancellable.clone());
        Some(SearchRun {
            generation: self.generation,
            folder: folder.to_owned(),
            text: self.query.trim().to_owned(),
            scope: self.scope,
            source,
            coverage,
            search_in: self.search_in,
            cancellable,
        })
    }

    /// Whether `run` is still the latest run, whose results may be shown.
    pub(crate) fn is_current(&self, run: &SearchRun) -> bool {
        run.generation == self.generation && self.running.is_some()
    }

    /// Shows the rows `run` has found so far, which it keeps adding to,
    /// while it goes on.
    pub(crate) fn show_found(&mut self, run: &SearchRun, rows: gio::ListStore) {
        if self.is_current(run) {
            self.results = Some(rows);
        }
    }

    /// Shows `results` of `run`.
    pub(crate) fn finish(&mut self, run: &SearchRun, results: gio::ListStore, is_truncated: bool) {
        self.running = None;
        self.results = Some(results);
        self.report = Some(SearchReport {
            source: run.source,
            coverage: run.coverage,
            progress: SearchProgress::Shown { is_truncated },
        });
    }

    /// Reports that `run` failed with `message`, showing no rows.
    pub(crate) fn fail(&mut self, run: &SearchRun, message: String) {
        self.running = None;
        self.results = Some(gio::ListStore::new::<crate::folder_view::item::FileItem>());
        self.report = Some(SearchReport {
            source: run.source,
            coverage: run.coverage,
            progress: SearchProgress::Failed(message),
        });
    }

    /// What the window reports, while something is searched.
    pub(crate) fn report(&self) -> Option<&SearchReport> {
        self.report.as_ref()
    }

    /// The rows of the last cached search; `None` while the listing is
    /// shown, filtered.
    pub(crate) fn results(&self) -> Option<&gio::ListStore> {
        self.results.as_ref()
    }

    /// Cancels the running search and starts a new generation, so its
    /// results are dropped when they arrive.
    fn cancel(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
    }
}

impl Drop for FolderSearch {
    /// Stops the running search when its window goes, so a walk does not
    /// go on reading files for no one.
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder_view::item::FileItem;
    use crate::search::test_roots::enabled_root;

    const FOLDER: &str = "file:///home/demo/Work";

    fn searching(text: &str) -> FolderSearch {
        let mut search = FolderSearch::default();
        search.edit(text, FOLDER, &[]);
        search
    }

    fn empty_results() -> gio::ListStore {
        gio::ListStore::new::<FileItem>()
    }

    /// parity: SRCH-002, SRCH-017, SAFE-013
    #[test]
    fn an_edit_cancels_the_running_search_and_drops_its_results() {
        let mut search = searching("report");
        let run = search
            .begin(FOLDER, &[enabled_root(FOLDER)])
            .expect("an indexed folder asks the cache");

        search.edit("reports", FOLDER, &[enabled_root(FOLDER)]);

        assert!(run.cancellable.is_cancelled());
        assert!(!search.is_current(&run), "a late answer is dropped");
        assert_eq!(search.report().map(SearchReport::caption), Some("Searching…"));
    }

    /// parity: SRCH-003, SRCH-035
    #[test]
    fn a_folder_nobody_indexed_is_filtered_then_walked() {
        let mut search = searching("report");

        let run = search.begin(FOLDER, &[]).expect("the folder's tree is walked");

        assert_eq!(run.source, SearchSource::CurrentFolder);
        assert!(search.results().is_none(), "the listing is shown, filtered");
        search.show_found(&run, empty_results());
        assert!(search.results().is_some(), "the walk's rows replace it");
        search.finish(&run, empty_results(), false);
        let report = search.report().expect("a search is reported");
        assert_eq!(report.caption(), "Current folder + subfolders");
    }

    /// A share nobody indexed is filtered only, not walked over the
    /// network.
    ///
    /// parity: SRCH-003, SRCH-035
    #[test]
    fn a_share_nobody_indexed_is_only_filtered() {
        let share = "smb://server/share/Work";
        let mut search = FolderSearch::default();
        search.edit("report", share, &[]);

        assert!(search.begin(share, &[]).is_none(), "nothing is walked");

        let report = search.report().expect("a search is reported");
        assert_eq!(report.caption(), "Current folder only");
        assert!(report.offers_to_cache_folder());
    }

    /// parity: SRCH-007
    #[test]
    fn the_latest_run_shows_its_results() {
        let mut search = searching("report");
        let run = search
            .begin(FOLDER, &[enabled_root(FOLDER)])
            .expect("a cached search");

        search.finish(&run, empty_results(), true);

        assert!(search.results().is_some());
        let report = search.report().expect("a search is reported");
        assert_eq!(
            report.freshness_note(),
            Some("First 500 results · narrow your search")
        );
        assert!(!search.is_current(&run), "a run finishes once");
    }

    /// parity: SRCH-002
    #[test]
    fn blanks_search_nothing() {
        let mut search = searching("   ");

        assert!(!search.is_active());
        assert!(search.report().is_none());
        assert!(search.begin(FOLDER, &[enabled_root(FOLDER)]).is_none());
    }

    /// parity: SRCH-036
    #[test]
    fn a_search_of_contents_walks_the_folder_even_where_it_is_cached() {
        let mut search = searching("budget");
        search.set_search_in(SearchIn::NamesAndContents);

        let run = search.begin(FOLDER, &[enabled_root(FOLDER)]).expect("a search");

        assert_eq!(run.source, SearchSource::CurrentFolder);
        assert_eq!(run.search_in, SearchIn::NamesAndContents);
        let report = search.report().expect("a search is reported");
        assert!(
            !report.offers_to_cache_folder(),
            "the folder is cached already; the button would switch it off"
        );
        search.end();
        assert_eq!(
            search.search_in(),
            SearchIn::NamesAndContents,
            "the choice is kept"
        );
    }

    #[test]
    fn ending_the_search_forgets_the_scope_and_the_text() {
        let mut search = searching("report");
        search.set_scope(SearchScope::AllCachedFolders);

        search.end();

        assert_eq!(search.query(), "");
        assert_eq!(search.scope(), SearchScope::ThisFolder);
        assert!(search.report().is_none());
    }

    #[test]
    fn a_failed_search_shows_no_rows_and_says_why() {
        let mut search = searching("report");
        let run = search
            .begin(FOLDER, &[enabled_root(FOLDER)])
            .expect("a cached search");

        search.fail(&run, "The search cache is not running.".to_owned());

        let rows = search.results().map(ListModelExt::n_items);
        assert_eq!(rows, Some(0));
        let report = search.report().expect("a search is reported");
        assert_eq!(report.caption(), "The search cache is not running.");
    }
}
