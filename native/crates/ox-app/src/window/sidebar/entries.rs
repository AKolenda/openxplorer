// SPDX-License-Identifier: AGPL-3.0-only
//! The sidebar's rows as data, in the order of `renderSidebar` in
//! `desktop/ui/app.js`: Home (the home folder), the Quick access folders
//! and pins, This PC with Local Disk and the drives and devices, and
//! Network with the merged network locations. Mounted SMB shares appear
//! once, under Network.
//!
//! [`sidebar_entries`] turns composed [`Places`] into rows without GTK, so
//! the order is tested on its own.

use gtk::gdk;
use ox_core::location::{LocationContext, NETWORK_URI, PC_URI};
use ox_core::places::{NetworkKind, NetworkLocation, Place};

use crate::icons::{ArtKind, Glyph};
use crate::places::Places;
use crate::volumes::{VolumeKind, VolumeRow, VolumeState};
use crate::window::location_kind::is_smb_location;

/// A group of rows; a separator is drawn where the group changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::window) enum Section {
    /// The home folder.
    Home,
    /// Known folders and pins.
    QuickAccess,
    /// This PC, Local Disk and the drives and devices.
    ThisPc,
    /// Network and the network locations.
    Network,
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

/// How a row's icon is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::window) enum RowIcon {
    /// A line glyph, in a fixed colour or the text colour.
    Glyph(Glyph, Option<gdk::RGBA>),
    /// Colour art (folders and network locations).
    Art(ArtKind),
}

/// What activating a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::window) enum RowTarget {
    /// Opens a location.
    Location(String),
    /// Mounts the volume with this identifier, then opens it.
    MountVolume(String),
}

/// One sidebar row.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::window) struct SidebarEntry {
    /// The group the row belongs to.
    pub section: Section,
    /// A place, a group head or an indented row.
    pub level: RowLevel,
    /// The visible name, which is also the accessible name.
    pub label: String,
    /// How the row's icon is drawn.
    pub icon: RowIcon,
    /// What activating the row does.
    pub target: RowTarget,
    /// Hover text and accessible description.
    pub tooltip: String,
    /// Shows the pin glyph of a Quick access row.
    pub pinned: bool,
}

/// Colours of the fixed rows (`add(...)` calls in `renderSidebar`).
const HOME_COLOR: &str = "#0078d4";
const THIS_PC_COLOR: &str = "#347ba7";
const NETWORK_COLOR: &str = "#318db9";

/// A CSS hex colour from the Python app's tables, which the tests check.
fn color(hex: &str) -> gdk::RGBA {
    gdk::RGBA::parse(hex).expect("the Python app's colour tables hold valid CSS colours")
}

fn place_entry(place: &Place, locations: &LocationContext) -> SidebarEntry {
    let known = place.icon.and_then(Glyph::for_known_folder);
    let shared = place.is_shared || is_smb_location(&place.uri);
    let icon = match known {
        Some(glyph) => RowIcon::Glyph(glyph, place.color.map(color)),
        None if shared => RowIcon::Art(ArtKind::NetworkFolder),
        None => RowIcon::Art(ArtKind::Folder),
    };
    SidebarEntry {
        section: Section::QuickAccess,
        level: RowLevel::Place,
        label: place.label.clone(),
        icon,
        target: RowTarget::Location(place.uri.clone()),
        tooltip: locations.display_location(&place.uri),
        pinned: true,
    }
}

fn drive_entry(row: &VolumeRow, locations: &LocationContext) -> SidebarEntry {
    let glyph = match row.kind {
        VolumeKind::Device => Glyph::Phone,
        VolumeKind::Drive => Glyph::Drive,
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
        icon: RowIcon::Glyph(glyph, None),
        target,
        tooltip,
        pinned: false,
    }
}

/// The state text of a network row, as `renderSidebar` titles it.
fn network_state(location: &NetworkLocation) -> &'static str {
    if location.connected {
        "Connected"
    } else if location.saved {
        "Saved · connect on open"
    } else {
        "Opened this session"
    }
}

fn network_entry(location: &NetworkLocation, locations: &LocationContext) -> SidebarEntry {
    let art = match location.kind {
        NetworkKind::Server => ArtKind::NetworkGlyph(Glyph::Server),
        NetworkKind::Share | NetworkKind::Mount => ArtKind::NetworkFolder,
    };
    let address = locations.display_location(&location.uri);
    SidebarEntry {
        section: Section::Network,
        level: RowLevel::Child,
        label: location.label.clone(),
        icon: RowIcon::Art(art),
        target: RowTarget::Location(location.uri.clone()),
        tooltip: format!("{address} · {}", network_state(location)),
        pinned: false,
    }
}

