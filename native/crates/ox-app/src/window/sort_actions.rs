// SPDX-License-Identifier: AGPL-3.0-only
//! The Sort menu's actions: the sort key, the direction, "Show in groups"
//! and "Folders first", which follow sorting by a column title too.
//!
//! Ports the Sort menu of `setup()` in `v2.0.0:desktop/ui/app.js`, with
//! Dolphin's further sort keys (VIEW-019), its "Show in Groups" (VIEW-022)
//! and its "Folders First". Every change is saved as the folder's style
//! ([`super::view_style`]).

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::actions::{choice_action, plain_action, toggle_action};
use super::window_action::WindowAction;
use super::BrowserWindow;
use crate::folder_view::sorting::{SortColumn, SortDirection};

impl BrowserWindow {
    /// The Sort menu's key and direction choices and its two toggles, which
    /// follow sorting by a column title too.
    pub(super) fn install_sort_actions(&self) {
        self.add_action_entries([
            choice_action(
                WindowAction::Sort,
                SortColumn::Name.as_str(),
                BrowserWindow::sort_by_key,
            ),
            choice_action(
                WindowAction::Direction,
                SortDirection::Ascending.as_str(),
                |window, key| {
                    let Some(direction) = SortDirection::from_key(key) else {
                        return false;
                    };
                    window.sort_in_direction(direction);
                    true
                },
            ),
            toggle_action(WindowAction::Groups, false, |window, grouped| {
                window.show_groups(grouped);
                window.remember_style();
            }),
            toggle_action(WindowAction::FoldersFirst, true, |window, first| {
                window.show_folders_first(first);
                window.remember_style();
            }),
            plain_action(WindowAction::ViewProperties, BrowserWindow::show_view_properties),
        ]);
        self.follow_header_sorting();
    }

    /// Keeps the Sort menu and the groups in step with sorting by a column
    /// title, which also ends sorting by a further key, and saves it.
    fn follow_header_sorting(&self) {
        for pane in self.folder_panes() {
            let Some(sorter) = pane.details().column_view().sorter() else {
                continue;
            };
            sorter.connect_changed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                #[weak]
                pane,
                move |_, _| {
                    // The window sorts by a saved style or a menu choice: it
                    // brings everything in step itself.
                    if window.imp().applying_style.get() {
                        return;
                    }
                    if pane.details().primary_sort().is_some() {
                        pane.model().set_sort_role(None);
                    }
                    if window.is_active_pane(&pane) {
                        window.show_sort_state();
                    }
                    window.remember_pane_style(&pane);
                }
            ));
        }
    }
}
