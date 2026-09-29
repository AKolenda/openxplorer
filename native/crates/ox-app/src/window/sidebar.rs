// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane (sidebar).
//!
//! Ports `renderSidebar` and `.sidebar` in `desktop/ui/app.js` and
//! `style.css`: the rows of [`entries::sidebar_entries`], separated by
//! list-row headers so keyboard and screen-reader users never land on an
//! empty separator row, and the "Map network location" button pinned below
//! the list (`.sidebar-bottom`). Rows run [`WindowAction::GoTo`] or
//! [`WindowAction::MountVolume`]; a middle-click opens a place in a tab.
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
mod entries;
mod menu;
mod row;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::same_location;

use crate::icons::{self, Icon};

use super::menu_popover::{MenuEntry, MenuPopover};
use super::place_menus::caching_in;
use super::window_action::WindowAction;
use super::{gestures, preferences, BrowserWindow};

pub(super) use drop_spots::SidebarDropSpot;
pub(super) use entries::sidebar_entries;
use entries::{RowTarget, Section, SidebarEntry};

/// The "+" of Map network location.
const MAP_NETWORK_GLYPH: i32 = 17;

mod imp {
    use std::cell::{OnceCell, RefCell};

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
    /// network location's; `None` for a row without one.
    fn menu_entries_at(&self, y: f64) -> Option<Vec<MenuEntry>> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let row = self.list().row_at_y(y as i32)?;
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        let entry = entries.get(index)?;
        if entry.pinned {
            let RowTarget::Location(uri) = &entry.target else {
                return None;
            };
            let window = self.root().and_downcast::<BrowserWindow>();
            let caching = window.and_then(|window| window.caching_of(uri));
            return Some(menu::pin_menu(uri, caching));
        }
        let place = entry.menu.as_ref()?;
        let place_menu = place.entries(caching_in(place, self));
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
            RowTarget::MountVolume(_) => None,
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
                row::sidebar_row(entry, edges)
            })
            .collect();
        // The header function reads the entries as the rows are added.
        self.imp().entries.replace(entries);
        for row in &rows {
            list.append(row);
        }
    }

    /// Highlights the row for `uri`, or none.
    pub(super) fn select(&self, uri: &str) {
        let index = self
            .imp()
            .entries
            .borrow()
            .iter()
            .position(|entry| match &entry.target {
                RowTarget::Location(candidate) => same_location(candidate, uri),
                RowTarget::MountVolume(_) => false,
            });
        let row = index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.list().row_at_index(index));
        self.list().select_row(row.as_ref());
    }

    /// The labels shown, for tests.
    #[cfg(test)]
    pub(super) fn labels(&self) -> Vec<String> {
        let entries = self.imp().entries.borrow();
        entries.iter().map(|entry| entry.label.clone()).collect()
    }
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
