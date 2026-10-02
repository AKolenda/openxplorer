// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane (sidebar).
//!
//! Ports `renderSidebar` and `.sidebar` in `v2.0.0:desktop/ui/app.js` and
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

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::location::same_location;

use crate::icons::{self, Icon};

use super::folder_tree::FolderTree;
use super::menu_popover::MenuPopover;
use super::window_action::WindowAction;
use super::{gestures, preferences};

pub(super) use drop_spots::SidebarDropSpot;
pub(super) use entries::{recent_and_bin_entries, sidebar_entries};
use entries::{RowTarget, Section, SidebarEntry};

/// Whether a row shows something hidden, listed while "Show all
/// entries" is on (SIDE-010).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum HiddenRow {
    /// A row shown as usual.
    #[default]
    Shown,
    /// A standard folder hidden from Quick access.
    Place,
    /// A row of a hidden section.
    Section,
}

/// The class that dims a hidden row.
const HIDDEN_ROW_CLASS: &str = "hidden-place";

/// The "+" of Map network location.
const MAP_NETWORK_GLYPH: i32 = 17;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{FolderTree, HiddenRow, MenuPopover, SidebarEntry};

    /// Private state of [`super::Sidebar`].
    #[derive(Debug, Default)]
    pub(crate) struct Sidebar {
        /// The rows, built by `constructed`.
        pub(super) list: OnceCell<gtk::ListBox>,
        /// The folder tree below them (SIDE-028).
        pub(super) folder_tree: OnceCell<FolderTree>,
        /// What each row shows and does, in row order.
        pub(super) entries: RefCell<Vec<SidebarEntry>>,
        /// The rows' context menu, built by `constructed`.
        pub(super) menu: OnceCell<MenuPopover>,
        /// The rows' icon size in pixels, 0 for automatic (SIDE-012).
        pub(super) icon_size: Cell<u32>,
        /// Which rows show something hidden, in row order (SIDE-010).
        pub(super) hidden_rows: RefCell<Vec<HiddenRow>>,
        /// Whether anything is hidden, which "Show all entries" lists.
        pub(super) anything_hidden: Cell<bool>,
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

    /// The folder tree below the places.
    pub(super) fn folder_tree(&self) -> &FolderTree {
        self.imp()
            .folder_tree
            .get()
            .expect("constructed builds the folder tree")
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
        // The folder tree, while shown, takes the lower part (SIDE-028).
        let folder_tree = FolderTree::default();
        let panes = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&scroller)
            .end_child(&folder_tree)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .vexpand(true)
            .build();
        self.append(&panes);
        self.append(&map_network_button());
        let imp = self.imp();
        imp.list.set(list).expect("constructed runs once per object");
        imp.folder_tree
            .set(folder_tree)
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
    /// a plain click goes on to the row. Like a middle-click it acts on
    /// release, so Ctrl+drag still drags the place: a drag cancels it.
    fn open_places_in_tabs_on_ctrl_click(&self, list: &gtk::ListBox) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let pending: Rc<RefCell<Option<(WindowAction, String)>>> = Rc::default();
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[strong]
            pending,
            move |gesture, _, _, y| {
                let action = tab_action_for_click(gestures::held_modifiers(gesture));
                let target = action.zip(sidebar.location_at(y));
                if target.is_none() {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                }
                pending.replace(target);
            }
        ));
        click.connect_stopped(glib::clone!(
            #[strong]
            pending,
            move |_| {
                pending.take();
            }
        ));
        click.connect_cancel(glib::clone!(
            #[strong]
            pending,
            move |_, _| {
                pending.take();
            }
        ));
        click.connect_released(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |gesture, _, _, _| {
                let Some((action, uri)) = pending.take() else {
                    return;
                };
                // Claiming keeps the row from also opening in this tab.
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
        self.imp().hidden_rows.replace(Vec::new());
        for row in &rows {
            list.append(row);
        }
    }

    /// Replaces the rows with `rows`, dimming the hidden ones shown, and
    /// records whether `anything_hidden` (SIDE-010).
    pub(super) fn set_rows(&self, rows: Vec<(SidebarEntry, HiddenRow)>, anything_hidden: bool) {
        let (entries, hidden): (Vec<SidebarEntry>, Vec<HiddenRow>) = rows.into_iter().unzip();
        self.set_entries(entries);
        self.mark_hidden(hidden);
        self.imp().anything_hidden.set(anything_hidden);
    }

    /// Redraws the one row that goes to the same place as `entry` as
    /// `entry`, keeping its selection, focus and dimming; the other rows
    /// stay as they are. False when no row goes there.
    pub(super) fn replace_entry(&self, entry: SidebarEntry) -> bool {
        let imp = self.imp();
        let Some(index) = imp
            .entries
            .borrow()
            .iter()
            .position(|row| row.target == entry.target)
        else {
            return false;
        };
        let Ok(position) = i32::try_from(index) else {
            return false;
        };
        let Some(old) = self.list().row_at_index(position) else {
            return false;
        };
        imp.entries.borrow_mut()[index] = entry;
        let row = {
            let entries = imp.entries.borrow();
            let edges = entries::section_edges(&entries, index);
            row::sidebar_row(&entries[index], edges, imp.icon_size.get())
        };
        let hidden = imp.hidden_rows.borrow().get(index).copied().unwrap_or_default();
        if hidden != HiddenRow::Shown {
            row.add_css_class(HIDDEN_ROW_CLASS);
        }
        let (selected, focused) = (old.is_selected(), old.has_focus());
        let list = self.list();
        list.remove(&old);
        list.insert(&row, position);
        if selected {
            list.select_row(Some(&row));
        }
        if focused {
            row.grab_focus();
        }
        true
    }

    /// Dims the rows `hidden` marks and remembers them for the menus.
    fn mark_hidden(&self, hidden: Vec<HiddenRow>) {
        for (index, state) in hidden.iter().enumerate() {
            let row = i32::try_from(index)
                .ok()
                .and_then(|index| self.list().row_at_index(index));
            if let (Some(row), true) = (row, *state != HiddenRow::Shown) {
                row.add_css_class(HIDDEN_ROW_CLASS);
            }
        }
        self.imp().hidden_rows.replace(hidden);
    }

    /// Draws the rows' icons `size` pixels big, or at the automatic size
    /// for 0 (Dolphin's Places panel Icon Size, SIDE-012).
    pub(super) fn set_icon_size(&self, size: u32) {
        if self.imp().icon_size.replace(size) == size {
            return;
        }
        let entries = self.imp().entries.borrow().clone();
        let hidden = self.imp().hidden_rows.borrow().clone();
        let selected = self.list().selected_row().map(|row| row.index());
        self.set_entries(entries);
        self.mark_hidden(hidden);
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

    /// The labels shown, for tests.
    #[cfg(test)]
    pub(super) fn labels(&self) -> Vec<String> {
        let entries = self.imp().entries.borrow();
        entries.iter().map(|entry| entry.label.clone()).collect()
    }
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
    content.append(&gtk::Label::new(Some(&ox_core::i18n::gettext(
        "Map network location",
    ))));
    let button = gtk::Button::builder()
        .child(&content)
        .action_name(WindowAction::MapNetworkLocation.detailed_name())
        .tooltip_text(&ox_core::i18n::gettext("Map network location"))
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
    use crate::icons::Art;

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
