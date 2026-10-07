// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's Compact view (View > Compact view, and Settings >
//! Appearance): the Details rows, the sidebar's rows and the folder tree's
//! rows stand closer, so more items fit (VIEW-067). Off by default, as in
//! Windows.
//!
//! Inside the app this is the compact density: "compact" already names the
//! List layout (`FolderView::Compact`). The choice is saved and every
//! window follows it. On, the window carries the [`DENSITY_CLASS`] class,
//! which the text-size stylesheet (`theme/fonts.rs`) and
//! `resources/skin/sidebar.css` match. The Icons and List layouts keep
//! their spacing: the List layout counts its rows per column from its row
//! height in code (`folder_view/grid.rs`).

use gtk::prelude::*;

use super::actions::toggle_action;
use super::preferences::Preference;
use super::widget_tree::toggle_class;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The window's class while Compact view is on.
const DENSITY_CLASS: &str = "compact-density";

impl BrowserWindow {
    /// Adds the Compact view toggle, starting from the saved preference.
    pub(super) fn install_compact_density_action(&self) {
        let compact = self.context().settings_data().preferences.compact_density;
        self.show_compact_density(compact);
        self.add_action_entries([toggle_action(
            WindowAction::CompactDensity,
            compact,
            |window, compact| {
                window.show_compact_density(compact);
                window.save_preference(Preference::CompactDensity(compact));
            },
        )]);
    }

    /// Draws the rows closer, or at their usual spacing.
    fn show_compact_density(&self, compact: bool) {
        toggle_class(self, DENSITY_CLASS, compact);
    }

    /// Whether Compact view is on, `None` before the action is installed.
    fn compact_density_state(&self) -> Option<bool> {
        let state = self.window_action_state(WindowAction::CompactDensity)?;
        state.get::<bool>()
    }

    /// Takes up a Compact view choice that Settings or another window saved.
    pub(super) fn follow_compact_density_preference(&self) {
        let saved = self.context().settings_data().preferences.compact_density;
        if self.compact_density_state().is_some_and(|shown| shown != saved) {
            self.set_action_state(WindowAction::CompactDensity, &saved.to_variant());
            self.show_compact_density(saved);
        }
    }

    /// Whether the rows are drawn close together, for tests.
    #[cfg(test)]
    pub(crate) fn shows_compact_density(&self) -> bool {
        self.has_css_class(DENSITY_CLASS)
    }
}
