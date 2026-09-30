// SPDX-License-Identifier: AGPL-3.0-only
//! The crumbs' keys and modified clicks: Left and Right move focus
//! between the crumbs, and a Ctrl+click or a Shift+click opens a crumb in
//! a tab or a window (NAV-019), as in Dolphin.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::super::window_action::WindowAction;
use super::AddressBar;

impl AddressBar {
    /// Left and Right move keyboard focus to the previous or next crumb
    /// and stop at the first and the last, as each crumb's `keydown` in
    /// `renderNavigation` does; GTK's own focus movement would leave the
    /// crumbs at either end. With a modifier the key goes on, so Alt+Left
    /// still goes back.
    pub(super) fn move_between_crumbs_with_arrows(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, modifiers| bar.move_crumb_focus(key, modifiers)
        ));
        self.imp().crumbs.add_controller(keys);
    }

    /// Moves focus one crumb in the direction of `key`, clamped at the
    /// ends; any other key, or an arrow with a modifier, goes on.
    fn move_crumb_focus(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> glib::Propagation {
        let step: isize = match key {
            gdk::Key::Left | gdk::Key::KP_Left => -1,
            gdk::Key::Right | gdk::Key::KP_Right => 1,
            _ => return glib::Propagation::Proceed,
        };
        let shortcut = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::SUPER_MASK;
        if modifiers.intersects(shortcut) {
            return glib::Propagation::Proceed;
        }
        let crumbs = self.crumb_buttons();
        let Some(focused) = crumbs.iter().position(WidgetExt::has_focus) else {
            return glib::Propagation::Proceed;
        };
        let last = crumbs.len() - 1;
        let next = focused.saturating_add_signed(step).min(last);
        crumbs[next].grab_focus();
        glib::Propagation::Stop
    }
}

/// Where a click with `modifiers` opens a crumb, as in Dolphin: Ctrl in a
/// background tab, Ctrl+Shift in a tab in front, Shift in a new window;
/// `None` for a plain click, which navigates the tab.
fn modified_click_action(modifiers: gdk::ModifierType) -> Option<WindowAction> {
    let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
    match (control, shift) {
        (true, false) => Some(WindowAction::OpenTabBackground),
        (true, true) => Some(WindowAction::OpenTab),
        (false, true) => Some(WindowAction::OpenWindow),
        (false, false) => None,
    }
}

/// Opens `uri` in a tab or a window on a Ctrl+click or a Shift+click on
/// `button` (NAV-019), before the button takes the click as its own.
pub(super) fn open_elsewhere_on_modified_click(button: &gtk::Button, uri: &str) {
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let uri = uri.to_owned();
    click.connect_pressed(move |gesture, _, _, _| {
        let Some(action) = modified_click_action(gesture.current_event_state()) else {
            return;
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(button) = gesture.widget() {
            action.activate_from(&button, Some(&uri.to_variant()));
        }
    });
    button.add_controller(click);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: NAV-019
    #[test]
    fn ctrl_opens_a_crumb_in_a_tab_and_shift_in_a_window() {
        let control = gdk::ModifierType::CONTROL_MASK;
        let shift = gdk::ModifierType::SHIFT_MASK;

        let action = modified_click_action;

        assert_eq!(action(control), Some(WindowAction::OpenTabBackground));
        assert_eq!(action(control | shift), Some(WindowAction::OpenTab));
        assert_eq!(action(shift), Some(WindowAction::OpenWindow));
        assert_eq!(action(gdk::ModifierType::empty()), None);
    }
}
