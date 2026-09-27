// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane (sidebar).
//!
//! Ports `renderSidebar` and `.sidebar` in `desktop/ui/app.js` and
//! `style.css`: the rows of [`entries::sidebar_entries`], separated by
//! list-row headers so keyboard and screen-reader users never land on an
//! empty separator row, and the "Map network location" button pinned below
//! the list (`.sidebar-bottom`). Rows activate `win.go-to` or
//! `win.mount-volume`; a middle-click opens a place in a tab.

mod entries;
mod row;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use ox_core::location::same_location;

use crate::icons::{self, Glyph};
use crate::theme::Appearance;

use super::{gestures, unported};

pub(super) use entries::sidebar_entries;
use entries::{RowTarget, SidebarEntry};

/// The action of the "Map network location" button (`#connect-sidebar`).
const MAP_NETWORK_ACTION: &str = "win.map-network-location";

/// The location of the row at `y` in `list`, for middle-clicks.
fn location_at(list: &gtk::ListBox, entries: &[SidebarEntry], y: f64) -> Option<String> {
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let row = list.row_at_y(y as i32)?;
    let index = usize::try_from(row.index()).ok()?;
    match &entries.get(index)?.target {
        RowTarget::Location(uri) => Some(uri.clone()),
        RowTarget::MountVolume(_) => None,
    }
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

/// A separator above every row that starts a new section.
fn separate_sections(list: &gtk::ListBox, entries: &Rc<RefCell<Vec<SidebarEntry>>>) {
    let sections = Rc::clone(entries);
    list.set_header_func(move |row, before| {
        let section_of = |row: &gtk::ListBoxRow| {
            let index = usize::try_from(row.index()).ok()?;
            sections.borrow().get(index).map(|entry| entry.section)
        };
        let starts_group = before.is_some_and(|before| section_of(before) != section_of(row));
        if !starts_group {
            row.set_header(None::<&gtk::Widget>);
            return;
        }
        let line = gtk::Separator::new(gtk::Orientation::Horizontal);
        line.add_css_class("side-separator");
        row.set_header(Some(&line));
    });
}

/// The sidebar's widgets and the rows it shows.
#[derive(Debug)]
pub(super) struct Sidebar {
    /// The pane: the scrolling list above the footer button.
    pub root: gtk::Box,
    /// The rows.
    pub list: gtk::ListBox,
    entries: Rc<RefCell<Vec<SidebarEntry>>>,
}

impl Sidebar {
    /// An empty navigation pane.
    pub fn new() -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Navigation pane")]);
        let entries: Rc<RefCell<Vec<SidebarEntry>>> = Rc::default();
        separate_sections(&list, &entries);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(140)
            .vexpand(true)
            .child(&list)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["sidebar"])
            .build();
        root.update_property(&[gtk::accessible::Property::Label("Folders and network locations")]);
        root.append(&scroller);
        root.append(&map_network_button());
        let sidebar = Self { root, list, entries };
        sidebar.open_places_on_middle_click();
        sidebar
    }

    /// A middle-click on a place opens it in a tab; volumes that still
    /// have to be mounted do nothing.
    fn open_places_on_middle_click(&self) {
        let entries = Rc::clone(&self.entries);
        let gesture = gestures::middle_click(move |gesture, _, y| {
            let Some(list) = gesture.widget().and_downcast::<gtk::ListBox>() else {
                return;
            };
            let Some(uri) = location_at(&list, &entries.borrow(), y) else {
                return;
            };
            let action = gestures::open_action(gesture.current_event_state());
            // The action exists on every browser window.
            let _ = list.activate_action(action, Some(&uri.to_variant()));
        });
        self.list.add_controller(gesture);
    }

    /// Replaces the rows.
    pub fn show(&self, entries: Vec<SidebarEntry>, appearance: Appearance, scale: i32) {
        self.list.remove_all();
        let rows: Vec<gtk::ListBoxRow> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let edges = entries::section_edges(&entries, index);
                row::sidebar_row(entry, edges, appearance, scale)
            })
            .collect();
        self.entries.replace(entries);
        for row in &rows {
            self.list.append(row);
        }
    }

    /// Highlights the row for `uri`, or none.
    pub fn select(&self, uri: &str) {
        let index = self
            .entries
            .borrow()
            .iter()
            .position(|entry| match &entry.target {
                RowTarget::Location(candidate) => same_location(candidate, uri),
                RowTarget::MountVolume(_) => false,
            });
        let row = index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.list.row_at_index(index));
        self.list.select_row(row.as_ref());
    }

    /// The labels shown, for tests.
    #[cfg(test)]
    pub fn labels(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .map(|entry| entry.label.clone())
            .collect()
    }
}
