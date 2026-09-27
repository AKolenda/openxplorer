// SPDX-License-Identifier: AGPL-3.0-only
//! The window's frame: title bar, navigation row, command bar, workspace,
//! status bar and the toast that reports messages.
//!
//! Ports the static layout of `desktop/ui/index.html`. Each band has a
//! module of its own ([`super::title_bar`], [`super::address_bar`],
//! [`super::search_box`], [`super::command_bar`], [`super::status_bar`]);
//! this one stacks them and owns the toast (`toast()` in app.js).

use std::cell::RefCell;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::icons::{self, Glyph};

use super::address_bar::AddressBar;
use super::command_bar::CommandBar;
use super::search_box::SearchBox;
use super::status_bar::StatusBar;
use super::tab_strip::TabStrip;
use super::title_bar::title_bar;

/// How long a toast stays (`setTimeout(..., 4000)` in `toast()`).
const TOAST_DURATION: Duration = Duration::from_secs(4);

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
    /// The toast's text; hidden when there is no message.
    pub message: gtk::Label,
    /// The sidebar beside the folder and details panes.
    pub workspace: gtk::Paned,
    toast_timer: RefCell<Option<glib::SourceId>>,
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
        let message = toast();
        let overlay = gtk::Overlay::builder().child(&workspace).vexpand(true).build();
        overlay.add_overlay(&message);
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
            message,
            workspace,
            toast_timer: RefCell::default(),
        }
    }

    /// Shows `message` in the toast for four seconds; an empty message
    /// hides the toast at once. A new message replaces the old one and
    /// restarts the time.
    pub fn show_message(&self, message: &str) {
        if let Some(timer) = self.toast_timer.take() {
            timer.remove();
        }
        self.message.set_text(message);
        self.message.set_visible(!message.is_empty());
        if message.is_empty() {
            return;
        }
        let toast = self.message.downgrade();
        let timer = glib::timeout_add_local_once(TOAST_DURATION, move || {
            if let Some(toast) = toast.upgrade() {
                toast.set_visible(false);
            }
        });
        self.toast_timer.replace(Some(timer));
    }

    /// Clears the search box.
    pub fn clear_filter(&self) {
        self.search.entry.set_text("");
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        if let Some(timer) = self.toast_timer.take() {
            timer.remove();
        }
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
            .child(&icons::glyph(glyph, 16))
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

/// The toast at the bottom centre of the window (`.toast`).
fn toast() -> gtk::Label {
    let toast = gtk::Label::builder()
        .wrap(true)
        .max_width_chars(80)
        .justify(gtk::Justification::Center)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::End)
        .selectable(true)
        .visible(false)
        .accessible_role(gtk::AccessibleRole::Status)
        .css_classes(["toast"])
        .build();
    toast.set_can_target(true);
    toast
}
