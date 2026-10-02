// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's rows as data, in the order of `renderSidebar` in
//! `v2.0.0:desktop/ui/app.js`: Home (the home folder), the Quick access folders
//! and pins, the searches saved to the sidebar (SRCH-038), This PC with
//! Local Disk and the drives and devices, and
//! Network with the merged network locations, then Recent files and the
//! Recycle Bin ([`recent_and_bin_entries`]). Mounted SMB shares appear
//! once, under Network.
//!
//! [`sidebar_entries`] turns composed [`Places`] into rows without GTK, so
//! the order is tested on its own. Drives and network locations carry
//! their context menu ([`PlaceMenu`]), and a drive that can be removed its
//! eject button (DEV-007).

use ox_core::location::{
    is_server_location, LocationContext, NETWORK_URI, PC_URI, RECENT_LOCATIONS_URI, RECENT_URI, TRASH_URI,
};
use ox_core::places::{NetworkLocation, Place};
use ox_core::search::SavedSearch;

use crate::devices::Removal;
use crate::icons::{Art, Icon, Storage, Tint};
use crate::locations::Page;
use crate::places::Places;
use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};
use crate::window::place_menus::PlaceMenu;

/// A group of rows; a separator is drawn where the group changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Section {
    /// The home folder.
    Home,
    /// Known folders and pins.
    QuickAccess,
    /// Searches saved to the sidebar, as Dolphin lists them among its
    /// places (SRCH-038).
    SavedSearches,
    /// This PC, Local Disk and the drives and devices.
    ThisPc,
    /// Network and the network locations.
    Network,
    /// Recent files and the Recycle Bin, as Dolphin's Places panel lists
    /// Recent Files and Trash (SIDE-025, SIDE-026).
    RecentAndBin,
}

impl Section {
    /// The key a hidden section is saved under, and its name in "Hide
    /// section"; `None` for Home, which cannot be hidden (SIDE-010).
    pub(in crate::window) fn hiding(self) -> Option<(&'static str, &'static str)> {
        match self {
            Section::Home => None,
            Section::QuickAccess => Some(("quickAccess", ox_core::i18n::gettext_static("Quick access"))),
            Section::SavedSearches => {
                Some(("savedSearches", ox_core::i18n::gettext_static("Saved searches")))
            }
            Section::ThisPc => Some(("thisPc", ox_core::i18n::gettext_static("This PC"))),
            Section::Network => Some(("network", ox_core::i18n::gettext_static("Network"))),
            Section::RecentAndBin => Some((
                "recent",
                ox_core::i18n::gettext_static("Recent files and Recycle Bin"),
            )),
        }
    }
}

/// How a row sits in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum RowLevel {
    /// A top-level place (Home, a Quick access folder).
    Place,
    /// A group head with an expander (This PC, Network).
    Group,
    /// A row inside a group, indented.
    Child,
}

/// What activating a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum RowTarget {
    /// Opens a location.
    Location(String),
    /// Mounts the volume with this identifier, then opens it.
    MountVolume(String),
    /// Nothing: the drop tail of an empty Quick access, which only takes
    /// dropped folders to pin (DND-014).
    PinDropTail,
    /// Opens the folder of a saved search and runs it again (SRCH-038).
    SavedSearch(SavedSearch),
}

/// One sidebar row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct SidebarEntry {
    /// The group the row belongs to.
    pub section: Section,
    /// A place, a group head or an indented row.
    pub level: RowLevel,
    /// The visible name, which is also the accessible name.
    pub label: String,
    /// The row's icon: a glyph, in its place's colour for Home, This PC,
    /// Network and the standard folders, or colour art.
    pub icon: Art,
    /// What activating the row does.
    pub target: RowTarget,
    /// Hover text and accessible description.
    pub tooltip: String,
    /// Shows the pin glyph of a Quick access row.
    pub pinned: bool,
    /// The row's context menu, for drives and network locations.
    pub menu: Option<PlaceMenu>,
    /// The eject button of a drive that can be removed.
    pub eject: Option<EjectButton>,
}

/// The eject button at the end of a removable drive's row, as GNOME's
/// places sidebar and Dolphin's show it (DEV-007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) struct EjectButton {
    /// Eject the medium, or unmount a drive that cannot be ejected.
    pub removal: Removal,
    /// The drive's root.
    pub uri: String,
    /// The tooltip and accessible name, such as "Eject".
    pub label: &'static str,
}

