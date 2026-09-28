// SPDX-License-Identifier: AGPL-3.0-only
//! A tab's context menu.
//!
//! Ports the `contextmenu` handler of `setupTabDrag` in
//! `desktop/ui/app.js`: Move tab to new window, Move tab to window…,
//! Duplicate tab, Open windows… and Close tab. Moving tabs between windows
//! arrives with drag and drop, so its two items are disabled with a
//! tooltip that says so ([`super::unported`]).

use gtk::subclass::prelude::*;

use crate::icons::Icon;

use super::menu_popover::{MenuEntry, MenuItem};
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The menu of the tab `id`, which shows `uri`.
pub(super) fn tab_menu(id: TabId, uri: &str) -> Vec<MenuEntry> {
    let tab = id.to_variant();
    vec![
        MenuItem::new(
            "Move tab to new window",
            Icon::Share,
            WindowAction::MoveTabToNewWindow,
        )
        .into(),
        MenuItem::new(
            "Move tab to window…",
            Icon::Desktop,
            WindowAction::MoveTabToWindow,
        )
        .into(),
        MenuItem::with_text_target("Duplicate tab", Icon::Copy, WindowAction::OpenTab, uri).into(),
        MenuItem::new("Open windows…", Icon::Desktop, WindowAction::OpenWindows).into(),
        MenuEntry::Divider,
        MenuItem::with_target("Close tab", Icon::Dismiss, WindowAction::CloseTabById, tab).into(),
    ]
}

impl BrowserWindow {
    /// "Open windows…": opens the title bar's list of open windows
    /// (`windowsMenu`).
    pub(super) fn show_open_windows(&self) {
        self.imp().open_windows_button.popup();
    }
}

#[cfg(test)]
mod tests {
    use gtk::prelude::*;

    use super::*;

    /// parity: TAB-012
    #[test]
    fn a_tab_menu_offers_the_python_items_in_order() {
        let tab = TabId::from_variant(&1_u64.to_variant()).expect("a tab id is a u64");

        let labels: Vec<String> = tab_menu(tab, "file:///tmp")
            .into_iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label,
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect();

        assert_eq!(
            labels,
            [
                "Move tab to new window",
                "Move tab to window…",
                "Duplicate tab",
                "Open windows…",
                "-",
                "Close tab",
            ]
        );
    }
}
