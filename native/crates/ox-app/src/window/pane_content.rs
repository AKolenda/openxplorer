// SPDX-License-Identifier: AGPL-3.0-only
//! Which page a folder pane shows for its tab: the listing, the empty or
//! error page, or a landing page, with the loading line over it.
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
        reloading,
        error,
    } = state;
    pane.set_loading(loading && page.is_none());
    if page.is_some() {
        pane.show_page(PanePage::Landing);
    } else if loading && reloading && pane.model().n_items() == 0 && pane.page() == Some(PanePage::Empty) {
        // Listing an empty or unavailable location again keeps its page
        // until the listing ends, as a reload keeps its rows. A tab with
        // rows never keeps the page another tab left.
    } else if pane.model().n_items() > 0 || (loading && error.is_none()) {
        // A folder being listed keeps the blank list, with its column
        // titles, until items come: no "Loading" text, no page swap.
        pane.show_page(PanePage::Listing);
        return error;
    } else {
        let state = match error {
            Some(error) => EmptyState::Unavailable(error),
            None => empty_state().unwrap_or_else(|| EmptyState::empty_listing(&uri)),
        };
        pane.show_empty(&state);
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
        self.learn_trash_support();
    }
}