impl EjectButton {
    /// The button of the mounted drive at `uri`, or `None` when the drive
    /// can be neither ejected nor unmounted.
    fn for_drive(uri: &str, kind: VolumeKind, controls: MountControls) -> Option<Self> {
        let removal = Removal::for_eject_button(controls)?;
        Some(Self {
            removal,
            uri: uri.to_owned(),
            label: removal.label(kind),
        })
    }
}

/// The Quick access row of `place`.
pub(in crate::window) fn place_entry(place: &Place, locations: &LocationContext) -> SidebarEntry {
    let storage = if place.is_shared || is_server_location(&place.uri) {
        Storage::Network
    } else {
        Storage::Local
    };
    SidebarEntry {
        section: Section::QuickAccess,
        level: RowLevel::Place,
        label: place.label.clone(),
        icon: Art::for_quick_access(place.known_folder, storage),
        target: RowTarget::Location(place.uri.clone()),
        tooltip: locations.display_location(&place.uri),
        pinned: true,
        menu: None,
        eject: None,
    }
}

fn saved_search_entry(search: &SavedSearch, locations: &LocationContext) -> SidebarEntry {
    SidebarEntry {
        section: Section::SavedSearches,
        level: RowLevel::Place,
        label: search.label.clone(),
        icon: Art::Glyph(Icon::Search),
        target: RowTarget::SavedSearch(search.clone()),
        tooltip: locations.display_location(&search.folder),
        pinned: false,
        menu: None,
        eject: None,
    }
}

fn drive_entry(row: &VolumeRow, locations: &LocationContext) -> SidebarEntry {
    let glyph = match row.kind {
        VolumeKind::Device => Icon::Phone,
        VolumeKind::Drive => Icon::HardDrive,
    };
    let (target, tooltip) = match &row.state {
        VolumeState::Mounted { uri, .. } => {
            (RowTarget::Location(uri.clone()), locations.display_location(uri))
        }
        VolumeState::Mountable { id } => (RowTarget::MountVolume(id.clone()), row.label.clone()),
    };
    SidebarEntry {
        section: Section::ThisPc,
        level: RowLevel::Child,
        label: row.label.clone(),
        icon: Art::Glyph(glyph),
        target,
        tooltip,
        pinned: false,
        menu: Some(drive_menu(row)),
        eject: drive_eject_button(row),
    }
}

/// A drive's menu (`driveMenu`): a mounted drive's, or Mount volume.
fn drive_menu(row: &VolumeRow) -> PlaceMenu {
    match &row.state {
        VolumeState::Mounted { uri, controls } => PlaceMenu::Drive {
            uri: uri.clone(),
            kind: row.kind,
            controls: *controls,
        },
        VolumeState::Mountable { id } => PlaceMenu::Volume { id: id.clone() },
    }
}

/// The eject button of a mounted drive that can be removed.
fn drive_eject_button(row: &VolumeRow) -> Option<EjectButton> {
    let VolumeState::Mounted { uri, controls } = &row.state else {
        return None;
    };
    EjectButton::for_drive(uri, row.kind, *controls)
}

/// The state text of a network row, as `renderSidebar` titles it.
fn network_state(location: &NetworkLocation) -> &'static str {
    if location.is_connected {
        ox_core::i18n::gettext_static("Connected")
    } else if location.is_saved {
        ox_core::i18n::gettext_static("Saved · connect on open")
    } else {
        ox_core::i18n::gettext_static("Opened this session")
    }
}

fn network_entry(location: &NetworkLocation, locations: &LocationContext) -> SidebarEntry {
    let address = locations.display_location(&location.uri);
    SidebarEntry {
        section: Section::Network,
        level: RowLevel::Child,
        label: location.label.clone(),
        icon: Art::for_network_row(location),
        target: RowTarget::Location(location.uri.clone()),
        tooltip: format!("{address} · {}", network_state(location)),
        pinned: false,
        menu: Some(PlaceMenu::Network(location.clone())),
        eject: None,
    }
}

