// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane (sidebar).
//!
//! Ports `renderSidebar` and `.sidebar` in `desktop/ui/app.js` and
//! `style.css`: the rows of [`entries::sidebar_entries`], separated by
//! list-row headers so keyboard and screen-reader users never land on an
//! empty separator row, and the "Map network location" button pinned below
//! the list (`.sidebar-bottom`). Rows run [`WindowAction::GoTo`] or
//! [`WindowAction::MountVolume`]; a middle-click or Ctrl+click opens a
//! place in a tab, and the current location highlights the closest place
//! that holds it.
//! A right-click opens the row's menu: a Quick access pin's ([`menu`],
//! `sidebarMenu` in app.js), or a drive's or a network location's
//! (`driveMenu` and `networkLocationMenu`,
//! [`PlaceMenu`](super::place_menus::PlaceMenu)).
//!
//! [`Sidebar`] is a `GtkBox` subclass that keeps the entries its rows show,
//! so the list's header function and its middle-click and right-click
//! handlers read them through the pane itself. Where a drop on it goes is
//! [`drop_spots`]'s.

mod drop_spots;
pub(super) mod entries;
mod menu;
mod row;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::location::same_location;

use crate::icons::{self, Art, Icon};

use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::saved_search::saved_search_menu;
use super::window_action::WindowAction;
use super::{gestures, preferences, BrowserWindow};

pub(super) use drop_spots::SidebarDropSpot;
pub(super) use entries::{recent_and_bin_entries, sidebar_entries};
use entries::{RowTarget, Section, SidebarEntry};

/// The "+" of Map network location.
const MAP_NETWORK_GLYPH: i32 = 17;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{MenuPopover, SidebarEntry};

    /// Private state of [`super::Sidebar`].
    #[derive(Debug, Default)]
    pub(crate) struct Sidebar {
        /// The rows, built by `constructed`.
        pub(super) list: OnceCell<gtk::ListBox>,
        /// What each row shows and does, in row order.
        pub(super) entries: RefCell<Vec<SidebarEntry>>,
        /// The rows' context menu, built by `constructed`.
        pub(super) menu: OnceCell<MenuPopover>,
        /// The rows' icon size in pixels, 0 for automatic (SIDE-012).
        pub(super) icon_size: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "OxSidebar";
        type Type = super::Sidebar;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for Sidebar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build_pane();
        }

        fn dispose(&self) {
            if let Some(menu) = self.menu.get() {
                menu.unparent();
            }
        }
    }

    impl WidgetImpl for Sidebar {}
    impl BoxImpl for Sidebar {}
}

