// SPDX-License-Identifier: AGPL-3.0-only
//! Changing location: tabs, history and Up.
//!
//! Ports `addTab`, `closeTab`, `switchTab`, `navigate` and `goHistory` in
//! `v2.0.0:desktop/ui/app.js`. Each tab keeps its own history, selection and
//! scroll position; moving to another location forgets the selection and
//! the scroll position, and showing another tab puts its own back. As in
//! Dolphin, Back and Forward return to where the view was, and going up
//! selects the folder just left.
//! [`super::location_view`] draws the result into the frame.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{self, normalise_navigation, parent_location, LocationError, VirtualPlace};

use crate::history::LeftView;
use crate::locations::{self, Page};

use super::loading::LoadMode;
use super::session::{Direction, PaneSide, TabId, TabPlacement, TabPosition};
use super::BrowserWindow;

/// Where keyboard focus goes when a tab is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FocusOnShow {
    /// Into the tab's list, on the item that had it, when the list had it
    /// before.
    Restore,
    /// Nowhere: a click or a focus change in a split tab's other pane made
    /// it active, and focus is going there already.
    Keep,
}

/// What a tab needs to be put back on screen.
#[derive(Debug)]
struct SavedTabView {
    /// The tab's items.
    store: gio::ListStore,
    /// The URIs of the items it had selected.
    selected: Vec<String>,
    /// The URI of the item that had keyboard focus.
    focused: Option<String>,
    /// Its vertical scroll position.
    scroll: f64,
    /// The folder pane that shows it.
    side: PaneSide,
    /// It was opened in the background and has not been listed yet.
    needs_listing: bool,
    /// Its network folder changed while it was in the background.
    changed_while_hidden: bool,
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
        // Recent locations, which the app lists itself (SIDE-026).
        let recent_locations = VirtualPlace::RecentLocations;
        if [VirtualPlace::from_uri(typed), VirtualPlace::from_title(typed)].contains(&Some(recent_locations))
        {
            return Ok(recent_locations.uri().to_owned());
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

    /// Adds a tab for `address` opened from the active one (a folder, a
    /// place or a crumb), in front or in the background: after the active
    /// tab, or at the end when the settings ask for that (TAB-017). A
    /// background tab is listed when it is first shown.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(super) fn open_tab(&self, address: &str, placement: TabPlacement) -> Result<(), LocationError> {
        self.open_tab_at(address, placement, self.opened_tab_position())
    }

    /// Where a tab opened from the active one goes (TAB-017).
    pub(super) fn opened_tab_position(&self) -> TabPosition {
        if self.context().settings_data().preferences.open_tabs_at_end {
            TabPosition::End
        } else {
            TabPosition::AfterActive
        }
    }

