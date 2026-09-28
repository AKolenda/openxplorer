// SPDX-License-Identifier: AGPL-3.0-only
//! The context menu of a Quick access pin in the sidebar (SIDE-014).
//!
//! Ports `sidebarMenu` in `desktop/ui/app.js` for pins: Open, Open in new
//! tab, Open in Terminal, Open folder with…, the cache entry, Sign out of
//! server… for SMB, Unpin from Quick access, Previous versions and
//! Properties. The commands another milestone brings are listed disabled
//! with a tooltip that names it.

use ox_core::location::is_smb_location;

use crate::icons::Icon;
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::window_action::WindowAction;

/// The menu of the pin that opens `uri`.
pub(super) fn pin_menu(uri: &str) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = vec![
        MenuItem::with_text_target("Open", Icon::Folder, WindowAction::GoTo, uri).into(),
        MenuItem::with_text_target("Open in new tab", Icon::Add, WindowAction::OpenTab, uri).into(),
        MenuItem::new(
            "Open in Terminal",
            Icon::WindowConsole,
            WindowAction::OpenInTerminal,
        )
        .into(),
        MenuItem::new("Open folder with…", Icon::Apps, WindowAction::OpenWith).into(),
        MenuItem::new(
            "Cache this folder for search",
            Icon::Search,
            WindowAction::CacheFolder,
        )
        .into(),
    ];
    if is_smb_location(uri) {
        entries.push(MenuItem::new("Sign out of server…", Icon::SignOut, WindowAction::SignOut).into());
    }
    entries.extend([
        MenuEntry::Divider,
        MenuItem::with_text_target("Unpin from Quick access", Icon::Pin, WindowAction::Unpin, uri).into(),
        MenuEntry::Divider,
        MenuItem::new("Previous versions", Icon::History, WindowAction::PreviousVersions).into(),
        MenuItem::new("Properties", Icon::Info, WindowAction::Properties).into(),
    ]);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let local = labels(&pin_menu("file:///home/user/Projects"));
        let share = labels(&pin_menu("smb://nas/share"));

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
}
