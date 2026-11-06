// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC page.
//!
//! Draws This PC as Windows 11 File Explorer does: an item view of tiles
//! under collapsible group headers, with no page heading. Devices and
//! drives come first (Local Disk, then drives, devices and volumes to
//! connect), then Network locations, the saved network locations; a group
//! with nothing in it is left out. Quick access lives in the sidebar and on
//! Home, as Windows 11 moved it there. Network is listed like a folder
//! instead ([`super::loading`]).
//!
//! A tile looks like an item of the Tiles view (VIEW-071): a 48-pixel icon
//! left of the name and, once GIO has measured the drive, a thin capacity
//! bar, red when nearly full, and "N free of M". A saved location shows
//! the art of its sidebar row, on the network bar and crossed out while it
//! is not mounted (the owner's icon mapping, 2026-09-28), and is named as
//! Windows names a mapped drive.
//!
//! Tiles are selected with a click and the arrow keys and opened with a
//! double-click or Enter ([`WindowAction::GoTo`], or
//! [`WindowAction::MountVolume`] for a volume); a middle-click opens a
//! folder in a background tab. A right-click, the Menu key or Shift+F10
//! opens the drive's menu ([`super::place_menus`]), and Alt+Enter its
//! Properties.

use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::location::{is_smb_location, split_location, LocationContext};
use ox_core::places::{NetworkKind, SavedShare};

use crate::folder_view::icon_size::{tiles_cell, CellSize, TILES_ICON};
use crate::icons::{self, Art, ArtImage, Connection, Icon};
use crate::locations::Page;
use crate::places::Places;
use crate::text_size::TextSize;
use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};

use super::context_menu::{CLASSIC_MENU_KEY, MENU_KEY};
use super::gestures;
use super::place_menus::{popup_place_menu, PlaceMenu};
use super::widget_tree::remove_children;
use super::window_action::WindowAction;

/// Share of used space from which the capacity bar turns red.
const NEARLY_FULL: f64 = 0.9;

/// The file system attributes a drive tile reads.
const CAPACITY_ATTRIBUTES: &str = "filesystem::size,filesystem::free";

/// Pixels between a tile's icon and its lines, as in the Tiles view.
const TILE_ICON_GAP: i32 = 10;
/// A tile's padding and margin across (8 and 2 on each side) and down
/// (8 and 1 on each side), as `gridview.files.tiles > child` has them.
const TILE_CHROME: (i32, i32) = (20, 18);
/// The chevron of a group header.
const CHEVRON: i32 = 16;
/// Most tiles a row of a group holds; the pane's width decides.
const MOST_TILES_PER_ROW: u32 = 32;
/// The class This PC's page carries, for its tighter padding.
const THIS_PC_CLASS: &str = "this-pc";

/// How full a file system is, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Capacity {
    /// The file system's size; never zero.
    pub(super) size: u64,
    /// The space still free; at most `size`.
    pub(super) free: u64,
}

impl Capacity {
    /// The share of the file system in use, from 0 to 1.
    pub(super) fn used_share(self) -> f64 {
        #[expect(clippy::cast_precision_loss, reason = "a bar needs no byte precision")]
        let share = (self.size - self.free) as f64 / self.size as f64;
        share
    }

    /// Whether the bar turns red (`.capacity.full`).
    pub(super) fn is_nearly_full(self) -> bool {
        self.used_share() >= NEARLY_FULL
    }

    /// "N free of M", as Windows 11 says it under a drive.
    fn text(self) -> String {
        let free = format::pretty_bytes(self.free);
        let size = format::pretty_bytes(self.size);
        ox_core::i18n::format_message("{free} free of {size}", &[("free", &free), ("size", &size)])
    }
}

fn label(text: &str, css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .css_classes([css_class])
        .build()
}

/// Adds a bar under `texts` once GIO has measured the file system at
/// `uri`, and "N free of M" as a line under it `with_text`, else as the
/// bar's tooltip. Never blocks: the tile or row is drawn first.
pub(super) fn show_capacity(texts: &gtk::Box, uri: &str, with_text: bool) {
    let file = gio::File::for_uri(uri);
    // Held weakly until GIO answers, so measuring never keeps a tile that
    // was replaced meanwhile.
    let texts = texts.downgrade();
    glib::spawn_future_local(async move {
        let Some(capacity) = measure_capacity(&file).await else {
            return;
        };
        // A tile replaced while GIO measured shows nothing.
        let Some(texts) = texts.upgrade() else {
            return;
        };
        let bar = capacity_bar(capacity);
        texts.append(&bar);
        if with_text {
            texts.append(&label(&capacity.text(), "card-sub"));
        } else {
            bar.set_tooltip_text(Some(&capacity.text()));
        }
    });
}

