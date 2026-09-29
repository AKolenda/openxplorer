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
use crate::icons::{self, Icon};

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
        new_tab.set_child(Some(&icons::image(Icon::Add, NEW_TAB_GLYPH)));
        WindowAction::NewTab.assign_to(new_tab);
        let open_windows = &*imp.open_windows_button;
        open_windows.set_child(Some(&icons::image(Icon::Desktop, OPEN_WINDOWS_GLYPH)));
        list_open_windows_on_click(open_windows);
    }
}

/// Makes the monitor button (`#windows-button`), or Settings' "Open
/// windows…", open the list of open windows.
pub(crate) fn list_open_windows_on_click(button: &gtk::MenuButton) {
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
/// Quit (`windowsMenu`). The tabs closed in this window come before New
/// window, most recent first, as Dolphin's "Recently Closed Tabs" (TAB-016).
pub(super) fn open_windows_menu(anchor: &gtk::MenuButton) -> Vec<MenuEntry> {
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
    entries.extend(closed_tab_items(&this_window));
    let new_window =
        MenuItem::new("New window", Icon::WindowNew, AppAction::NewWindow).with_shortcut("Ctrl+N");
    entries.push(MenuEntry::Divider);
    entries.push(new_window.into());
    let quit = MenuItem::new("Quit OpenXplorer", Icon::Dismiss, AppAction::Quit).with_shortcut("Ctrl+Q");
    entries.push(quit.into());
    entries
}

/// A divider and one "Reopen <title>" item per tab closed in `window`,
/// most recent first; nothing when no tab has closed.
fn closed_tab_items(window: &BrowserWindow) -> Vec<MenuEntry> {
    let closed = window.closed_tabs();
    if closed.is_empty() {
        return Vec::new();
    }
    let locations = window.imp().locations.borrow();
    let items = closed.iter().zip(0_u32..).map(|(tab, index)| {
        let label = format!("Reopen {}", locations.title_for(tab.uri()));
        let item = MenuItem::with_target(
            &label,
            Icon::History,
            WindowAction::RestoreClosedTab,
            index.to_variant(),
        );
        let item = if index == 0 {
            item.with_shortcut("Ctrl+Shift+T")
        } else {
            item
        };
        MenuEntry::from(item)
    });
    std::iter::once(MenuEntry::Divider).chain(items).collect()
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
        ..MenuItem::new(&title, Icon::Desktop, AppAction::FocusWindow)
    };
    item.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::{Fixture, TestWindow};

    /// The label and check mark of each item of `entries`, `None` for a
    /// divider.
    fn items(entries: &[MenuEntry]) -> Vec<Option<(String, bool)>> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => Some((item.label.clone(), item.check == ItemCheck::Fixed(true))),
                MenuEntry::Divider => None,
            })
            .collect()
    }

    /// The window is titled after its active tab, and the open-windows
    /// menu lists every window by title, this one checked, then New window
    /// and Quit.
    ///
    /// parity: TAB-044, TAB-045
    #[gtk::test]
    fn the_windows_menu_lists_every_window_by_its_tab_title() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri_of("Documents"));
        let _other = test.open_beside(&fixture.uri());
        let root_name = fixture
            .root()
            .file_name()
            .expect("a named folder")
            .to_string_lossy();

        let entries = open_windows_menu(&test.window.imp().open_windows_button);

        assert_eq!(test.window.title().as_deref(), Some("Documents — OpenXplorer"));
        let listed = items(&entries);
        let (windows, rest) = listed.split_at(listed.len() - 3);
        let windows: Vec<_> = windows.iter().flatten().cloned().collect();
        assert!(windows.contains(&("Documents — OpenXplorer".to_owned(), true)));
        assert!(windows.contains(&(format!("{root_name} — OpenXplorer"), false)));
        assert_eq!(
            windows.iter().filter(|(_, checked)| *checked).count(),
            1,
            "only this window is checked"
        );
        assert_eq!(
            rest,
            [
                None,
                Some(("New window".to_owned(), false)),
                Some(("Quit OpenXplorer".to_owned(), false))
            ]
        );
    }

    /// The blank part of the title bar is inside the window handle, so
    /// dragging it moves the window and a double-click maximises it; the
    /// buttons claim their own clicks, and the tab strip a double-click on
    /// a tab (TAB-014).
    ///
    /// parity: TAB-048
    #[gtk::test]
    fn the_blank_title_bar_is_the_window_handle() {
        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let bar = test.window.titlebar().expect("the window has its title bar");
        let blank = crate::test_support::harness::descendants::<gtk::Box>(&bar)
            .into_iter()
            .find(|child| child.has_css_class("title-drag"))
            .expect("the bar has its blank drag area");

        assert!(bar.is::<gtk::WindowHandle>());
        assert!(blank.hexpands(), "the blank area takes the rest of the bar");
        assert!(blank.ancestor(gtk::WindowHandle::static_type()).is_some());
    }
}
