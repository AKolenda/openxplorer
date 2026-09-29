// SPDX-License-Identifier: AGPL-3.0-only
//! The context menus of places: the drives and network locations of the
//! sidebar, This PC and the Network page.
//!
//! Ports `driveMenu`, `networkLocationMenu` and the menus of This PC's
//! cards (`shares()` and the Devices and drives cards of `renderLanding`)
//! in `desktop/ui/app.js`, with Dolphin's Eject and Safely remove added
//! where a drive allows them. A menu is data ([`PlaceMenu`]); its items
//! run window actions with the place as their target, and the menu opens
//! where the pointer is ([`popup_place_menu`]).
//!
//! Drives and network places also offer Open in Terminal
//! (`terminalMenuItem`, disabled where no terminal can open: phones,
//! cameras and a server's share list), "Cache this folder for search"
//! where the folder can be indexed (`cacheMenuItems`) and Properties. The
//! Python app had the Terminal item on network places only, and the cache
//! item on drives only. A removable drive also offers Open in Disks and
//! Format… where GNOME Disks is installed.

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{is_device_location, is_smb_location, is_smb_server};
use ox_core::places::{NetworkKind, NetworkLocation};
use ox_core::search::Caching;

use crate::devices::Removal;
use crate::icons::Icon;
use crate::volumes::{MountControls, VolumeKind};

use super::cache_folder::cache_item;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;
use super::BrowserWindow;

/// What a place's menu is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PlaceMenu {
    /// A mounted drive or device in the sidebar, Local Disk included
    /// (`driveMenu`).
    Drive {
        /// Its root.
        uri: String,
        /// A drive or a phone.
        kind: VolumeKind,
        /// What the mount allows.
        controls: MountControls,
    },
    /// A volume still to be mounted (`driveMenu` with a volume).
    Volume {
        /// The volume's id for the mount request.
        id: String,
    },
    /// A mounted drive or device's card on This PC.
    DriveCard {
        /// Its root.
        uri: String,
        /// A drive or a phone.
        kind: VolumeKind,
        /// What the mount allows.
        controls: MountControls,
    },
    /// A network location in the sidebar or on the Network page
    /// (`networkLocationMenu`).
    Network(NetworkLocation),
    /// A saved share's card on This PC (`shares()`).
    SavedShare {
        /// The saved share.
        uri: String,
    },
}

/// An item that runs `action` with the place `target`.
fn item(label: &str, glyph: Icon, action: WindowAction, target: &str) -> MenuEntry {
    let item = MenuItem {
        target: Some(target.to_variant()),
        ..MenuItem::new(label, glyph, action)
    };
    MenuEntry::Item(item)
}

/// The window action of `removal`.
pub(super) fn removal_action(removal: Removal) -> WindowAction {
    match removal {
        Removal::Disconnect => WindowAction::Disconnect,
        Removal::Eject => WindowAction::Eject,
        Removal::SafelyRemove => WindowAction::SafelyRemove,
    }
}

/// Disconnect, Eject and Safely remove, as `controls` allow them.
fn removal_items(uri: &str, kind: VolumeKind, controls: MountControls) -> Vec<MenuEntry> {
    let removals = Removal::offered(controls).into_iter();
    let items = removals.map(|removal| {
        item(
            removal.label(kind),
            Icon::ArrowEject,
            removal_action(removal),
            uri,
        )
    });
    items.collect()
}

/// Open in Disks and Format… for the removable drive at `uri`, which
/// GNOME Disks handles (DEV-012), as Dolphin offers its partition manager.
fn disks_items(uri: &str) -> [MenuEntry; 2] {
    [
        item("Open in Disks", Icon::Settings, WindowAction::OpenInDisks, uri),
        item("Format…", Icon::HardDrive, WindowAction::FormatDrive, uri),
    ]
}

/// Open in Terminal for the place at `uri`, disabled where no terminal
/// can open there (`terminalMenuItem`): anywhere but a local folder or an
/// SMB share.
fn terminal_item(uri: &str) -> MenuEntry {
    let can_open = uri.starts_with("file:") || (is_smb_location(uri) && !is_smb_server(uri));
    let terminal = MenuItem::with_text_target(
        "Open in Terminal",
        Icon::WindowConsole,
        WindowAction::OpenInTerminalOf,
        uri,
    );
    terminal.disabled_when(!can_open).into()
}