/// How full the file system of `file` is, or `None` when GIO cannot tell
/// or it has no size (`if(m.total)` in app.js).
pub(super) async fn measure_capacity(file: &gio::File) -> Option<Capacity> {
    let filesystem = file
        .query_filesystem_info_future(CAPACITY_ATTRIBUTES, glib::Priority::LOW)
        .await
        .ok()?;
    let size = filesystem.attribute_uint64("filesystem::size");
    let free = filesystem.attribute_uint64("filesystem::free").min(size);
    (size > 0).then_some(Capacity { size, free })
}

/// The bar that shows how full a drive is, red when it is nearly full.
fn capacity_bar(capacity: Capacity) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::builder()
        .fraction(capacity.used_share())
        .css_classes(["capacity"])
        .build();
    if capacity.is_nearly_full() {
        bar.add_css_class("full");
    }
    bar
}

/// The groups of This PC, in Windows 11's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    /// Local Disk, drives, devices and volumes.
    DevicesAndDrives,
    /// The saved network locations.
    NetworkLocations,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::DevicesAndDrives => ox_core::i18n::gettext_static("Devices and drives"),
            Group::NetworkLocations => ox_core::i18n::gettext_static("Network locations"),
        }
    }

    /// The class the page carries while the group is collapsed, so a
    /// collapsed group stays collapsed when This PC is drawn again.
    fn collapsed_class(self) -> &'static str {
        match self {
            Group::DevicesAndDrives => "drives-collapsed",
            Group::NetworkLocations => "network-collapsed",
        }
    }
}

/// What opening a tile does: an action and its target.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Opening {
    action: WindowAction,
    target: String,
}

/// One tile of This PC and what it does.
struct Tile {
    child: gtk::FlowBoxChild,
    /// Double-click and Enter.
    open: Opening,
    /// The right-click menu.
    menu: PlaceMenu,
    /// The location Alt+Enter shows the Properties of, if it can.
    properties: Option<String>,
}

impl Tile {
    /// Runs `opening` from the tile.
    fn open(&self) {
        let target = self.open.target.to_variant();
        self.open.action.activate_from(&self.child, Some(&target));
    }

    /// Opens the tile's menu at `x`, `y` in the tile.
    fn popup_menu(&self, x: f64, y: f64) {
        popup_place_menu(&self.menu, &self.child, x, y);
    }
}

/// An empty tile showing `icon` left of `name`, at the Tiles view's size,
/// and the box of lines beside the icon, for what the tile says under the
/// name.
fn tile_frame(icon: &impl IsA<gtk::Widget>, name: &str, cell: CellSize) -> (gtk::FlowBoxChild, gtk::Box) {
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 1);
    lines.set_valign(gtk::Align::Center);
    lines.set_hexpand(true);
    lines.add_css_class("tile-lines");
    lines.append(&label(name, "tile-name"));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, TILE_ICON_GAP);
    icon.set_valign(gtk::Align::Center);
    content.append(icon);
    content.append(&lines);
    content.set_size_request(cell.width - TILE_CHROME.0, cell.height - TILE_CHROME.1);
    let child = gtk::FlowBoxChild::builder().child(&content).build();
    child.update_property(&[gtk::accessible::Property::Label(name)]);
    (child, lines)
}

/// A drive's tile: Local Disk, a drive, a device, or a volume to connect.
fn drive_tile(row: &VolumeRow, cell: CellSize) -> Tile {
    let glyph = match row.kind {
        VolumeKind::Device => Icon::Phone,
        VolumeKind::Drive => Icon::HardDrive,
    };
    let (child, lines) = tile_frame(&icons::image(glyph, TILES_ICON), &row.label, cell);
    match &row.state {
        VolumeState::Mounted { uri, controls } => {
            show_capacity(&lines, uri, true);
            gestures::open_folder_on_middle_click(&child, uri);
            Tile {
                child,
                open: Opening {
                    action: WindowAction::GoTo,
                    target: uri.clone(),
                },
                menu: PlaceMenu::Drive {
                    uri: uri.clone(),
                    kind: row.kind,
                    controls: *controls,
                },
                properties: Some(uri.clone()),
            }
        }
        VolumeState::Mountable { id } => {
            lines.append(&label(
                ox_core::i18n::gettext_static("Click to connect"),
                "card-sub",
            ));
            Tile {
                child,
                open: Opening {
                    action: WindowAction::MountVolume,
                    target: id.clone(),
                },
                menu: PlaceMenu::Volume { id: id.clone() },
                properties: None,
            }
        }
    }
}

