// SPDX-License-Identifier: AGPL-3.0-only
//! The searches saved to the sidebar, which every window lists under Quick
//! access (SRCH-038).
//!
//! The file is read and written on a GIO worker thread; the searches as
//! last read are kept here, and every window redraws its places when they
//! change. One change at a time reads and writes the file, so two quick
//! changes both last, and a slower, older answer never replaces a newer
//! one.

use std::sync::{Mutex, PoisonError};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::search::{SavedSearch, SavedSearches, SearchError};
use ox_core::LOG_DOMAIN;

use super::AppContext;

/// Held while a change reads and writes the file; counts the readings, so
/// the newest is known.
static FILE_TURN: Mutex<u64> = Mutex::new(0);

impl AppContext {
    /// The saved searches, oldest first, as last read.
    pub(crate) fn saved_searches(&self) -> Vec<SavedSearch> {
        self.imp().saved_searches.borrow().clone()
    }

    /// Reads the saved searches off the main thread; every window redraws
    /// its places once they are read.
    pub(super) fn read_saved_searches(&self) {
        let file = SavedSearches::new(&self.settings_directory());
        self.update_saved_searches(move || Ok(file.read()), |_| {});
    }

    /// Saves `search` to the sidebar; `reply` hears the outcome.
    pub(crate) fn save_search(
        &self,
        search: SavedSearch,
        reply: impl FnOnce(Result<(), SearchError>) + 'static,
    ) {
        let file = SavedSearches::new(&self.settings_directory());
        self.update_saved_searches(move || file.add(search), reply);
    }

    /// Removes `search` from the sidebar.
    pub(crate) fn forget_search(&self, search: SavedSearch) {
        let file = SavedSearches::new(&self.settings_directory());
        self.update_saved_searches(
            move || file.remove(&search),
            |result| {
                if let Err(error) = result {
                    glib::g_warning!(LOG_DOMAIN, "The saved search could not be removed: {error}");
                }
            },
        );
    }

    /// Runs `change` on a worker thread and keeps the searches it returns.
    fn update_saved_searches(
        &self,
        change: impl FnOnce() -> Result<Vec<SavedSearch>, SearchError> + Send + 'static,
        reply: impl FnOnce(Result<(), SearchError>) + 'static,
    ) {
        let context = self.downgrade();
        let in_turn = move || {
            let mut reading = FILE_TURN.lock().unwrap_or_else(PoisonError::into_inner);
            *reading += 1;
            (*reading, change())
        };
        glib::spawn_future_local(async move {
            // A change that panicked changed nothing.
            let Ok((reading, outcome)) = gio::spawn_blocking(in_turn).await else {
                return;
            };
            let Some(context) = context.upgrade() else {
                return;
            };
            match outcome {
                Ok(searches) => {
                    if reading > context.imp().saved_searches_reading.get() {
                        context.imp().saved_searches_reading.set(reading);
                        context.imp().saved_searches.replace(searches);
                        context.notify_places_changed();
                    }
                    reply(Ok(()));
                }
                Err(error) => reply(Err(error)),
            }
        });
    }
}
