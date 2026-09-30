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
use ox_core::location::{is_remote_location, is_smb_location, is_smb_server};
use ox_core::places::{NetworkKind, NetworkLocation};
use ox_core::search::Caching;

use crate::devices::Removal;
use crate::icons::Icon;
use crate::volumes::{MountControls, VolumeKind};

use super::cache_folder::cache_item;
use super::menu_popover::{MenuEntry, MenuItem, MenuPopover};
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

/// A drive in the sidebar: its root, what it is and allows, and whether it
/// is cached for search.
#[derive(Debug, Clone, Copy)]
struct DriveMenuParts<'a> {
    uri: &'a str,
    kind: VolumeKind,
    controls: MountControls,
    /// `None` where the drive cannot be cached, such as a phone.
    caching: Option<Caching>,
}

/// `driveMenu`: Open, Open in new window and the search cache entry, then
/// what the drive allows (Dolphin's removals), then Properties.
fn drive_entries(drive: DriveMenuParts<'_>) -> Vec<MenuEntry> {
    let uri = drive.uri;
    let mut entries = vec![
        item("Open", Icon::HardDrive, WindowAction::GoTo, uri),
        item("Open in new window", Icon::Add, WindowAction::OpenWindow, uri),
    ];
    if let Some(caching) = drive.caching {
        entries.push(cache_item(uri, caching).into());
    }
    entries.push(MenuEntry::Divider);
    let removals = removal_items(uri, drive.kind, drive.controls);
    if !removals.is_empty() {
        entries.extend(removals);
        entries.push(MenuEntry::Divider);
    }
    entries.push(item("Properties", Icon::Info, WindowAction::PropertiesOf, uri));
    entries
}

/// "Open in Terminal" for a network location, disabled for a server,
/// which has no folder to open (`terminalMenuItem` refuses
/// `isSmbServer`).
fn network_terminal_item(uri: &str) -> MenuEntry {
    let terminal = MenuItem::with_text_target(
        "Open in Terminal",
        Icon::WindowConsole,
        WindowAction::OpenInTerminalOf,
        uri,
    );
    terminal.disabled_when(is_smb_server(uri)).into()
}

/// `networkLocationMenu`: opening, keeping or removing a share, Sign out
/// for SMB, and Properties for a mount.
fn network_entries(location: &NetworkLocation) -> Vec<MenuEntry> {
    let uri = location.uri.as_str();
    let mut entries = vec![
        item("Open", Icon::Folder, WindowAction::GoTo, uri),
        item("Open in new tab", Icon::Add, WindowAction::OpenTab, uri),
        item("Open in new window", Icon::Share, WindowAction::OpenWindow, uri),
        network_terminal_item(uri),
    ];
    let is_smb = is_smb_location(uri);
    let is_remote = is_remote_location(uri);
    let is_share = (is_smb || is_remote) && !is_smb_server(uri) && location.kind != NetworkKind::Server;
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
    // SFTP, FTP, WebDAV and NFS keep no OpenXplorer credentials to forget:
    // their connection is ended as a mount, as Dolphin and Files do.
    if is_remote && location.is_connected {
        entries.push(item(
            "Disconnect",
            Icon::ArrowEject,
            WindowAction::Disconnect,
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
        self.entries_with_cache(None)
    }

    /// The menu's lines. A drive's menu has the search cache entry
    /// `caching` describes (`cacheMenuItems`), none for `None`.
    pub(super) fn entries_with_cache(&self, caching: Option<Caching>) -> Vec<MenuEntry> {
        match self {
            PlaceMenu::Drive { uri, kind, controls } => drive_entries(DriveMenuParts {
                uri,
                kind: *kind,
                controls: *controls,
                caching,
            }),
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
    use crate::window::menu_popover::{ItemAvailability, ItemCheck};

    /// The labels of `entries`, a divider as `-`.
    fn labels_of(entries: &[MenuEntry]) -> Vec<String> {
        let label = |entry: &MenuEntry| match entry {
            MenuEntry::Item(item) => item.label.clone(),
            MenuEntry::Divider => "-".to_owned(),
        };
        entries.iter().map(label).collect()
    }

    /// The labels of `menu`, a divider as `-`.
    fn labels(menu: &PlaceMenu) -> Vec<String> {
        labels_of(&menu.entries())
    }

    /// The item of `menu` labelled `label`.
    fn item_labelled(menu: &PlaceMenu, label: &str) -> MenuItem {
        let entries = menu.entries();
        let items = entries.into_iter().filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Divider => None,
        });
        let mut matching = items.filter(|item| item.label == label);
        matching.next().unwrap_or_else(|| panic!("the menu has {label}"))
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

    /// A server has no folder to open a terminal in; a share has, and a
    /// stable mount also has Properties (`networkLocationMenu`).
    ///
    /// parity: SIDE-020
    #[test]
    fn a_network_terminal_needs_a_folder_and_a_mount_has_properties() {
        let server = PlaceMenu::Network(studio_nas_server());
        let share = PlaceMenu::Network(studio_nas_mapped_drive());
        let mount = PlaceMenu::Network(NetworkLocation {
            kind: NetworkKind::Mount,
            ..studio_nas_mapped_drive()
        });

        let server_terminal = item_labelled(&server, "Open in Terminal");
        let share_terminal = item_labelled(&share, "Open in Terminal");
        let mount_properties = item_labelled(&mount, "Properties");

        assert_eq!(server_terminal.availability, ItemAvailability::Disabled);
        assert_eq!(share_terminal.availability, ItemAvailability::FollowsAction);
        assert_eq!(share_terminal.action, WindowAction::OpenInTerminalOf.into());
        assert_eq!(mount_properties.action, WindowAction::PropertiesOf.into());
        assert!(!labels(&share).contains(&"Properties".to_owned()));
    }

    /// parity: NET-030
    #[test]
    fn a_connected_sftp_folder_can_be_kept_and_disconnected() {
        let mount = ox_core::places::NetworkMount {
            uri: "sftp://anna@build/".into(),
            label: "build".into(),
            is_mounted: true,
        };
        let visited = ox_core::settings::Bookmark {
            uri: "sftp://anna@build/home/anna".into(),
            label: String::new(),
        };
        let rows = ox_core::places::merge_network_locations(&[], &[mount], &[], &[visited]);
        let [browsed] = <[NetworkLocation; 1]>::try_from(rows).expect("one row for the server");
        let menu = labels(&PlaceMenu::Network(browsed));
        assert_eq!(menu[4..], ["Keep in Network", "Disconnect"]);
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
            labels_of(&drive.entries_with_cache(Some(Caching::Disabled))),
            [
                "Open",
                "Open in new window",
                "Cache this folder for search",
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
    }

    /// The cache entry and Properties act on the drive, and the entry is
    /// checked while the drive is cached.
    ///
    /// parity: SIDE-017
    #[test]
    fn a_drive_menu_caches_and_shows_properties_of_the_drive() {
        let uri = "file:///media/demo/USB";
        let drive = PlaceMenu::Drive {
            uri: uri.into(),
            kind: VolumeKind::Drive,
            controls: MountControls::FIXED,
        };

        let entries = drive.entries_with_cache(Some(Caching::Enabled));

        let items: Vec<&MenuItem> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) => Some(item),
                MenuEntry::Divider => None,
            })
            .collect();
        let cache = items
            .iter()
            .find(|item| item.label == "Cache this folder for search")
            .expect("a drive can be cached");
        assert_eq!(cache.check, ItemCheck::Fixed(true));
        for item in &items {
            assert_eq!(item.target, Some(uri.to_variant()), "{}", item.label);
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
