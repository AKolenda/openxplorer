// SPDX-License-Identifier: AGPL-3.0-only
//! The search cache every window shares, kept in step with the settings.
//!
//! The Python app started one `IndexService` per process and passed it the
//! Auto-index and network interval preferences on every tick
//! (`v2.0.0:desktop/winspace.py:148-163`). Here the context starts the
//! [`SearchCache`] and hands it the preferences and Quick access pins each
//! time the places change, which includes every preference change and
//! every pin made in any window or process (SRCH-026, SRCH-030, SRCH-040).

use gtk::glib;
use gtk::subclass::prelude::*;
use ox_core::search::IndexSettings;

use super::AppContext;
use crate::search::{CacheLocation, IndexerStart, SearchCache, SettingsReading};

impl AppContext {
    /// The search cache the windows share. Until
    /// [`Self::start_search_cache`], it indexes nothing and every folder is
    /// only filtered.
    pub(crate) fn search_cache(&self) -> &SearchCache {
        &self.imp().search_cache
    }

    /// Starts the index service with the cache at `location`, and keeps it
    /// following the settings.
    pub(crate) fn start_search_cache(&self, location: CacheLocation) {
        let data = self.settings_data();
        let start = IndexerStart {
            location,
            settings: IndexSettings::from_preferences(&data.preferences),
            pins: data.pins,
        };
        self.search_cache().start(start);
        self.connect_places_changed(glib::clone!(
            #[weak(rename_to = context)]
            self,
            move || context.update_search_cache()
        ));
    }

    /// Hands the search cache the current preferences and pins.
    fn update_search_cache(&self) {
        let data = self.settings_data();
        let reading = match self.settings_warning() {
            Some(_) => SettingsReading::FellBackToDefaults,
            None => SettingsReading::Sound,
        };
        let settings = IndexSettings::from_preferences(&data.preferences);
        self.search_cache().follow_settings(settings, &data.pins, reading);
    }
}
