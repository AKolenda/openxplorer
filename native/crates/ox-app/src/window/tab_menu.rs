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

    use super::super::menu_popover::ItemAvailability;
    use super::*;

    fn tab() -> TabId {
        TabId::from_variant(&1_u64.to_variant()).expect("a tab id is a u64")
    }

    /// The facts of the middle of three tabs, with or without the extra items.
    fn middle_tab(extra_items: bool) -> TabMenuFacts {
        TabMenuFacts {
            is_only_tab: false,
            is_last_tab: false,
            extra_items,
        }
    }

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    }

    fn item<'a>(entries: &'a [MenuEntry], label: &str) -> &'a MenuItem {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == label => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the menu has {label}"))
    }

    /// Windows 11 File Explorer's tab menu, in its order: Close tab with
    /// its X and Ctrl+W, Close other tabs and Close tabs to the right
    /// without an icon, and Duplicate tab; nothing else.
    ///
    /// parity: TAB-012, TAB-059
    #[test]
    fn a_tab_menu_offers_windows_items_in_order() {
        let entries = tab_menu(tab(), "file:///tmp", middle_tab(false));

        assert_eq!(
            labels(&entries),
            [
                "Close tab",
                "Close other tabs",
                "Close tabs to the right",
                "Duplicate tab",
            ]
        );
        let close = item(&entries, "Close tab");
        assert!(close.shows_glyph);
        assert_eq!(close.glyph, Icon::Dismiss);
        assert_eq!(close.shortcut, Some("Ctrl+W"));
        assert!(!item(&entries, "Close other tabs").shows_glyph);
        assert!(!item(&entries, "Close tabs to the right").shows_glyph);
        let duplicate = item(&entries, "Duplicate tab");
        assert!(duplicate.shows_glyph);
        assert_eq!(duplicate.glyph, Icon::Copy);
    }

    /// "Show more items in the tab menu" adds the app's own items back
    /// after Windows', so Move tab to new window, Move tab to window… and
    /// Open windows… are not lost.
    ///
    /// parity: TAB-012, TAB-060
    #[test]
    fn the_extra_items_follow_windows_items_when_asked_for() {
        let entries = tab_menu(tab(), "file:///tmp", middle_tab(true));

        assert_eq!(
            labels(&entries),
            [
                "Close tab",
                "Close other tabs",
                "Close tabs to the right",
                "Duplicate tab",
                "-",
                "Move tab to new window",
                "Move tab to window…",
                "Open windows…",
            ]
        );
    }

    /// Close other tabs has nothing to close on the only tab, and Close
    /// tabs to the right nothing on the last one, so they are grayed out
    /// there.
    ///
    /// parity: TAB-015, TAB-059
    #[test]
    fn close_items_are_disabled_when_there_is_nothing_to_close() {
        let disabled = |facts: TabMenuFacts| -> Vec<String> {
            tab_menu(tab(), "file:///tmp", facts)
                .into_iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Item(item) if item.availability == ItemAvailability::Disabled => {
                        Some(item.label)
                    }
                    _ => None,
                })
                .collect()
        };
        let only = TabMenuFacts {
            is_only_tab: true,
            is_last_tab: true,
            extra_items: false,
        };
        let last = TabMenuFacts {
            is_last_tab: true,
            ..middle_tab(false)
        };

        assert_eq!(disabled(only), ["Close other tabs", "Close tabs to the right"]);
        assert_eq!(disabled(last), ["Close tabs to the right"]);
        assert!(disabled(middle_tab(false)).is_empty());
    }
}
