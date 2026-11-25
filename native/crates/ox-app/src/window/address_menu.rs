// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar's context menu and middle-click, which app.js lacked.
//!
//! A right-click on a crumb or on blank space of the address bar shows
//! Windows 11 File Explorer's menu, exactly its four commands (checked on
//! Windows 11 build 26200): Copy Address, Copy Address as Text, Edit
//! Address and Delete History (NAV-031). On a crumb the copies are of that
//! crumb's folder, elsewhere of the location shown. Copy Address puts the
//! folder itself on the clipboard, as Copy does, so a paste into another
//! folder copies it; Copy Address as Text puts its address as text; Delete
//! History forgets the typed addresses (NAV-043). Keep address editable
//! and Show full path stay in the entry's own menu and in Settings. A
//! middle-click on the crumbs' blank space opens the primary selection's
//! text as a pasted address (NAV-032); a middle-click on a crumb still
//! opens that crumb in a tab. While the address is edited, the entry keeps
//! GTK's own menu.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::Icon;

use super::address_bar::{AddressBar, AddressMode};
use super::gestures;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The address bar's menu, whose copies are of `uri`: the crumb under
/// the pointer, or the location shown.
pub(super) fn location_menu(uri: &str) -> Vec<MenuEntry> {
    vec![
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Copy Address"),
            Icon::Copy,
            WindowAction::CopyAddressOf,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Copy Address as Text"),
            Icon::Copy,
            WindowAction::CopyAddressAsText,
            uri,
        )
        .into(),
        MenuItem::new(
            &ox_core::i18n::gettext("Edit Address"),
            Icon::Rename,
            WindowAction::Location,
        )
        .into(),
        MenuItem::new(
            &ox_core::i18n::gettext("Delete History"),
            Icon::Delete,
            WindowAction::DeleteAddressHistory,
        )
        .into(),
    ]
}

impl AddressBar {
    /// Gives the crumbs their context menu and their middle-click paste.
    pub(super) fn add_location_menu(&self) {
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |gesture, _, x, y| {
                if bar.mode() != AddressMode::Crumbs {
                    return;
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
                bar.popup_location_menu(x, y);
            }
        ));
        self.add_controller(right_click);
        self.add_controller(gestures::middle_click(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_, _, _| {
                if bar.mode() == AddressMode::Crumbs {
                    bar.open_primary_selection();
                }
            }
        )));
    }

    /// Opens the address bar's menu at `x`, `y`.
    pub(super) fn popup_location_menu(&self, x: f64, y: f64) -> MenuPopover {
        let shown = self
            .root()
            .and_downcast::<BrowserWindow>()
            .and_then(|window| window.current_uri())
            .unwrap_or_default();
        let uri = self.crumb_location_at(x, y).unwrap_or(shown);
        // The bar keeps one menu and shows it again each time
        // ([`MenuPopover::kept_by`]).
        let popover = MenuPopover::kept_by(self, "location-menu");
        popover.set_entries(location_menu(&uri));
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&point));
        popover.popup();
        popover
    }

    /// Goes to the primary selection's text as a pasted address (NAV-032).
    fn open_primary_selection(&self) {
        let primary = WidgetExt::display(self).primary_clipboard();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            async move {
                let Ok(Some(text)) = primary.read_text_future().await else {
                    return;
                };
                if let Some(window) = bar.root().and_downcast::<BrowserWindow>() {
                    if !text.trim().is_empty() {
                        window.go_to_pasted_address(&text);
                    }
                }
            }
        ));
    }
}

impl BrowserWindow {
    /// Copies the current location as the address bar shows it.
    pub(super) fn copy_address(&self) {
        let Some(uri) = self.current_uri() else { return };
        let address = self.imp().locations.borrow().copied_location(&uri);
        self.clipboard().set_text(&address);
        self.show_message(&ox_core::i18n::gettext("Address copied."));
    }

    /// Copy Address: puts the folder at `uri` on the clipboard as Copy
    /// does, so a paste into another folder copies it. A page, such as
    /// This PC, has no folder to copy and its address is copied as text.
    pub(super) fn copy_address_of(&self, uri: &str) {
        if crate::window::file_drag::is_draggable_location(uri) {
            self.copy_items(ox_core::clipboard::ClipboardMode::Copy, &[uri.to_owned()]);
        } else {
            self.copy_address_as_text(uri);
        }
    }

    /// Copy Address as Text: puts the address of `uri`, as the address
    /// bar shows it, on the clipboard as text.
    pub(super) fn copy_address_as_text(&self, uri: &str) {
        let address = self.imp().locations.borrow().copied_location(uri);
        self.clipboard().set_text(&address);
        self.show_message(&ox_core::i18n::gettext("Address copied."));
    }

    /// Delete History: forgets the typed addresses of every window, as
    /// Windows' address bar menu does.
    pub(super) fn delete_address_history(&self) {
        self.context().forget_typed_addresses();
        self.address_bar().hide_suggestions();
    }

    /// Opens the clipboard's text as a typed address.
    pub(super) fn paste_address(&self) {
        let clipboard = self.clipboard();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                match clipboard.read_text_future().await {
                    Ok(Some(text)) if !text.trim().is_empty() => window.go_to_pasted_address(&text),
                    _ => window.show_message(&ox_core::i18n::gettext("The clipboard holds no address.")),
                }
            }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        let label = |entry: &MenuEntry| match entry {
            MenuEntry::Item(item) => item.label.clone(),
            MenuEntry::Divider => "-".to_owned(),
        };
        entries.iter().map(label).collect()
    }

    /// parity: NAV-031
    #[test]
    fn the_address_menu_has_windows_four_commands_copying_the_given_folder() {
        let uri = "file:///home/demo/Documents";

        let menu = location_menu(uri);

        assert_eq!(
            labels(&menu),
            [
                "Copy Address",
                "Copy Address as Text",
                "Edit Address",
                "Delete History"
            ]
        );
        for entry in &menu[..2] {
            let MenuEntry::Item(copy) = entry else {
                panic!("an item");
            };
            assert_eq!(copy.target, Some(uri.to_variant()));
        }
    }
}
