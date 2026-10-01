// SPDX-License-Identifier: AGPL-3.0-only
//! Back, Forward, Up and Refresh: the buttons before the address bar,
//! and the Alt+Left, Alt+Right, Alt+Up and Alt+Home keys that go the same
//! ways.
//!
//! Ports `.nav-buttons` in `v2.0.0:desktop/ui/index.html` and the history keys
//! of `onKey` in `v2.0.0:desktop/ui/app.js`. The window template
//! (`resources/ui/window.ui`) places their row, 5 pixels apart by its CSS
//! `border-spacing`; this module adds the buttons from
//! [`NAVIGATION_BUTTONS`] and the keys from [`NAVIGATION_KEYS`], and
//! [`super::history_menu`] their menus and middle-clicks.
//!
//! The keys are the window's own shortcuts, not application accelerators,
//! which GTK would run before a text field sees them: `onKey` leaves them
//! to the search box and the address field, and ignores them while a
//! dialog is open (NAV-002). F5, Ctrl+R, Ctrl+L and Alt+D stay application
//! accelerators, since app.js runs them from text fields too.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::icons::{self, Icon};

use super::window_action::WindowAction;
use super::BrowserWindow;

/// The glyphs of the Back, Forward, Up and Refresh buttons.
const NAVIGATION_GLYPH: i32 = 16;

/// One of the buttons before the address bar.
#[derive(Debug)]
struct NavigationButton {
    glyph: Icon,
    /// The accessible name (`aria-label`).
    name: &'static str,
    /// The tooltip, with the keyboard shortcut (`title`).
    tooltip: &'static str,
    action: WindowAction,
}

/// Back, Forward, Up and Refresh, in that order.
const NAVIGATION_BUTTONS: [NavigationButton; 4] = [
    NavigationButton {
        glyph: Icon::ArrowLeft,
        name: "Back",
        tooltip: "Back (Alt+Left)",
        action: WindowAction::Back,
    },
    NavigationButton {
        glyph: Icon::ArrowRight,
        name: "Forward",
        tooltip: "Forward (Alt+Right)",
        action: WindowAction::Forward,
    },
    NavigationButton {
        glyph: Icon::ArrowUp,
        name: "Up",
        tooltip: "Up (Alt+Up)",
        action: WindowAction::Up,
    },
    NavigationButton {
        glyph: Icon::ArrowClockwise,
        name: "Refresh",
        tooltip: "Refresh (F5)",
        action: WindowAction::Refresh,
    },
];

/// Each history key of `onKey` and the action it runs, as GTK parses it,
/// then the keys Dolphin and Explorer add: Alt+Home, and the Back,
/// Forward and `HomePage` keys of multimedia keyboards.
const NAVIGATION_KEYS: [(WindowAction, &str); 7] = [
    (WindowAction::Back, "<Alt>Left"),
    (WindowAction::Forward, "<Alt>Right"),
    (WindowAction::Up, "<Alt>Up"),
    (WindowAction::Home, "<Alt>Home"),
    (WindowAction::Back, "Back"),
    (WindowAction::Forward, "Forward"),
    (WindowAction::Home, "HomePage"),
];

impl BrowserWindow {
    /// Fills the template's `.nav-buttons` row and adds the keys that
    /// press its buttons.
    pub(super) fn add_navigation_buttons(&self) {
        let row = &*self.imp().navigation_buttons;
        for command in &NAVIGATION_BUTTONS {
            let button = navigation_button(command);
            if command.action != WindowAction::Refresh {
                self.add_history_gestures(&button, command.action);
            }
            row.append(&button);
        }
        self.install_navigation_keys();
    }

    /// Adds the keys of [`NAVIGATION_KEYS`], handled after the focused
    /// widget had its turn.
    fn install_navigation_keys(&self) {
        let shortcuts = gtk::ShortcutController::new();
        shortcuts.set_propagation_phase(gtk::PropagationPhase::Bubble);
        for (action, keys) in NAVIGATION_KEYS {
            let trigger = gtk::ShortcutTrigger::parse_string(keys);
            let run = gtk::CallbackAction::new(move |widget, _| {
                let Some(window) = widget.downcast_ref::<BrowserWindow>() else {
                    return glib::Propagation::Proceed;
                };
                window.run_navigation_key(action)
            });
            shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(run)));
        }
        self.add_controller(shortcuts);
    }

    /// Runs `action` for its key, unless a text field keeps the key or a
    /// dialog is open. At either end of the history the action is
    /// disabled, so nothing happens.
    fn run_navigation_key(&self, action: WindowAction) -> glib::Propagation {
        if self.focus_is_in_text_field() || !self.takes_navigation_input() {
            return glib::Propagation::Proceed;
        }
        action.activate_from(self, None);
        glib::Propagation::Stop
    }

    /// False while a dialog of the window is open, in the window or as a
    /// window of its own: the history keys and the mouse's side buttons
    /// then do nothing (NAV-002, NAV-003).
    pub(super) fn takes_navigation_input(&self) -> bool {
        self.dialog_layer().shown().is_none() && !self.shows_dialog()
    }
}

fn navigation_button(command: &NavigationButton) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::image(command.glyph, NAVIGATION_GLYPH))
        .tooltip_text(command.tooltip)
        .action_name(command.action.detailed_name())
        .build();
    button.update_property(&[gtk::accessible::Property::Label(command.name)]);
    button
}