/// A top-level row with a coloured glyph: Home, This PC or Network.
fn fixed_entry(section: Section, label: &str, glyph: Glyph, glyph_color: &str, uri: &str) -> SidebarEntry {
    let level = if section == Section::Home {
        RowLevel::Place
    } else {
        RowLevel::Group
    };
    SidebarEntry {
        section,
        level,
        label: label.to_owned(),
        icon: RowIcon::Glyph(glyph, Some(color(glyph_color))),
        target: RowTarget::Location(uri.to_owned()),
        tooltip: label.to_owned(),
        pinned: false,
    }
}

fn local_disk_entry(locations: &LocationContext) -> SidebarEntry {
    SidebarEntry {
        section: Section::ThisPc,
        level: RowLevel::Child,
        label: "Local Disk".to_owned(),
        icon: RowIcon::Glyph(Glyph::Drive, None),
        target: RowTarget::Location("file:///".to_owned()),
        tooltip: locations.display_location("file:///"),
        pinned: false,
    }
}

/// The sidebar rows, in the Python app's order.
pub(in crate::window) fn sidebar_entries(places: &Places, locations: &LocationContext) -> Vec<SidebarEntry> {
    let home_uri = locations.home_uri();
    let mut home = fixed_entry(Section::Home, "Home", Glyph::Home, HOME_COLOR, &home_uri);
    home.tooltip = locations.display_location(&home_uri);
    let this_pc = fixed_entry(Section::ThisPc, "This PC", Glyph::Desktop, THIS_PC_COLOR, PC_URI);
    let network = fixed_entry(
        Section::Network,
        "Network",
        Glyph::Network,
        NETWORK_COLOR,
        NETWORK_URI,
    );
    let quick_access = places
        .quick_access
        .iter()
        .map(|place| place_entry(place, locations));
    let drives = places.drives.iter().map(|row| drive_entry(row, locations));
    let network_rows = places.network.iter().map(|row| network_entry(row, locations));
    let mut entries = vec![home];
    entries.extend(quick_access);
    entries.push(this_pc);
    entries.push(local_disk_entry(locations));
    entries.extend(drives);
    entries.push(network);
    entries.extend(network_rows);
    entries
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

    use ox_core::settings::{Bookmark, SettingsData};

    use super::*;
    use crate::places::{compose, PlaceSources};

    fn mounted(label: &str, uri: &str, kind: VolumeKind) -> VolumeRow {
        VolumeRow {
            label: label.into(),
            kind,
            state: VolumeState::Mounted {
                uri: uri.into(),
                can_unmount: true,
            },
        }
    }

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
        sidebar_entries(&places, &locations)
    }

    fn labels(entries: &[SidebarEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.label.as_str()).collect()
    }

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
            mounted("USB", "file:///media/u/USB", VolumeKind::Drive),
            mounted("media on nas", "smb://nas/media", VolumeKind::Drive),
            mounted("Pixel 7", "mtp://[usb:001,010]/", VolumeKind::Device),
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

    #[test]
    fn home_opens_the_home_folder() {
        let entries = entries_for(&SettingsData::default(), &[]);
        assert_eq!(entries[0].target, RowTarget::Location("file:///home/demo".into()));
        assert_eq!(entries[0].tooltip, "/home/demo");
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
    }

    #[test]
    fn fixed_and_known_folder_colours_are_valid() {
        for hex in [HOME_COLOR, THIS_PC_COLOR, NETWORK_COLOR] {
            assert!(gdk::RGBA::parse(hex).is_ok(), "{hex}");
        }
        for place in ox_core::places::known_folders() {
            let hex = place.color.expect("known folders have a colour");
            assert!(gdk::RGBA::parse(hex).is_ok(), "{hex}");
            let icon = place.icon.expect("known folders have a glyph");
            assert!(Glyph::for_known_folder(icon).is_some(), "{icon}");
        }
    }

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
        let edges: Vec<(bool, bool)> = (0..4)
            .map(|index| section_edges(&entries, index))
            .map(|edges| (edges.first, edges.last))
            .collect();
        // Home alone; Work and Play in Quick access; This PC starts a group.
        assert_eq!(edges, [(true, true), (true, false), (false, true), (true, false)]);
    }
}
