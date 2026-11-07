// SPDX-License-Identifier: AGPL-3.0-only
//! The place menus' entries for drives, devices and network places.

use super::*;
use crate::test_support::{studio_nas_mapped_drive, studio_nas_server};
use crate::window::menu_popover::{ItemAvailability, ItemCheck};

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

/// parity: SIDE-020
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

/// parity: HOME-005, OPEN-016
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
            "Cache this folder for search",
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
        can_format: true,
    };
    let drive = PlaceMenu::Drive {
        uri: "file:///media/demo/USB".into(),
        kind: VolumeKind::Drive,
        controls: usb_disk,
    };
    // Without GNOME Disks the drive is still formatted in the app's own
    // dialog.
    let without_disks = PlaceMenu::Drive {
        uri: "file:///media/demo/USB".into(),
        kind: VolumeKind::Drive,
        controls: MountControls {
            can_open_in_disks: false,
            ..usb_disk
        },
    };
    let local_disk = PlaceMenu::Drive {
        uri: "file:///".into(),
        kind: VolumeKind::Drive,
        controls: MountControls::FIXED,
    };
    let volume = PlaceMenu::Volume {
        id: "1234-ABCD".into(),
    };
    // The classic menu ("Show more options") lists every item.
    assert_eq!(
        labels(&drive),
        [
            "Open",
            "Open in new tab",
            "Open in new window",
            "Pin to Quick access",
            "Open in Terminal",
            "Cache this folder for search",
            "-",
            "Format…",
            "Open in Disks",
            "Disconnect",
            "Eject",
            "Safely remove",
            "-",
            "Copy",
            "Rename",
            "-",
            "Properties"
        ]
    );
    assert_eq!(
        labels(&without_disks)[7..],
        [
            "Format…",
            "Disconnect",
            "Eject",
            "Safely remove",
            "-",
            "Copy",
            "Rename",
            "-",
            "Properties"
        ]
    );
    assert_eq!(
        labels(&local_disk),
        [
            "Open",
            "Open in new tab",
            "Open in new window",
            "Pin to Quick access",
            "Open in Terminal",
            "Cache this folder for search",
            "-",
            "Copy",
            "Rename",
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
    let menu = PlaceMenu::Network(browsed);
    let shown = labels(&menu);
    assert!(shown.contains(&"Keep in Network".to_owned()), "{shown:?}");
    assert!(shown.contains(&"Disconnect".to_owned()), "{shown:?}");
    assert!(!is_disabled(&menu, "Open in Terminal"));
}

/// A drive and a share offer to cache their folder after the Open
/// items and Open in Terminal, checked while it is cached; a server gets
/// no offer.
///
/// parity: SRCH-020
#[test]
fn drives_and_shares_offer_to_cache_their_folder() {
    let drive = PlaceMenu::Drive {
        uri: "file:///media/demo/USB".into(),
        kind: VolumeKind::Drive,
        controls: MountControls::FIXED,
    };
    let share = PlaceMenu::Network(studio_nas_mapped_drive());
    let saved = PlaceMenu::SavedShare {
        uri: "smb://nas/media".into(),
    };
    let cache = "Cache this folder for search";

    assert_eq!(labels(&drive)[5], cache);
    assert_eq!(labels(&share)[4], cache);
    assert_eq!(labels(&saved)[2], cache);
    let checked = share
        .entries(Some(Caching::Enabled))
        .into_iter()
        .any(|entry| matches!(entry, MenuEntry::Item(item) if item.check == ItemCheck::Fixed(true)));
    assert!(checked, "a cached share's item is checked");
    assert!(!labels(&PlaceMenu::Network(studio_nas_server())).contains(&cache.to_owned()));
}

/// The compact menu of a drive is Windows 11's (checked on Windows 11,
/// 2026-11-07): Copy and Rename in the strip, greyed out, then Open, Open
/// in new tab, Open in new window, Format… and Eject where the drive
/// allows them, Pin to Quick access (Unpin while pinned) and Properties;
/// Open in Terminal and the cache item wait in the classic menu.
///
/// parity: HOME-003, SIDE-017, DEV-012
#[test]
fn a_drive_menu_follows_windows_11_in_the_compact_style() {
    let usb = PlaceMenu::Drive {
        uri: "file:///media/demo/USB".into(),
        kind: VolumeKind::Drive,
        controls: MountControls {
            can_unmount: false,
            can_eject: true,
            can_stop: false,
            can_open_in_disks: true,
            can_format: true,
        },
    };
    let local_disk = PlaceMenu::Drive {
        uri: "file:///".into(),
        kind: VolumeKind::Drive,
        controls: MountControls::FIXED,
    };
    let names = |entries: &[MenuEntry]| -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.label.clone(),
                MenuEntry::Divider => "-".to_owned(),
            })
            .collect()
    };
    let facts = PlaceFacts {
        caching: Some(Caching::Disabled),
        pinned: false,
    };

    let (entries, strip) = usb.styled(facts, MenuStyle::Compact);
    assert_eq!(
        names(&entries),
        [
            "Open",
            "Open in new tab",
            "Open in new window",
            "Format…",
            "Eject",
            "Pin to Quick access",
            "Properties"
        ]
    );
    let strip: Vec<(String, bool)> = strip
        .into_iter()
        .map(|item| (item.label, item.availability == ItemAvailability::Disabled))
        .collect();
    assert_eq!(
        strip,
        [("Copy".to_owned(), false), ("Rename".to_owned(), true)],
        "Copy copies the drive for a paste to make a shortcut to it; Rename is greyed out"
    );
    let pinned = PlaceFacts {
        pinned: true,
        ..facts
    };
    let (entries, _) = local_disk.styled(pinned, MenuStyle::Compact);
    assert_eq!(
        names(&entries),
        [
            "Open",
            "Open in new tab",
            "Open in new window",
            "Unpin from Quick access",
            "Properties"
        ]
    );
    let finished = finish_menu(entries, MenuStyle::Compact, WindowAction::PlaceMoreOptions);
    assert_eq!(names(&finished)[5..], ["-", "Show more options"]);
}
