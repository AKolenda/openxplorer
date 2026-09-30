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

use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{is_smb_location, is_smb_server};
use ox_core::places::{NetworkKind, NetworkLocation};
use ox_core::search::Caching;

use crate::devices::Removal;
use crate::icons::Icon;
use crate::volumes::{MountControls, VolumeKind};

use super::cache_folder::cache_item;
use super::menu_popover::{ItemAvailability, MenuEntry, MenuItem, MenuPopover};
use super::window_action::WindowAction;

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

/// `driveMenu`: Open, Open in new window and the cache entry, then what
/// the drive allows and Properties.
fn drive_entries(
    uri: &str,
    kind: VolumeKind,
    controls: MountControls,
    caching: Option<Caching>,
) -> Vec<MenuEntry> {
    let mut entries = vec![
        item("Open", Icon::HardDrive, WindowAction::GoTo, uri),
        item("Open in new window", Icon::Add, WindowAction::OpenWindow, uri),
    ];
    if let Some(caching) = caching {
        entries.push(cache_item(uri, caching).into());
    }
    let removals = removal_items(uri, kind, controls);
    if !removals.is_empty() {
        entries.push(MenuEntry::Divider);
        entries.extend(removals);
    }
    entries.push(MenuEntry::Divider);
    entries.push(item("Properties", Icon::Info, WindowAction::PropertiesOf, uri));
    entries
}

/// `terminalMenuItem`: Open in Terminal, disabled on a server's share list,
/// which has no folder to open a terminal in.
fn terminal_item(uri: &str) -> MenuEntry {
    let item = MenuItem::with_text_target(
        "Open in Terminal",
        Icon::WindowConsole,
        WindowAction::OpenInTerminalOf,
        uri,
    );
    let availability = if is_smb_server(uri) {
        ItemAvailability::Disabled
    } else {
        ItemAvailability::FollowsAction
    };
    MenuItem { availability, ..item }.into()
}

/// `networkLocationMenu`: opening, keeping or removing a share, Sign out
/// for SMB and Properties for a mount.
fn network_entries(location: &NetworkLocation) -> Vec<MenuEntry> {
    let uri = location.uri.as_str();
    let mut entries = vec![
        item("Open", Icon::Folder, WindowAction::GoTo, uri),
        item("Open in new tab", Icon::Add, WindowAction::OpenTab, uri),
        item("Open in new window", Icon::Share, WindowAction::OpenWindow, uri),
        terminal_item(uri),
    ];
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
    if location.kind == NetworkKind::Mount {
        entries.push(item("Properties", Icon::Info, WindowAction::PropertiesOf, uri));
    }
    entries
}

impl PlaceMenu {
    /// The menu's lines, without a search cache entry.
    pub(super) fn entries(&self) -> Vec<MenuEntry> {
        self.entries_with_caching(None)
    }

    /// The menu's lines; a drive's has the search cache entry `caching`
    /// describes, none for `None`.
    pub(super) fn entries_with_caching(&self, caching: Option<Caching>) -> Vec<MenuEntry> {
        match self {
            PlaceMenu::Drive { uri, kind, controls } => drive_entries(uri, *kind, *controls, caching),
            PlaceMenu::Volume { id } => vec![item(
                "Mount volume",
                Icon::HardDrive,
                WindowAction::MountVolume,
                id,
            )],
            PlaceMenu::DriveCard { uri, kind, controls } => removal_items(uri, *kind, *controls),
            PlaceMenu::Network(location) => network_entries(location),
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
    let entries = menu.entries();
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

    /// The labels of `menu`, a divider as `-`.
    fn labels(menu: &PlaceMenu) -> Vec<String> {
        let entries = menu.entries();
        let label = |entry: &MenuEntry| match entry {
            MenuEntry::Item(item) => item.label.clone(),
            MenuEntry::Divider => "-".to_owned(),
        };
        entries.iter().map(label).collect()
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
        assert_eq!(saved_menu[4..], ["Remove saved location", "Sign out of server…"]);
        assert_eq!(browsed_menu[4..], ["Keep in Network", "Sign out of server…"]);
        assert_eq!(
            server_menu[4..],
            ["Sign out of server…"],
            "a server is never saved"
        );
    }

    /// A server has no folder to open a terminal in, and a local CIFS
    /// mount offers Properties (`networkLocationMenu`).
    ///
    /// parity: SIDE-020
    #[test]
    fn a_server_cannot_open_a_terminal_and_a_mount_has_properties() {
        let availability_of_terminal = |menu: &PlaceMenu| {
            menu.entries().into_iter().find_map(|entry| match entry {
                MenuEntry::Item(item) if item.label == "Open in Terminal" => Some(item.availability),
                _ => None,
            })
        };
        let server = PlaceMenu::Network(studio_nas_server());
        let share = PlaceMenu::Network(studio_nas_mapped_drive());
        let mount = PlaceMenu::Network(NetworkLocation {
            uri: "file:///mnt/nas".into(),
            label: "nas".into(),
            is_saved: false,
            is_connected: true,
            kind: NetworkKind::Mount,
        });
        assert_eq!(
            availability_of_terminal(&server),
            Some(ItemAvailability::Disabled)
        );
        assert_eq!(
            availability_of_terminal(&share),
            Some(ItemAvailability::FollowsAction)
        );
        assert_eq!(labels(&mount).last().map(String::as_str), Some("Properties"));
        assert!(!labels(&share).contains(&"Properties".to_owned()));
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

    /// parity: SIDE-017, DEV-003, DEV-007, DEV-008
    #[test]
    fn a_drive_opens_and_offers_what_it_allows_and_a_volume_mounts() {
        let usb_disk = MountControls {
            can_unmount: true,
            can_eject: true,
            can_stop: true,
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
                "-",
                "Disconnect mount",
                "Eject",
                "Safely remove",
                "-",
                "Properties"
            ]
        );
        assert_eq!(
            labels(&local_disk),
            ["Open", "Open in new window", "-", "Properties"]
        );
        assert_eq!(labels(&volume), ["Mount volume"]);
        let cached: Vec<String> = local_disk
            .entries_with_caching(Some(Caching::Enabled))
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect();
        assert_eq!(
            cached,
            [
                "Open",
                "Open in new window",
                "Cache this folder for search",
                "-",
                "Properties"
            ],
            "the cache entry follows Open in new window"
        );
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
