// SPDX-License-Identifier: AGPL-3.0-only
//! The hover selection marker (SEL-014).
//!
//! Ports Dolphin's `KItemListSelectionToggle` (`ShowSelectionToggle`, on by
//! default): hovering an item shows a small button over its icon's corner,
//! a plus while the item is not selected and a minus while it is. Clicking
//! it toggles that item alone and keeps the rest of the selection, as a
//! Ctrl+click does; Windows Explorer's "Item check boxes" do the same.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{CellOwners, FileCell};
use crate::icons::{self, Icon};

/// The marker's glyph edge.
const MARKER_GLYPH: i32 = 12;

/// The glyph and the name of the marker of an item that is `selected`.
fn marker_look(selected: bool) -> (Icon, &'static str) {
    if selected {
        (Icon::Subtract, "Deselect")
    } else {
        (Icon::Add, "Select")
    }
}

impl FileCell {
    /// A Settings change hides an already hovered marker immediately.
    pub(crate) fn hide_selection_marker(&self) {
        self.imp().marker.set_visible(false);
    }

    /// Shows the marker while the pointer is over the cell and markers are
    /// on in `owners`; clicking it toggles `list_item`'s item.
    pub(crate) fn follow_selection_marker(&self, list_item: &gtk::ListItem, owners: &Rc<CellOwners>) {
        let marker = &self.imp().marker;
        marker.add_css_class("selection-marker");
        marker.set_halign(gtk::Align::Start);
        marker.set_valign(gtk::Align::Start);
        marker.set_focusable(false);
        marker.set_visible(false);
        marker.connect_clicked(glib::clone!(
            #[weak]
            list_item,
            move |marker| toggle_item(marker, &list_item)
        ));
        let hover = gtk::EventControllerMotion::new();
        hover.connect_enter(glib::clone!(
            #[weak]
            marker,
            #[weak]
            owners,
            move |_, _, _| marker.set_visible(owners.shows_selection_markers())
        ));
        hover.connect_leave(glib::clone!(
            #[weak]
            marker,
            move |_| marker.set_visible(false)
        ));
        self.add_controller(hover);
        self.show_marker_for(list_item.is_selected());
        list_item.connect_selected_notify(glib::clone!(
            #[weak(rename_to = cell)]
            self,
            move |list_item| cell.show_marker_for(list_item.is_selected())
        ));
    }

    /// Draws the marker of an item that is `selected`.
    fn show_marker_for(&self, selected: bool) {
        let marker = &self.imp().marker;
        let (glyph, name) = marker_look(selected);
        marker.set_child(Some(&icons::image(glyph, MARKER_GLYPH)));
        marker.set_tooltip_text(Some(name));
        marker.update_property(&[gtk::accessible::Property::Label(name)]);
    }

    /// The selection marker, for tests.
    #[cfg(test)]
    pub(crate) fn selection_marker(&self) -> gtk::Button {
        self.imp().marker.clone()
    }
}

/// Toggles the item `list_item` shows, keeping the rest of the selection,
/// through the list's own Ctrl+click action.
fn toggle_item(marker: &gtk::Button, list_item: &gtk::ListItem) {
    let position = list_item.position();
    if position == gtk::INVALID_LIST_POSITION {
        return;
    }
    let toggle = (position, true, false).to_variant();
    // Fails only for a cell outside a list, which has nothing to select.
    let _ = marker.activate_action("list.select-item", Some(&toggle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_adds_or_takes_out() {
        assert_eq!(marker_look(false), (Icon::Add, "Select"));
        assert_eq!(marker_look(true), (Icon::Subtract, "Deselect"));
    }
}
