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
        self.set_action_enabled(WindowAction::Open, selected == 1);
        // Copy path copies one item, or the folder when none is selected.
        self.set_action_enabled(WindowAction::CopyPath, selected <= 1);
    }

    /// Runs `change`, which swaps, reloads or clears the folder model,
    /// without saving the selection changes it causes as the tab's own.
    pub(super) fn change_model(&self, change: impl FnOnce()) {
        self.imp().changing_model.set(true);
        change();
        self.imp().changing_model.set(false);
    }

    /// Remembers the active tab's selection, for a reload or tab switch.
    pub(super) fn save_selection(&self) {
        let selected = self.folder_pane().model().selected_uris();
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.selected = selected;
        }
    }

    /// Shows the item count and the selection in the status bar.
    pub(super) fn update_status(&self) {
        let on_page = self.current_uri().as_deref().and_then(Page::from_uri).is_some();
        let subject = if on_page {
            StatusSubject::Page
        } else {
            StatusSubject::Folder {
                shown: self.folder_pane().model().n_items(),
                loading: self.is_loading(),
            }
        };
        let selected = self.folder_pane().model().summary();
        self.status_bar().show(subject, selected);
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
        let network = self.network_locations();
        let locations = self.imp().locations.borrow();
        let content = details_pane::pane_content(&PaneFacts {
            selection: &selection,
            folder_uri: &folder_uri,
            folder_item_count,
            locations: &locations,
            network: &network,
        });
        self.details_pane().show(&content);
    }
}