fn local_disk() -> VolumeRow {
    VolumeRow {
        label: ox_core::i18n::gettext_static("Local Disk").to_owned(),
        kind: VolumeKind::Drive,
        state: VolumeState::Mounted {
            uri: "file:///".to_owned(),
            controls: MountControls::FIXED,
        },
    }
}

/// A saved share's tile, as Windows 11 draws a mapped drive in This PC:
/// its name with its server and letter, and, while it is connected, how
/// full it is, else that it connects on open. Its address is its tooltip.
fn share_tile(share: &SavedShare, locations: &LocationContext, cell: CellSize) -> Tile {
    let uri = &share.bookmark.uri;
    let art = ArtImage::new(saved_share_art(share), TILES_ICON);
    let name = share_card_name(&share.bookmark.label, uri);
    let (child, lines) = tile_frame(&art, &name, cell);
    child.set_tooltip_text(Some(&locations.display_location(uri)));
    // Measuring never mounts: only a share that is mounted is asked.
    if share.is_connected {
        show_capacity(&lines, uri, true);
    } else {
        lines.append(&label(
            ox_core::i18n::gettext_static("Connect on open"),
            "card-sub",
        ));
    }
    gestures::open_folder_on_middle_click(&child, uri);
    Tile {
        child,
        open: Opening {
            action: WindowAction::GoTo,
            target: uri.clone(),
        },
        menu: PlaceMenu::SavedShare { uri: uri.clone() },
        properties: share.is_connected.then(|| uri.clone()),
    }
}

/// A saved share's art: the share, or the drive its label maps it to, on
/// the network bar, crossed out while it is not mounted. The Network list
/// counts every saved location as a share, so the tile and the sidebar row
/// show the same.
fn saved_share_art(share: &SavedShare) -> Art {
    let connection = Connection::from_mounted(share.is_connected);
    Art::for_network_location(NetworkKind::Share, &share.bookmark.label, connection)
}

/// A saved share's name, as Windows 11 names a mapped drive in This PC:
/// its label, then its server, before the drive letter the label ends
/// with, if any, such as `Backup (\\fileserver) (S:)`. An SMB server is
/// written `\\server`, any other by its host name; a label that already
/// names the server is kept as it is.
pub(super) fn share_card_name(label: &str, uri: &str) -> String {
    let Some(host) = split_location(uri).ok().and_then(|parts| parts.hostname()) else {
        return label.to_owned();
    };
    let server = if is_smb_location(uri) {
        format!("\\\\{host}")
    } else {
        host
    };
    if label.contains(&format!("({server})")) {
        return label.to_owned();
    }
    let letter = label
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .filter(|(_, letter)| is_drive_letter(letter));
    match letter {
        Some((name, letter)) => format!("{name} ({server}) ({letter})"),
        None => format!("{label} ({server})"),
    }
}

/// Whether `text` is a drive letter as a label writes it, such as `S:`.
fn is_drive_letter(text: &str) -> bool {
    let mut characters = text.chars();
    matches!(
        (characters.next(), characters.next(), characters.next()),
        (Some(letter), Some(':'), None) if letter.is_ascii_alphabetic()
    )
}

/// "Devices and drives (3)": a group's title and how many tiles it holds,
/// as the details view heads its groups.
fn header_text(title: &str, count: usize) -> String {
    ox_core::i18n::format_message(
        "{title} ({count})",
        &[
            ("title", title),
            (
                "count",
                &format::group_digits(u64::try_from(count).unwrap_or(u64::MAX)),
            ),
        ],
    )
}

/// A group's header: a chevron, the title and count, and a line to the
/// right edge. Clicking it collapses or expands `tiles`.
fn group_header(page: &gtk::Box, group: Group, count: usize, tiles: &gtk::FlowBox) -> gtk::Button {
    let collapsed = page.has_css_class(group.collapsed_class());
    let chevron_of = |collapsed: bool| {
        if collapsed {
            Icon::ChevronRight16
        } else {
            Icon::ChevronDown16
        }
    };
    let chevron = icons::image(chevron_of(collapsed), CHEVRON);
    let title = gtk::Label::builder()
        .label(header_text(group.title(), count))
        .xalign(0.0)
        .css_classes(["group-title"])
        .build();
    let line = gtk::Separator::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&chevron);
    row.append(&title);
    row.append(&line);
    // Not a Tab stop, as in Explorer: the keyboard goes to the tiles, and
    // the arrows run on from one group into the next.
    let header = gtk::Button::builder()
        .child(&row)
        .focusable(false)
        .css_classes(["group-header"])
        .build();
    header.update_state(&[gtk::accessible::State::Expanded(Some(!collapsed))]);
    tiles.set_visible(!collapsed);
    header.connect_clicked(glib::clone!(
        #[weak]
        page,
        #[weak]
        tiles,
        move |header| {
            let collapse = tiles.is_visible();
            tiles.set_visible(!collapse);
            if collapse {
                page.add_css_class(group.collapsed_class());
            } else {
                page.remove_css_class(group.collapsed_class());
            }
            icons::set_icon(&chevron, chevron_of(collapse), CHEVRON);
            header.update_state(&[gtk::accessible::State::Expanded(Some(!collapse))]);
        }
    ));
    header
}