/// "Cache this folder for search" for the place at `uri`, where it can be
/// indexed.
fn cache_entries(uri: &str, caching: Option<Caching>) -> Vec<MenuEntry> {
    caching
        .map(|caching| cache_item(uri, caching).into())
        .into_iter()
        .collect()
}

/// `driveMenu`: Open, Open in new window, Open in Terminal and the cache
/// item, then what the drive allows, then Properties.
fn drive_entries(
    uri: &str,
    kind: VolumeKind,
    controls: MountControls,
    caching: Option<Caching>,
) -> Vec<MenuEntry> {
    let mut entries = vec![
        item("Open", Icon::HardDrive, WindowAction::GoTo, uri),
        item("Open in new window", Icon::Add, WindowAction::OpenWindow, uri),
        terminal_item(uri),
    ];
    if !is_device_location(uri) {
        entries.extend(cache_entries(uri, caching));
    }
    let mut drive_tools = removal_items(uri, kind, controls);
    if controls.can_open_in_disks {
        drive_tools.extend(disks_items(uri));
    }
    if !drive_tools.is_empty() {
        entries.push(MenuEntry::Divider);
        entries.extend(drive_tools);
    }
    entries.push(MenuEntry::Divider);
    entries.push(item("Properties", Icon::Info, WindowAction::PropertiesOf, uri));
    entries
}

/// `networkLocationMenu`: opening, Open in Terminal, the cache item,
/// keeping or removing a share, Sign out for SMB, and Properties of a
/// mount or a connected share.
fn network_entries(location: &NetworkLocation, caching: Option<Caching>) -> Vec<MenuEntry> {
    let uri = location.uri.as_str();
    let mut entries = vec![
        item("Open", Icon::Folder, WindowAction::GoTo, uri),
        item("Open in new tab", Icon::Add, WindowAction::OpenTab, uri),
        item("Open in new window", Icon::Share, WindowAction::OpenWindow, uri),
        terminal_item(uri),
    ];
    entries.extend(cache_entries(uri, caching));
    let is_smb = is_smb_location(uri);
    let is_share = is_smb && !is_smb_server(uri) && location.kind != NetworkKind::Server;
    if is_share && location.is_saved {
        entries.push(item(
            "Remove saved location",
            Icon::Pin,
            WindowAction::RemoveSavedLocation,
            uri,
        ));
    } else if is_share {
        entries.push(item(
            "Keep in Network",
            Icon::Pin,
            WindowAction::KeepInNetwork,
            uri,
        ));
    }
    if is_smb {
        entries.push(item(
            "Sign out of server…",
            Icon::ArrowEject,
            WindowAction::SignOut,
            uri,
        ));
    }
    let is_readable = location.kind == NetworkKind::Mount || (is_share && location.is_connected);
    if is_readable {
        entries.push(item("Properties", Icon::Info, WindowAction::PropertiesOf, uri));
    }
    entries
}

impl PlaceMenu {
    /// The location the menu is about, which may be cached for search.
    fn location(&self) -> Option<&str> {
        match self {
            PlaceMenu::Drive { uri, .. }
            | PlaceMenu::DriveCard { uri, .. }
            | PlaceMenu::SavedShare { uri } => Some(uri),
            PlaceMenu::Network(location) => Some(&location.uri),
            PlaceMenu::Volume { .. } => None,
        }
    }

    /// The menu's lines, with the cache item as `caching` says: `None`
    /// where the place cannot be indexed.
    pub(super) fn entries(&self, caching: Option<Caching>) -> Vec<MenuEntry> {
        match self {
            PlaceMenu::Drive { uri, kind, controls } => drive_entries(uri, *kind, *controls, caching),
            PlaceMenu::Volume { id } => vec![item(
                "Mount volume",
                Icon::HardDrive,
                WindowAction::MountVolume,
                id,
            )],
            PlaceMenu::DriveCard { uri, kind, controls } => removal_items(uri, *kind, *controls),
            PlaceMenu::Network(location) => network_entries(location, caching),
            PlaceMenu::SavedShare { uri } => vec![
                item("Open", Icon::Folder, WindowAction::GoTo, uri),
                item("Open in new tab", Icon::Add, WindowAction::OpenTab, uri),
                item(
                    "Remove saved location",
                    Icon::Pin,
                    WindowAction::RemoveSavedLocation,
                    uri,
                ),
                item(
                    "Sign out of server…",
                    Icon::ArrowEject,
                    WindowAction::SignOut,
                    uri,
                ),
            ],
        }
    }
}