glib::wrapper! {
    /// The navigation pane: the scrolling list above the footer button.
    pub(crate) struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Sidebar {
    /// The list of rows.
    pub(super) fn list(&self) -> &gtk::ListBox {
        self.imp().list.get().expect("constructed builds the list")
    }

    /// Builds the list and, below it, the footer into the pane.
    fn build_pane(&self) {
        self.set_orientation(gtk::Orientation::Vertical);
        self.add_css_class("sidebar");
        self.update_property(&[gtk::accessible::Property::Label("Folders and network locations")]);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Navigation pane")]);
        self.separate_sections(&list);
        self.open_places_on_middle_click(&list);
        self.open_places_in_tabs_on_ctrl_click(&list);
        self.open_menus_on_right_click(&list);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            // The list is never narrower than the narrowest saved sidebar.
            .min_content_width(*preferences::sidebar_widths().start())
            .vexpand(true)
            .child(&list)
            .build();
        self.append(&scroller);
        self.append(&map_network_button());
        self.imp()
            .list
            .set(list)
            .expect("constructed runs once per object");
    }

    /// Draws a separator above every row that starts a new section.
    fn separate_sections(&self, list: &gtk::ListBox) {
        list.set_header_func(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |row, before| {
                let starts_section =
                    before.is_some_and(|before| sidebar.section_of(before) != sidebar.section_of(row));
                if starts_section {
                    row.set_header(Some(&section_separator()));
                } else {
                    row.set_header(None::<&gtk::Widget>);
                }
            }
        ));
    }

    /// The section of the entry `row` shows.
    fn section_of(&self, row: &gtk::ListBoxRow) -> Option<Section> {
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        entries.get(index).map(|entry| entry.section)
    }

    /// A middle-click on a place opens it in a tab; volumes that still
    /// have to be mounted do nothing.
    fn open_places_on_middle_click(&self, list: &gtk::ListBox) {
        let gesture = gestures::middle_click(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |gesture, _, y| {
                let Some(uri) = sidebar.location_at(y) else {
                    return;
                };
                let action = gestures::open_action(gesture.current_event_state());
                action.activate_from(&sidebar, Some(&uri.to_variant()));
            }
        ));
        list.add_controller(gesture);
    }

    /// Ctrl+click on a place opens it in a background tab and
    /// Ctrl+Shift+click in a tab in front, as in Dolphin's Places panel;
    /// a plain click goes on to the row.
    fn open_places_in_tabs_on_ctrl_click(&self, list: &gtk::ListBox) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |gesture, _, _, y| {
                let action = tab_action_for_click(gesture.current_event_state());
                let target = action.zip(sidebar.location_at(y));
                let Some((action, uri)) = target else {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                action.activate_from(&sidebar, Some(&uri.to_variant()));
            }
        ));
        list.add_controller(click);
    }

    /// A right-click on a Quick access pin, a drive or a network location
    /// opens its menu there.
    fn open_menus_on_right_click(&self, list: &gtk::ListBox) {
        let menu = MenuPopover::new(Vec::new());
        menu.set_offset(0, 0);
        menu.set_parent(self);
        self.imp()
            .menu
            .set(menu)
            .expect("constructed runs once per object");
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |gesture, _, x, y| {
                if sidebar.show_menu(x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        ));
        list.add_controller(click);
    }

    /// Opens the menu of the row at (`x`, `y`) of the list; false where
    /// the row has none.
    fn show_menu(&self, x: f64, y: f64) -> bool {
        let Some(entries) = self.menu_entries_at(y) else {
            return false;
        };
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let in_list = gtk::graphene::Point::new(x as f32, y as f32);
        let Some(point) = self.list().compute_point(self, &in_list) else {
            return false;
        };
        let menu = self.imp().menu.get().expect("constructed builds the menu");
        menu.set_entries(entries);
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let target = gdk::Rectangle::new(point.x() as i32, point.y() as i32, 1, 1);
        menu.set_pointing_to(Some(&target));
        menu.popup();
        true
    }

    /// The menu of the row at `y` in the list: a pin's, or a drive's or a
    /// network location's; on empty space, "Add entry…" (SIDE-031);
    /// `None` for a row without one.
    pub(super) fn menu_entries_at(&self, y: f64) -> Option<Vec<MenuEntry>> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let Some(row) = self.list().row_at_y(y as i32) else {
            return Some(empty_space_menu());
        };
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        let entry = entries.get(index)?;
        let window = self.root().and_downcast::<BrowserWindow>();
        if let RowTarget::SavedSearch(search) = &entry.target {
            return Some(saved_search_menu(search));
        }
        if entry.pinned {
            let RowTarget::Location(uri) = &entry.target else {
                return None;
            };
            let caching = window.as_ref().and_then(|window| window.caching_of(uri));
            let editable = window.is_some_and(|window| {
                let quick_access = window.places().quick_access;
                quick_access
                    .iter()
                    .any(|place| place.known_folder.is_none() && same_location(&place.uri, uri))
            });
            return Some(menu::pin_menu(uri, caching, editable));
        }
        let place_menu = entry.menu.as_ref()?.entries_in(window.as_ref());
        // A drive the system keeps mounted may have nothing to offer.
        (!place_menu.is_empty()).then_some(place_menu)
    }

    /// Right-clicks the row labelled `label` and returns the sidebar's
    /// menu, for tests.
    #[cfg(test)]
    pub(super) fn right_click_row(&self, label: &str) -> MenuPopover {
        let index = self
            .labels()
            .iter()
            .position(|shown| shown == label)
            .unwrap_or_else(|| panic!("the sidebar shows {label}"));
        let row = i32::try_from(index)
            .ok()
            .and_then(|index| self.list().row_at_index(index))
            .expect("every entry has a row");
        let bounds = row.compute_bounds(self.list()).expect("a shown row has bounds");
        let middle = f64::from(bounds.y() + bounds.height() / 2.0);
        self.show_menu(1.0, middle);
        self.imp()
            .menu
            .get()
            .expect("constructed builds the menu")
            .clone()
    }

    /// The location of the row at `y` in the list, if it opens one.
    pub(super) fn location_at(&self, y: f64) -> Option<String> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let row = self.list().row_at_y(y as i32)?;
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        match &entries.get(index)?.target {
            RowTarget::Location(uri) => Some(uri.clone()),
            RowTarget::MountVolume(_) | RowTarget::PinDropTail | RowTarget::SavedSearch(_) => None,
        }
    }

    /// Replaces the rows.
    pub(super) fn set_entries(&self, entries: Vec<SidebarEntry>) {
        let list = self.list();
        list.remove_all();
        let rows: Vec<gtk::ListBoxRow> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let edges = entries::section_edges(&entries, index);
                row::sidebar_row(entry, edges, self.imp().icon_size.get())
            })
            .collect();
        // The header function reads the entries as the rows are added.
        self.imp().entries.replace(entries);
        for row in &rows {
            list.append(row);
        }
    }

    /// Draws the rows' icons `size` pixels big, or at the automatic size
    /// for 0 (Dolphin's Places panel Icon Size, SIDE-012).
    pub(super) fn set_icon_size(&self, size: u32) {
        if self.imp().icon_size.replace(size) == size {
            return;
        }
        let entries = self.imp().entries.borrow().clone();
        let selected = self.list().selected_row().map(|row| row.index());
        self.set_entries(entries);
        let row = selected.and_then(|index| self.list().row_at_index(index));
        self.list().select_row(row.as_ref());
    }

    /// Highlights the row for `uri`, else the closest place that holds it
    /// (Dolphin's Places panel), or none.
    pub(super) fn select(&self, uri: &str) {
        let index = closest_place(&self.imp().entries.borrow(), uri);
        let row = index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.list().row_at_index(index));
        self.list().select_row(row.as_ref());
    }

    /// The places as menu items, a divider between sections, for the
    /// Places button shown while the pane is hidden (SIDE-024).
    pub(super) fn places_menu(&self) -> Vec<MenuEntry> {
        let entries = self.imp().entries.borrow();
        let mut menu = Vec::new();
        let mut section = None;
        for entry in entries.iter() {
            let RowTarget::Location(uri) = &entry.target else {
                continue;
            };
            if section.is_some_and(|section| section != entry.section) {
                menu.push(MenuEntry::Divider);
            }
            section = Some(entry.section);
            let glyph = match entry.icon {
                Art::Glyph(icon) | Art::TintedGlyph(icon, _) => icon,
                Art::Folder | Art::ZipFolder | Art::File(_) | Art::Network(_) => Icon::Folder,
            };
            menu.push(MenuItem::with_text_target(&entry.label, glyph, WindowAction::GoTo, uri).into());
        }
        menu
    }

    /// The labels shown, for tests.
    #[cfg(test)]
    pub(super) fn labels(&self) -> Vec<String> {
        let entries = self.imp().entries.borrow();
        entries.iter().map(|entry| entry.label.clone()).collect()
    }
}

