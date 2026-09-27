// SPDX-License-Identifier: AGPL-3.0-only
//! Changing location: tabs, history, Up, and what the frame shows for the
//! active tab.
//!
//! Ports `addTab`, `closeTab`, `switchTab`, `navigate`, `goHistory` and
//! `renderNavigation` in `desktop/ui/app.js`. Titles, addresses and crumbs
//! come from the window's [`LocationContext`], so a phone is called by its
//! mount name everywhere.
//!
//! [`LocationContext`]: ox_core::location::LocationContext

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{self, is_device_location, parent_location, LocationError};

use crate::icons::{ArtKind, Glyph};
use crate::locations::{self, Page};

use super::address_bar::{AddressIcon, ArtStyle, CrumbButton};
use super::loading::LoadMode;
use super::session::{TabId, TabPlacement};
use super::tab_strip::{TabIcon, TabLabel};
use super::BrowserWindow;

/// The address-bar icon for a location (`address-icon` in
/// `renderNavigation`): the page's glyph, the network glyph for SMB, a
/// phone for devices, else the colour folder.
fn address_icon(uri: &str) -> AddressIcon {
    if let Some(page) = Page::from_uri(uri) {
        return AddressIcon::Glyph(page.glyph());
    }
    if uri.starts_with("smb:") {
        AddressIcon::Glyph(Glyph::Network)
    } else if is_device_location(uri) {
        AddressIcon::Glyph(Glyph::Phone)
    } else {
        AddressIcon::Folder
    }
}

/// A tab's icon: the page glyph, a phone, network art or a folder.
fn tab_icon(uri: &str) -> TabIcon {
    if let Some(page) = Page::from_uri(uri) {
        return TabIcon::Glyph(page.glyph());
    }
    if is_device_location(uri) {
        TabIcon::Glyph(Glyph::Phone)
    } else if uri.starts_with("smb:") {
        TabIcon::Art(ArtKind::NetworkFolder)
    } else {
        TabIcon::Art(ArtKind::Folder)
    }
}

impl BrowserWindow {
    /// The canonical location for an address: the home folder for its
    /// legacy page names, a landing page by URI or title, else a folder
    /// relative to the current one (or to home on a landing page).
    pub(super) fn resolve_address(&self, address: &str) -> Result<String, LocationError> {
        let home = self.imp().locations.borrow().home_uri();
        let typed = address.trim();
        if locations::is_home_alias(typed) || typed.eq_ignore_ascii_case("home") {
            return Ok(home);
        }
        if let Some(page) = Page::from_uri(typed).or_else(|| Page::from_title(typed)) {
            return Ok(page.uri().to_owned());
        }
        let current = self.current_uri();
        let base = current
            .as_deref()
            .filter(|uri| Page::from_uri(uri).is_none())
            .unwrap_or(&home);
        location::normalise_location(address, Some(base), &glib::home_dir())
    }

    /// Adds a tab for `address`, in front or in the background. A
    /// background tab is listed when it is first shown.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub fn open_tab(&self, address: &str, placement: TabPlacement) -> Result<(), LocationError> {
        let uri = self.resolve_address(address)?;
        self.save_tab_view();
        let id = self.imp().session.borrow_mut().add(&uri, placement);
        self.context().remember_network(&uri);
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
    pub fn add_tab(&self, address: &str) -> Result<(), LocationError> {
        self.open_tab(address, TabPlacement::Foreground)
    }

