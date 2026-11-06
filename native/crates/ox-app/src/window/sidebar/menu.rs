// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's context menus.
//!
//! A pin's (SIDE-014) ports `sidebarMenu` in `v2.0.0:desktop/ui/app.js`: Open,
//! Open in new tab, Dolphin's Open in new window (SIDE-015), Open in
//! Terminal, Open folder with…, the cache entry, Sign out of server… for
//! SMB, Edit… for a pin of the user's own (SIDE-011), Unpin from Quick
//! access, Previous versions and Properties; every item acts on the pin's
//! location, not on the folder shown. Drives and network locations have
//! their [`PlaceMenu`](crate::window::place_menus::PlaceMenu), saved
//! searches theirs. This PC's and Network's ([`head_menu`]) list Windows
//! 11's items in its order, where the app has them: Expand or Collapse,
//! Open in new tab, Open in new window, Add a network location (This PC
//! only), Map network drive…, Disconnect network drive and, for This PC,
//! Properties. Every row's menu ends with Hide section, and a hidden
//! row's offers Show (SIDE-010). The empty space offers Add entry…
//! (SIDE-031), Show all entries and the icon sizes (SIDE-012). While the
//! pane is hidden, the Places button lists the places (SIDE-024).

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{is_smb_location, same_location};
use ox_core::search::Caching;

use crate::icons::{Art, Icon};
use crate::window::cache_folder::cache_item;
use crate::window::command_bar::{disconnect_network_drive, NetworkDrive};
use crate::window::menu_popover::{MenuEntry, MenuItem};
use crate::window::place_menus::caching_in;
use crate::window::saved_search::saved_search_menu;
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

use super::entries::{RowLevel, RowTarget, Section, SidebarEntry};
use super::{HiddenRow, Sidebar};

/// The menu of the pin that opens `uri`; `caching` says whether it is
/// cached for search, `None` where it cannot be. A pin of the user's own
/// is `editable` (SIDE-011); a standard folder is not.
pub(super) fn pin_menu(uri: &str, caching: Option<Caching>, editable: bool) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = vec![
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open"),
            Icon::Folder,
            WindowAction::GoTo,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open in Terminal"),
            Icon::WindowConsole,
            WindowAction::OpenInTerminalOf,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open folder with…"),
            Icon::Apps,
            WindowAction::OpenWithOf,
            uri,
        )
        .into(),
    ];
    if let Some(caching) = caching {
        entries.push(cache_item(uri, caching).into());
    }
    if is_smb_location(uri) {
        let sign_out = MenuItem::with_text_target(
            &ox_core::i18n::gettext("Sign out of server…"),
            Icon::ArrowEject,
            WindowAction::SignOut,
            uri,
        );
        entries.push(sign_out.into());
    }
    entries.push(MenuEntry::Divider);
    if editable {
        entries.push(
            MenuItem::with_text_target(
                &ox_core::i18n::gettext("Edit…"),
                Icon::Rename,
                WindowAction::EditPin,
                uri,
            )
            .into(),
        );
    }
    entries.extend([
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Unpin from Quick access"),
            Icon::Pin,
            WindowAction::Unpin,
            uri,
        )
        .into(),
        MenuEntry::Divider,
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Previous versions"),
            Icon::History,
            WindowAction::PreviousVersionsOf,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Properties"),
            Icon::Info,
            WindowAction::PropertiesOf,
            uri,
        )
        .into(),
    ]);
    entries
}

/// What This PC's and Network's menus show besides their fixed items.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct HeadMenuState {
    /// Whether the section is collapsed: Expand, else Collapse.
    pub collapsed: bool,
    /// The connected network shares, for Disconnect network drive.
    pub network_drives: Vec<NetworkDrive>,
    /// Whether the desktop has a system information app for This PC's
    /// Properties to open, as "…" > Properties does.
    pub system_information: bool,
}

