// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar's two options, as Dolphin has them, which app.js
//! lacked: an address that stays editable text instead of crumbs
//! (NAV-029), and crumbs that show the full path from `/` instead of
//! starting at the home folder (NAV-024).
//!
//! Both are toggles in the address bar's menu. The full-path option is
//! saved and every window follows it; the editable address is per window,
//! and Settings chooses how new windows start.

use gtk::prelude::*;

use super::actions::toggle_action;
use super::preferences::Preference;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Adds the two toggles, starting from the saved preferences.
    pub(super) fn install_address_actions(&self) {
        let preferences = self.context().settings_data().preferences;
        self.address_bar()
            .set_always_editable(preferences.editable_location);
        self.add_action_entries([
            toggle_action(
                WindowAction::EditableLocation,
                preferences.editable_location,
                |window, editable| {
                    window.address_bar().set_always_editable(editable);
                    window.render_navigation();
                },
            ),
            toggle_action(
                WindowAction::ShowFullPath,
                preferences.show_full_path,
                |window, full_path| {
                    window.render_navigation();
                    window.save_preference(Preference::ShowFullPath(full_path));
                },
            ),
        ]);
    }

    /// Whether the crumbs show the full path from `/`, `None` before the
    /// actions are installed.
    fn full_path_state(&self) -> Option<bool> {
        let state = self.window_action_state(WindowAction::ShowFullPath)?;
        state.get::<bool>()
    }

    /// Whether the crumbs show the full path from `/`.
    pub(super) fn shows_full_path(&self) -> bool {
        self.full_path_state().unwrap_or(false)
    }

    /// Takes up a full-path option another window saved.
    pub(super) fn follow_full_path_preference(&self) {
        let saved = self.context().settings_data().preferences.show_full_path;
        if self.full_path_state().is_some_and(|shown| shown != saved) {
            self.set_action_state(WindowAction::ShowFullPath, &saved.to_variant());
            self.render_navigation();
        }
    }
}