/// The menu of the sidebar's empty space: "Add entry…" (SIDE-031) and
/// the icon sizes (SIDE-012).
fn empty_space_menu() -> Vec<MenuEntry> {
    let size = |label: &str, pixels: &str| {
        MenuItem::choice(label, Icon::Grid, WindowAction::SidebarIconSize, pixels).into()
    };
    vec![
        MenuItem::new("Add entry…", Icon::Add, WindowAction::AddPlace).into(),
        MenuEntry::Divider,
        size("Automatic icon size", "0"),
        size("Small icons", "16"),
        size("Medium icons", "22"),
        size("Large icons", "32"),
        size("Huge icons", "48"),
    ]
}

/// The tab a primary click with `modifiers` opens a place in: with Ctrl,
/// a background tab, or one in front with Shift too; `None` for a click
/// that opens the place in the current tab.
fn tab_action_for_click(modifiers: gdk::ModifierType) -> Option<WindowAction> {
    modifiers
        .contains(gdk::ModifierType::CONTROL_MASK)
        .then(|| gestures::open_action(modifiers))
}

/// The index of the entry for `uri`: the first that opens it, else the one
/// whose folder holds it most closely, as Dolphin highlights Documents in
/// Documents/Reports; `None` when no place holds it.
fn closest_place(entries: &[SidebarEntry], uri: &str) -> Option<usize> {
    let location = |entry: &SidebarEntry| match &entry.target {
        RowTarget::Location(candidate) => Some(candidate.clone()),
        RowTarget::MountVolume(_) | RowTarget::PinDropTail | RowTarget::SavedSearch(_) => None,
    };
    let exact = entries
        .iter()
        .position(|entry| location(entry).is_some_and(|candidate| same_location(&candidate, uri)));
    if exact.is_some() {
        return exact;
    }
    let file = gio::File::for_uri(uri);
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| Some((index, location(entry)?)))
        .filter(|(_, candidate)| file.has_prefix(&gio::File::for_uri(candidate)))
        // The deepest holder wins; among equals, the first row.
        .min_by_key(|(index, candidate)| (std::cmp::Reverse(candidate.trim_end_matches('/').len()), *index))
        .map(|(index, _)| index)
}

