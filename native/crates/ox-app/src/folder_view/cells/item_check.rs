// SPDX-License-Identifier: AGPL-3.0-only
//! Item check boxes (SEL-014), as Windows Explorer's View > Show > Item
//! check boxes.
//!
//! Each item has a check box that shows while the pointer is anywhere on
//! its row or tile and stays, checked, while the item is selected; it is
//! checked exactly when the item is selected. Clicking it selects or
//! deselects that item alone and keeps the rest of the selection, as a
//! Ctrl+click does. In Details and the compact list it sits before the
//! icon, keeping its room so names do not move as it shows; on an icon
//! tile it sits on the icon's corner. When item check boxes are off there
//! are none and they take no room.
//!
//! Whether a check box shows is the stylesheet's (`.item-check` in
//! folder-views.css: on a hovered or selected row or tile, else
//! transparent), so a Details row shows it when hovered in any column.
//! It replaces Dolphin's plus/minus selection toggle, which showed only
//! on the icon of the item under the pointer.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::{CellOwners, FileCell};

/// The check box's accessible name, as Explorer's.
fn check_name() -> &'static str {
    ox_core::i18n::gettext_static("Select")
}

impl FileCell {
    /// Shows the check box when item check boxes are `shown`; with them
    /// off it takes no room.
    pub(crate) fn show_item_check(&self, shown: bool) {
        self.imp().check.set_visible(shown);
    }

    /// Checks the box while `list_item`'s item is selected and makes a
    /// click on it select or deselect that item alone; follows `owners`
    /// for whether check boxes show at all.
    pub(crate) fn follow_item_check(&self, list_item: &gtk::ListItem, owners: &Rc<CellOwners>) {
        let check = &self.imp().check;
        check.add_css_class("item-check");
        check.set_focusable(false);
        check.set_tooltip_text(Some(check_name()));
        check.update_property(&[gtk::accessible::Property::Label(check_name())]);
        check.set_visible(owners.shows_item_checks());
        own_clicks(check);
        let handler = check.connect_toggled(glib::clone!(
            #[weak]
            list_item,
            move |check| {
                // A click asks for the other state; the selection decides.
                if check.is_active() != list_item.is_selected() {
                    toggle_item(check, &list_item);
                }
            }
        ));
        self.check_for(list_item.is_selected(), &handler);
        list_item.connect_selected_notify(glib::clone!(
            #[weak(rename_to = cell)]
            self,
            move |list_item| cell.check_for(list_item.is_selected(), &handler)
        ));
    }

    /// Checks the box of an item that is `selected`, without the change
    /// reaching `handler`, which acts on clicks.
    fn check_for(&self, selected: bool, handler: &glib::SignalHandlerId) {
        let check = &self.imp().check;
        check.block_signal(handler);
        check.set_active(selected);
        check.unblock_signal(handler);
    }

    /// The item's check box, for tests.
    #[cfg(test)]
    pub(crate) fn item_check(&self) -> gtk::CheckButton {
        self.imp().check.clone()
    }
}

/// Makes a press on `check` its own, before the row or tile around it sees
/// it, and toggles `check` on the release. The row would otherwise open
/// its item on a quick second click, and a column title sort its view; a
/// GTK check box claims a click only on its release.
pub(crate) fn own_clicks(check: &gtk::CheckButton) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    click.connect_released(glib::clone!(
        #[weak]
        check,
        move |_, _, _, _| check.set_active(!check.is_active())
    ));
    check.add_controller(click);
}

/// Selects or deselects the item `list_item` shows, keeping the rest of
/// the selection, through the list's own Ctrl+click action; the check box
/// then follows the selection.
fn toggle_item(check: &gtk::CheckButton, list_item: &gtk::ListItem) {
    let position = list_item.position();
    if position == gtk::INVALID_LIST_POSITION {
        check.set_active(list_item.is_selected());
        return;
    }
    let toggle = (position, true, false).to_variant();
    if check.activate_action("list.select-item", Some(&toggle)).is_err() {
        // Outside a list there is nothing to select: the box goes back.
        check.set_active(list_item.is_selected());
    }
}
