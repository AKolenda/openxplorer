// SPDX-License-Identifier: AGPL-3.0-only
//! The search box's search of the current folder: filtering the listing,
//! or asking the search cache, and showing what it found.
//!
//! Ports `queueSearch`, `runSearch`, `resetSearch` and the search parts of
//! `renderRows`, `updateStatus` and `onKey` in `desktop/ui/app.js`
//! (SRCH-001 to SRCH-003, SRCH-007, SRCH-012 to SRCH-015, SRCH-018,
//! NAV-013). Each keystroke stops the running search, clears the
//! selection, scrolls to the top and filters the listing at once; after
//! the 120 ms pause the search runs. A folder the cache covers, or a
//! search of every cached folder, shows the cache's results in place of
//! the listing, with Folder path in place of Date modified. The window's
//! part of the work is here; the search itself is in [`crate::search`].

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::parent_location;
use ox_core::search::{HiddenItems, SearchQuery, SearchResults};

use crate::folder_view::details::DetailsListing;
use crate::folder_view::item::FileItem;
use crate::locations::Page;
use crate::search::{
    merge_results, related_roots, CacheError, Listing, SearchCount, SearchInfoStrip, SearchRun, SearchScope,
    RESULT_LIMIT,
};

use super::empty_page::EmptyState;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// The strip above the columns while searching.
    pub(super) fn search_strip(&self) -> &SearchInfoStrip {
        &self.imp().search_strip
    }

    /// Follows the search box, the strip and the shared search cache.
    pub(super) fn connect_search(&self) {
        let search_box = self.search_box();
        search_box.connect_query_edited(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |text| window.search_edited(text)
        ));
        search_box.connect_query_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.run_search()
        ));
        let strip = self.search_strip();
        strip.connect_scope_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |scope| window.change_search_scope(scope)
        ));
        strip.connect_clear_requested(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.search_box().clear()
        ));
        self.connect_is_active_notify(|window| {
            if window.is_active() {
                window.context().search_cache().refresh_status();
            }
        });
    }

    /// Follows the search cache every window shares: a new status may
    /// change what the strip offers, and changed names run a shown search
    /// again (`cacheChanged`, SRCH-018). Returns the handlers, which the
    /// window disconnects when it goes away.
    pub(super) fn follow_search_cache(&self) -> Vec<glib::SignalHandlerId> {
        let cache = self.context().search_cache();
        let status = cache.connect_status_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                window.update_cache_folder_action();
                window.show_index_candidates();
            }
        ));
        let contents = cache.connect_contents_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.run_search_again()
        ));
        vec![status, contents]
    }

    /// Each keystroke: stops the running search, drops its results,
    /// filters the listing by the text, clears the selection and scrolls
    /// to the top (`queueSearch`).
    fn search_edited(&self, text: &str) {
        let folder = self.current_uri().unwrap_or_default();
        let roots = self.context().search_cache().roots();
        self.imp().search.borrow_mut().edit(text, &folder, &roots);
        self.show_searched_items();
        self.folder_pane().model().select_none();
        if self.is_searching() {
            self.folder_pane().restore_scroll_position(0.0);
        }
        self.show_search_state();
    }

    /// Runs the search once typing paused (`runSearch`).
    fn run_search(&self) {
        let Some(folder) = self.searched_folder() else {
            return;
        };
        let roots = self.context().search_cache().roots();
        let run = self.imp().search.borrow_mut().begin(&folder, &roots);
        self.show_searched_items();
        self.show_search_state();
        if let Some(run) = run {
            self.ask_search_cache(run);
        }
    }

    /// Runs a shown search again, after the cache changed.
    fn run_search_again(&self) {
        if self.is_searching() {
            self.run_search();
        }
    }

    /// The folder the search box searches: the active tab's folder, not a
    /// landing page.
    fn searched_folder(&self) -> Option<String> {
        let uri = self.current_uri()?;
        Page::from_uri(&uri).is_none().then_some(uri)
    }

    /// Whether the search box holds a search.
    pub(super) fn is_searching(&self) -> bool {
        self.imp().search.borrow().is_active()
    }

    /// Whether a search is waiting for the typing pause or for the cache,
    /// including for the cache's first status, after which it runs again.
    pub(crate) fn is_search_running(&self) -> bool {
        let awaits_status = self.context().search_cache().status().is_none();
        let is_running = self.imp().search.borrow().is_running();
        self.is_searching() && (is_running || awaits_status)
    }

    /// Types `text` into the search box, as the snapshot hook asks.
    pub(crate) fn search_folder(&self, text: &str) {
        self.search_box().set_query(text);
    }

    /// Searches the cache for `run` off the main thread and shows what it
    /// found, unless a newer edit replaced the run.
    fn ask_search_cache(&self, run: SearchRun) {
        let query = SearchQuery {
            text: run.text.clone(),
            scope: (run.scope == SearchScope::ThisFolder).then(|| run.folder.clone()),
            limit: RESULT_LIMIT,
            hidden_items: self.hidden_items(),
        };
        let cache = self.context().search_cache().clone();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let outcome = cache.search(query, run.cancellable.clone()).await;
            if let Some(window) = window.upgrade() {
                window.finish_search(&run, outcome);
            }
        });
    }

    /// Whether "Show hidden files" is on.
    fn hidden_files_shown(&self) -> bool {
        let state = self.window_action_state(WindowAction::Hidden);
        state.and_then(|state| state.get::<bool>()).unwrap_or(false)
    }

    /// Whether hidden items are searched too: while they are shown.
    fn hidden_items(&self) -> HiddenItems {
        if self.hidden_files_shown() {
            HiddenItems::Include
        } else {
            HiddenItems::Skip
        }
    }

    /// Shows what `run` found, or why it failed. Safety rule "a late
    /// answer never replaces a newer one" (SAFE-013): an answer for an
    /// older run or another folder is dropped.
    fn finish_search(&self, run: &SearchRun, outcome: Result<SearchResults, CacheError>) {
        let is_current = self.imp().search.borrow().is_current(run);
        let is_same_folder = self.current_uri().as_deref() == Some(run.folder.as_str());
        if !is_current || !is_same_folder {
            return;
        }
        match outcome {
            Ok(found) => self.keep_search_results(run, found),
            Err(error) if error.is_cancelled() => return,
            Err(error) => self.imp().search.borrow_mut().fail(run, error.to_string()),
        }
        self.show_searched_items();
        self.show_search_state();
    }

    /// Merges the listing's matches with `found` and keeps them as the
    /// search's rows.
    fn keep_search_results(&self, run: &SearchRun, found: SearchResults) {
        let active = self.imp().session.borrow().active_id();
        let store = active.and_then(|id| self.tab_store(id));
        let shows_hidden = self.hidden_files_shown();
        let listing = store.as_ref().filter(|_| run.scope == SearchScope::ThisFolder);
        let listing = listing.map(|items| Listing {
            items,
            folder: &run.folder,
            shows_hidden,
        });
        let merged = merge_results(listing, &run.text, found);
        let rows = gio::ListStore::new::<FileItem>();
        rows.extend_from_slice(&merged.items);
        self.imp()
            .search
            .borrow_mut()
            .finish(run, rows, merged.is_truncated);
    }

    /// Shows the search's rows: the cache's results, or else the listing
    /// filtered by the search box's words.
    fn show_searched_items(&self) {
        let search = self.imp().search.borrow();
        let results = search.results().cloned();
        let words = if results.is_some() {
            String::new()
        } else {
            search.query().to_owned()
        };
        drop(search);
        let active = self.imp().session.borrow().active_id();
        let rows = results.or_else(|| active.and_then(|id| self.tab_store(id)));
        let model = self.folder_pane().model();
        self.change_model(|| {
            model.set_query(&words);
            model.set_store(rows.as_ref());
        });
    }

    /// Shows where the search stands: the strip, the columns, the status
    /// bar and the empty page.
    fn show_search_state(&self) {
        let search = self.imp().search.borrow();
        self.search_strip().show_report(search.report(), search.scope());
        let listing = if search.is_active() {
            DetailsListing::SearchResults
        } else {
            DetailsListing::Folder
        };
        drop(search);
        self.folder_pane().details().show_listing(listing);
        self.update_content();
        self.update_details_pane();
    }

    /// The user chose another scope: the search runs again in it.
    fn change_search_scope(&self, scope: SearchScope) {
        let changed = self.imp().search.borrow().scope() != scope;
        if changed {
            self.imp().search.borrow_mut().set_scope(scope);
            self.run_search();
        }
    }

    /// Ends the search as leaving the folder does: empties the box and
    /// forgets the scope.
    pub(super) fn end_search(&self) {
        self.imp().search.borrow_mut().end();
        self.search_box().clear();
        self.show_searched_items();
        self.show_search_state();
    }

    /// What the empty page says while a search shows nothing.
    pub(super) fn search_empty_state(&self) -> Option<EmptyState> {
        let search = self.imp().search.borrow();
        let report = search.report()?;
        Some(EmptyState::NoMatches(report.empty_message().to_owned()))
    }

    /// What the status bar counts while searching.
    pub(super) fn search_count(&self, shown: u32) -> Option<SearchCount> {
        let search = self.imp().search.borrow();
        search.report().map(|report| report.count(shown))
    }

    /// F5 or Ctrl+R while searching: rescans the first indexed folder
    /// around this one and runs the search again, instead of listing the
    /// folder again (NAV-013). Returns false while nothing is searched.
    pub(super) fn refresh_search(&self) -> bool {
        if !self.is_searching() {
            return false;
        }
        let folder = self.current_uri().unwrap_or_default();
        let roots = self.context().search_cache().roots();
        let root = related_roots(&roots, &folder).next().map(|root| root.uri.clone());
        if let Some(root) = root {
            let cache = self.context().search_cache().clone();
            glib::spawn_future_local(async move {
                // A refused refresh changes nothing the search shows.
                let _ = cache.refresh(Some(&root)).await;
            });
        }
        self.run_search();
        true
    }

    /// "Open file location" on a search result: opens the folder it is in,
    /// with the result selected and scrolled into view (SRCH-015).
    pub(super) fn open_result_location(&self) {
        let items = self.folder_pane().model().selected_items();
        let [item] = items.as_slice() else {
            return;
        };
        let uri = item.entry().uri.clone();
        let Some(folder) = parent_location(&uri) else {
            return;
        };
        if let Err(error) = self.navigate(&folder) {
            self.show_message(&error.to_string());
            return;
        }
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.selected = vec![uri.clone()];
            tab.revealed_item = Some(uri);
        }
    }

    /// Scrolls tab `id`'s item that "Open file location" asked for into
    /// view, once the tab has listed its folder.
    pub(super) fn reveal_located_item(&self, id: super::session::TabId) {
        let revealed = {
            let mut session = self.imp().session.borrow_mut();
            session.tab_mut(id).and_then(|tab| tab.revealed_item.take())
        };
        let Some(uri) = revealed else {
            return;
        };
        let model = self.folder_pane().model();
        let position = (0..model.n_items()).find(|position| {
            let item = model.item(*position);
            item.is_some_and(|item| item.entry().uri == uri)
        });
        if let Some(position) = position {
            self.folder_pane().reveal(position);
        }
    }
}
