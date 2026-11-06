// SPDX-License-Identifier: AGPL-3.0-only
//! Which page a folder pane shows for its tab: the listing, the empty or
//! error page, or a landing page, and "Working on it..." while a
//! location takes a moment to open
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

/// How a pane's listing stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listing {
    /// No listing runs.
    Idle,
    /// The location is listed after the tab moved there.
    First,
    /// The same location is listed again (F5, Try again, a change the
    /// folder watch saw), so the page on screen stays.
    Again,
}

/// What decides the page of a pane: where its tab is and how its listing
/// stands.
#[derive(Debug)]
pub(super) struct PaneState {
    /// The tab's location.
    uri: String,
    /// The landing page at that location, if it is one; Network is
    /// listed instead.
    page: Option<Page>,
    /// Whether, and how, its listing runs.
    listing: Listing,
    /// It is going to another location, still looked up or mounted.
    opening: bool,
    /// Its location has taken longer than the delay to open and list.
    slow: bool,
    /// Why its last listing failed.
    error: Option<String>,
}

impl PaneState {
    /// The state of `tab`.
    pub(super) fn of(tab: &Tab) -> Self {
        Self {
            uri: tab.uri().to_owned(),
            page: Page::drawn_at(tab.uri()),
            listing: match (tab.listing_state.is_listing(), tab.reloading) {
                (false, _) => Listing::Idle,
                (true, false) => Listing::First,
                (true, true) => Listing::Again,
            },
            opening: tab.opening.is_some(),
            slow: tab.is_slow(),
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
        listing,
        opening,
        slow,
        error,
    } = state;
    pane.hide_rows(false);
    // VIEW-068: once the tab's location has taken a moment (see
    // `slow_delay.rs`), the file area says "Working on it..." in place of
    // the previous folder, as Windows Explorer does. A reload keeps its
    // rows, and a location that answers sooner shows nothing.
    let loading = listing != Listing::Idle;
    let is_first_listing = listing == Listing::First && page.is_none() && error.is_none();
    if slow && (opening || (is_first_listing && pane.model().n_items() == 0)) {
        pane.show_page(PanePage::Listing);
        pane.show_working_note(opening);
        return None;
    }
    let shows_empty = pane.page() == Some(PanePage::Empty) || pane.shows_empty_note();
    if page.is_some() {
        pane.show_empty_note(false);
        pane.show_page(PanePage::Landing);
    } else if listing == Listing::Again && pane.model().n_items() == 0 && shows_empty {
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
        let (id, state) = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else { return };
            (tab.id, PaneState::of(tab))
        };
        self.redraw_when_slow(id);
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
