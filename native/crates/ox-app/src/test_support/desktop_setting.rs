// SPDX-License-Identifier: AGPL-3.0-only
//! A desktop setting that a test changes, only where `GSettings` keeps its
//! values in memory, and puts back when the test ends.

use gtk::gio;
use gtk::prelude::*;

/// The type of GIO's memory backend, which `GSETTINGS_BACKEND=memory`
/// selects (`native/tools/check.py` sets it).
const MEMORY_BACKEND: &str = "GMemorySettingsBackend";

/// One key of `settings` that a test may write. Dropping the guard resets
/// the key, also when an assertion fails, so the tests that run after it in
/// the same process start from the default.
pub(crate) struct DesktopSetting {
    settings: gio::Settings,
    key: &'static str,
}

impl DesktopSetting {
    /// The guard for `key`, or `None` unless `settings` keep their values
    /// in memory: a test run outside isolation must never write the user's
    /// dconf database.
    pub(crate) fn in_memory(settings: gio::Settings, key: &'static str) -> Option<Self> {
        let backend = settings.property::<gio::SettingsBackend>("backend");
        (backend.type_().name() == MEMORY_BACKEND).then_some(Self { settings, key })
    }

    /// Writes the string `value` to the key.
    pub(crate) fn set_string(&self, value: &str) {
        self.settings
            .set_string(self.key, value)
            .expect("the key is writable in memory");
    }

    /// Writes the boolean `value` to the key.
    pub(crate) fn set_boolean(&self, value: bool) {
        self.settings
            .set_boolean(self.key, value)
            .expect("the key is writable in memory");
    }
}

impl Drop for DesktopSetting {
    fn drop(&mut self) {
        self.settings.reset(self.key);
    }
}
