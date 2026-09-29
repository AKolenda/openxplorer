// SPDX-License-Identifier: AGPL-3.0-only
//! The address bar's context menu and middle-click, which app.js lacked:
//! Dolphin's location bar has both, and Explorer's address bar the copy
//! and edit items.
//!
//! A right-click on the crumbs offers Copy address (the location as the
//! address bar shows it), Paste and go (the clipboard's text, opened as a
//! typed address), Open "<crumb>" in new tab and in new window for the
//! crumb under the pointer, Edit address, and the toggles Keep address
//! editable and Show full path (NAV-031). A middle-click on
//! the crumbs' blank space opens the primary selection's text the same way
//! (NAV-032); a middle-click on a crumb still opens that crumb in a tab.
//! While the address is edited, the entry keeps GTK's own menu.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use crate::icons::Icon;

use super::address_bar::{AddressBar, AddressMode};
use super::gestures;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// A crumb under the pointer: its label and its location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PointedCrumb {
    pub label: String,
    pub uri: String,
}

/// The address bar's menu, with the items for `crumb` when the pointer
/// is over one.
pub(super) fn location_menu(crumb: Option<&PointedCrumb>) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = vec![
        MenuItem::new("Copy address", Icon::Copy, WindowAction::CopyAddress).into(),
        MenuItem::new("Paste and go", Icon::ClipboardPaste, WindowAction::PasteAddress).into(),
    ];
    if let Some(crumb) = crumb {
        let label = &crumb.label;
        let uri = crumb.uri.as_str();
        entries.push(MenuEntry::Divider);
        let tab = format!("Open “{label}” in new tab");
        entries.push(MenuItem::with_text_target(&tab, Icon::Add, WindowAction::OpenTab, uri).into());
        let window = format!("Open “{label}” in new window");
        entries
            .push(MenuItem::with_text_target(&window, Icon::WindowNew, WindowAction::OpenWindow, uri).into());
    }
    entries.push(MenuEntry::Divider);
    entries.push(MenuItem::new("Edit address", Icon::Rename, WindowAction::Location).into());
    entries.push(
        MenuItem::toggle(
            "Keep address editable",
            Icon::Code,
            WindowAction::EditableLocation,
        )
        .into(),
    );
    entries.push(MenuItem::toggle("Show full path", Icon::FileFolder, WindowAction::ShowFullPath).into());
    entries
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
        let crumb = self.crumb_at(x, y).and_then(|crumb| {
            let uri = crumb.action_target_value()?.str()?.to_owned();
            let label = crumb.label()?.to_string();
            Some(PointedCrumb { label, uri })
        });
        let popover = MenuPopover::new(location_menu(crumb.as_ref()));
        popover.set_parent(self);
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let point = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&point));
        popover.connect_closed(|popover| {
            let closed = popover.clone();
            glib::idle_add_local_once(move || closed.unparent());
        });
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
        let address = self.imp().locations.borrow().display_location(&uri);
        self.clipboard().set_text(&address);
        self.show_message("Address copied.");
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
                    _ => window.show_message("The clipboard holds no address."),
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
    fn the_address_menu_copies_pastes_opens_the_crumb_elsewhere_and_edits() {
        let crumb = PointedCrumb {
            label: "Documents".into(),
            uri: "file:///home/demo/Documents".into(),
        };

        let over_crumb = location_menu(Some(&crumb));
        let elsewhere = location_menu(None);

        assert_eq!(
            labels(&over_crumb),
            [
                "Copy address",
                "Paste and go",
                "-",
                "Open “Documents” in new tab",
                "Open “Documents” in new window",
                "-",
                "Edit address",
                "Keep address editable",
                "Show full path"
            ]
        );
        assert_eq!(
            labels(&elsewhere)[..4],
            ["Copy address", "Paste and go", "-", "Edit address"]
        );
        let MenuEntry::Item(new_tab) = &over_crumb[3] else {
            panic!("an item");
        };
        assert_eq!(new_tab.target, Some(crumb.uri.to_variant()));
    }
}