/// The menu of This PC's or Network's row, which opens `uri`, laid out as
/// Windows 11's navigation pane lays it out. Windows' Rename, Pin to Quick
/// access, Pin to Start and Show more options are left out: the app
/// cannot rename or pin a page, and has no Start menu or classic menu
/// here. Network has no Add a network location, as in Windows, and no
/// Properties, whose Windows twin is the Network and Sharing Center.
pub(super) fn head_menu(section: Section, uri: &str, state: &HeadMenuState) -> Vec<MenuEntry> {
    let mut entries = Vec::new();
    if let Some((key, _)) = section.hiding() {
        let (label, glyph) = if state.collapsed {
            (ox_core::i18n::gettext("Expand"), Icon::ChevronRight16)
        } else {
            (ox_core::i18n::gettext("Collapse"), Icon::ChevronDown16)
        };
        entries
            .push(MenuItem::with_text_target(&label, glyph, WindowAction::ToggleSidebarSection, key).into());
    }
    entries.extend([
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open in new tab"),
            Icon::Add,
            WindowAction::OpenTab,
            uri,
        )
        .into(),
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Open in new window"),
            Icon::WindowNew,
            WindowAction::OpenWindow,
            uri,
        )
        .into(),
    ]);
    if section == Section::ThisPc {
        entries.push(
            MenuItem::new(
                &ox_core::i18n::gettext("Add a network location"),
                Icon::AddNetworkLocation24,
                WindowAction::AddNetworkLocation,
            )
            .into(),
        );
    }
    entries.extend([
        MenuItem::new(
            &ox_core::i18n::gettext("Map network drive…"),
            Icon::MapNetworkDrive24,
            WindowAction::MapNetworkLocation,
        )
        .into(),
        disconnect_network_drive(&state.network_drives).into(),
    ]);
    // As "…" > Properties in This PC: left out, not greyed, without a
    // system information app (the user's decision, 2026-11-04).
    if section == Section::ThisPc && state.system_information {
        entries.push(
            MenuItem::new(
                &ox_core::i18n::gettext("Properties"),
                Icon::Info,
                WindowAction::SystemProperties,
            )
            .into(),
        );
    }
    entries
}

impl Sidebar {
    /// The menu of the row at `y` in the list: a pin's, or a drive's or a
    /// network location's; on empty space, "Add entry…" (SIDE-031);
    /// `None` for a row without one.
    pub(in crate::window) fn menu_entries_at(&self, y: f64) -> Option<Vec<MenuEntry>> {
        #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
        let Some(row) = self.list().row_at_y(y as i32) else {
            return Some(empty_space_menu(self.imp().anything_hidden.get()));
        };
        let index = usize::try_from(row.index()).ok()?;
        let entries = self.imp().entries.borrow();
        let entry = entries.get(index)?;
        let hidden = self
            .imp()
            .hidden_rows
            .borrow()
            .get(index)
            .copied()
            .unwrap_or_default();
        let mut menu = match hidden {
            HiddenRow::Place => return Some(show_place_menu(entry)),
            HiddenRow::Section => Vec::new(),
            HiddenRow::Shown => self.row_menu(entry).unwrap_or_default(),
        };
        if let (Some(hide), HiddenRow::Shown) = (hide_place_item(entry), hidden) {
            if !menu.is_empty() {
                menu.push(MenuEntry::Divider);
            }
            menu.push(hide);
        }
        if let Some((key, name)) = entry.section.hiding() {
            if !menu.is_empty() {
                menu.push(MenuEntry::Divider);
            }
            menu.push(section_item(key, name, hidden == HiddenRow::Section));
        }
        (!menu.is_empty()).then_some(menu)
    }

