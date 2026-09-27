// SPDX-License-Identifier: AGPL-3.0-only
//! The navigation pane (sidebar).
//!
//! Ports `renderSidebar` in `desktop/ui/app.js`, in its order: Home (the
//! home folder), the Quick access folders and pins, This PC with Local Disk
//! and the drives and devices, and Network with the merged network
//! locations. Mounted SMB shares appear once, under Network. Groups are
//! separated by list-row headers, so keyboard and screen-reader users never
//! land on an empty separator row.
//!
//! [`sidebar_entries`] turns composed [`Places`] into rows without GTK, so
//! the order is tested on its own; rows activate `win.go-to` or
//! `win.mount-volume`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use ox_core::location::{same_location, LocationContext, NETWORK_URI, PC_URI};
use ox_core::places::{NetworkKind, NetworkLocation, Place};

use crate::icons::{self, ArtKind, Glyph};
use crate::places::Places;
use crate::theme::Appearance;
use crate::volumes::{VolumeKind, VolumeRow, VolumeState};

use super::gestures;

/// A group of rows; a separator is drawn where the group changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Home,
    QuickAccess,
    ThisPc,
    Network,
}

/// How a row sits in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowLevel {
    /// A top-level place (Home, a Quick access folder).
    Place,
    /// A group head with an expander (This PC, Network).
    Group,
    /// A row inside a group, indented.
    Child,
}

/// How a row's icon is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum RowIcon {
    /// A line glyph, in a fixed colour or the text colour.
    Glyph(Glyph, Option<gdk::RGBA>),
    /// Colour art (folders and network locations).
    Art(ArtKind),
}

/// What activating a row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowTarget {
    /// Opens a location.
    Location(String),
    /// Mounts the volume with this identifier, then opens it.
    MountVolume(String),
}

/// One sidebar row.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SidebarEntry {
    pub section: Section,
    pub level: RowLevel,
    pub label: String,
    pub icon: RowIcon,
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
    let shared = place.is_shared || place.uri.starts_with("smb:");
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
pub(super) fn sidebar_entries(places: &Places, locations: &LocationContext) -> Vec<SidebarEntry> {
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

fn row_icon(icon: RowIcon, appearance: Appearance, scale: i32) -> gtk::Image {
    match icon {
        RowIcon::Glyph(glyph, Some(fixed)) => icons::colored_glyph(glyph, 18, fixed),
        RowIcon::Glyph(glyph, None) => icons::glyph(glyph, 18),
        RowIcon::Art(kind) => icons::art_image(kind, 19, appearance, scale),
    }
}

fn row_content(entry: &SidebarEntry, appearance: Appearance, scale: i32) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 11);
    let pill = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pill.add_css_class("pill");
    pill.set_valign(gtk::Align::Center);
    content.append(&pill);
    if entry.level == RowLevel::Group {
        let expander = icons::glyph(Glyph::Down, 9);
        expander.add_css_class("expand");
        content.append(&expander);
    }
    content.append(&row_icon(entry.icon, appearance, scale));
    let label = gtk::Label::builder()
        .label(&entry.label)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    content.append(&label);
    if entry.pinned {
        let pin = icons::glyph(Glyph::Pin, 11);
        pin.add_css_class("pin");
        content.append(&pin);
    }
    content
}

fn sidebar_row(entry: &SidebarEntry, appearance: Appearance, scale: i32) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&row_content(entry, appearance, scale)));
    if entry.level == RowLevel::Child {
        row.add_css_class("indent");
    }
    row.set_tooltip_text(Some(&entry.tooltip));
    // The visible label names the row; the address is its description.
    row.update_property(&[
        gtk::accessible::Property::Label(&entry.label),
        gtk::accessible::Property::Description(&entry.tooltip),
    ]);
    match &entry.target {
        RowTarget::Location(uri) => {
            row.set_action_name(Some("win.go-to"));
            row.set_action_target_value(Some(&uri.to_variant()));
        }
        RowTarget::MountVolume(id) => {
            row.set_action_name(Some("win.mount-volume"));
            row.set_action_target_value(Some(&id.to_variant()));
        }
    }
    row
}

/// The location of the row at `y` in `list`, for middle-clicks.
fn location_at(list: &gtk::ListBox, entries: &[SidebarEntry], y: f64) -> Option<String> {
    #[expect(clippy::cast_possible_truncation, reason = "pointer positions are small")]
    let row = list.row_at_y(y as i32)?;
    let index = usize::try_from(row.index()).ok()?;
    match &entries.get(index)?.target {
        RowTarget::Location(uri) => Some(uri.clone()),
        RowTarget::MountVolume(_) => None,
    }
}

/// The sidebar's widgets and the rows it shows.
#[derive(Debug)]
pub(super) struct Sidebar {
    /// The scrolling pane.
    pub root: gtk::ScrolledWindow,
    /// The rows.
    pub list: gtk::ListBox,
    entries: Rc<RefCell<Vec<SidebarEntry>>>,
}

impl Sidebar {
    pub fn new() -> Self {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(true)
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Navigation pane")]);
        let entries: Rc<RefCell<Vec<SidebarEntry>>> = Rc::default();
        let sections = Rc::clone(&entries);
        list.set_header_func(move |row, before| {
            let section_of = |row: &gtk::ListBoxRow| {
                let index = usize::try_from(row.index()).ok()?;
                sections.borrow().get(index).map(|entry| entry.section)
            };
            let starts_group = before.is_some_and(|before| section_of(before) != section_of(row));
            if !starts_group {
                row.set_header(None::<&gtk::Widget>);
                return;
            }
            let line = gtk::Separator::new(gtk::Orientation::Horizontal);
            line.add_css_class("side-separator");
            row.set_header(Some(&line));
        });
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(140)
            .child(&list)
            .build();
        root.add_css_class("sidebar");
        let sidebar = Self { root, list, entries };
        sidebar.open_places_on_middle_click();
        sidebar
    }

    /// A middle-click on a place opens it in a tab; volumes that still
    /// have to be mounted do nothing.
    fn open_places_on_middle_click(&self) {
        let entries = Rc::clone(&self.entries);
        let gesture = gestures::middle_click(move |gesture, _, y| {
            let Some(list) = gesture.widget().and_downcast::<gtk::ListBox>() else {
                return;
            };
            let Some(uri) = location_at(&list, &entries.borrow(), y) else {
                return;
            };
            let action = gestures::open_action(gesture.current_event_state());
            // The action exists on every browser window.
            let _ = list.activate_action(action, Some(&uri.to_variant()));
        });
        self.list.add_controller(gesture);
    }

    /// Replaces the rows.
    pub fn show(&self, entries: Vec<SidebarEntry>, appearance: Appearance, scale: i32) {
        self.list.remove_all();
        let rows: Vec<gtk::ListBoxRow> = entries
            .iter()
            .map(|entry| sidebar_row(entry, appearance, scale))
            .collect();
        self.entries.replace(entries);
        for row in &rows {
            self.list.append(row);
        }
    }

    /// Highlights the row for `uri`, or none.
    pub fn select(&self, uri: &str) {
        let index = self
            .entries
            .borrow()
            .iter()
            .position(|entry| match &entry.target {
                RowTarget::Location(candidate) => same_location(candidate, uri),
                RowTarget::MountVolume(_) => false,
            });
        let row = index
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.list.row_at_index(index));
        self.list.select_row(row.as_ref());
    }

    /// The labels shown, for tests.
    #[cfg(test)]
    pub fn labels(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .map(|entry| entry.label.clone())
            .collect()
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
}
