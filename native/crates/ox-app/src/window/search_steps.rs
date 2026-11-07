// SPDX-License-Identifier: AGPL-3.0-only
//! Search results as a step of the tab's history, as in Windows 11 File
//! Explorer (SRCH-042).
//!
//! When a search runs ([`super::folder_search`]), the tab's history gets
//! a step after the folder, titled "Search Results in Documents" in the
//! tab, the window title, the address bar's last crumb and the Back and
//! Forward menus ([`crate::history::SearchStep`]). Back returns to the
//! folder with the search box empty, Forward to the results, and Back
//! from a folder opened from them shows them again: the step keeps what
//! was searched for and how, and the search runs again. Emptying the box
//! (Escape, its ×, or the strip's Clear search) goes back to the folder
//! as Back does. Editing a shown search changes its step in place. A tab
//! on results that comes back to the front searches again, and once the
//! results are found puts back what was selected in them and how far
//! they were scrolled ([`ResultsView`]).

use gtk::subclass::prelude::*;

use crate::history::SearchStep;

use super::session::{Direction, TabId};
use super::BrowserWindow;

/// Where a tab on search results was in them when another tab came to
/// the front: the results are found again when it comes back, and this
/// is put back once they are.
#[derive(Debug, Clone)]
pub(super) struct ResultsView {
    /// The tab.
    pub(super) tab: TabId,
    /// The URIs of the results it had selected.
    pub(super) selected: Vec<String>,
    /// The URI of the result that had keyboard focus.
    pub(super) focused: Option<String>,
    /// Its vertical scroll position.
    pub(super) scroll: f64,
}

/// How a search that runs is recorded in the tab's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepChange {
    /// A step of its own after the folder, unless the current step shows
    /// a search already, which then changes in place.
    New,
    /// The current step, whatever it shows: the search the strip's pin
    /// keeps runs again in the folder just opened (SRCH-005), whose
    /// navigation was the step.
    InPlace,
}

impl BrowserWindow {
    /// Whether the active tab's current step shows search results.
    pub(super) fn shows_search_step(&self) -> bool {
        let session = self.imp().session.borrow();
        session
            .active()
            .is_some_and(|tab| tab.history.current_search().is_some())
    }

    /// Records the search the window runs in the active tab's history,
    /// as `change` says, and redraws the tab, the title and the address
    /// once the step shows results.
    pub(super) fn record_search_step(&self, change: StepChange) {
        let step = {
            let search = self.imp().search.borrow();
            if !search.is_active() {
                return;
            }
            SearchStep {
                query: search.query().to_owned(),
                scope: search.scope(),
                search_in: search.search_in(),
                facets: search.facets(),
            }
        };
        let shows_results_now = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.active_mut() else {
                return;
            };
            if change == StepChange::InPlace || tab.history.current_search().is_some() {
                tab.history.set_current_search(Some(step))
            } else {
                tab.history.push_search(step);
                tab.left_views.forget_from(tab.history.position());
                true
            }
        };
        if shows_results_now {
            self.render_location();
        }
    }

    /// Remembers where the view is in the folder before the search box
    /// starts filtering it, so Back from the results returns there
    /// (NAV-008).
    pub(super) fn remember_view_before_search(&self) {
        if self.is_searching() || self.shows_search_step() {
            return;
        }
        let Some(view) = self.left_view() else {
            return;
        };
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            let position = tab.history.position();
            tab.left_views.remember(position, view);
        }
    }

    /// The user emptied the search box on search results: back to the
    /// folder, as Back goes, when it is the step before; otherwise the
    /// step stops showing a search.
    pub(super) fn leave_search_step(&self) {
        let goes_back = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else {
                return;
            };
            if tab.history.current_search().is_none() {
                return;
            }
            tab.history.follows_its_folder()
        };
        if goes_back {
            self.go_history(Direction::Backward);
            return;
        }
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.history.set_current_search(None);
        }
        self.render_location();
    }

    /// Shows the search of the active tab's current step again, as it
    /// was searched: its text in the box, its scope and options in the
    /// strip. It runs again once the typing pause is over.
    pub(super) fn show_step_search(&self) {
        self.show_step_search_restoring(None);
    }

    /// [`Self::show_step_search`] for a tab that comes back to the front:
    /// once its results are found again, `view` puts back where it was in
    /// them ([`Self::restore_results_view`]). Putting the text back is
    /// not the user editing it, which starts again from no selection at
    /// the top.
    pub(super) fn show_step_search_restoring(&self, view: Option<ResultsView>) {
        let step = {
            let session = self.imp().session.borrow();
            session
                .active()
                .and_then(|tab| tab.history.current_search().cloned())
        };
        let Some(step) = step else {
            self.imp().results_view.replace(None);
            return;
        };
        self.imp().results_view.replace(view);
        let mut search = self.imp().search.borrow_mut();
        search.set_scope(step.scope);
        search.set_search_in(step.search_in);
        search.set_facets(step.facets);
        drop(search);
        self.imp().restores_results.set(true);
        self.search_box().set_query(&step.query);
        self.imp().restores_results.set(false);
    }

    /// Whether the window is putting a tab's search back in the box,
    /// rather than the user editing it.
    pub(super) fn is_restoring_results(&self) -> bool {
        self.imp().restores_results.get()
    }

    /// Puts back where the active tab was in its search results now that
    /// they are found again: what was selected, and how far they were
    /// scrolled. Nothing changes when another tab is in front; the user
    /// selecting meanwhile forgets it ([`Self::forget_results_view`]).
    pub(super) fn restore_results_view(&self) {
        let Some(view) = self.imp().results_view.take() else {
            return;
        };
        let is_active = self.imp().session.borrow().active_id() == Some(view.tab);
        let pane = self.folder_pane();
        if !is_active {
            return;
        }
        pane.model().select_uris(&view.selected);
        pane.restore_scroll_position(view.scroll);
    }

    /// Forgets where the active tab was in its search results, before
    /// they are found again: the user edited the search or the selection,
    /// or the tab went elsewhere.
    pub(super) fn forget_results_view(&self) {
        self.imp().results_view.replace(None);
    }

    /// Empties the search box for the window's own reasons, such as
    /// leaving the folder or showing another tab, which unlike the user
    /// emptying it does not leave search results.
    pub(super) fn clear_search_box_quietly(&self) {
        self.imp().clears_search_box.set(true);
        self.search_box().clear();
        self.imp().clears_search_box.set(false);
    }

    /// Whether the window is emptying the search box itself.
    pub(super) fn is_clearing_search_box(&self) -> bool {
        self.imp().clears_search_box.get()
    }
}
