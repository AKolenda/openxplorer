// SPDX-License-Identifier: AGPL-3.0-only
//! The place menus' entries for drives, devices and network places.

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