/// A top-level row with a coloured glyph: Home, This PC or Network, in
/// the colours of the `add(...)` calls in `renderSidebar`.
fn fixed_entry(section: Section, label: &str, icon: Art, uri: &str) -> SidebarEntry {
    let level = if matches!(section, Section::Home | Section::RecentAndBin) {
        RowLevel::Place
    } else {
        RowLevel::Group
    };
    SidebarEntry {
        section,
        level,
        label: label.to_owned(),
        icon,
        target: RowTarget::Location(uri.to_owned()),
        tooltip: label.to_owned(),
        pinned: false,
        menu: None,
        eject: None,
    }
}

fn local_disk_entry(locations: &LocationContext) -> SidebarEntry {
    let root = "file:///";
    SidebarEntry {
        section: Section::ThisPc,
        level: RowLevel::Child,
        label: ox_core::i18n::gettext("Local Disk"),
        icon: Art::Glyph(Icon::HardDrive),
        target: RowTarget::Location(root.to_owned()),
        tooltip: locations.display_location(root),
        pinned: false,
        menu: Some(PlaceMenu::Drive {
            uri: root.to_owned(),
            kind: VolumeKind::Drive,
            controls: MountControls::FIXED,
        }),
        eject: None,
    }
}

/// The dashed "Pin to Quick access" row an empty Quick access keeps, so
/// folders can still be dropped there to pin them (`.quick-drop-tail` in
/// `.quick-empty`).
fn pin_drop_tail() -> SidebarEntry {
    SidebarEntry {
        section: Section::QuickAccess,
        level: RowLevel::Place,
        label: ox_core::i18n::gettext("Pin to Quick access"),
        icon: Art::Glyph(Icon::Add),
        target: RowTarget::PinDropTail,
        tooltip: ox_core::i18n::gettext("Quick access — drop folders here to pin"),
        pinned: false,
        menu: None,
        eject: None,
    }
}

/// The sidebar rows, in the Python app's order, with the saved
/// `searches` after Quick access.
pub(in crate::window) fn sidebar_entries(
    places: &Places,
    searches: &[SavedSearch],
    locations: &LocationContext,
) -> Vec<SidebarEntry> {
    let home_uri = locations.home_uri();
    let home_icon = Art::TintedGlyph(Icon::Home, Tint::Home);
    let mut home = fixed_entry(Section::Home, "Home", home_icon, &home_uri);
    home.tooltip = locations.display_location(&home_uri);
    let this_pc_icon = Art::TintedGlyph(Page::ThisPc.icon(), Tint::ThisPc);
    let this_pc = fixed_entry(Section::ThisPc, "This PC", this_pc_icon, PC_URI);
    let network_icon = Art::TintedGlyph(Page::Network.icon(), Tint::Network);
    let network = fixed_entry(Section::Network, "Network", network_icon, NETWORK_URI);
    let quick_access = places
        .quick_access
        .iter()
        .map(|place| place_entry(place, locations));
    let drives = places.drives.iter().map(|row| drive_entry(row, locations));
    let network_rows = places.network.iter().map(|row| network_entry(row, locations));
    let saved = searches
        .iter()
        .map(|search| saved_search_entry(search, locations));
    let mut entries = vec![home];
    entries.extend(quick_access);
    if places.quick_access.is_empty() {
        entries.push(pin_drop_tail());
    }
    entries.extend(saved);
    entries.push(this_pc);
    entries.push(local_disk_entry(locations));
    entries.extend(drives);
    entries.push(network);
    entries.extend(network_rows);
    entries
}

/// "Recent files" (GIO's `recent:///`, the desktop's recently used files),
/// "Recent locations" (the folders visited lately, SIDE-026) and the
/// Recycle Bin, whose glyph takes the accent colour and whose tooltip
/// counts the items while `trash_items` are in it.
pub(in crate::window) fn recent_and_bin_entries(trash_items: u32) -> [SidebarEntry; 3] {
    let recent = SidebarEntry {
        tooltip: ox_core::i18n::gettext("Recently used files"),
        menu: Some(PlaceMenu::RecentFiles),
        ..fixed_entry(
            Section::RecentAndBin,
            "Recent files",
            Art::Glyph(Icon::History),
            RECENT_URI,
        )
    };
    let recent_locations = SidebarEntry {
        tooltip: ox_core::i18n::gettext("Recently visited folders"),
        menu: Some(PlaceMenu::RecentLocations),
        ..fixed_entry(
            Section::RecentAndBin,
            "Recent locations",
            Art::Glyph(Icon::Clock),
            RECENT_LOCATIONS_URI,
        )
    };
    let (icon, state) = match trash_items {
        0 => (Art::Glyph(Icon::Delete), ox_core::i18n::gettext("Empty")),
        1 => (Art::TintedGlyph(Icon::Delete, Tint::Home), "1 item".to_owned()),
        count => (
            Art::TintedGlyph(Icon::Delete, Tint::Home),
            ox_core::i18n::format_message("{count} items", &[("count", &count.to_string())]),
        ),
    };
    let bin = SidebarEntry {
        tooltip: ox_core::i18n::format_message("Recycle Bin · {state}", &[("state", &state)]),
        menu: Some(PlaceMenu::RecycleBin {
            has_items: trash_items > 0,
        }),
        ..fixed_entry(Section::RecentAndBin, "Recycle Bin", icon, TRASH_URI)
    };
    [recent, recent_locations, bin]
}