/// The tile `child` is, among `tiles`.
fn tile_of<'a>(tiles: &'a [Tile], child: &gtk::FlowBoxChild) -> Option<&'a Tile> {
    tiles.iter().find(|tile| tile.child == *child)
}

/// The selected tile of `flow`, among `tiles`.
fn selected_tile<'a>(flow: &gtk::FlowBox, tiles: &'a [Tile]) -> Option<&'a Tile> {
    let child = flow.selected_children().into_iter().next()?;
    tile_of(tiles, &child)
}

/// Gives every tile its right-click menu, which selects it first, as
/// Explorer does.
fn connect_menus(flow: &gtk::FlowBox, tiles: &Rc<Vec<Tile>>) {
    for tile in tiles.iter() {
        let right_click = gtk::GestureClick::new();
        right_click.set_button(gtk::gdk::BUTTON_SECONDARY);
        right_click.connect_pressed(glib::clone!(
            #[weak]
            flow,
            #[weak]
            tiles,
            move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(child) = gesture.widget().and_downcast::<gtk::FlowBoxChild>() else {
                    return;
                };
                flow.select_child(&child);
                child.grab_focus();
                if let Some(tile) = tile_of(&tiles, &child) {
                    tile.popup_menu(x, y);
                }
            }
        ));
        tile.child.add_controller(right_click);
    }
}

/// The Menu key and Shift+F10 open the selected tile's menu under it;
/// Alt+Enter opens its Properties.
fn connect_keys(flow: &gtk::FlowBox, tiles: &Rc<Vec<Tile>>) {
    let shortcuts = gtk::ShortcutController::new();
    let menu = gtk::CallbackAction::new(glib::clone!(
        #[strong]
        tiles,
        move |flow, _| {
            let Some(flow) = flow.downcast_ref::<gtk::FlowBox>() else {
                return glib::Propagation::Proceed;
            };
            let Some(tile) = selected_tile(flow, &tiles) else {
                return glib::Propagation::Proceed;
            };
            let width = f64::from(tile.child.width());
            let height = f64::from(tile.child.height());
            tile.popup_menu(width / 2.0, height / 2.0);
            glib::Propagation::Stop
        }
    ));
    for key in [MENU_KEY, CLASSIC_MENU_KEY] {
        let trigger = gtk::ShortcutTrigger::parse_string(key);
        shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(menu.clone())));
    }
    let properties = gtk::CallbackAction::new(glib::clone!(
        #[strong]
        tiles,
        move |flow, _| {
            let Some(flow) = flow.downcast_ref::<gtk::FlowBox>() else {
                return glib::Propagation::Proceed;
            };
            let Some((tile, uri)) =
                selected_tile(flow, &tiles).and_then(|tile| Some((tile, tile.properties.as_ref()?)))
            else {
                return glib::Propagation::Proceed;
            };
            WindowAction::PropertiesOf.activate_from(&tile.child, Some(&uri.to_variant()));
            glib::Propagation::Stop
        }
    ));
    for key in ["<Alt>Return", "<Alt>KP_Enter"] {
        let trigger = gtk::ShortcutTrigger::parse_string(key);
        shortcuts.add_shortcut(gtk::Shortcut::new(trigger, Some(properties.clone())));
    }
    flow.add_controller(shortcuts);
}

/// Adds `group`'s header and its `tiles` to `page`; a group without tiles
/// is left out, as Windows 11 leaves out an empty group.
fn tile_group(page: &gtk::Box, group: Group, tiles: Vec<Tile>) {
    if tiles.is_empty() {
        return;
    }
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .activate_on_single_click(false)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(MOST_TILES_PER_ROW)
        .column_spacing(0)
        .row_spacing(0)
        .valign(gtk::Align::Start)
        .css_classes(["drive-tiles"])
        .build();
    flow.update_property(&[gtk::accessible::Property::Label(group.title())]);
    page.append(&group_header(page, group, tiles.len(), &flow));
    page.append(&flow);
    for tile in &tiles {
        flow.append(&tile.child);
    }
    let tiles = Rc::new(tiles);
    // The tiles are held by the box's own handlers; their gestures hold
    // them weakly, so no tile keeps itself alive.
    flow.connect_child_activated(glib::clone!(
        #[strong]
        tiles,
        move |_, child| {
            if let Some(tile) = tile_of(&tiles, child) {
                tile.open();
            }
        }
    ));
    connect_menus(&flow, &tiles);
    connect_keys(&flow, &tiles);
}

