// SPDX-License-Identifier: AGPL-3.0-only
//! Changing location: tabs, history and Up.
//!
//! Ports `addTab`, `closeTab`, `switchTab`, `navigate` and `goHistory` in
//! `desktop/ui/app.js`. Each tab keeps its own history, selection and
//! scroll position; moving to another location forgets the selection and
//! the scroll position, and showing another tab puts its own back.
//! [`super::location_view`] draws the result into the frame.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{self, normalise_navigation, parent_location, LocationError, VirtualPlace};

use crate::locations::{self, Page};

use super::loading::LoadMode;
use super::session::{Direction, TabId, TabPlacement};
use super::BrowserWindow;

/// What a tab needs to be put back on screen.
#[derive(Debug)]
struct SavedTabView {
    /// The tab's items.
    store: gio::ListStore,
    /// The URIs of the items it had selected.
    selected: Vec<String>,
    /// Its vertical scroll position.
    scroll: f64,
    /// It was opened in the background and has not been listed yet.
    needs_listing: bool,
}

impl BrowserWindow {
    /// The canonical location for an address: the home folder for its
    /// legacy page names, a landing page by URI, the home folder or a
    /// landing page by title, the Recycle Bin or a folder in it, else a
    /// folder relative to the current one (see [`Self::resolve_relative`]).
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open.
    pub(super) fn resolve_address(&self, address: &str) -> Result<String, LocationError> {
        let typed = address.trim();
        if locations::is_home_alias(typed) {
            return Ok(self.imp().locations.borrow().home_uri());
        }
        if let Some(page) = Page::from_uri(typed) {
            return Ok(page.uri().to_owned());
        }
        if let Some(place) = self.place_titled(typed) {
            return Ok(place);
        }
        if let Some(recycle_bin) = recycle_bin_location(typed)? {
            return Ok(recycle_bin);
        }
        self.resolve_relative(address)
    }

    /// The home folder or landing page whose title is `typed` ("Home",
    /// "This PC", "Network"), as the address bar shows them.
    pub(super) fn place_titled(&self, typed: &str) -> Option<String> {
        if typed.trim().eq_ignore_ascii_case("home") {
            return Some(self.imp().locations.borrow().home_uri());
        }
        Page::from_title(typed).map(|page| page.uri().to_owned())
    }

    /// `address` as a location, relative to the current folder, or to the
    /// home folder on a landing page.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open.
    pub(super) fn resolve_relative(&self, address: &str) -> Result<String, LocationError> {
        let base = self.address_base();
        location::normalise_location(address, Some(&base), &glib::home_dir())
    }

    /// Where a relative address starts: the current folder, or the home
    /// folder on a landing page and before the first tab.
    fn address_base(&self) -> String {
        let folder = self.current_uri().filter(|uri| Page::from_uri(uri).is_none());
        folder.unwrap_or_else(|| self.imp().locations.borrow().home_uri())
    }