/// Where a row sits in its section, which decides its spacing: the
/// Quick access rows sit in a box of their own in app.js.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) struct SectionEdges {
    /// The row is the first of its section.
    pub first: bool,
    /// The row is the last of its section.
    pub last: bool,
}

/// The section edges of row `index` of `entries`.
pub(in crate::window) fn section_edges(entries: &[SidebarEntry], index: usize) -> SectionEdges {
    let section = entries.get(index).map(|entry| entry.section);
    let before = index.checked_sub(1).and_then(|before| entries.get(before));
    let after = entries.get(index + 1);
    SectionEdges {
        first: before.map(|entry| entry.section) != section,
        last: after.map(|entry| entry.section) != section,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ox_core::places::{KnownFolder, StableMount};
    use ox_core::settings::{Bookmark, SettingsData};

    use super::*;
    use crate::icons::Connection;
    use crate::places::{compose, PlaceSources};
    use crate::test_support::mounted_volume;

    fn entries_for(settings: &SettingsData, volumes: &[VolumeRow]) -> Vec<SidebarEntry> {
        let places = compose(PlaceSources {
            settings,
            known_folders: &[],
            volumes,
            stable_mounts: &[],
            visited_network: &[],
        });
        let locations = LocationContext {
            home: Some(PathBuf::from("/home/demo")),
            ..LocationContext::default()
        };
        sidebar_entries(&places, &[], &locations)
    }

    fn labels(entries: &[SidebarEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.label.as_str()).collect()
    }

    /// parity: SIDE-001, SIDE-019
    #[test]
    fn rows_follow_the_python_order_and_smb_mounts_appear_once_under_network() {
        let settings = SettingsData {
            pins: vec![Bookmark {
                uri: "file:///srv/work".into(),
                label: "Work".into(),
            }],
            shares: vec![Bookmark {
                uri: "smb://nas/media".into(),
                label: "Media".into(),
            }],
            ..SettingsData::default()
        };
        let volumes = [
            mounted_volume("USB", "file:///media/u/USB", VolumeKind::Drive),
            mounted_volume("media on nas", "smb://nas/media", VolumeKind::Drive),
            mounted_volume("Pixel 7", "mtp://[usb:001,010]/", VolumeKind::Device),
        ];
        let entries = entries_for(&settings, &volumes);
        assert_eq!(
            labels(&entries),
            [
                "Home",
                "Work",
                "This PC",
                "Local Disk",
                "USB",
                "Pixel 7",
                "Network",
                "Media"
            ]
        );
        let media = entries.last().expect("network row");
        assert_eq!(media.tooltip, "\\\\nas\\media · Connected");
        assert_eq!(media.section, Section::Network);
    }

    /// parity: DND-014
    #[test]
    fn an_empty_quick_access_keeps_a_drop_tail_to_pin_into() {
        let entries = entries_for(&SettingsData::default(), &[]);

        let tail = &entries[1];
        assert_eq!(tail.label, "Pin to Quick access");
        assert_eq!(tail.target, RowTarget::PinDropTail);
        assert_eq!(tail.section, Section::QuickAccess);
    }

    #[test]
    fn home_opens_the_home_folder() {
        let entries = entries_for(&SettingsData::default(), &[]);
        assert_eq!(entries[0].target, RowTarget::Location("file:///home/demo".into()));
        assert_eq!(entries[0].tooltip, "/home/demo");
    }

    /// parity: SIDE-025
    #[test]
    fn the_recycle_bin_row_is_drawn_full_or_empty() {
        let [recent, locations, empty] = recent_and_bin_entries(0);
        let [_, _, full] = recent_and_bin_entries(3);
        assert_eq!(recent.target, RowTarget::Location(RECENT_URI.into()));
        assert_eq!(locations.target, RowTarget::Location(RECENT_LOCATIONS_URI.into()));
        assert_eq!(empty.icon, Art::Glyph(Icon::Delete));
        assert_eq!(full.icon, Art::TintedGlyph(Icon::Delete, Tint::Home));
        assert_eq!(full.tooltip, "Recycle Bin · 3 items");
        assert_eq!(empty.menu, Some(PlaceMenu::RecycleBin { has_items: false }));
    }

    #[test]
    fn an_unmounted_volume_mounts_when_clicked() {
        let volume = VolumeRow {
            label: "Backup".into(),
            kind: VolumeKind::Drive,
            state: VolumeState::Mountable { id: "uuid-1".into() },
        };
        let entries = entries_for(&SettingsData::default(), &[volume]);
        let backup = entries
            .iter()
            .find(|entry| entry.label == "Backup")
            .expect("volume row");
        assert_eq!(backup.target, RowTarget::MountVolume("uuid-1".into()));
        assert_eq!(backup.level, RowLevel::Child);
        assert_eq!(backup.menu, Some(PlaceMenu::Volume { id: "uuid-1".into() }));
        assert_eq!(backup.eject, None, "nothing to eject before it is mounted");
    }

    /// A USB stick's row has an eject button and a menu; Local Disk has
    /// the menu only.
    ///
    /// parity: DEV-007, SIDE-017
    #[test]
    fn a_removable_drive_has_an_eject_button_and_every_drive_a_menu() {
        let usb_stick = VolumeRow {
            label: "USB".into(),
            kind: VolumeKind::Drive,
            state: VolumeState::Mounted {
                uri: "file:///media/u/USB".into(),
                controls: MountControls {
                    can_unmount: true,
                    can_eject: true,
                    can_stop: true,
                    can_open_in_disks: false,
                },
            },
        };
        let entries = entries_for(&SettingsData::default(), &[usb_stick]);
        let entry_of = |label: &str| {
            entries
                .iter()
                .find(|entry| entry.label == label)
                .expect("a drive row")
        };
        let eject = EjectButton {
            removal: Removal::Eject,
            uri: "file:///media/u/USB".into(),
            label: "Eject",
        };
        assert_eq!(entry_of("USB").eject, Some(eject));
        assert_eq!(entry_of("Local Disk").eject, None);
        assert!(matches!(
            entry_of("Local Disk").menu,
            Some(PlaceMenu::Drive { .. })
        ));
    }

    /// parity: SIDE-001
    #[test]
    fn home_this_pc_and_network_show_their_glyphs_in_their_own_colours() {
        let entries = entries_for(&SettingsData::default(), &[]);
        let icon_of = |label: &str| {
            let entry = entries.iter().find(|entry| entry.label == label);
            entry.map(|entry| entry.icon)
        };
        assert_eq!(icon_of("Home"), Some(Art::TintedGlyph(Icon::Home, Tint::Home)));
        assert_eq!(
            icon_of("This PC"),
            Some(Art::TintedGlyph(Icon::Laptop, Tint::ThisPc))
        );
        assert_eq!(
            icon_of("Network"),
            Some(Art::TintedGlyph(Icon::Organization, Tint::Network))
        );
        assert_eq!(icon_of("Local Disk"), Some(Art::Glyph(Icon::HardDrive)));
    }

    /// parity: SIDE-006
    #[test]
    fn every_quick_access_folder_has_a_glyph_in_its_colour() {
        for folder in KnownFolder::QUICK_ACCESS {
            let tint = Tint::for_known_folder(folder).expect("Quick access folders have a colour");
            let glyph = Icon::for_known_folder(folder).expect("Quick access folders have a glyph");
            assert_eq!(Art::for_known_folder(folder), Art::TintedGlyph(glyph, tint));
        }
    }

    /// parity: LOOK-016, SIDE-019
    #[test]
    fn a_saved_share_that_is_not_mounted_shows_the_red_cross() {
        let settings = SettingsData {
            shares: vec![Bookmark {
                uri: "smb://studio-nas/projects".into(),
                label: "Studio NAS (Z:)".into(),
            }],
            ..SettingsData::default()
        };
        let entries = entries_for(&settings, &[]);
        let share = entries.last().expect("the saved share's row");
        let disconnected_drive = Art::for_network_location(
            ox_core::places::NetworkKind::Share,
            "Studio NAS (Z:)",
            Connection::Disconnected,
        );
        assert_eq!(share.icon, disconnected_drive);
    }

    /// Each network row's tooltip is its UNC address and its state, as
    /// `renderSidebar` titles it.
    ///
    /// parity: SIDE-019
    #[test]
    fn network_rows_say_whether_they_are_connected_saved_or_visited() {
        let connected = NetworkLocation {
            uri: "smb://nas/media".to_owned(),
            label: "Media".to_owned(),
            is_saved: true,
            is_connected: true,
            kind: ox_core::places::NetworkKind::Share,
        };
        let saved = NetworkLocation {
            is_connected: false,
            ..connected.clone()
        };
        let visited = NetworkLocation {
            is_saved: false,
            ..saved.clone()
        };
        let locations = LocationContext::default();
        let tooltip_of = |location: &NetworkLocation| network_entry(location, &locations).tooltip;
        assert_eq!(tooltip_of(&connected), "\\\\nas\\media · Connected");
        assert_eq!(tooltip_of(&saved), "\\\\nas\\media · Saved · connect on open");
        assert_eq!(tooltip_of(&visited), "\\\\nas\\media · Opened this session");
    }

    /// Home, Quick access, This PC and Network are sections of their own,
    /// which the sidebar separates.
    ///
    /// parity: SIDE-001
    #[test]
    fn a_section_knows_its_first_and_last_rows() {
        let settings = SettingsData {
            pins: vec![
                Bookmark {
                    uri: "file:///srv/work".into(),
                    label: "Work".into(),
                },
                Bookmark {
                    uri: "file:///srv/play".into(),
                    label: "Play".into(),
                },
            ],
            ..SettingsData::default()
        };
        let entries = entries_for(&settings, &[]);
        assert_eq!(labels(&entries)[..4], ["Home", "Work", "Play", "This PC"]);
        let alone = SectionEdges {
            first: true,
            last: true,
        };
        let starting = SectionEdges {
            first: true,
            last: false,
        };
        let ending = SectionEdges {
            first: false,
            last: true,
        };
        assert_eq!(section_edges(&entries, 0), alone, "Home is a section of its own");
        assert_eq!(section_edges(&entries, 1), starting, "Work starts Quick access");
        assert_eq!(section_edges(&entries, 2), ending, "Play ends Quick access");
        assert_eq!(section_edges(&entries, 3), starting, "This PC starts a group");
    }

    /// A standard folder moved onto a CIFS mount keeps its glyph on the
    /// network pipe, and a standard folder the user hid has no row
    /// (`folder_locations.py`, `hidden_quick`).
    ///
    /// parity: SIDE-006
    #[test]
    fn a_standard_folder_on_a_cifs_mount_shows_the_network_pipe_and_a_hidden_one_no_row() {
        let known_folders = [
            Place {
                label: "Documents".into(),
                uri: "file:///mnt/nas/Documents".into(),
                known_folder: Some(KnownFolder::Documents),
                is_shared: false,
            },
            Place {
                label: "Music".into(),
                uri: "file:///home/demo/Music".into(),
                known_folder: Some(KnownFolder::Music),
                is_shared: false,
            },
        ];
        let settings = SettingsData {
            hidden_quick: vec!["file:///home/demo/Music".into()],
            ..SettingsData::default()
        };
        let cifs_mount = StableMount {
            path: PathBuf::from("/mnt/nas"),
            label: String::new(),
            filesystem: "cifs".into(),
        };
        let places = compose(PlaceSources {
            settings: &settings,
            known_folders: &known_folders,
            volumes: &[],
            stable_mounts: &[cifs_mount],
            visited_network: &[],
        });

        let entries = sidebar_entries(&places, &[], &LocationContext::default());

        let documents = entries.iter().find(|entry| entry.label == "Documents");
        let documents = documents.expect("Documents is in Quick access");
        let on_network = Art::for_quick_access(Some(KnownFolder::Documents), Storage::Network);
        assert_eq!(documents.icon, on_network);
        assert!(
            !labels(&entries).contains(&"Music"),
            "a hidden standard folder stays hidden"
        );
    }
}