    /// Adds a tab for `address` at `position`, in front or in the
    /// background.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(super) fn open_tab_at(
        &self,
        address: &str,
        placement: TabPlacement,
        position: TabPosition,
    ) -> Result<(), LocationError> {
        let uri = self.resolve_address(address)?;
        self.save_tab_view();
        let id = self.imp().session.borrow_mut().add_at(&uri, placement, position);
        if self.imp().session.borrow().is_active(id) {
            self.show_tab(id);
        } else {
            self.render_tabs();
        }
        Ok(())
    }

    /// Adds a tab for `address` at the end and shows it: Ctrl+T, and
    /// locations from other applications and the command line.
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(crate) fn add_tab(&self, address: &str) -> Result<(), LocationError> {
        self.open_tab_at(address, TabPlacement::Foreground, TabPosition::End)
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
        let left = self.current_uri();
        let Some(id) = self.push_location(&uri) else {
            return self.add_tab(&uri);
        };
        self.leave_location();
        self.render_navigation();
        if let Some(child) = left.as_deref().and_then(|left| child_toward(&uri, left)) {
            self.reveal_when_listed(id, child);
        }
        self.load_tab(id, LoadMode::Navigate);
        Ok(())
    }

    /// Goes to the folder holding `item` and selects it there once
    /// listed, as a pasted file's address does rather than launch it.
    pub(super) fn show_in_its_folder(&self, item: &str) {
        let Some(folder) = parent_location(item) else {
            return;
        };
        self.navigate_or_report(&folder);
        let active = self.imp().session.borrow().active_id();
        if let Some(id) = active.filter(|_| self.current_uri().as_deref() == Some(folder.as_str())) {
            self.reveal_when_listed(id, item.to_owned());
        }
    }

    /// Navigates, showing a refused address in the message line.
    pub(super) fn navigate_or_report(&self, address: &str) {
        if let Err(error) = self.navigate(address) {
            self.show_message(&error.to_string());
        }
    }

    /// Adds `uri` to the active tab's history, remembering where the view
    /// was in the location it leaves; the tab, or `None` before the window
    /// has one.
    fn push_location(&self, uri: &str) -> Option<TabId> {
        let view = self.left_view();
        let mut session = self.imp().session.borrow_mut();
        let tab = session.active_mut()?;
        let left = tab.history.position();
        if tab.history.push(uri) {
            tab.left_views.forget_from(left + 1);
            if let Some(view) = view {
                tab.left_views.remember(left, view);
            }
        }
        tab.forget_location_state();
        Some(tab.id)
    }

    /// Where the view is in the active tab's folder: what Back or Forward
    /// returns to; `None` on a landing page, which does not scroll.
    fn left_view(&self) -> Option<LeftView> {
        let uri = self.current_uri()?;
        if Page::from_uri(&uri).is_some() {
            return None;
        }
        let pane = self.folder_pane();
        Some(LeftView {
            scroll: pane.scroll_position(),
            current: pane.model().selected_uris().into_iter().next(),
            expanded: pane.model().tree().expanded_uris(),
        })
    }

    /// Makes tab `id` select `item`, give it keyboard focus and scroll it
    /// into view once its folder is listed: the folder just left, when
    /// going up (NAV-011), as Explorer and Dolphin do.
    fn reveal_when_listed(&self, id: TabId, item: String) {
        if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
            tab.selected = vec![item.clone()];
            tab.revealed_item = Some(item);
        }
    }

    /// Clears what belonged to the folder the active tab leaves: the
    /// filter, the selection and the type-to-select prefix.
    fn leave_location(&self) {
        self.change_model(|| {
            self.leave_search();
            self.folder_pane().model().select_none();
        });
        self.reset_typeahead();
    }

    /// Remembers both panes of the active tab before another tab is shown,
    /// including an inactive pane scrolled without receiving focus.
    pub(super) fn save_tab_view(&self) {
        self.save_beside_view();
        self.save_selection();
        let pane = self.folder_pane();
        let scroll = pane.scroll_position();
        let focused = pane
            .focused_position()
            .and_then(|position| pane.model().item(position));
        if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
            tab.scroll = scroll;
            tab.focused = focused.map(|item| item.entry().uri.clone());
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
        self.show_tab_with(id, FocusOnShow::Restore);
    }

    /// [`Self::show_tab`], with keyboard focus going where `focus` says,
    /// and the pane beside it when the tab is split.
    pub(super) fn show_tab_with(&self, id: TabId, focus: FocusOnShow) {
        self.keep_activations_of(id);
        self.reset_typeahead();
        self.hide_message();
        self.show_dialog_of_tab(id);
        let Some(view) = self.saved_tab_view(id) else {
            return;
        };
        let had_focus =
            focus == FocusOnShow::Restore && self.folder_panes().iter().any(|pane| pane.view_has_focus());
        self.imp().active_side.set(view.side);
        if let Some(uri) = self.current_uri() {
            self.follow_folder_style(&uri);
        }
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
        self.refresh_free_space();
        self.show_pane_view_state();
        self.show_beside_pane();
        let pane = self.folder_pane();
        pane.restore_scroll_position(view.scroll);
        if had_focus {
            pane.focus_view();
            if let Some(uri) = view.focused {
                pane.focus_item_later(uri);
            }
        }
        if view.needs_listing {
            self.load_tab(id, LoadMode::Navigate);
        } else if view.changed_while_hidden {
            self.folder_changed(id);
        }
    }

    /// What tab `id` needs to be shown again, while it is open.
    fn saved_tab_view(&self, id: TabId) -> Option<SavedTabView> {
        let mut session = self.imp().session.borrow_mut();
        let tab = session.tab_mut(id)?;
        let changed_while_hidden = std::mem::take(&mut tab.changed_while_hidden);
        Some(SavedTabView {
            changed_while_hidden,
            store: tab.store.clone(),
            selected: tab.selected.clone(),
            focused: tab.focused.clone(),
            scroll: tab.scroll,
            side: tab.side,
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
        self.remember_closed_tab(id);
        self.remove_tab(id);
    }

    /// Takes tab `id` out after another window took it: it is not
    /// remembered as closed, and the last tab leaving closes the window.
    pub(super) fn release_moved_tab(&self, id: TabId) {
        if self.tab_count() <= 1 {
            self.request_close();
            return;
        }
        self.remove_tab(id);
    }

    /// Removes tab `id` and its dialog, and shows the next tab when it was
    /// in front.
    fn remove_tab(&self, id: TabId) {
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
        self.go_history_steps(direction.offset());
    }

    /// Jumps `steps` entries through the active tab's history, back for a
    /// negative number, as an entry of the Back or Forward menu does; out
    /// of range, nothing happens.
    pub(super) fn go_history_by(&self, steps: i32) {
        if let Ok(steps) = isize::try_from(steps) {
            self.go_history_steps(steps);
        }
    }

    fn go_history_steps(&self, steps: isize) {
        if self.refuse_while_active_tab_moves() {
            return;
        }
        let Some(id) = self.step_history(steps) else {
            return;
        };
        self.leave_location();
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
    }

    /// The mouse's Back or Forward side button: one step through the
    /// history, unless a dialog is open (`winspace.py` ignores the buttons
    /// while a dialog or sign-in prompt is shown).
    pub(super) fn go_history_from_mouse(&self, direction: Direction) {
        if self.takes_navigation_input() {
            self.go_history(direction);
        }
    }

    /// Moves the active tab's history `steps` entries; the tab, or `None`
    /// when there is no such entry. Where the view was in the location it
    /// returns to comes back once that is listed (NAV-008): the current
    /// item selected and focused, and the scroll position.
    fn step_history(&self, steps: isize) -> Option<TabId> {
        let view = self.left_view();
        let mut session = self.imp().session.borrow_mut();
        let tab = session.active_mut()?;
        let left = tab.history.position();
        tab.history.go(steps)?;
        if let Some(view) = view {
            tab.left_views.remember(left, view);
        }
        tab.forget_location_state();
        if let Some(returned) = tab.left_views.take(tab.history.position()) {
            tab.selected = returned.current.into_iter().collect();
            tab.scroll_after_listing = Some(returned.scroll);
            tab.expand_after_listing = returned.expanded;
        }
        Some(tab.id)
    }

    /// Opens the folder that contains the current one.
    pub(super) fn go_up(&self) {
        let parent = self.current_uri().as_deref().and_then(parent_location);
        if let Some(parent) = parent {
            self.navigate_or_report(&parent);
        }
    }

    /// Opens the home folder in the active tab (Alt+Home), as the sidebar's
    /// Home does.
    pub(super) fn go_home(&self) {
        let home = self.imp().locations.borrow().home_uri();
        self.navigate_or_report(&home);
    }
}

/// The child of `ancestor` on the way down to `descendant`, as Up and the
/// crumbs leave it; `None` when `ancestor` is not above `descendant`.
fn child_toward(ancestor: &str, descendant: &str) -> Option<String> {
    let mut child = descendant.to_owned();
    loop {
        let parent = parent_location(&child).filter(|parent| *parent != child)?;
        if location::same_location(&parent, ancestor) {
            return Some(child);
        }
        child = parent;
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
