// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11's View > Show > Item check boxes (SEL-014), here View >
//! Item check boxes and Settings > Item check boxes: one choice, saved,
//! that every window follows. On by default, as in Windows 11.
//!
//! The check boxes are the folder views' (`folder_view/cells/item_check.rs`
//! and `folder_view/details/select_all.rs`); this is the toggle. The
//! preference keeps its saved name, `selectionMarker`, from when this was a
//! hover marker.

use gtk::prelude::*;

use super::actions::toggle_action;
use super::preferences::Preference;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Adds the Item check boxes toggle, starting from the saved
    /// preference.
    pub(super) fn install_item_check_boxes_action(&self) {
        let shown = self.context().settings_data().preferences.selection_marker;
        self.add_action_entries([toggle_action(
            WindowAction::ItemCheckBoxes,
            shown,
            |window, shown| {
                window.show_item_check_boxes(shown);
                window.save_preference(Preference::ItemCheckBoxes(shown));
            },
        )]);
    }

    /// Gives every item of both panes a check box, and the Details header
    /// its select-all box, or none.
    pub(super) fn show_item_check_boxes(&self, shown: bool) {
        for pane in self.folder_panes() {
            pane.owners().set_item_checks(shown);
            pane.details().show_select_all(shown);
        }
    }

    /// Whether the toggle is on, `None` before the action is installed.
    fn item_check_boxes_state(&self) -> Option<bool> {
        let state = self.window_action_state(WindowAction::ItemCheckBoxes)?;
        state.get::<bool>()
    }

    /// Ticks the toggle for a choice that Settings or another window
    /// saved; the panes take it up in `follow_item_preferences`.
    pub(super) fn follow_item_check_boxes_preference(&self) {
        let saved = self.context().settings_data().preferences.selection_marker;
        if self.item_check_boxes_state().is_some_and(|shown| shown != saved) {
            self.set_action_state(WindowAction::ItemCheckBoxes, &saved.to_variant());
        }
    }
}