    /// The menu of `entry` itself: a saved search's, a pin's, or a drive's
    /// or a network location's; `None` for a row without one.
    fn row_menu(&self, entry: &SidebarEntry) -> Option<Vec<MenuEntry>> {
        let window = self.root().and_downcast::<BrowserWindow>();
        if let RowTarget::SavedSearch(search) = &entry.target {
            return Some(saved_search_menu(search));
        }
        if entry.pinned {
            let RowTarget::Location(uri) = &entry.target else {
                return None;
            };
            let caching = window.as_ref().and_then(|window| window.caching_of(uri));
            let editable = window.is_some_and(|window| {
                let quick_access = window.places().quick_access;
                quick_access
                    .iter()
                    .any(|place| place.known_folder.is_none() && same_location(&place.uri, uri))
            });
            return Some(pin_menu(uri, caching, editable));
        }
        if entry.level == RowLevel::Group {
            if let RowTarget::Location(uri) = &entry.target {
                return Some(head_menu(
                    entry.section,
                    uri,
                    &self.head_menu_state(entry.section),
                ));
            }
        }
        let place = entry.menu.as_ref()?;
        let place_menu = place.entries(caching_in(place, self));
        // A drive the system keeps mounted may have nothing to offer.
        (!place_menu.is_empty()).then_some(place_menu)
    }

    /// What This PC's or Network's menu shows for `section` now.
    fn head_menu_state(&self, section: Section) -> HeadMenuState {
        let window = self.root().and_downcast::<BrowserWindow>();
        HeadMenuState {
            collapsed: self.is_collapsed(section),
            network_drives: window
                .as_ref()
                .map(BrowserWindow::network_drives)
                .unwrap_or_default(),
            // Only This PC asks GIO, which lists every application.
            system_information: section == Section::ThisPc
                && crate::integration::system_information().is_some(),
        }
    }

    /// The places as menu items, a divider between sections, for the
    /// Places button shown while the pane is hidden (SIDE-024).
    pub(in crate::window) fn places_menu(&self) -> Vec<MenuEntry> {
        let entries = self.imp().entries.borrow();
        let mut menu = Vec::new();
        let mut section = None;
        for entry in entries.iter() {
            let RowTarget::Location(uri) = &entry.target else {
                continue;
            };
            if section.is_some_and(|section| section != entry.section) {
                menu.push(MenuEntry::Divider);
            }
            section = Some(entry.section);
            let glyph = match entry.icon {
                Art::Glyph(icon) | Art::TintedGlyph(icon, _) => icon,
                Art::Folder | Art::ZipFolder | Art::File(_) | Art::Network(_) => Icon::Folder,
            };
            menu.push(MenuItem::with_text_target(&entry.label, glyph, WindowAction::GoTo, uri).into());
        }
        menu
    }
}

/// The menu of the sidebar's empty space: "Add entry…" (SIDE-031), "Show
/// all entries" while `anything_hidden` (SIDE-010) and the icon sizes
/// (SIDE-012).
fn empty_space_menu(anything_hidden: bool) -> Vec<MenuEntry> {
    let size = |label: &str, pixels: &str| {
        MenuItem::choice(label, Icon::Grid, WindowAction::SidebarIconSize, pixels).into()
    };
    let show_all = MenuItem::toggle(
        &ox_core::i18n::gettext("Show all entries"),
        Icon::Eye,
        WindowAction::SidebarShowAll,
    );
    vec![
        MenuItem::new(
            &ox_core::i18n::gettext("Add entry…"),
            Icon::Add,
            WindowAction::AddPlace,
        )
        .into(),
        show_all.disabled_when(!anything_hidden).into(),
        MenuEntry::Divider,
        size(ox_core::i18n::gettext_static("Automatic icon size"), "0"),
        size(ox_core::i18n::gettext_static("Small icons"), "16"),
        size(ox_core::i18n::gettext_static("Medium icons"), "22"),
        size(ox_core::i18n::gettext_static("Large icons"), "32"),
        size(ox_core::i18n::gettext_static("Huge icons"), "48"),
    ]
}

/// The menu of a hidden standard folder listed by "Show all entries".
fn show_place_menu(entry: &SidebarEntry) -> Vec<MenuEntry> {
    let RowTarget::Location(uri) = &entry.target else {
        return Vec::new();
    };
    vec![MenuItem::with_text_target(
        &ox_core::i18n::gettext("Show"),
        Icon::Eye,
        WindowAction::ShowPlace,
        uri,
    )
    .into()]
}