    /// Navigates the active tab, or opens a first tab.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; the current
    /// folder stays.
    pub fn navigate(&self, address: &str) -> Result<(), LocationError> {
        let uri = self.resolve_address(address)?;
        let id = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.active_mut() else {
                drop(session);
                return self.add_tab(&uri);
            };
            tab.history.push(&uri);
            tab.forget_location_state();
            tab.id
        };
        self.leave_location();
        self.context().remember_network(&uri);
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
        Ok(())
    }

    /// Navigates, showing a refused address in the message line.
    pub(super) fn navigate_or_report(&self, address: &str) {
        if let Err(error) = self.navigate(address) {
            self.chrome().show_message(error.message());
        }
    }

    /// Clears what belonged to the folder the active tab leaves: the
    /// filter, the selection and the type-to-select prefix.
    fn leave_location(&self) {
        self.imp().changing_model.set(true);
        self.chrome().clear_filter();
        self.content().model.set_query("");
        self.content().model.select_none();
        self.imp().changing_model.set(false);
        self.reset_typeahead();
    }

    /// Remembers the active tab's selection, for a reload or tab switch.
    pub(super) fn save_selection(&self) {
        let selected = self.content().model.selected_uris();
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.selected = selected;
        }
    }

    /// Remembers the active tab's selection and scroll position before
    /// another tab is shown.
    fn save_tab_view(&self) {
        self.save_selection();
        let scroll = self.content().scroll_position();
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.scroll = scroll;
        }
    }

    /// Shows another tab.
    pub(super) fn switch_tab(&self, id: TabId) {
        let session = self.imp().session.borrow();
        if session.is_active(id) || session.tab(id).is_none() {
            return;
        }
        drop(session);
        self.save_tab_view();
        self.imp().session.borrow_mut().active = Some(id);
        self.show_tab(id);
    }

    /// Puts the active tab's items, selection and scroll position on
    /// screen, and lists a tab that was opened in the background.
    fn show_tab(&self, id: TabId) {
        self.reset_typeahead();
        self.chrome().show_message("");
        let (store, selected, scroll, needs_listing) = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.tab(id) else { return };
            let needs_listing = !tab.loaded && !tab.loading;
            (tab.store.clone(), tab.selected.clone(), tab.scroll, needs_listing)
        };
        let had_focus = self.content().has_focus();
        let model = &self.content().model;
        self.imp().changing_model.set(true);
        self.chrome().clear_filter();
        model.set_query("");
        model.set_store(Some(&store));
        model.select_uris(&selected);
        self.imp().changing_model.set(false);
        self.render_navigation();
        self.update_content();
        self.update_details_pane();
        self.content().restore_scroll_position(scroll);
        if had_focus {
            self.content().focus();
        }
        if needs_listing {
            self.load_tab(id, LoadMode::Navigate);
        }
    }

    /// Closes a tab; closing the last one closes the window.
    pub(super) fn close_tab(&self, id: TabId) {
        if self.tab_count() <= 1 {
            self.close();
            return;
        }
        self.save_tab_view();
        let was_active = self.imp().session.borrow().is_active(id);
        self.imp().session.borrow_mut().remove(id);
        if !was_active {
            self.render_tabs();
            return;
        }
        let next = self.imp().session.borrow().active;
        match next {
            Some(next) => self.show_tab(next),
            None => self.close(),
        }
    }

    /// Shows the tab `delta` places from the active one, wrapping around.
    pub(super) fn cycle_tabs(&self, delta: isize) {
        let next = self.imp().session.borrow().adjacent(delta);
        if let Some(id) = next {
            self.switch_tab(id);
        }
    }

    /// Lists the active folder again, keeping its rows, selection and
    /// scroll position, and re-reads the shared settings.
    pub fn refresh(&self) {
        self.context().reload_settings();
        self.save_selection();
        let active = self.imp().session.borrow().active;
        if let Some(id) = active {
            self.load_tab(id, LoadMode::Reload);
        }
    }

    /// Moves through the active tab's history; out-of-range steps do nothing.
    pub fn go_history(&self, delta: isize) {
        let id = {
            let mut session = self.imp().session.borrow_mut();
            let Some(tab) = session.active_mut() else { return };
            if tab.history.go(delta).is_none() {
                return;
            }
            tab.forget_location_state();
            tab.id
        };
        self.leave_location();
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
    }

    /// Opens the folder that contains the current one.
    pub(super) fn go_up(&self) {
        let parent = self.current_uri().as_deref().and_then(parent_location);
        if let Some(parent) = parent {
            self.navigate_or_report(&parent);
        }
    }

    /// Updates the frame after the active tab moved: the address bar
    /// returns to breadcrumbs.
    pub(super) fn render_navigation(&self) {
        self.render_location();
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.chrome().address.show_crumbs(&address);
        }
    }

    /// Updates the window title, history buttons, breadcrumbs, tabs,
    /// sidebar highlight and landing page for the active tab's location.
    pub(super) fn render_location(&self) {
        let (uri, can_go_back, can_go_forward) = {
            let session = self.imp().session.borrow();
            let Some(tab) = session.active() else { return };
            let history = &tab.history;
            (
                tab.uri().to_owned(),
                history.can_go_back(),
                history.can_go_forward(),
            )
        };
        let locations = self.imp().locations.borrow().clone();
        self.set_title(Some(&format!("{} — OpenXplorer", locations.title_for(&uri))));
        self.set_action_enabled("back", can_go_back);
        self.set_action_enabled("forward", can_go_forward);
        self.set_action_enabled("up", parent_location(&uri).is_some());
        let breadcrumbs = locations.breadcrumbs(&uri);
        let crumbs: Vec<CrumbButton> = breadcrumbs
            .iter()
            .enumerate()
            .map(|(index, crumb)| CrumbButton {
                address: locations.display_location(&crumb.uri),
                divider_before: location::crumb_divider(&uri, &breadcrumbs, index),
                crumb: crumb.clone(),
            })
            .collect();
        let address = locations.display_location(&uri);
        let style = ArtStyle {
            appearance: self.skin().appearance(),
            scale: self.scale_factor(),
        };
        self.chrome()
            .address
            .show_location(&crumbs, &address, address_icon(&uri), style);
        let search = &self.chrome().search;
        search.set_folder_title(&locations.title_for(&uri));
        search.set_enabled(Page::from_uri(&uri).is_none() && !is_device_location(&uri));
        self.set_action_enabled("pin-folder", Page::from_uri(&uri).is_none());
        self.render_tabs();
        self.sidebar().select(&uri);
        self.render_landing();
    }

    /// Redraws the tab strip.
    pub(super) fn render_tabs(&self) {
        let labels: Vec<TabLabel> = {
            let session = self.imp().session.borrow();
            let locations = self.imp().locations.borrow();
            session
                .tabs
                .iter()
                .map(|tab| {
                    let uri = tab.uri();
                    let mut tooltip = locations.display_location(uri);
                    if uri.starts_with("smb:") {
                        tooltip.push_str(" · Network location");
                    }
                    TabLabel {
                        id: tab.id,
                        title: locations.title_for(uri),
                        tooltip,
                        icon: tab_icon(uri),
                        active: session.is_active(tab.id),
                    }
                })
                .collect()
        };
        let appearance = self.skin().appearance();
        self.chrome().tabs.show(&labels, appearance, self.scale_factor());
    }

    /// Replaces the breadcrumbs with the editable address (Ctrl+L).
    pub(super) fn edit_address(&self) {
        let Some(uri) = self.current_uri() else { return };
        let address = self.imp().locations.borrow().display_location(&uri);
        self.chrome().address.edit(&address);
    }

    /// Ends editing with Enter or Escape: back to the breadcrumbs, with
    /// keyboard focus in the folder view.
    pub(super) fn finish_address(&self) {
        if let Some(uri) = self.current_uri() {
            let address = self.imp().locations.borrow().display_location(&uri);
            self.chrome().address.show_crumbs(&address);
        }
        self.content().focus();
    }
}