/// Opens `menu` at `x`, `y` in `anchor`, as `openMenu` does at the
/// pointer. A menu without items opens nothing, as a This PC card that
/// cannot be removed has no menu.
pub(super) fn popup_place_menu(
    menu: &PlaceMenu,
    anchor: &impl IsA<gtk::Widget>,
    x: f64,
    y: f64,
) -> Option<MenuPopover> {
    let entries = menu.entries(caching_in(menu, anchor));
    if entries.is_empty() {
        return None;
    }
    let popover = MenuPopover::new(entries);
    popover.set_parent(anchor);
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let point = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
    popover.set_pointing_to(Some(&point));
    // A menu belongs to one right-click; it lets go of its anchor once it
    // closes and its item has run.
    popover.connect_closed(|popover| {
        let closed = popover.clone();
        glib::idle_add_local_once(move || closed.unparent());
    });
    popover.popup();
    Some(popover)
}

/// Whether the place of `menu` is cached for search, as the window of
/// `widget` knows it.
pub(super) fn caching_in(menu: &PlaceMenu, widget: &impl IsA<gtk::Widget>) -> Option<Caching> {
    let window = widget.root().and_downcast::<BrowserWindow>()?;
    window.caching_of(menu.location()?)
}

/// Gives `card` the context menu `menu` on a right-click.
pub(super) fn attach_place_menu(card: &impl IsA<gtk::Widget>, menu: PlaceMenu) {
    let right_click = gtk::GestureClick::new();
    right_click.set_button(gdk::BUTTON_SECONDARY);
    right_click.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(card) = gesture.widget() {
            popup_place_menu(&menu, &card, x, y);
        }
    });
    card.add_controller(right_click);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{studio_nas_mapped_drive, studio_nas_server};
    use crate::window::menu_popover::ItemAvailability;

    /// The labels of `menu`, a divider as `-`, for a place that is not
    /// cached for search.
    fn labels(menu: &PlaceMenu) -> Vec<String> {
        let caching = menu
            .location()
            .filter(|uri| !is_smb_server(uri))
            .map(|_| Caching::Disabled);
        let entries = menu.entries(caching);
        let label = |entry: &MenuEntry| match entry {
            MenuEntry::Item(item) => item.label.clone(),
            MenuEntry::Divider => "-".to_owned(),
        };
        entries.iter().map(label).collect()
    }

    /// Whether the item labelled `label` of `menu` is disabled in it.
    fn is_disabled(menu: &PlaceMenu, label: &str) -> bool {
        let entries = menu.entries(None);
        let item = entries.iter().find_map(|entry| match entry {
            MenuEntry::Item(item) if item.label == label => Some(item),
            _ => None,
        });
        let item = item.unwrap_or_else(|| panic!("the menu has {label}"));
        item.availability == ItemAvailability::Disabled
    }

    /// parity: SIDE-020, HOME-010
    #[test]
    fn a_network_share_can_be_kept_removed_and_signed_out_of() {
        let saved = studio_nas_mapped_drive();
        let browsed = NetworkLocation {
            is_saved: false,
            ..saved.clone()
        };
        let common = [
            "Open",
            "Open in new tab",
            "Open in new window",
            "Open in Terminal",
        ];
        let saved_menu = labels(&PlaceMenu::Network(saved));
        let browsed_menu = labels(&PlaceMenu::Network(browsed));
        let server_menu = labels(&PlaceMenu::Network(studio_nas_server()));
        assert_eq!(saved_menu[..4], common);
        assert_eq!(
            saved_menu[4..],
            [
                "Cache this folder for search",
                "Remove saved location",
                "Sign out of server…"
            ]
        );
        assert_eq!(browsed_menu[5..], ["Keep in Network", "Sign out of server…"]);
        assert_eq!(
            server_menu[4..],
            ["Sign out of server…"],
            "a server is never saved"
        );
    }

    /// Network places offer Open in Terminal (not on a server's share
    /// list), the cache item where they can be indexed, and Properties of
    /// a mount or a connected share, beside `networkLocationMenu`'s items.
    ///
    /// parity: PROP-001
    #[test]
    fn network_places_open_a_terminal_and_properties_where_they_can() {
        let connected = NetworkLocation {
            is_connected: true,
            ..studio_nas_mapped_drive()
        };
        let mount = NetworkLocation {
            uri: "file:///mnt/nas".into(),
            label: "nas".into(),
            is_saved: false,
            is_connected: true,
            kind: NetworkKind::Mount,
        };
        let server = PlaceMenu::Network(studio_nas_server());

        assert_eq!(
            labels(&PlaceMenu::Network(connected)).last().map(String::as_str),
            Some("Properties")
        );
        assert_eq!(
            labels(&PlaceMenu::Network(mount.clone())),
            [
                "Open",
                "Open in new tab",
                "Open in new window",
                "Open in Terminal",
                "Cache this folder for search",
                "Properties"
            ]
        );
        assert!(!labels(&PlaceMenu::Network(studio_nas_mapped_drive())).contains(&"Properties".to_owned()));
        assert!(is_disabled(&server, "Open in Terminal"));
        assert!(!is_disabled(&PlaceMenu::Network(mount), "Open in Terminal"));
    }

    /// parity: HOME-005
    #[test]
    fn a_saved_share_card_opens_removes_and_signs_out() {
        let menu = PlaceMenu::SavedShare {
            uri: "smb://nas/media".into(),
        };
        assert_eq!(
            labels(&menu),
            [
                "Open",
                "Open in new tab",
                "Remove saved location",
                "Sign out of server…"
            ]
        );
    }

    /// parity: SIDE-017, DEV-003, DEV-007, DEV-008, DEV-012
    #[test]
    fn a_drive_opens_and_offers_what_it_allows_and_a_volume_mounts() {
        let usb_disk = MountControls {
            can_unmount: true,
            can_eject: true,
            can_stop: true,
            can_open_in_disks: true,
        };
        let drive = PlaceMenu::Drive {
            uri: "file:///media/demo/USB".into(),
            kind: VolumeKind::Drive,
            controls: usb_disk,
        };
        let local_disk = PlaceMenu::Drive {
            uri: "file:///".into(),
            kind: VolumeKind::Drive,
            controls: MountControls::FIXED,
        };
        let volume = PlaceMenu::Volume {
            id: "1234-ABCD".into(),
        };
        assert_eq!(
            labels(&drive),
            [
                "Open",
                "Open in new window",
                "Open in Terminal",
                "Cache this folder for search",
                "-",
                "Disconnect mount",
                "Eject",
                "Safely remove",
                "Open in Disks",
                "Format…",
                "-",
                "Properties"
            ]
        );
        assert_eq!(
            labels(&local_disk),
            [
                "Open",
                "Open in new window",
                "Open in Terminal",
                "Cache this folder for search",
                "-",
                "Properties"
            ]
        );
        assert_eq!(labels(&volume), ["Mount volume"]);
    }

    /// A phone's menu has no cache item and its Open in Terminal is
    /// disabled; every drive item acts on the drive (`driveMenu`).
    ///
    /// parity: PROP-001, SRCH-021
    #[test]
    fn a_phone_is_not_cached_or_opened_in_a_terminal_and_drive_items_target_the_drive() {
        let uri = "mtp://%5Busb%3A001%2C010%5D/";
        let phone = PlaceMenu::Drive {
            uri: uri.into(),
            kind: VolumeKind::Device,
            controls: MountControls::UNMOUNTABLE,
        };

        let entries = phone.entries(Some(Caching::Disabled));

        assert!(!labels(&phone).contains(&"Cache this folder for search".to_owned()));
        assert!(is_disabled(&phone, "Open in Terminal"));
        for entry in &entries {
            if let MenuEntry::Item(item) = entry {
                assert_eq!(item.target, Some(uri.to_variant()), "{}", item.label);
            }
        }
    }

    /// This PC's cards offer Disconnect device or Disconnect mount, and no
    /// menu where the system keeps the mount (`canUnmount === false`).
    ///
    /// parity: DEV-006, HOME-003
    #[test]
    fn a_drive_card_offers_disconnect_only_where_the_mount_allows_it() {
        let phone = PlaceMenu::DriveCard {
            uri: "mtp://[usb:001,010]/".into(),
            kind: VolumeKind::Device,
            controls: MountControls::UNMOUNTABLE,
        };
        let fixed = PlaceMenu::DriveCard {
            uri: "file:///mnt/data".into(),
            kind: VolumeKind::Drive,
            controls: MountControls::FIXED,
        };
        assert_eq!(labels(&phone), ["Disconnect device"]);
        assert!(labels(&fixed).is_empty());
    }
}