/// The line between two sections.
fn section_separator() -> gtk::Separator {
    let line = gtk::Separator::new(gtk::Orientation::Horizontal);
    line.add_css_class("side-separator");
    line
}

/// The "Map network location" button below the list (`#connect-sidebar`).
fn map_network_button() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 11);
    content.append(&icons::image(Icon::Add, MAP_NETWORK_GLYPH));
    content.append(&gtk::Label::new(Some("Map network location")));
    let button = gtk::Button::builder()
        .child(&content)
        .action_name(WindowAction::MapNetworkLocation.detailed_name())
        .tooltip_text("Map network location")
        .build();
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["sidebar-bottom"])
        .build();
    footer.append(&button);
    footer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(label: &str, uri: &str) -> SidebarEntry {
        SidebarEntry {
            section: Section::QuickAccess,
            level: entries::RowLevel::Place,
            label: label.to_owned(),
            icon: Art::Glyph(Icon::Folder),
            target: RowTarget::Location(uri.to_owned()),
            tooltip: label.to_owned(),
            pinned: false,
            menu: None,
            eject: None,
        }
    }

    /// parity: SIDE-004
    #[test]
    fn the_closest_place_that_holds_the_location_is_highlighted() {
        let entries = [
            place("Local Disk", "file:///"),
            place("Home", "file:///home/demo"),
            place("Documents", "file:///home/demo/Documents"),
            place("Docs", "file:///home/demo/Docs"),
        ];
        let at = |uri: &str| closest_place(&entries, uri).map(|index| entries[index].label.as_str());
        assert_eq!(at("file:///home/demo/Documents/Reports/2026"), Some("Documents"));
        assert_eq!(at("file:///home/demo/Documents/"), Some("Documents"));
        assert_eq!(at("file:///home/demo/Docs2"), Some("Home"), "not a name prefix");
        assert_eq!(at("file:///etc"), Some("Local Disk"));
        assert_eq!(at("smb://studio-nas/projects"), None);
    }

    /// parity: SIDE-015
    #[test]
    fn ctrl_click_opens_a_place_in_a_background_tab_and_with_shift_in_front() {
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let shift = gdk::ModifierType::SHIFT_MASK;
        assert_eq!(tab_action_for_click(gdk::ModifierType::empty()), None);
        assert_eq!(tab_action_for_click(shift), None);
        assert_eq!(tab_action_for_click(ctrl), Some(WindowAction::OpenTabBackground));
        assert_eq!(tab_action_for_click(ctrl | shift), Some(WindowAction::OpenTab));
    }
}
