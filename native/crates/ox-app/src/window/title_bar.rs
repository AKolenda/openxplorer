// SPDX-License-Identifier: AGPL-3.0-only
//! The title bar: tabs, the new-tab button, an empty drag area, the
//! open-windows button and the caption buttons.
//!
//! Ports `header.titlebar` in `desktop/ui/index.html` and `windowsMenu` in
//! `desktop/ui/app.js`. The "+" sits right after the last tab and the drag
//! area takes the rest of the width. The whole bar is a `GtkWindowHandle`,
//! so dragging or double-clicking any empty part moves or maximises the
//! window as the desktop expects.

use gtk::glib;
use gtk::prelude::*;

use crate::application::AppAction;
use crate::icons::{self, Glyph};

use super::caption_buttons::CaptionButtons;
use super::menu_popover::{ItemCheck, MenuEntry, MenuItem, MenuPopover};
use super::tab_strip::TabStrip;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The title bar of `window` around `tabs`, to set as its title bar.
pub(super) fn title_bar(window: &gtk::Window, tabs: &TabStrip) -> gtk::WindowHandle {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("ox-titlebar");
    bar.append(&CaptionButtons::new(window, gtk::PackType::Start));
    bar.append(&tabs.root);
    bar.append(&new_tab_button());
    bar.append(&drag_area());
    bar.append(&open_windows_button());
    bar.append(&CaptionButtons::new(window, gtk::PackType::End));
    gtk::WindowHandle::builder().child(&bar).build()
}

fn new_tab_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Plus, 14))
        .tooltip_text("New tab (Ctrl+T)")
        .action_name(WindowAction::NewTab.detailed_name())
        .valign(gtk::Align::End)
        .css_classes(["newtab"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("New tab")]);
    button
}

/// The empty part of the title bar (`.title-drag`), at least 16 pixels
/// wide so the window can always be grabbed.
fn drag_area() -> gtk::Box {
    gtk::Box::builder()
        .hexpand(true)
        .css_classes(["title-drag"])
        .build()
}

/// The monitor button that lists the open windows (`#windows-button`).
fn open_windows_button() -> gtk::MenuButton {
    let popover = MenuPopover::new(Vec::new());
    let button = gtk::MenuButton::builder()
        .child(&icons::glyph(Glyph::Desktop, 18))
        .tooltip_text("Open windows")
        .popover(&popover)
        .valign(gtk::Align::Center)
        .css_classes(["windows-button"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Open windows")]);
    // Built as it opens, so it lists the windows open now.
    button.set_create_popup_func(glib::clone!(
        #[weak]
        popover,
        move |button| popover.set_entries(open_windows_menu(button))
    ));
    button
}

/// Every browser window by title, this one checked, then New window and
/// Quit (`windowsMenu`).
fn open_windows_menu(anchor: &gtk::MenuButton) -> Vec<MenuEntry> {
    let Some(this_window) = anchor.root().and_downcast::<BrowserWindow>() else {
        return Vec::new();
    };
    let windows = this_window
        .application()
        .map(|app| app.windows())
        .unwrap_or_default();
    let browsers = windows
        .into_iter()
        .filter_map(|window| window.downcast::<BrowserWindow>().ok());
    let mut entries: Vec<MenuEntry> = browsers
        .map(|window| window_item(&window, &this_window))
        .collect();
    let new_window = MenuItem::new("New window", Glyph::Plus, AppAction::NewWindow).with_shortcut("Ctrl+N");
    entries.push(MenuEntry::Divider);
    entries.push(new_window.into());
    entries.push(MenuItem::new("Quit OpenXplorer", Glyph::Close, AppAction::Quit).into());
    entries
}

/// The item that brings `window` to the front, checked when it is
/// `this_window`, the one whose menu is open.
fn window_item(window: &BrowserWindow, this_window: &BrowserWindow) -> MenuEntry {
    let title = window
        .title()
        .map_or_else(|| "OpenXplorer".to_owned(), |title| title.to_string());
    let item = MenuItem {
        target: Some(window.id().to_variant()),
        check: ItemCheck::Fixed(window == this_window),
        ..MenuItem::new(&title, Glyph::Desktop, AppAction::FocusWindow)
    };
    item.into()
}
