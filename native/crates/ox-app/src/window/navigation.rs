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
use ox_core::location::{self, parent_location, LocationError};

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
        let Some(id) = self.push_location(&uri) else {
            return self.add_tab(&uri);
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
            self.chrome().search.clear();
            self.content().model.set_query("");
            self.content().model.select_none();
        });
        self.reset_typeahead();
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
        let can_switch = {
            let session = self.imp().session.borrow();
            !session.is_active(id) && session.tab(id).is_some()
        };
        if !can_switch {
            return;
        }
        self.save_tab_view();
        self.imp().session.borrow_mut().active = Some(id);
        self.show_tab(id);
    }

    /// Puts the active tab's items, selection and scroll position on
    /// screen, and lists a tab that was opened in the background.
    fn show_tab(&self, id: TabId) {
        self.reset_typeahead();
        self.chrome().show_message("");
        let Some(view) = self.saved_tab_view(id) else {
            return;
        };
        let had_focus = self.content().has_focus();
        self.change_model(|| {
            let model = &self.content().model;
            self.chrome().search.clear();
            model.set_query("");
            model.set_store(Some(&view.store));
            model.select_uris(&view.selected);
        });
        self.render_navigation();
        self.update_content();
        self.update_details_pane();
        self.content().restore_scroll_position(view.scroll);
        if had_focus {
            self.content().focus();
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
            needs_listing: !tab.loaded && !tab.loading,
        })
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

    /// Shows the tab next to the active one in `direction`, wrapping
    /// around at either end.
    pub(super) fn cycle_tabs(&self, direction: Direction) {
        let next = self.imp().session.borrow().adjacent(direction);
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

    /// Moves one step through the active tab's history; at either end of
    /// it nothing happens.
    pub(super) fn go_history(&self, direction: Direction) {
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
