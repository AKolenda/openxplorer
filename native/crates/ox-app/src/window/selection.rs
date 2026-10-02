// SPDX-License-Identifier: AGPL-3.0-only
//! Following the selection: each tab remembers its own, and the status
//! bar, the details pane and the commands that act on one item follow it.
//!
//! Ports `updateStatus`, `renderDetails` and the selection bookkeeping of
//! `v2.0.0:desktop/ui/app.js` (each tab's `selected` set). When the window swaps,
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

/// The position to select once the items at `selected` (ascending) are
/// removed from `count` shown items: the first one after the last removed
/// item, or the one before when nothing follows (SEL-017).
fn position_after_removal(count: u32, selected: &[u32]) -> Option<u32> {
    let last = *selected.last()?;
    let kept = |position: &u32| selected.binary_search(position).is_err();
    (last + 1..count)
        .find(kept)
        .or_else(|| (0..last).rev().find(kept))
}

impl BrowserWindow {
    /// Selects only the item called `name` in the active tab, as the
    /// snapshot hook's scene asks; `false` when no such item is listed.
    pub(crate) fn select_named(&self, name: &str) -> bool {
        let model = self.folder_pane().model();
        let position =
            (0..model.n_items()).find(|position| model.name_at(*position).as_deref() == Some(name));
        position
            .inspect(|position| model.select_only(*position))
            .is_some()
    }

    /// The item to select after the selection is moved to the Trash or
    /// deleted, so that Delete can be pressed again; `None` when nothing
    /// would be left.
    pub(super) fn uri_after_selection(&self) -> Option<String> {
        let model = self.folder_pane().model();
        let position = position_after_removal(model.n_items(), &model.selected_positions())?;
        model.item(position).map(|item| item.entry().uri.clone())
    }

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
        self.follow_quick_look();
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
        self.picker_selection_changed();
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
                loading: self.is_loading() && self.folder_pane().shows_loading_line(),
            }
        };
        let selected = self.folder_pane().model().summary();
        self.status_bar().set_counts(subject, selected);
    }

    /// Shows the selection's properties, or the folder's, in the details
    /// pane.
    pub(super) fn update_details_pane(&self) {
        let pane = self.details_pane();
        let selection = match pane.hovered() {
            Some(hovered) => vec![hovered],
            None => self.folder_pane().model().selected_items(),
        };
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
            condensed_dates: pane.options().condensed_dates,
        });
        pane.set_content(&content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: SEL-017
    #[test]
    fn removal_selects_the_next_item_or_the_one_before() {
        assert_eq!(position_after_removal(6, &[1, 2]), Some(3));
        assert_eq!(position_after_removal(6, &[0, 5]), Some(4));
        assert_eq!(position_after_removal(6, &[4, 5]), Some(3));
        assert_eq!(position_after_removal(2, &[0, 1]), None);
        assert_eq!(position_after_removal(3, &[]), None);
    }
}
