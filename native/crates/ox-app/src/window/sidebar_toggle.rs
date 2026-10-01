// SPDX-License-Identifier: AGPL-3.0-only
//! Showing and hiding the navigation pane (SIDE-024).
//!
//! Dolphin closes its Places panel with F9 and Explorer turns off View >
//! Show > Navigation pane; here View > Navigation pane and F9 run
//! `win.sidebar`, and the choice is saved for every window
//! (`hideSidebar`). While the pane is hidden, a Places button after Up and
//! Refresh lists the same places, as Dolphin's location bar does when its
//! Places panel is closed.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};

use super::menu_popover::{name_menu_button, MenuPopover};
use super::BrowserWindow;

/// The glyph of the Places button, as big as Back's and Up's.
const PLACES_GLYPH: i32 = 16;

impl BrowserWindow {
    /// Adds the Places button to the navigation row and shows the pane as
    /// the preferences say.
    pub(super) fn install_sidebar_toggle(&self) {
        let button = gtk::MenuButton::builder()
            .child(&icons::image(Icon::Folder, PLACES_GLYPH))
            .tooltip_text("Places (F9 shows the navigation pane)")
            .valign(gtk::Align::Center)
            .css_classes(["nav-places"])
            .visible(false)
            .build();
        name_menu_button(&button, "Places");
        let popover = MenuPopover::new(Vec::new());
        button.set_popover(Some(&popover));
        // Built as it opens, so it lists the places shown now.
        button.set_create_popup_func(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[weak]
            popover,
            move |_| popover.set_entries(window.sidebar().places_menu())
        ));
        let imp = self.imp();
        imp.navigation_row
            .insert_child_after(&button, Some(&*imp.navigation_buttons));
        imp.places_button
            .set(button)
            .expect("the Places button is added once");
        let preferences = self.context().settings_data().preferences;
        self.show_sidebar(!preferences.hide_sidebar);
        self.sidebar().set_icon_size(preferences.sidebar_icon_size);
    }

    /// Shows or hides the navigation pane, its resizer with it, and the
    /// Places button in its stead.
    pub(super) fn show_sidebar(&self, shown: bool) {
        self.sidebar().set_visible(shown);
        self.sidebar_resizer().set_visible(shown);
        if let Some(button) = self.imp().places_button.get() {
            button.set_visible(!shown);
        }
    }

    /// The Places button shown while the navigation pane is hidden.
    #[cfg(test)]
    pub(super) fn places_button(&self) -> &gtk::MenuButton {
        self.imp()
            .places_button
            .get()
            .expect("install_sidebar_toggle adds the button")
    }
}
