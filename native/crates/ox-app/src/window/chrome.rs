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

/// The glyphs of the Back, Forward, Up and Refresh buttons.
const NAVIGATION_GLYPH: i32 = 16;

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
    /// The sidebar beside the folder and details panes.
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
        overlay.add_overlay(toast.widget());
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

    /// Shows `message` in the toast (see [`Toast::show`]); an empty message
    /// hides it.
    pub fn show_message(&self, message: &str) {
        self.toast.show(message);
    }
}

/// Back, Forward, Up and Refresh (`.nav-buttons`), 5 pixels apart by
/// their CSS `border-spacing`.
fn navigation_buttons() -> gtk::Box {
    let buttons = gtk::Box::builder()
        .valign(gtk::Align::Center)
        .css_classes(["nav-buttons"])
        .build();
    for (glyph, name, tooltip, action) in [
        (Glyph::Back, "Back", "Back (Alt+Left)", "win.back"),
        (Glyph::Forward, "Forward", "Forward (Alt+Right)", "win.forward"),
        (Glyph::Up, "Up", "Up (Alt+Up)", "win.up"),
        (Glyph::Refresh, "Refresh", "Refresh (F5)", "win.refresh"),
    ] {
        let button = gtk::Button::builder()
            .child(&icons::glyph(glyph, NAVIGATION_GLYPH))
            .tooltip_text(tooltip)
            .action_name(action)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(name)]);
        buttons.append(&button);
    }
    buttons
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
