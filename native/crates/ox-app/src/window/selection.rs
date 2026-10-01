// SPDX-License-Identifier: AGPL-3.0-only
//! Following the selection: each tab remembers its own, and the status
//! bar, the details pane and the commands that act on one item follow it.
//!
//! Ports `updateStatus`, `renderDetails` and the selection bookkeeping of
//! `desktop/ui/app.js` (each tab's `selected` set). When the window swaps,
//! reloads or clears the folder model, GTK reports selection changes the
//! user did not make; [`BrowserWindow::change_model`] keeps those from
//! overwriting the tab's saved selection.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::is_smb_location;

use crate::locations::Page;

use super::details_pane::{self, PaneFacts};
use super::status_bar::StatusSubject;
use super::window_action::WindowAction;
use super::BrowserWindow;

impl BrowserWindow {
    /// Updates the status bar and the details pane whenever the selection
    /// or the shown items change.
    pub(super) fn follow_selection(&self) {
        let model = self.folder_pane().model();
        model.selection().connect_selection_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _| window.selection_changed()
        ));
        model.sorted().connect_items_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _, _, _| window.update_status()
        ));
    }

    fn selection_changed(&self) {
        if !self.imp().changing_model.get() {
            self.save_selection();
        }
        self.update_status();
        self.update_details_pane();
        let selected = self.folder_pane().model().summary().count;
        self.set_action_enabled(WindowAction::Open, selected >= 1);
        // Copy path copies one item, or the folder when none is selected.
        self.set_action_enabled(WindowAction::CopyPath, selected <= 1);
        self.set_action_enabled(WindowAction::PinSelected, selected == 1);
        self.update_file_commands();
        self.update_open_location_action(selected);
        self.update_properties_actions();
        self.update_size_actions();
        self.update_archive_actions();
    }

    /// Runs `change`, which swaps, reloads or clears the folder model,
    /// without saving the selection changes it causes as the tab's own.
    pub(super) fn change_model(&self, change: impl FnOnce()) {
        // A change may run inside another, as ending a search does while
        // the window leaves a folder.
        let was_changing = self.imp().changing_model.replace(true);
        change();
        self.imp().changing_model.set(was_changing);
    }

    /// Remembers the active tab's selection, for a reload or tab switch.
    /// A selection a Show in folder request asked for is kept until its
    /// listing shows it.
    pub(super) fn save_selection(&self) {
        let selected = self.folder_pane().model().selected_uris();
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            if !tab.reveals_selection {
                tab.selected = selected;
            }
        }
    }

    /// Shows the item count and the selection in the status bar.
    pub(super) fn update_status(&self) {
        let on_page = self.current_uri().as_deref().and_then(Page::from_uri).is_some();
        let shown = self.folder_pane().model().n_items();
        let subject = if on_page {
            StatusSubject::Page
        } else if let Some(count) = self.search_count(shown) {
            StatusSubject::Search(count)
        } else {
            StatusSubject::Folder {
                shown,
                loading: self.is_loading(),
            }
        };
        let selected = self.folder_pane().model().summary();
        self.status_bar().set_counts(subject, selected);
    }

    /// Shows the selection's properties, or the folder's, in the details
    /// pane.
    pub(super) fn update_details_pane(&self) {
        let selection = self.folder_pane().model().selected_items();
        let Some(folder_uri) = self.current_uri() else {
            return;
        };
        let active = self.imp().session.borrow().active_id();
        let store = active.and_then(|id| self.tab_store(id));
        let model = self.folder_pane().model();
        let folder_item_count = store.map_or(0, |store| model.listed_count(&store));
        // Only an SMB folder's picture needs the Network list, and the pane
        // follows every change of the selection.
        let network = if is_smb_location(&folder_uri) {
            self.network_locations()
        } else {
            Vec::new()
        };
        let locations = self.imp().locations.borrow();
        let content = details_pane::pane_content(&PaneFacts {
            selection: &selection,
            folder_uri: &folder_uri,
            folder_item_count,
            locations: &locations,
            network: &network,
        });
        self.details_pane().set_content(&content);
    }
}
