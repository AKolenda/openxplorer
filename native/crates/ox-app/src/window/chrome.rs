// SPDX-License-Identifier: AGPL-3.0-only
//! The window's frame: title bar, navigation row, command bar, workspace,
//! status bar and the toast that reports messages.
//!
//! Ports the static layout of `desktop/ui/index.html`. Each band has a
//! module of its own ([`super::title_bar`], [`super::address_bar`],
//! [`super::search_box`], [`super::command_bar`], [`super::status_bar`],
//! [`super::toast`]); this one stacks them.

use gtk::prelude::*;

use crate::icons::{self, Glyph};

use super::address_bar::AddressBar;
use super::command_bar::CommandBar;
use super::search_box::SearchBox;
use super::status_bar::StatusBar;
use super::tab_strip::TabStrip;
use super::title_bar::title_bar;
use super::toast::Toast;
use super::window_action::WindowAction;

/// The glyphs of the Back, Forward, Up and Refresh buttons.
const NAVIGATION_GLYPH: i32 = 16;

/// One of the buttons before the address bar (`.nav-buttons` in
/// index.html).
#[derive(Debug)]
struct NavigationButton {
    glyph: Glyph,
    /// The accessible name (`aria-label`).
    name: &'static str,
    /// The tooltip, with the keyboard shortcut (`title`).
    tooltip: &'static str,
    action: WindowAction,
}

/// Back, Forward, Up and Refresh, in that order.
const NAVIGATION_BUTTONS: [NavigationButton; 4] = [
    NavigationButton {
        glyph: Glyph::Back,
        name: "Back",
        tooltip: "Back (Alt+Left)",
        action: WindowAction::Back,
    },
    NavigationButton {
        glyph: Glyph::Forward,
        name: "Forward",
        tooltip: "Forward (Alt+Right)",
        action: WindowAction::Forward,
    },
    NavigationButton {
        glyph: Glyph::Up,
        name: "Up",
        tooltip: "Up (Alt+Up)",
        action: WindowAction::Up,
    },
    NavigationButton {
        glyph: Glyph::Refresh,
        name: "Refresh",
        tooltip: "Refresh (F5)",
        action: WindowAction::Refresh,
    },
];

/// The frame's widgets that the controller updates.
#[derive(Debug)]
pub(super) struct Chrome {
    /// The tab strip in the title bar.
    pub tabs: TabStrip,
    /// Breadcrumbs or the editable address.
    pub address: AddressBar,
    /// The search box that filters the folder.
    pub search: SearchBox,
    /// New, the edit commands, Sort, View, More, appearance and Details.
    pub commands: CommandBar,
    /// Counts, the type-to-select hint and the view buttons.
    pub status: StatusBar,
    /// The message at the bottom of the window.
    pub toast: Toast,
    /// The split between the sidebar and the folder and details panes
    /// (`.sidebar-resizer`).
    pub workspace: gtk::Paned,
}

impl Chrome {
    /// Builds the frame into `window`; the workspace is empty.
    pub fn new(window: &gtk::ApplicationWindow) -> Self {
        let tabs = TabStrip::new();
        window.set_titlebar(Some(&title_bar(window.upcast_ref(), &tabs)));
        let address = AddressBar::new();
        let search = SearchBox::new();
        let commands = CommandBar::new();
        let status = StatusBar::new();
        let workspace = workspace();
        let toast = Toast::new();
        let overlay = gtk::Overlay::builder().child(&workspace).vexpand(true).build();
        overlay.add_overlay(&toast);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&navigation_row(&address, &search));
        root.append(&commands.root);
        root.append(&overlay);
        root.append(&status.root);
        window.set_child(Some(&root));
        Self {
            tabs,
            address,
            search,
            commands,
            status,
            toast,
            workspace,
        }
    }

    /// Shows `message` in the toast (see [`Toast::show`]).
    pub fn show_message(&self, message: &str) {
        self.toast.show(message);
    }

    /// Hides the toast's message at once (see [`Toast::hide`]).
    pub fn hide_message(&self) {
        self.toast.hide();
    }
}

/// Back, Forward, Up and Refresh (`.nav-buttons`), 5 pixels apart by
/// their CSS `border-spacing`.
fn navigation_buttons() -> gtk::Box {
    let buttons = gtk::Box::builder()
        .valign(gtk::Align::Center)
        .css_classes(["nav-buttons"])
        .build();
    for command in &NAVIGATION_BUTTONS {
        buttons.append(&navigation_button(command));
    }
    buttons
}

fn navigation_button(command: &NavigationButton) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::glyph(command.glyph, NAVIGATION_GLYPH))
        .tooltip_text(command.tooltip)
        .action_name(command.action.detailed_name())
        .build();
    button.update_property(&[gtk::accessible::Property::Label(command.name)]);
    button
}

/// The navigation buttons, the address bar and the search box, 14 pixels
/// apart by their CSS `border-spacing`, which narrow windows shrink.
fn navigation_row(address: &AddressBar, search: &SearchBox) -> gtk::Box {
    let navigation = gtk::Box::builder().css_classes(["navrow"]).build();
    navigation.update_property(&[gtk::accessible::Property::Label("Navigation")]);
    navigation.append(&navigation_buttons());
    navigation.append(&address.root);
    navigation.append(&search.root);
    navigation
}

/// The sidebar and the panes beside it, split by a 6-pixel handle
/// (`.sidebar-resizer`).
fn workspace() -> gtk::Paned {
    gtk::Paned::builder()
        .orientation(gtk::Orientation::Horizontal)
        .vexpand(true)
        .wide_handle(true)
        .shrink_start_child(false)
        .shrink_end_child(false)
        .resize_start_child(false)
        .css_classes(["workspace"])
        .build()
}
