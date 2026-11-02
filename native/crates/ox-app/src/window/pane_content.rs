// SPDX-License-Identifier: AGPL-3.0-only
//! Which page a folder pane shows for its tab: the listing, the empty or
//! error page, or a landing page, with the loading line over it, and
//! "Working on it..." while a location takes a moment to open
//! ([`super::slow_location`]).
//!
//! Ports the page choice of `renderContent` in `v2.0.0:desktop/ui/app.js`.
//! The active pane's page goes with the status bar and the commands; the
//! pane beside it in a split tab shows its own page alone
//! ([`super::split_view`]).

use gtk::subclass::prelude::*;

use crate::locations::Page;

use super::empty_page::EmptyState;
use super::folder_pane::{FolderPane, PanePage};
use super::session::Tab;
use super::BrowserWindow;

/// What decides the page of a pane: where its tab is and how its listing
/// stands.
#[derive(Debug)]
pub(super) struct PaneState {
    /// The tab's location.
    uri: String,
    /// The landing page at that location, if it is one.
    page: Option<Page>,
    /// Its listing runs.
    loading: bool,
    /// It is going to another location, still looked up or mounted.
    opening: bool,
    /// The running listing lists the same location again.
    reloading: bool,
    /// Why its last listing failed.
    error: Option<String>,
}

impl PaneState {
    /// The state of `tab`.
    pub(super) fn of(tab: &Tab) -> Self {
        Self {
            uri: tab.uri().to_owned(),
            page: Page::from_uri(tab.uri()),
            loading: tab.listing_state.is_listing(),
            opening: tab.opening.is_some(),
            reloading: tab.reloading,
            error: tab.error.as_ref().map(ToString::to_string),
        }
    }
}

/// Shows the page that fits `state` in `pane`; what an empty listing
/// shows comes from `empty_state` when it has something to say (a search
/// without matches). Returns the error a listing with rows shows in the
/// toast instead of a page.
pub(super) fn show_pane_state(
    pane: &FolderPane,
    state: PaneState,
    empty_state: impl FnOnce() -> Option<EmptyState>,
) -> Option<String> {
    let PaneState {
        uri,
        page,
        loading,
        opening,
        reloading,
        error,
    } = state;
    pane.set_loading(opening || (loading && page.is_none()));
    pane.hide_rows(false);
    // VIEW-068: once the location has taken a moment (the loading line's
    // delay), the file area says "Working on it..." in place of the
    // previous folder, as Windows Explorer does. A reload keeps its rows,
    // and a location that answers sooner shows nothing.
    let is_slow = pane.shows_loading_line();
    let is_first_listing = loading && !reloading && page.is_none() && error.is_none();
    if is_slow && (opening || (is_first_listing && pane.model().n_items() == 0)) {
        pane.show_page(PanePage::Listing);
        pane.show_working_note(opening);
        return None;
    }
    let shows_empty = pane.page() == Some(PanePage::Empty) || pane.shows_empty_note();
    if page.is_some() {
        pane.show_empty_note(false);
        pane.show_page(PanePage::Landing);
    } else if loading && reloading && pane.model().n_items() == 0 && shows_empty {
        // Listing an empty or unavailable location again keeps its page
        // until the listing ends, as a reload keeps its rows. A tab with
        // rows never keeps the page another tab left.
    } else if pane.model().n_items() > 0 || (loading && error.is_none()) {
        // A folder being listed keeps the blank list, with its column
        // titles, until items come: no "Loading" text, no page swap.
        pane.show_empty_note(false);
        pane.show_page(PanePage::Listing);
        return error;
    } else {
        let state = match error {
            Some(error) => EmptyState::Unavailable(error),
            None => empty_state().unwrap_or_else(|| EmptyState::empty_listing(&uri)),
        };
        if state == EmptyState::EmptyFolder {
            // As in Windows Explorer, an empty folder keeps its list, with
            // the column titles, and says so near its top.
            pane.show_page(PanePage::Listing);
            pane.show_empty_note(true);
        } else {
            pane.show_empty_note(false);
            pane.show_empty(&state);
        }
    }
    None
}

impl BrowserWindow {
    /// Shows the folder pane state that fits the active tab.
    pub(super) fn update_content(&self) {
        let state = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else { return };
            PaneState::of(tab)
        };
        self.folder_pane()
            .details()
            .show_listing(self.details_listing(&state.uri));
        let error = show_pane_state(self.folder_pane(), state, || self.search_empty_state());
        if let Some(error) = error {
            self.show_message(&error);
        }
        self.apply_view_options();
        self.update_status();
        self.update_file_commands();
        self.show_busy_state();
    }
}