fn this_pc(page: &gtk::Box, places: &Places, locations: &LocationContext, text_size: TextSize) {
    let cell = tiles_cell(text_size);
    let drives = std::iter::once(local_disk()).chain(places.drives.iter().cloned());
    let drive_tiles = drives.map(|row| drive_tile(&row, cell)).collect();
    tile_group(page, Group::DevicesAndDrives, drive_tiles);
    let share_tiles = places.saved_shares.iter();
    let share_tiles = share_tiles
        .map(|share| share_tile(share, locations, cell))
        .collect();
    tile_group(page, Group::NetworkLocations, share_tiles);
}

/// Draws `page` into `body`, replacing what it showed.
pub(super) fn render(
    body: &gtk::Box,
    page: Page,
    places: &Places,
    locations: &LocationContext,
    text_size: TextSize,
) {
    remove_children(body);
    match page {
        Page::ThisPc => {
            body.add_css_class(THIS_PC_CLASS);
            this_pc(body, places, locations, text_size);
        }
        // Network is listed like a folder; the Settings page replaces the
        // whole browsing area, landing pages included
        // (`super::settings_tab`). Neither draws anything here.
        Page::Network | Page::Settings => body.remove_css_class(THIS_PC_CLASS),
    }
}

/// The group titles `body` shows, for tests.
#[cfg(test)]
pub(super) fn section_titles(body: &gtk::Box) -> Vec<String> {
    super::widget_tree::children(body)
        .filter(|child| child.has_css_class("group-header"))
        .filter_map(|header| {
            let row = header.first_child()?;
            let title = row.first_child()?.next_sibling().and_downcast::<gtk::Label>()?;
            Some(title.text().to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::descendants;
    use crate::test_support::mounted_volume;

    /// The texts of `widget`, in order.
    fn texts_of(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
        let labels = descendants::<gtk::Label>(widget);
        labels.iter().map(|label| label.text().to_string()).collect()
    }

    /// Local Disk comes first, then each drive and device by name; a
    /// volume says "Click to connect" and mounts when opened.
    ///
    /// parity: HOME-003
    #[gtk::test]
    fn drive_tiles_name_each_drive_and_a_volume_connects_when_opened() {
        let volume = VolumeRow {
            label: "Backup".into(),
            kind: VolumeKind::Drive,
            state: VolumeState::Mountable { id: "uuid-1".into() },
        };
        let rows = [
            local_disk(),
            mounted_volume("USB", "file:///media/u/USB", VolumeKind::Drive),
            mounted_volume("Pixel 7", "mtp://[usb:001,010]/", VolumeKind::Device),
            volume,
        ];
        let cell = tiles_cell(TextSize::from_percent(100));
        let tiles: Vec<Tile> = rows.iter().map(|row| drive_tile(row, cell)).collect();

        let first_lines: Vec<Vec<String>> = tiles.iter().map(|tile| texts_of(&tile.child)).collect();
        assert_eq!(
            first_lines,
            [
                vec!["Local Disk"],
                vec!["USB"],
                vec!["Pixel 7"],
                vec!["Backup", "Click to connect"],
            ]
        );
        let mount = tiles.last().expect("the volume's tile");
        assert_eq!(
            mount.open,
            Opening {
                action: WindowAction::MountVolume,
                target: "uuid-1".into()
            }
        );
        assert_eq!(mount.properties, None);
    }

    #[test]
    fn the_capacity_bar_turns_red_from_ninety_percent_used() {
        let nearly_full = Capacity { size: 100, free: 10 };
        let roomy = Capacity { size: 100, free: 11 };
        assert!(nearly_full.is_nearly_full());
        assert!(!roomy.is_nearly_full());
        assert!((roomy.used_share() - 0.89).abs() < f64::EPSILON);
    }

    #[test]
    fn the_capacity_line_says_how_much_is_free_of_the_size() {
        let capacity = Capacity {
            size: 2048,
            free: 1024,
        };
        let free = format::pretty_bytes(1024);
        let size = format::pretty_bytes(2048);
        assert_eq!(capacity.text(), format!("{free} free of {size}"));
    }
}
