// SPDX-License-Identifier: AGPL-3.0-only
//! The title bar: tabs, the new-tab button, an empty drag area, the
//! open-windows button and the caption buttons.
//!
//! Ports `header.titlebar` in `desktop/ui/index.html` and `windowsMenu` in
//! `desktop/ui/app.js`. The window template (`resources/ui/window.ui`)
//! lays the bar out: the "+" right after the last tab, the drag area taking
//! the rest of the width, and the whole bar a `GtkWindowHandle`, so
//! dragging or double-clicking any empty part moves or maximises the
//! window as the desktop expects. This module gives the bar's buttons
//! their glyphs, the new-tab action and the open-windows menu.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::application::AppAction;
use crate::icons::{self, Glyph};

use super::menu_popover::{ItemCheck, MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The "+" glyph of the new-tab button (`.newtab svg{width:14px}`).
const NEW_TAB_GLYPH: i32 = 14;
/// The open-windows button's glyph, at app.js's default icon size
/// (`icon(name, size=18)`).
const OPEN_WINDOWS_GLYPH: i32 = 18;

impl BrowserWindow {
    /// Gives the template's title bar buttons their glyphs, the new-tab
    /// action and the open-windows menu.
    pub(super) fn finish_title_bar(&self) {
        let imp = self.imp();
        let new_tab = &*imp.new_tab_button;
        new_tab.set_child(Some(&icons::glyph(Glyph::Plus, NEW_TAB_GLYPH)));
        WindowAction::NewTab.assign_to(new_tab);
        let open_windows = &*imp.open_windows_button;
        open_windows.set_child(Some(&icons::glyph(Glyph::Desktop, OPEN_WINDOWS_GLYPH)));
        list_open_windows_on_click(open_windows);
    }
}

/// Makes the monitor button (`#windows-button`) open the list of open
/// windows.
fn list_open_windows_on_click(button: &gtk::MenuButton) {
    let popover = MenuPopover::new(Vec::new());
    button.set_popover(Some(&popover));
    // Built as it opens, so it lists the windows open now.
    button.set_create_popup_func(glib::clone!(
        #[weak]
        popover,
        move |button| popover.set_entries(open_windows_menu(button))
    ));
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
