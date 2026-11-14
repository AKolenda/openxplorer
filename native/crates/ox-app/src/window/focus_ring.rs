// SPDX-License-Identifier: AGPL-3.0-only
//! Showing where the keyboard is only after keyboard navigation, as
//! Windows 11 File Explorer does (ACC-009).
//!
//! GTK draws focus rings (`:focus-visible`) while the window's
//! `focus-visible` is on. A window starts with it off, but GTK 4.14 turns
//! it on for any key pressed with Ctrl or Alt, such as Ctrl+C, and never
//! turns it off again on a click, so after one shortcut the navigation
//! pane's Home row, the crumbs and every control the keyboard sits on
//! kept a ring for good, though the user had only used the mouse. Windows
//! draws them only once the keyboard moves (Tab, F6, the arrows).
//!
//! So a press of any pointer button or a touch hides the rings again, and
//! a shortcut with Ctrl or Alt that moves the keyboard nowhere keeps them
//! hidden. Tab, Shift+Tab, F6, the arrows and Home, End, Page Up and Page
//! Down show them, with or without Ctrl or Alt (GTK 4.14 shows none for a
//! plain arrow), as do the window's own keyboard paths (Alt alone,
//! `focus_regions.rs`, and Home's arrow keys, `home_page.rs`).

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::BrowserWindow;

/// Whether `key` moves the keyboard, so its press may show the ring.
fn moves_the_keyboard(key: gdk::Key) -> bool {
    matches!(
        key,
        gdk::Key::Tab
            | gdk::Key::ISO_Left_Tab
            | gdk::Key::KP_Tab
            | gdk::Key::F6
            | gdk::Key::Up
            | gdk::Key::Down
            | gdk::Key::Left
            | gdk::Key::Right
            | gdk::Key::KP_Up
            | gdk::Key::KP_Down
            | gdk::Key::KP_Left
            | gdk::Key::KP_Right
            | gdk::Key::Home
            | gdk::Key::End
            | gdk::Key::Page_Up
            | gdk::Key::Page_Down
            | gdk::Key::KP_Home
            | gdk::Key::KP_End
            | gdk::Key::KP_Page_Up
            | gdk::Key::KP_Page_Down
    )
}

/// Whether a press of `key` with `held` is a shortcut that moves the
/// keyboard nowhere, such as Ctrl+C or Alt+Print: GTK shows the ring for
/// it, Windows does not.
pub(super) fn is_a_shortcut_alone(key: gdk::Key, held: gdk::ModifierType) -> bool {
    let shortcut = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;
    held.intersects(shortcut) && !moves_the_keyboard(key)
}

impl BrowserWindow {
    /// Hides the focus ring on a click or a touch, and after a shortcut
    /// that moves the keyboard nowhere (see the module's text).
    pub(super) fn show_the_focus_ring_only_for_keyboard_moves(&self) {
        // Whether the last key pressed was a shortcut that moves the
        // keyboard nowhere, pressed while no ring showed.
        let after_a_shortcut = Rc::new(Cell::new(false));
        let pointer = gtk::GestureClick::new();
        pointer.set_button(0);
        pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
        pointer.connect_pressed(glib::clone!(
            #[strong]
            after_a_shortcut,
            move |gesture, _, _, _| {
                after_a_shortcut.set(false);
                if let Some(window) = gesture.widget().and_downcast::<gtk::Window>() {
                    window.set_focus_visible(false);
                }
            }
        ));
        self.add_controller(pointer);

        // GTK turns the ring on for a shortcut while it handles the key,
        // or as the key is let go; so the last key pressed is kept, and
        // the ring that comes on after a shortcut alone is put away. A key
        // that moves the keyboard shows it, as Tab does.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[strong]
            after_a_shortcut,
            move |keys, key, _, held| {
                if let Some(window) = keys.widget().and_downcast::<gtk::Window>() {
                    after_a_shortcut.set(is_a_shortcut_alone(key, held) && !window.gets_focus_visible());
                    if moves_the_keyboard(key) {
                        window.set_focus_visible(true);
                    }
                }
                glib::Propagation::Proceed
            }
        ));
        self.connect_focus_visible_notify(move |window| {
            if window.gets_focus_visible() && after_a_shortcut.get() {
                let weak = window.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(window) = weak.upgrade() {
                        window.set_focus_visible(false);
                    }
                });
            }
        });
        self.add_controller(keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: ACC-009
    #[test]
    fn only_shortcuts_that_move_the_keyboard_nowhere_hide_the_ring() {
        let control = gdk::ModifierType::CONTROL_MASK;
        let alt = gdk::ModifierType::ALT_MASK;
        let none = gdk::ModifierType::empty();
        assert!(is_a_shortcut_alone(gdk::Key::c, control));
        assert!(is_a_shortcut_alone(gdk::Key::Print, alt));
        assert!(is_a_shortcut_alone(gdk::Key::l, control));
        assert!(!is_a_shortcut_alone(gdk::Key::Tab, control), "Ctrl+Tab");
        assert!(!is_a_shortcut_alone(gdk::Key::Left, alt), "Alt+Left");
        assert!(!is_a_shortcut_alone(gdk::Key::Down, none));
        assert!(!is_a_shortcut_alone(gdk::Key::a, none), "typing");
        assert!(
            !is_a_shortcut_alone(gdk::Key::Tab, gdk::ModifierType::SHIFT_MASK),
            "Shift+Tab"
        );
    }
}