/// "Hide" for a place other than a pin or a standard folder, which are
/// unpinned instead, and other than the heads of This PC and Network.
fn hide_place_item(entry: &SidebarEntry) -> Option<MenuEntry> {
    let RowTarget::Location(uri) = &entry.target else {
        return None;
    };
    if entry.pinned || entry.level == RowLevel::Group {
        return None;
    }
    Some(
        MenuItem::with_text_target(
            &ox_core::i18n::gettext("Hide"),
            Icon::Eye,
            WindowAction::HidePlace,
            uri,
        )
        .into(),
    )
}

/// "Hide section" for a shown section, or "Show section" for a hidden
/// one, named `name` and saved as `key`.
fn section_item(key: &str, name: &str, hidden: bool) -> MenuEntry {
    let (verb, action) = if hidden {
        (ox_core::i18n::gettext_static("Show"), WindowAction::ShowSection)
    } else {
        (ox_core::i18n::gettext_static("Hide"), WindowAction::HideSection)
    };
    let label = ox_core::i18n::format_message("{verb} section “{name}”", &[("verb", verb), ("name", name)]);
    MenuItem::with_text_target(&label, Icon::Eye, action, key).into()
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

    /// parity: SIDE-014, SIDE-015
    #[test]
    fn a_pin_menu_lists_the_python_items_and_sign_out_only_for_smb() {
        let local = labels(&pin_menu(
            "file:///home/user/Projects",
            Some(Caching::Disabled),
            false,
        ));
        let share = labels(&pin_menu("smb://nas/share", Some(Caching::Disabled), false));

        assert_eq!(
            local,
            [
                "Open",
                "Open in new tab",
                "Open in new window",
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
        assert_eq!(share[6], "Sign out of server…");
        assert_eq!(share.len(), local.len() + 1);
    }

    /// parity: SIDE-014
    #[test]
    fn every_pin_menu_item_acts_on_the_pin() {
        let uri = "smb://nas/share";

        let entries = pin_menu(uri, Some(Caching::Enabled), true);

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

    /// This PC's menu follows Windows 11's order; Properties opens the
    /// system information app where there is one, and Disconnect network
    /// drive lists the connected shares.
    ///
    /// parity: SIDE-033, NET-001
    #[test]
    fn this_pc_menu_lists_windows_items_with_properties_and_the_drives() {
        let state = HeadMenuState {
            collapsed: true,
            network_drives: vec![NetworkDrive {
                label: "Backup (S:)".to_owned(),
                uri: "smb://fileserver/backup".to_owned(),
            }],
            system_information: true,
        };

        let entries = head_menu(Section::ThisPc, "ox:pc", &state);

        assert_eq!(
            labels(&entries),
            [
                "Expand",
                "Open in new tab",
                "Open in new window",
                "Add a network location",
                "Map network drive…",
                "Disconnect network drive",
                "Properties",
            ]
        );
        let item = |label: &str| {
            entries
                .iter()
                .find_map(|entry| match entry {
                    MenuEntry::Item(item) if item.label == label => Some(item.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label}"))
        };
        assert_eq!(item("Expand").target, Some("thisPc".to_variant()));
        assert_eq!(item("Disconnect network drive").submenu.len(), 1);
        assert_eq!(
            item("Properties").action,
            crate::window::menu_popover::MenuAction::Window(WindowAction::SystemProperties)
        );
    }

    /// Network's menu has no Add a network location, as in Windows, and no
    /// Properties, whose Windows twin the app does not have.
    ///
    /// parity: SIDE-033, NET-001
    #[test]
    fn network_menu_has_no_add_a_network_location_or_properties() {
        let state = HeadMenuState {
            system_information: true,
            ..HeadMenuState::default()
        };

        let entries = labels(&head_menu(Section::Network, "network:///", &state));

        assert_eq!(
            entries,
            [
                "Collapse",
                "Open in new tab",
                "Open in new window",
                "Map network drive…",
                "Disconnect network drive",
            ]
        );
    }

    #[test]
    fn a_pin_that_cannot_be_indexed_has_no_cache_entry() {
        let entries = labels(&pin_menu("mtp://phone/", None, false));

        assert!(!entries.contains(&"Cache this folder for search".to_owned()));
    }
}