    /// Adds a tab for `address`, in front or in the background. A
    /// background tab is listed when it is first shown.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(super) fn open_tab(&self, address: &str, placement: TabPlacement) -> Result<(), LocationError> {
        let uri = self.resolve_address(address)?;
        self.save_tab_view();
        let id = self.imp().session.borrow_mut().add(&uri, placement);
        if self.imp().session.borrow().is_active(id) {
            self.show_tab(id);
        } else {
            self.render_tabs();
        }
        Ok(())
    }

    /// Adds a tab for `address` and shows it.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(crate) fn add_tab(&self, address: &str) -> Result<(), LocationError> {
        self.open_tab(address, TabPlacement::Foreground)
    }

    /// Navigates the active tab, or opens a first tab. A tab that is
    /// being dragged stays where it is (TAB-003).
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; the current
    /// folder stays.
    pub(super) fn navigate(&self, address: &str) -> Result<(), LocationError> {
        let uri = self.resolve_address(address)?;
        if self.refuse_while_active_tab_moves() {
            return Ok(());
        }
        let Some(id) = self.push_location(&uri) else {
            return self.add_tab(&uri);
        };
        self.leave_location();
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
        Ok(())
    }

    /// Navigates, showing a refused address in the message line.
    pub(super) fn navigate_or_report(&self, address: &str) {
        if let Err(error) = self.navigate(address) {
            self.show_message(&error.to_string());
        }
    }

    /// Adds `uri` to the active tab's history; the tab, or `None` before
    /// the window has one.
    fn push_location(&self, uri: &str) -> Option<TabId> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.active_mut()?;
        tab.history.push(uri);
        tab.forget_location_state();
        Some(tab.id)
    }

    /// Clears what belonged to the folder the active tab leaves: the
    /// filter, the selection and the type-to-select prefix.
    fn leave_location(&self) {
        self.change_model(|| {
            self.end_search();
            self.folder_pane().model().select_none();
        });
        self.reset_typeahead();
    }

    /// Remembers the active tab's selection and scroll position before
    /// another tab is shown.
    pub(super) fn save_tab_view(&self) {
        self.save_selection();
        let scroll = self.folder_pane().scroll_position();
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.scroll = scroll;
        }
    }

    /// Shows another tab.
    pub(super) fn switch_tab(&self, id: TabId) {
        let can_switch = self.imp().session.borrow().can_activate(id);
        if !can_switch {
            return;
        }
        // The tab in front keeps its selection and scroll position first.
        self.save_tab_view();
        self.imp().session.borrow_mut().activate(id);
        self.show_tab(id);
    }

    /// Puts the active tab's items, selection and scroll position on
    /// screen, and lists a tab that was opened in the background.
    pub(super) fn show_tab(&self, id: TabId) {
        self.reset_typeahead();
        self.hide_message();
        self.show_dialog_of_tab(id);
        let Some(view) = self.saved_tab_view(id) else {
            return;
        };
        let had_focus = self.folder_pane().view_has_focus();
        self.change_model(|| {
            let model = self.folder_pane().model();
            self.search_box().clear();
            model.set_query("");
            model.set_store(Some(&view.store));
            model.select_uris(&view.selected);
        });
        self.render_navigation();
        self.update_content();
        self.update_details_pane();
        self.folder_pane().restore_scroll_position(view.scroll);
        if had_focus {
            self.folder_pane().focus_view();
        }
        if view.needs_listing {
            self.load_tab(id, LoadMode::Navigate);
        }
    }

    /// What tab `id` needs to be shown again, while it is open.
    fn saved_tab_view(&self, id: TabId) -> Option<SavedTabView> {
        let session = self.imp().session.borrow();
        let tab = session.tab(id)?;
        Some(SavedTabView {
            store: tab.store.clone(),
            selected: tab.selected.clone(),
            scroll: tab.scroll,
            needs_listing: tab.listing_state.needs_listing(),
        })
    }

    /// Closes a tab; closing the last one closes the window, after asking
    /// as its Close button does (`closeTab` calls `askClose`). A tab that
    /// is being dragged stays (TAB-003).
    pub(super) fn close_tab(&self, id: TabId) {
        if self.refuse_while_moving(id) {
            return;
        }
        if self.tab_count() <= 1 {
            self.request_close();
            return;
        }
        self.save_tab_view();
        self.discard_dialog_of_tab(id);
        let was_active = self.imp().session.borrow().is_active(id);
        self.imp().session.borrow_mut().remove(id);
        if !was_active {
            self.render_tabs();
            return;
        }
        let next = self.imp().session.borrow().active_id();
        match next {
            Some(next) => self.show_tab(next),
            None => self.close(),
        }
    }

    /// Shows the tab next to the active one in `direction`, wrapping
    /// around at either end.
    pub(super) fn cycle_tabs(&self, direction: Direction) {
        let next = self.imp().session.borrow().adjacent(direction);
        if let Some(id) = next {
            self.switch_tab(id);
        }
    }

    /// Lists the active folder again, keeping its rows, selection and
    /// scroll position, and re-reads the shared settings. While searching
    /// it refreshes the search instead ([`Self::refresh_search`]).
    pub(super) fn refresh(&self) {
        if self.refresh_search() {
            return;
        }
        self.context().reload_settings();
        self.save_selection();
        let active = self.imp().session.borrow().active_id();
        if let Some(id) = active {
            self.load_tab(id, LoadMode::Reload);
        }
    }

    /// Moves one step through the active tab's history; at either end of
    /// it nothing happens.
    pub(super) fn go_history(&self, direction: Direction) {
        if self.refuse_while_active_tab_moves() {
            return;
        }
        let Some(id) = self.step_history(direction) else {
            return;
        };
        self.leave_location();
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
    }

    /// Moves the active tab's history one step in `direction`; the tab, or
    /// `None` when there is no step to take.
    fn step_history(&self, direction: Direction) -> Option<TabId> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.active_mut()?;
        tab.history.go(direction.offset())?;
        tab.forget_location_state();
        Some(tab.id)
    }

    /// Opens the folder that contains the current one.
    pub(super) fn go_up(&self) {
        let parent = self.current_uri().as_deref().and_then(parent_location);
        if let Some(parent) = parent {
            self.navigate_or_report(&parent);
        }
    }
}

/// The Recycle Bin by its title or URI, or a folder in it, for `typed`;
/// `None` for any other address. The Python app could not show the Trash;
/// the native app lists it like a folder (OPS-040).
///
/// # Errors
///
/// A `trash:` address that is not a canonical location.
fn recycle_bin_location(typed: &str) -> Result<Option<String>, LocationError> {
    let is_titled = VirtualPlace::from_title(typed) == Some(VirtualPlace::RecycleBin);
    if !is_titled && !typed.starts_with("trash:") {
        return Ok(None);
    }
    let address = if is_titled {
        VirtualPlace::RecycleBin.uri()
    } else {
        typed
    };
    normalise_navigation(address, None, &glib::home_dir()).map(Some)
}
