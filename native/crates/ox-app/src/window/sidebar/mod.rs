// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane (sidebar).
//!
//! Ports `renderSidebar` and `.sidebar` in `desktop/ui/app.js` and
//! `style.css`: the rows of [`entries::sidebar_entries`], separated by
//! list-row headers so keyboard and screen-reader users never land on an
//! empty separator row, and the "Map network location" button pinned below
//! the list (`.sidebar-bottom`). Rows activate `win.go-to` or
//! `win.mount-volume`; a middle-click opens a place in a tab.
//!
//! [`Sidebar`] is a `GtkBox` subclass that keeps the entries its rows show,
//! so the list's header function and middle-click handler read them
//! through the pane itself.

mod entries;
mod row;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::same_location;

use crate::icons::{self, Glyph};

use super::art_style::ArtStyle;
use super::{gestures, unported};

pub(super) use entries::sidebar_entries;
use entries::{RowTarget, Section, SidebarEntry};

/// The action of the "Map network location" button (`#connect-sidebar`).
const MAP_NETWORK_ACTION: &str = "win.map-network-location";

/// The narrowest the list gets, the Python app's narrowest sidebar.
const NARROWEST_LIST: i32 = 140;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::SidebarEntry;

    /// Private state of [`super::Sidebar`].
    #[derive(Debug, Default)]
    pub struct Sidebar {
        /// The rows, built by `constructed`.
        pub(super) list: OnceCell<gtk::ListBox>,
        /// What each row shows and does, in row order.
        pub(super) entries: RefCell<Vec<SidebarEntry>>,
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
    }

    impl WidgetImpl for Sidebar {}
    impl BoxImpl for Sidebar {}
}

glib::wrapper! {
    /// The navigation pane: the scrolling list above the footer button.
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Sidebar {
    /// An empty navigation pane.
    pub(super) fn new() -> Self {
        glib::Object::builder()
            .property("orientation", gtk::Orientation::Vertical)
            .build()
    }

    /// The list of rows.
    pub(super) fn list(&self) -> &gtk::ListBox {
        self.imp().list.get().expect("constructed builds the list")
    }

    /// Builds the list and the footer into the pane.
    fn build_pane(&self) {
        self.add_css_class("sidebar");
        self.update_property(&[gtk::accessible::Property::Label("Folders and network locations")]);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Navigation pane")]);
        self.separate_sections(&list);
        self.open_places_on_middle_click(&list);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(NARROWEST_LIST)
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
                // The action exists on every browser window.
                let _ = sidebar.activate_action(action, Some(&uri.to_variant()));
            }
        ));
        list.add_controller(gesture);
    }

    /// The location of the row at `y` in the list, if it opens one.
    fn location_at(&self, y: f64) -> Option<String> {
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
    pub(super) fn show(&self, entries: Vec<SidebarEntry>, style: ArtStyle) {
        let list = self.list();
        list.remove_all();
        let rows: Vec<gtk::ListBoxRow> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let edges = entries::section_edges(&entries, index);
                row::sidebar_row(entry, edges, style)
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

/// The "Map network location" button below the list.
fn map_network_button() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 11);
    content.append(&icons::glyph(Glyph::Plus, 17));
    content.append(&gtk::Label::new(Some("Map network location")));
    let button = gtk::Button::builder()
        .child(&content)
        .action_name(MAP_NETWORK_ACTION)
        .tooltip_text(unported::tooltip(MAP_NETWORK_ACTION, "Map network location"))
        .build();
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["sidebar-bottom"])
        .build();
    footer.append(&button);
    footer
}
