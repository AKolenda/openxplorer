// SPDX-License-Identifier: AGPL-3.0-only
//! The context menu of a Quick access pin in the sidebar (SIDE-014).
//!
//! Ports `sidebarMenu` in `desktop/ui/app.js` for pins: Open, Open in new
//! tab, Open in Terminal, Open folder with…, the cache entry, Sign out of
//! server… for SMB, Unpin from Quick access, Previous versions and
//! Properties. Every item acts on the pin's location, not on the folder
//! shown.

use ox_core::location::is_smb_location;
use ox_core::search::Caching;

use crate::icons::Icon;
use crate::window::cache_folder::cache_item;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;

/// The menu of the pin that opens `uri`; `caching` says whether it is
/// cached for search, `None` where it cannot be.
pub(super) fn pin_menu(uri: &str, caching: Option<Caching>) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = vec![
        MenuItem::with_text_target("Open", Icon::Folder, WindowAction::GoTo, uri).into(),
        MenuItem::with_text_target("Open in new tab", Icon::Add, WindowAction::OpenTab, uri).into(),
        MenuItem::with_text_target(
            "Open in Terminal",
            Icon::WindowConsole,
            WindowAction::OpenInTerminalOf,
            uri,
        )
        .into(),
        MenuItem::with_text_target("Open folder with…", Icon::Apps, WindowAction::OpenWithOf, uri).into(),
    ];
    if let Some(caching) = caching {
        entries.push(cache_item(uri, caching).into());
    }
    if is_smb_location(uri) {
        let sign_out = MenuItem::with_text_target(
            "Sign out of server…",
            Icon::ArrowEject,
            WindowAction::SignOut,
            uri,
        );
        entries.push(sign_out.into());
    }
    entries.extend([
        MenuEntry::Divider,
        MenuItem::with_text_target("Unpin from Quick access", Icon::Pin, WindowAction::Unpin, uri).into(),
        MenuEntry::Divider,
        MenuItem::with_text_target(
            "Previous versions",
            Icon::History,
            WindowAction::PreviousVersionsOf,
            uri,
        )
        .into(),
        MenuItem::with_text_target("Properties", Icon::Info, WindowAction::PropertiesOf, uri).into(),
    ]);
    entries
}

#[cfg(test)]
mod tests {
    use gtk::prelude::ToVariant;

    use super::*;
    use crate::window::menu_popover::ItemCheck;

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    }

    /// parity: SIDE-014
    #[test]
    fn a_pin_menu_lists_the_python_items_and_sign_out_only_for_smb() {
        let local = labels(&pin_menu("file:///home/user/Projects", Some(Caching::Disabled)));
        let share = labels(&pin_menu("smb://nas/share", Some(Caching::Disabled)));

        assert_eq!(
            local,
            [
                "Open",
                "Open in new tab",
                "Open in Terminal",
                "Open folder with…",
                "Cache this folder for search",
                "-",
                "Unpin from Quick access",
                "-",
                "Previous versions",
                "Properties",
            ]
        );
        assert_eq!(share[5], "Sign out of server…");
        assert_eq!(share.len(), local.len() + 1);
    }

    /// parity: SIDE-014
    #[test]
    fn every_pin_menu_item_acts_on_the_pin() {
        let uri = "smb://nas/share";

        let entries = pin_menu(uri, Some(Caching::Enabled));

        let items: Vec<&MenuItem> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item),
                MenuEntry::Divider => None,
            })
            .collect();
        for item in &items {
            assert_eq!(item.target, Some(uri.to_variant()), "{}", item.label);
        }
        let cache = items
            .iter()
            .find(|item| item.label == "Cache this folder for search")
            .expect("a share can be cached");
        assert_eq!(cache.check, ItemCheck::Fixed(true), "the cached pin is checked");
    }

    #[test]
    fn a_pin_that_cannot_be_indexed_has_no_cache_entry() {
        let entries = labels(&pin_menu("mtp://phone/", None));

        assert!(!entries.contains(&"Cache this folder for search".to_owned()));
    }
}
