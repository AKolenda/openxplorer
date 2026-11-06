// SPDX-License-Identifier: AGPL-3.0-only
//! A tab's context menu.
//!
//! Ports the `contextmenu` handler of `setupTabDrag` in
//! `v2.0.0:desktop/ui/app.js`: Move tab to new window, Move tab to window…,
//! Duplicate tab, Open windows… and Close tab, and Dolphin's Close other
//! tabs (TAB-015). The two moves are [`super::tab_moves`]'s.

use gtk::subclass::prelude::*;

use crate::icons::Icon;

use super::menu_popover::{MenuEntry, MenuItem};
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// The menu of the tab `id`, which shows `uri`; Close other tabs is
/// disabled when it is the only tab.
pub(super) fn tab_menu(id: TabId, uri: &str, is_only_tab: bool) -> Vec<MenuEntry> {
    let tab = id.to_variant();
    vec![
        MenuItem::with_target(
            &ox_core::i18n::gettext("Move tab to new window"),
            Icon::Share,
            WindowAction::MoveTabToNewWindow,
            tab.clone(),
        )
        .into(),
        MenuItem::with_target(
            &ox_core::i18n::gettext("Move tab to window…"),
            Icon::Desktop,
            WindowAction::MoveTabToWindow,
            tab.clone(),
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Duplicate tab"),
            Icon::Copy,
            WindowAction::OpenTab,
            uri,
        )
        .into(),
        MenuItem::new(
            &ox_core::i18n::gettext("Open windows…"),
            Icon::Desktop,
            WindowAction::OpenWindows,
        )
        .into(),
        MenuEntry::Divider,
        MenuItem::with_target(
            &ox_core::i18n::gettext("Close tab"),
            Icon::Dismiss,
            WindowAction::CloseTabById,
            tab.clone(),
        )
        .into(),
        MenuItem::with_target(
            &ox_core::i18n::gettext("Close other tabs"),
            Icon::Dismiss,
            WindowAction::CloseOtherTabs,
            tab,
        )
        .disabled_when(is_only_tab)
        .into(),
    ]
}

impl BrowserWindow {
    /// "Open windows…": opens the list of open windows (`windowsMenu`)
    /// below the tab in front, for the tab menu, `--windows` and the
    /// launcher's "Open windows…" (TAB-044). Windows 11's title bar has
    /// no button for it, so neither has this one.
    pub(crate) fn show_open_windows(&self) {
        let entries = super::title_bar::open_windows_menu(self);
        self.imp().tab_strip.show_windows_menu(entries);
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

        let labels: Vec<String> = tab_menu(tab, "file:///tmp", false)
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
                "Close other tabs",
            ]
        );
    }
}
