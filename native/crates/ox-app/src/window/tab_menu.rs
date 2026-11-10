// SPDX-License-Identifier: AGPL-3.0-only
//! A tab's context menu.
//!
//! By default the app's own menu, the port of the `contextmenu` handler of
//! `setupTabDrag` in `v2.0.0:desktop/ui/app.js`: Move tab to new window,
//! Move tab to window…, Duplicate tab, Open windows… and Close tab, then
//! Dolphin's Close other tabs (TAB-015) and Windows 11's Close tabs to the
//! right (TAB-059). With Settings' "Show more items in the tab menu" off
//! it is Windows 11 File Explorer's, in its order: Close tab (with its X
//! and Ctrl+W), Close other tabs, Close tabs to the right and Duplicate
//! tab (TAB-060). The two moves are [`super::tab_moves`]'s.

use gtk::subclass::prelude::*;

use crate::icons::Icon;

use super::menu_popover::{MenuEntry, MenuItem};
use super::session::TabId;
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What the menu of a tab depends on besides the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TabMenuFacts {
    /// It is the window's only tab: there are no others to close.
    pub is_only_tab: bool,
    /// It is the last tab of the strip: none is to its right.
    pub is_last_tab: bool,
    /// Settings' "Show more items in the tab menu" is on.
    pub extra_items: bool,
}

/// The menu of the tab `id`, which shows `uri`: the app's own with
/// `facts.extra_items`, else Windows 11's. Close other tabs is disabled
/// on the only tab and Close tabs to the right on the last one.
pub(super) fn tab_menu(id: TabId, uri: &str, facts: TabMenuFacts) -> Vec<MenuEntry> {
    let tab = id.to_variant();
    let close_tab = MenuItem::with_target(
        &ox_core::i18n::gettext("Close tab"),
        Icon::Dismiss,
        WindowAction::CloseTabById,
        tab.clone(),
    )
    .with_shortcut("Ctrl+W");
    let close_others = MenuItem::with_target(
        &ox_core::i18n::gettext("Close other tabs"),
        Icon::Dismiss,
        WindowAction::CloseOtherTabs,
        tab.clone(),
    )
    .disabled_when(facts.is_only_tab);
    let close_right = MenuItem::with_target(
        &ox_core::i18n::gettext("Close tabs to the right"),
        Icon::Dismiss,
        WindowAction::CloseTabsToTheRight,
        tab.clone(),
    )
    .disabled_when(facts.is_last_tab);
    let duplicate = MenuItem::with_text_target(
        &ox_core::i18n::gettext("Duplicate tab"),
        Icon::Copy,
        WindowAction::OpenTab,
        uri,
    );
    if !facts.extra_items {
        // Windows 11 draws no icon beside its two Close … tabs rows.
        return vec![
            close_tab.into(),
            close_others.without_glyph().into(),
            close_right.without_glyph().into(),
            duplicate.into(),
        ];
    }
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
            tab,
        )
        .into(),
        duplicate.into(),
        MenuItem::new(
            &ox_core::i18n::gettext("Open windows…"),
            Icon::Desktop,
            WindowAction::OpenWindows,
        )
        .into(),
        MenuEntry::Divider,
        close_tab.into(),
        close_others.into(),
        close_right.into(),
    ]
}

impl BrowserWindow {
    /// Opens the menu of the tab at `number`, counted from 1, under it, as
    /// the Menu key on that tab does, for the snapshot hook's `tab-menu=`
    /// step; false when there is no such tab.
    pub(crate) fn open_tab_menu(&self, number: usize) -> bool {
        number
            .checked_sub(1)
            .is_some_and(|index| self.imp().tab_strip.open_menu_of_tab(index))
    }

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

    /// With "Show more items in the tab menu" off, the menu is Windows 11
    /// File Explorer's, in its order: Close tab with its X and Ctrl+W,
    /// Close other tabs and Close tabs to the right without an icon, and
    /// Duplicate tab; nothing else.
    ///
    /// parity: TAB-059, TAB-060
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

    /// With "Show more items in the tab menu" on, the default, the menu is
    /// the app's own, in its order, with Windows 11's Close tabs to the
    /// right after Close other tabs.
    ///
    /// parity: TAB-012, TAB-015, TAB-059, TAB-060
    #[test]
    fn the_extra_items_keep_the_apps_own_menu() {
        let entries = tab_menu(tab(), "file:///tmp", middle_tab(true));

        assert_eq!(
            labels(&entries),
            [
                "Move tab to new window",
                "Move tab to window…",
                "Duplicate tab",
                "Open windows…",
                "-",
                "Close tab",
                "Close other tabs",
                "Close tabs to the right",
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
