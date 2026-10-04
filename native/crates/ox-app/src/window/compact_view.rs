// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's Compact view (View > Compact view, and Settings >
//! Appearance): the Details rows, the sidebar's rows and the folder tree's
//! rows stand closer, so more items fit (VIEW-067). Off by default, as in
//! Windows.
//!
//! The choice is saved and every window follows it. On, the window carries
//! the [`COMPACT_CLASS`] class, which the text-size stylesheet
//! (`theme/fonts.rs`) and `resources/skin/sidebar.css` match. The Icons
//! and List layouts keep their spacing: the List layout counts its rows
//! per column from its row height in code (`folder_view/grid.rs`).

use gtk::prelude::*;

use super::actions::toggle_action;
use super::preferences::Preference;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The window's class while Compact view is on. Not `compact`, which the
/// List layout's grid and the compact menus already use.
pub(crate) const COMPACT_CLASS: &str = "compact-density";

impl BrowserWindow {
    /// Adds the Compact view toggle, starting from the saved preference.
    pub(super) fn install_compact_view_action(&self) {
        let on = self.context().settings_data().preferences.compact_view;
        self.show_compact_view(on);
        self.add_action_entries([toggle_action(WindowAction::CompactView, on, |window, on| {
            window.show_compact_view(on);
            window.save_preference(Preference::CompactView(on));
        })]);
    }

    /// Draws the rows closer, or at their usual spacing.
    fn show_compact_view(&self, on: bool) {
        if on {
            self.add_css_class(COMPACT_CLASS);
        } else {
            self.remove_css_class(COMPACT_CLASS);
        }
    }

    /// Whether Compact view is on, `None` before the action is installed.
    fn compact_view_state(&self) -> Option<bool> {
        self.window_action_state(WindowAction::CompactView)?.get::<bool>()
    }

    /// Takes up a Compact view choice that Settings or another window saved.
    pub(super) fn follow_compact_view_preference(&self) {
        let saved = self.context().settings_data().preferences.compact_view;
        if self.compact_view_state().is_some_and(|shown| shown != saved) {
            self.set_action_state(WindowAction::CompactView, &saved.to_variant());
            self.show_compact_view(saved);
        }
    }

    /// Whether the rows are drawn close together, for tests.
    #[cfg(test)]
    pub(crate) fn shows_compact_view(&self) -> bool {
        self.has_css_class(COMPACT_CLASS)
    }
}
