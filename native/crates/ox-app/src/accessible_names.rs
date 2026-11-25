// SPDX-License-Identifier: AGPL-3.0-only
//! Names, roles and states screen readers hear where GTK 4.14's own would
//! mislead them.
//!
//! GTK's AT-SPI bridge sends a widget's role, name, description and
//! states, and little else; these helpers fix what Orca heard wrong when
//! it read the app (native/crates/ox-app/src/window/tests/screen_reader.rs
//! reads it back the same way):
//!
//! - [`name_value_label`]: a read-only value beside its caption, such as
//!   Properties' "Type of file:", is named "Type of file: Text Document".
//!   Labelled by its caption alone, Orca read the caption twice and never
//!   the value.
//! - [`describe_menu_button`]: a `GtkMenuButton`'s keyboard focus is on a
//!   toggle button inside it, which GTK leaves a plain toggle button, so
//!   Orca read "New toggle button not pressed". The inner button is made a
//!   button that opens a menu, collapsed or expanded as the menu is, as
//!   Narrator reads Windows File Explorer's command bar.

use gtk::glib;
use gtk::prelude::*;

/// Names `value` after `caption` and the value it shows, and keeps the
/// name when the value changes (sizes are counted after the window opens).
///
/// The caption itself is left out for screen readers, which read it in
/// the value's name: Orca read Properties' page as "Type of file: Type of
/// file: Text Document ...". GTK allows that only until the caption is
/// realized.
pub(crate) fn name_value_label(value: &gtk::Label, caption: &gtk::Label) {
    if !caption.is_realized() && caption.accessible_role() == gtk::AccessibleRole::Label {
        caption.set_accessible_role(gtk::AccessibleRole::Presentation);
    }
    let name = |caption: &gtk::Label, value: &gtk::Label| joined(&caption.text(), &value.text());
    value.update_property(&[gtk::accessible::Property::Label(&name(caption, value))]);
    value.connect_label_notify(glib::clone!(
        #[weak]
        caption,
        move |value| {
            value.update_property(&[gtk::accessible::Property::Label(&name(&caption, value))]);
        }
    ));
}

/// The caption and the value as one name: "Size: 20 bytes". A caption
/// that does not end in punctuation is followed by a comma, so a screen
/// reader pauses between them.
fn joined(caption: &str, value: &str) -> String {
    let caption = caption.trim_end();
    if caption.is_empty() {
        return value.to_owned();
    }
    if caption.ends_with(|character: char| character.is_ascii_punctuation() || character == '：') {
        format!("{caption} {value}")
    } else {
        format!("{caption}, {value}")
    }
}

/// The toggle button inside `button`, which takes the keyboard.
pub(crate) fn menu_button_toggle(button: &gtk::MenuButton) -> Option<gtk::ToggleButton> {
    button.first_child().and_downcast::<gtk::ToggleButton>()
}

/// Makes the keyboard's stop inside `button` a button that opens a menu:
/// the button role, a popup, and collapsed or expanded as the menu is,
/// with no pressed state; and names both `name`, when given.
///
/// A menu button needs a name of its own: GTK names a button after
/// what is inside it, and the open menu is inside it, so Orca heard
/// "View Extra large icons Large icons Medium icons ..." while View's
/// menu was open. A button named by a relation instead (a Settings row's
/// title) gets `None`.
pub(crate) fn describe_menu_button(button: &gtk::MenuButton, name: Option<&str>) {
    if let Some(name) = name {
        name_menu_button(button, name);
    }
    let Some(toggle) = menu_button_toggle(button) else {
        return;
    };
    // GTK allows a new role only until the widget is realized (a critical
    // warning after that); a menu button described later keeps the
    // toggle role and still gets the popup and the expanded state.
    if !toggle.is_realized() && toggle.accessible_role() == gtk::AccessibleRole::ToggleButton {
        toggle.set_accessible_role(gtk::AccessibleRole::Button);
    }
    toggle.update_property(&[gtk::accessible::Property::HasPopup(true)]);
    show_menu_state(&toggle);
    toggle.connect_active_notify(show_menu_state);
}

/// Names `button` and the toggle inside it `name`.
pub(crate) fn name_menu_button(button: &gtk::MenuButton, name: &str) {
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    if let Some(toggle) = menu_button_toggle(button) {
        toggle.update_property(&[gtk::accessible::Property::Label(name)]);
    }
}

/// Shows `toggle`'s menu as expanded while it is pressed, and drops the
/// pressed state GTK gives a toggle button.
fn show_menu_state(toggle: &gtk::ToggleButton) {
    toggle.update_state(&[gtk::accessible::State::Expanded(Some(toggle.is_active()))]);
    toggle.reset_state(gtk::AccessibleState::Pressed);
}
