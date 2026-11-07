// SPDX-License-Identifier: AGPL-3.0-only
//! What Folder Options' own choices do in a window (VIEW-021), the ones
//! no other part of the app sets: folders opened in a window of their own,
//! the infotips, the status bar, the Preview pane's contents, typing into
//! the search box, searching names and contents, and Clear File Explorer
//! history. Every window follows the saved choices as soon as they are
//! saved, through [`BrowserWindow::follow_item_preferences`].

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{RECENT_LOCATIONS_URI, RECENT_URI};
use ox_core::search::SearchIn;
use ox_core::settings::Preferences;

use super::details_pane::PaneMode;
use super::BrowserWindow;

impl BrowserWindow {
    /// Follows the saved choices of Folder Options that only it sets.
    pub(super) fn follow_folder_options(&self, preferences: &Preferences) {
        for pane in self.folder_panes() {
            pane.owners().set_hides_infotips(preferences.hide_infotips);
        }
        self.fit_status_bar();
        if self
            .details_pane()
            .set_hides_media(preferences.hide_preview_handlers)
            && self.details_pane().mode() == PaneMode::Preview
        {
            self.update_details_pane();
        }
        self.follow_search_contents(preferences.search_contents);
    }

    /// Shows the status bar while files are browsed, unless Folder
    /// Options' "Show status bar" is off.
    pub(super) fn fit_status_bar(&self) {
        let hidden = self.context().settings_data().preferences.hide_status_bar;
        self.status_bar().set_visible(!hidden && !self.shows_settings());
    }

    /// Whether a folder opened from the file list opens in a window of its
    /// own (Folder Options' "Open each folder in its own window"). A file
    /// dialog always opens folders in place.
    pub(super) fn opens_folders_in_own_window(&self) -> bool {
        self.context()
            .settings_data()
            .preferences
            .open_folders_in_own_window
            && !self.is_file_dialog()
    }

    /// Whether text typed in the file list goes to the search box (Folder
    /// Options' "Automatically type into the Search Box"), which it can
    /// while the box takes text.
    pub(super) fn types_into_search_box(&self) -> bool {
        let search_box = self.search_box();
        self.context().settings_data().preferences.type_into_search_box
            && search_box.is_sensitive()
            && search_box.is_visible()
    }

    /// Searches start with "Names and contents" while Folder Options'
    /// "Always search file names and contents" is on, and with "File
    /// names" again once it is off; a search shown runs again with it. The
    /// strip still changes it for the window, as before.
    fn follow_search_contents(&self, on: bool) {
        if self.imp().searches_contents.replace(Some(on)) == Some(on) {
            return;
        }
        let search_in = if on {
            SearchIn::NamesAndContents
        } else {
            SearchIn::Names
        };
        if self.is_searching() {
            let facets = self.imp().search.borrow().facets();
            self.change_search_options(search_in, facets);
        } else {
            self.imp().search.borrow_mut().set_search_in(search_in);
        }
    }

    /// Clear `OpenXplorer` history: forgets the recent files, the recent
    /// locations and the addresses typed in every window, at once as in
    /// Windows, and lists Recent files and Recent locations again where a
    /// window shows them.
    pub(super) fn clear_explorer_history(&self) {
        self.context().clear_recent_files();
        crate::folder_view::recent_locations::clear_recent_locations();
        for window in self.browser_windows() {
            window.address_bar().forget_typed_history();
            let shows_recent = window
                .current_uri()
                .is_some_and(|uri| [RECENT_URI, RECENT_LOCATIONS_URI].contains(&uri.as_str()));
            if shows_recent {
                window.refresh();
            }
        }
    }
}
