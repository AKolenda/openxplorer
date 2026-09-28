// SPDX-License-Identifier: AGPL-3.0-only
//! Back, Forward, Up and Refresh: the buttons before the address bar.
//!
//! Ports `.nav-buttons` in `desktop/ui/index.html`. The window template
//! (`resources/ui/window.ui`) places their row, 5 pixels apart by its CSS
//! `border-spacing`; this module adds the buttons from
//! [`NAVIGATION_BUTTONS`].

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

impl BrowserWindow {
    /// Fills the template's `.nav-buttons` row.
    pub(super) fn add_navigation_buttons(&self) {
        let row = &*self.imp().navigation_buttons;
        for command in &NAVIGATION_BUTTONS {
            row.append(&navigation_button(command));
        }
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
