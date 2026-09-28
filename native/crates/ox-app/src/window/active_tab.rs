// SPDX-License-Identifier: AGPL-3.0-only
//! What the window says about its tabs: how many there are, where the
//! active one is and whether it is listed.
//!
//! Ports `active()` and the tab bookkeeping reads of `desktop/ui/app.js`.
//! The application, the snapshot tool and the tests ask these questions;
//! the tabs themselves are kept by [`super::session`].

use gtk::gio;
use gtk::subclass::prelude::*;

use super::session::TabId;
use super::BrowserWindow;

impl BrowserWindow {
    /// Number of tabs in this window.
    pub(crate) fn tab_count(&self) -> usize {
        self.imp().session.borrow().tabs().len()
    }

    /// The items of tab `id`, unfiltered and unsorted, while it is open.
    pub(super) fn tab_store(&self, id: TabId) -> Option<gio::ListStore> {
        let session = self.imp().session.borrow();
        session.tab(id).map(|tab| tab.store.clone())
    }

    /// The active location, or `None` before the first tab is added.
    pub(crate) fn current_uri(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        session.active().map(|tab| tab.uri().to_owned())
    }

    /// Whether the active tab has finished its first listing (a landing
    /// page counts as listed).
    pub(crate) fn is_listed(&self) -> bool {
        let session = self.imp().session.borrow();
        session.active().is_some_and(|tab| tab.listing_state.is_listed())
    }

    /// Whether the active tab is still receiving directory entries.
    pub(crate) fn is_loading(&self) -> bool {
        let session = self.imp().session.borrow();
        session.active().is_some_and(|tab| tab.listing_state.is_listing())
    }

    /// The active listing's failure, if one occurred, for tests.
    #[cfg(test)]
    pub(super) fn load_error(&self) -> Option<String> {
        let session = self.imp().session.borrow();
        let error = session.active()?.error.as_ref()?;
        Some(error.to_string())
    }
}
