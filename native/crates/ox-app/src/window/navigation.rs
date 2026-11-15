// SPDX-License-Identifier: AGPL-3.0-only
//! Changing location: tabs, history and Up.
//!
//! Ports `addTab`, `closeTab`, `switchTab`, `navigate` and `goHistory` in
//! `v2.0.0:desktop/ui/app.js`. Each tab keeps its own history, selection and
//! scroll position; moving to another location forgets the selection and
//! the scroll position, and showing another tab puts its own back. As in
//! Dolphin, Back and Forward return to where the view was, and going up
//! selects the folder just left. Search results are a step of the
//! history too ([`super::search_steps`]).
//! [`super::location_view`] draws the result into the frame.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{
    self, normalise_navigation, parent_location, ArchiveLocation, LocationError, VirtualPlace,
};

use crate::history::{LeftView, SearchStep};
use crate::locations::{self, Page};

use super::loading::LoadMode;
use super::search_steps::ResultsView;
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

/// What leaving a location does to the search shown there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeftSearch {
    /// It ends, unless the strip's pin keeps it (SRCH-005).
    MayKeep,
    /// It ends.
    End,
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
    /// landing page by title, the Recycle Bin or Recent or a location in
    /// them, else a folder relative to the current one (see
    /// [`Self::resolve_relative`]).
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open.
    pub(super) fn resolve_address(&self, address: &str) -> Result<String, LocationError> {
        let typed = address.trim();
        if locations::is_home_alias(typed) {
            return Ok(self.home_location());
        }
        if let Some(page) = Page::from_uri(typed) {
            return Ok(page.uri().to_owned());
        }
        // Gallery's own address (HOME-018); its title is matched below,
        // after a folder of that name.
        if VirtualPlace::from_uri(typed) == Some(VirtualPlace::Gallery) {
            return Ok(VirtualPlace::Gallery.uri().to_owned());
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
        if let Some(listed) = gio_folder_location(typed)? {
            return Ok(listed);
        }
        if let Some(inside) = ArchiveLocation::parse(typed)? {
            return Ok(inside.uri());
        }
        let resolved = self.resolve_relative(address)?;
        Ok(self.typed_path_through_zip(&resolved).unwrap_or(resolved))
    }

    /// The location inside a ZIP that the path `uri` runs through, as the
    /// address bar shows a ZIP browsed as a folder:
    /// `/home/ana/Downloads/tidewater.zip/tidewater` (ARC-026). Only while
    /// ZIPs open like folders: "In a pop-up window" never shows a ZIP in
    /// the tab, so the path stays a path, which names no folder.
    pub(super) fn typed_path_through_zip(&self, uri: &str) -> Option<String> {
        if self.opens_zips_as_folders() {
            path_through_zip(uri)
        } else {
            None
        }
    }

    /// The place whose title is `typed` ("Home", "This PC", "Network",
    /// "Recycle Bin", "Recent", "Recent locations"), as the address bar
    /// shows them: "Home" leads where the navigation pane's Home does, the
    /// home folder or, while "Open Quick access on Home" is on, the Home
    /// page. Typing "Recycle Bin" and pressing Enter opens it, as in
    /// Windows 11 File Explorer; a
    /// folder of that name in the current folder still comes first, as for
    /// the other titles.
    pub(super) fn place_titled(&self, typed: &str) -> Option<String> {
        if typed.trim().eq_ignore_ascii_case("home") {
            return Some(self.home_location());
        }
        Page::from_title(typed)
            .map(|page| page.uri().to_owned())
            .or_else(|| listed_place(VirtualPlace::from_title(typed)))
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
        let folder = self
            .current_uri()
            .filter(|uri| Page::from_uri(uri).is_none())
            .map(|uri| match ArchiveLocation::from_uri(&uri) {
                // Inside a ZIP, relative to the folder that holds it.
                Some(inside) => parent_location(&inside.archive_uri).unwrap_or(uri),
                None => uri,
            });
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
        self.add_tab_showing(address, placement, position, None)
    }

    /// Adds a tab opened from the active one on the results of `search`
    /// in `address`, a step after the folder, as if searched there
    /// (SRCH-042): a middle-click on Back to "Search Results in Documents".
    ///
    /// # Errors
    ///
    /// The address is not a location the app can open; nothing changes.
    pub(super) fn open_search_tab(
        &self,
        address: &str,
        placement: TabPlacement,
        search: SearchStep,
    ) -> Result<(), LocationError> {
        self.add_tab_showing(address, placement, self.opened_tab_position(), Some(search))
    }

    /// Adds a tab for `address` at `position`, showing `search` there.
    fn add_tab_showing(
        &self,
        address: &str,
        placement: TabPlacement,
        position: TabPosition,
        search: Option<SearchStep>,
    ) -> Result<(), LocationError> {
        let uri = self.resolve_address(address)?;
        // A file dialog keeps one tab and no Settings (INT-032): a new tab
        // goes to the location in the one there is.
        if self.is_picking() && self.tab_count() >= 1 {
            if Page::from_uri(&uri) == Some(Page::Settings) {
                return Ok(());
            }
            return self.navigate(&uri);
        }
        self.save_tab_view();
        let id = self.imp().session.borrow_mut().add_at(&uri, placement, position);
        if let Some(search) = search {
            if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
                tab.history.push_search(search);
            }
        }
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
        self.leave_location(LeftSearch::MayKeep);
        self.render_navigation();
        if let Some(child) = left.as_deref().and_then(|left| child_toward(&uri, left)) {
            self.reveal_when_listed(id, child);
        }
        self.load_tab(id, LoadMode::Navigate);
        self.focus_files_after_navigation();
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
    pub(super) fn left_view(&self) -> Option<LeftView> {
        let uri = self.current_uri()?;
        if Page::drawn_at(&uri).is_some() {
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
    /// filter, the selection and the type-to-select prefix. A search the
    /// strip's pin keeps goes on in the folder opened, unless `search`
    /// ends it: Back and Forward show each step's own search (SRCH-042).
    fn leave_location(&self, search: LeftSearch) {
        self.change_model(|| {
            match search {
                LeftSearch::MayKeep => self.leave_search(),
                LeftSearch::End => self.end_search(),
            }
            self.folder_pane().model().select_none();
        });
        self.forget_results_view();
        self.reset_typeahead();
    }

    /// Remembers both panes of the active tab before another tab is shown,
    /// including an inactive pane scrolled without receiving focus.
    pub(super) fn save_tab_view(&self) {
        self.save_beside_view();
        // Search results still being found again: the tab keeps where it
        // was in them.
        let results_view = self.imp().results_view.borrow().clone();
        if let Some(view) = results_view {
            if let Some(tab) = self.imp().session.borrow_mut().active_mut() {
                if tab.id == view.tab {
                    tab.selected = view.selected;
                    tab.focused = view.focused;
                    tab.scroll = view.scroll;
                    return;
                }
            }
        }
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
        // A rename a slow click scheduled is for the pane clicked.
        self.cancel_slow_click_rename();
        self.keep_activations_of(id);
        self.reset_typeahead();
        self.hide_message();
        let Some(view) = self.saved_tab_view(id) else {
            return;
        };
        let had_focus =
            focus == FocusOnShow::Restore && self.folder_panes().iter().any(|pane| pane.view_has_focus());
        self.imp().active_side.set(view.side);
        // A pane that shows this tab at its location already keeps the
        // style it shows, as the pane beside does: the pane just left when
        // the other becomes active again, or the pane that stays when the
        // other closes and takes its style along. A choice made there a
        // moment ago may not be saved yet, and the saved style would undo
        // it.
        if let Some(uri) = self.current_uri() {
            if !self.folder_pane().is_styled_for(&view.store, &uri) {
                self.follow_folder_style(&view.store, &uri);
            }
        }
        self.change_model(|| {
            let model = self.folder_pane().model();
            self.clear_search_box_quietly();
            model.set_query("");
            model.set_store(Some(&view.store));
            model.select_uris(&view.selected);
        });
        self.render_navigation();
        self.update_content();
        self.update_details_pane();
        self.show_pane_view_state();
        self.show_beside_pane();
        let pane = self.folder_pane();
        pane.restore_scroll_position(view.scroll);
        if had_focus {
            pane.focus_view();
            if let Some(uri) = view.focused.clone() {
                pane.focus_item_later(uri);
            }
        }
        // A tab on search results shows them again (SRCH-042), and where
        // it was in them once they are found.
        self.show_step_search_restoring(Some(ResultsView {
            tab: id,
            selected: view.selected,
            focused: view.focused,
            scroll: view.scroll,
        }));
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
    /// is being dragged stays (TAB-003), and so does every other tab while
    /// one is dragged along the row ([`Self::refuse_while_tabs_move`]).
    pub(super) fn close_tab(&self, id: TabId) {
        if self.refuse_while_moving(id) || self.refuse_while_tabs_move() {
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
    pub(super) fn release_moved_tab(&self, id: TabId, destination: Option<&BrowserWindow>) {
        if self.tab_count() <= 1 {
            // Properties opened here go on with the tab's new window, with
            // any change they hold (PROP-001).
            if let Some(destination) = destination {
                destination.adopt_properties_windows(self);
            }
            self.request_close();
            return;
        }
        self.remove_tab(id);
    }

    /// Removes tab `id` and its dialog, and shows the next tab when it was
    /// in front.
    fn remove_tab(&self, id: TabId) {
        self.save_tab_view();
        self.forget_snapshot_tab(id);
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
        // A folder grouped by date counts from today, also after a night
        // the computer slept through.
        self.follow_the_day();
        if self.refresh_search() {
            return;
        }
        self.context().reload_settings();
        self.save_selection();
        let active = self.imp().session.borrow().active_id();
        if let Some(id) = active {
            // The user asks for it: a share another application asked to
            // show may be mounted now (Try again, NET-004).
            if let Some(tab) = self.imp().session.borrow_mut().tab_mut(id) {
                tab.waits_to_mount = false;
            }
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
        self.leave_location(LeftSearch::End);
        self.render_navigation();
        self.load_tab(id, LoadMode::Navigate);
        // Search results the step showed run again there (SRCH-042).
        self.show_step_search();
        self.focus_files_after_navigation();
    }

    /// Puts the keyboard in the folder the tab went to, wherever the click or
    /// key came from: the navigation pane, a crumb, a landing page, a command
    /// or the address bar. Typing to select then works at once, instead of
    /// going to a button or a row the navigation left behind. A text field
    /// that keeps the keyboard, such as an Open dialog's file name, and the
    /// tab strip, which the keyboard may be working in while another tab
    /// changes folder, are left alone.
    fn focus_files_after_navigation(&self) {
        if self.focus_is_in_text_field() {
            return;
        }
        let on_tabs = GtkWindowExt::focus(self).is_some_and(|focus| focus.is_ancestor(self.tab_strip()));
        if on_tabs {
            return;
        }
        self.folder_pane().focus_view();
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

    /// Opens Home in the active tab (Alt+Home), as the sidebar's Home
    /// does.
    pub(super) fn go_home(&self) {
        let home = self.home_location();
        self.navigate_or_report(&home);
    }

    /// Where Home leads: the Home page with Quick access, Recent and
    /// Favorites while its option is on (HOME-012), else the home folder,
    /// as in the Python app (NAV-038).
    pub(super) fn home_location(&self) -> String {
        if self.context().settings_data().preferences.home_quick_access {
            return Page::Home.uri().to_owned();
        }
        self.imp().locations.borrow().home_uri()
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

/// The location inside a ZIP that the local path `uri` runs through: the
/// first ancestor that is a ZIP or TAR file, then the folders after it. `None`
/// when no ancestor is a ZIP, or for anything but a local path (a share
/// would answer slowly here, on the main thread).
fn path_through_zip(uri: &str) -> Option<String> {
    // A share mounted through GVfs has a local path too, which would be
    // read over the network here.
    if !uri.starts_with("file:") {
        return None;
    }
    let path = gio::File::for_uri(uri).path()?;
    let zip = path.ancestors().skip(1).find(|ancestor| {
        let is_zip_name = ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| ox_core::archive::is_supported_archive(name, None));
        is_zip_name && ancestor.is_file()
    })?;
    let rest = path.strip_prefix(zip).ok()?;
    let mut member = String::new();
    for component in rest.components() {
        member.push_str(component.as_os_str().to_str()?);
        member.push('/');
    }
    let root = ArchiveLocation::root(&ox_core::location::file_uri(zip));
    ox_core::archive::is_safe_member_name(&member).then(|| root.member(&member).uri())
}

/// The URI of `place` when it is one the app lists like a folder and the
/// navigation pane may leave out (SIDE-034): the Recycle Bin, Recent or
/// Recent locations, and Gallery (HOME-018).
pub(super) fn listed_place(place: Option<VirtualPlace>) -> Option<String> {
    place
        .filter(|place| {
            matches!(
                place,
                VirtualPlace::RecycleBin
                    | VirtualPlace::Recent
                    | VirtualPlace::RecentLocations
                    | VirtualPlace::Gallery
            )
        })
        .map(|place| place.uri().to_owned())
}

/// The Recycle Bin or Recent by its title or URI, or a location in it,
/// for `typed`; `None` for any other address. The Python app could show
/// neither; the native app lists both like folders, as GIO's `trash:` and
/// `recent:` (OPS-040, SIDE-026).
///
/// # Errors
///
/// A `trash:` or `recent:` address that is not a canonical location.
fn gio_folder_location(typed: &str) -> Result<Option<String>, LocationError> {
    let titled = VirtualPlace::from_title(typed)
        .filter(|place| matches!(place, VirtualPlace::RecycleBin | VirtualPlace::Recent));
    let address = match titled {
        Some(place) => place.uri(),
        None if typed.starts_with("trash:") || typed.starts_with("recent:") => typed,
        None => return Ok(None),
    };
    normalise_navigation(address, None, &glib::home_dir()).map(Some)
}
