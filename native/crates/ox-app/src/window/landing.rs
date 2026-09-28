// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC and Network landing pages.
//!
//! Ports the `pc:` branch of `renderLanding` in `desktop/ui/app.js`, and
//! draws the title of every page ([`super::network_page`] draws the rest
//! of Network). This PC lists Quick access (cards in a stretching grid,
//! [`super::card_grid`]), then Devices and drives (Local Disk, drives and
//! devices with a capacity bar, unmounted volumes that connect on click),
//! then the saved network locations with their state. A saved location's
//! card shows it on the network bar, as its sidebar row does (the owner's
//! icon mapping, 2026-09-28), where app.js drew a blue server for every
//! one.
//!
//! Cards run [`WindowAction::GoTo`] or [`WindowAction::MountVolume`]; a
//! middle-click opens a folder in a background tab, and a right-click on
//! a drive that can be removed or on a saved location opens its menu
//! ([`super::place_menus`]).

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::location::{is_smb_location, LocationContext};
use ox_core::places::{NetworkKind, Place, SavedShare};

use crate::icons::{self, Art, ArtImage, Connection, Icon};
use crate::locations::Page;
use crate::network::DiscoveryState;
use crate::places::Places;
use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};

use super::card_grid::{card_grid, DRIVE_GRID, QUICK_GRID};
use super::place_menus::{attach_place_menu, PlaceMenu};
use super::widget_tree::remove_children;
use super::window_action::WindowAction;
use super::{gestures, network_page};

/// Share of used space from which the capacity bar turns red.
const NEARLY_FULL: f64 = 0.9;

/// The glyph before a section title.
const SECTION_GLYPH: i32 = 14;
/// The folder art of a Quick access card (`folderIcon(43)`).
const QUICK_CARD_ART: i32 = 43;
/// The icon of a drive, device or saved network location's card.
const DRIVE_CARD_ICON: i32 = 46;
/// Pixels between a card's picture and its texts.
pub(super) const CARD_ICON_GAP: i32 = 15;

/// The file system attributes a drive card reads.
const CAPACITY_ATTRIBUTES: &str = "filesystem::size,filesystem::free";

/// How full a file system is, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Capacity {
    /// The file system's size; never zero.
    size: u64,
    /// The space still free; at most `size`.
    free: u64,
}

impl Capacity {
    /// The share of the file system in use, from 0 to 1.
    fn used_share(self) -> f64 {
        #[expect(clippy::cast_precision_loss, reason = "a bar needs no byte precision")]
        let share = (self.size - self.free) as f64 / self.size as f64;
        share
    }

    /// Whether the bar turns red (`.capacity.full`).
    fn is_nearly_full(self) -> bool {
        self.used_share() >= NEARLY_FULL
    }

    /// "N free of M", as the drive cards of app.js say it.
    fn text(self) -> String {
        let free = format::pretty_bytes(self.free);
        let size = format::pretty_bytes(self.size);
        format!("{free} free of {size}")
    }
}

fn label(text: &str, css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes([css_class])
        .build()
}

/// A section heading: a glyph and a bold title.
pub(super) fn section_title(text: &str, glyph: Icon) -> gtk::Box {
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    title.add_css_class("section-title");
    title.append(&icons::image(glyph, SECTION_GLYPH));
    title.append(&gtk::Label::new(Some(text)));
    title
}

/// A card button that opens `uri`.
pub(super) fn location_card(css_class: &str, uri: &str, content: &gtk::Box) -> gtk::Button {
    let card = gtk::Button::builder()
        .child(content)
        .css_classes([css_class])
        .action_name(WindowAction::GoTo.detailed_name())
        .action_target(&uri.to_variant())
        .build();
    gestures::open_folder_on_middle_click(&card, uri);
    card
}

/// A card's name above `subtitle`.
fn card_texts_with(name: &str, subtitle: &gtk::Label) -> gtk::Box {
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    texts.add_css_class("drive-info");
    texts.append(&label(name, "card-name"));
    texts.append(subtitle);
    texts
}

/// A card's name above a subtitle that ends in "…" when it is too long.
pub(super) fn card_texts(name: &str, subtitle: &str) -> gtk::Box {
    card_texts_with(name, &label(subtitle, "card-sub"))
}

fn quick_card(place: &Place) -> gtk::Button {
    let network = is_smb_location(&place.uri);
    let (art, subtitle) = if network {
        (Art::SHARE, "Network folder")
    } else {
        (Art::Folder, "Stored on this PC")
    };
    let content = gtk::Box::new(gtk::Orientation::Horizontal, CARD_ICON_GAP);
    content.append(&ArtImage::new(art, QUICK_CARD_ART));
    // "Stored on this PC" is never cut short: in a narrow card it runs
    // into the padding, as `.quick-card .card-sub` lets it.
    let whole_subtitle = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .css_classes(["card-sub"])
        .build();
    content.append(&card_texts_with(&place.label, &whole_subtitle));
    location_card("quick-card", &place.uri, &content)
}

fn quick_access(body: &gtk::Box, places: &Places) {
    body.append(&section_title("Quick access", Icon::Pin));
    let cards = card_grid(QUICK_GRID);
    for place in &places.quick_access {
        cards.append(&quick_card(place));
    }
    body.append(&cards);
}

/// Adds "N free of M" and a bar under a drive card's texts once GIO has
/// measured the file system. Never blocks: the card is drawn first.
fn show_capacity(texts: &gtk::Box, uri: &str) {
    let file = gio::File::for_uri(uri);
    // Held weakly until GIO answers, so measuring never keeps a card that
    // was replaced meanwhile.
    let texts = texts.downgrade();
    glib::spawn_future_local(async move {
        let Some(capacity) = measure_capacity(&file).await else {
            return;
        };
        // A card replaced while GIO measured shows nothing.
        let Some(texts) = texts.upgrade() else {
            return;
        };
        texts.append(&capacity_bar(capacity));
        texts.append(&label(&capacity.text(), "card-sub"));
    });
}

/// How full the file system of `file` is, or `None` when GIO cannot tell
/// or it has no size (`if(m.total)` in app.js).
async fn measure_capacity(file: &gio::File) -> Option<Capacity> {
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

/// A drive card: Local Disk, a drive, a device, or a volume to connect.
fn drive_card(row: &VolumeRow, locations: &LocationContext) -> gtk::Button {
    let glyph = match row.kind {
        VolumeKind::Device => Icon::Phone,
        VolumeKind::Drive => Icon::HardDrive,
    };
    let subtitle = match (&row.state, row.kind) {
        (VolumeState::Mounted { .. }, VolumeKind::Device) => "Connected device".to_owned(),
        (VolumeState::Mounted { uri, .. }, VolumeKind::Drive) => locations.display_location(uri),
        (VolumeState::Mountable { .. }, _) => "Click to connect".to_owned(),
    };
    let texts = card_texts(&row.label, &subtitle);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, CARD_ICON_GAP);
    content.append(&icons::image(glyph, DRIVE_CARD_ICON));
    content.append(&texts);
    match &row.state {
        VolumeState::Mounted { uri, controls } => {
            show_capacity(&texts, uri);
            let card = location_card("drive-card", uri, &content);
            let menu = PlaceMenu::DriveCard {
                uri: uri.clone(),
                kind: row.kind,
                controls: *controls,
            };
            // Local Disk and mounts the system keeps have no menu.
            if !menu.entries().is_empty() {
                attach_place_menu(&card, menu);
            }
            card
        }
        VolumeState::Mountable { id } => gtk::Button::builder()
            .child(&content)
            .css_classes(["drive-card"])
            .action_name(WindowAction::MountVolume.detailed_name())
            .action_target(&id.to_variant())
            .build(),
    }
}

fn local_disk() -> VolumeRow {
    VolumeRow {
        label: "Local Disk".to_owned(),
        kind: VolumeKind::Drive,
        state: VolumeState::Mounted {
            uri: "file:///".to_owned(),
            controls: MountControls::FIXED,
        },
    }
}

fn devices_and_drives(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    body.append(&section_title("Devices and drives", Icon::HardDrive));
    let cards = card_grid(DRIVE_GRID);
    let drives = std::iter::once(local_disk()).chain(places.drives.iter().cloned());
    for row in drives {
        cards.append(&drive_card(&row, locations));
    }
    body.append(&cards);
}

fn saved_share_card(share: &SavedShare, locations: &LocationContext) -> gtk::Button {
    let bookmark = &share.bookmark;
    let address = locations.display_location(&bookmark.uri);
    let texts = card_texts(&bookmark.label, &address);
    texts.append(&share_state(share));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, CARD_ICON_GAP);
    content.append(&ArtImage::new(saved_share_art(share), DRIVE_CARD_ICON));
    content.append(&texts);
    let card = location_card("drive-card", &bookmark.uri, &content);
    let menu = PlaceMenu::SavedShare {
        uri: bookmark.uri.clone(),
    };
    attach_place_menu(&card, menu);
    card
}

/// A saved share's art: the share, or the drive its label maps it to, on
/// the network bar, crossed out while it is not mounted. The Network list
/// counts every saved location as a share, so the card and the sidebar row
/// show the same.
fn saved_share_art(share: &SavedShare) -> Art {
    let connection = Connection::from_mounted(share.is_connected);
    Art::for_network_location(NetworkKind::Share, &share.bookmark.label, connection)
}

/// A saved share's state line: a status dot, marked offline while the
/// share is not mounted, and what opening it does.
fn share_state(share: &SavedShare) -> gtk::Box {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    dot.set_valign(gtk::Align::Center);
    if !share.is_connected {
        dot.add_css_class("offline");
    }
    let state_text = if share.is_connected {
        "Mounted in this session"
    } else {
        "Connect on open"
    };
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    state.add_css_class("connected");
    state.append(&dot);
    state.append(&gtk::Label::new(Some(state_text)));
    state
}

/// "Map network location" at the right of the Network locations heading.
fn map_network_button() -> gtk::Button {
    gtk::Button::builder()
        .label("Map network location")
        .action_name(WindowAction::MapNetworkLocation.detailed_name())
        .hexpand(true)
        .halign(gtk::Align::End)
        .build()
}

/// The saved network locations with their state (`shares()` in app.js).
fn saved_shares(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    let title = section_title("Network locations", Icon::Organization);
    title.append(&map_network_button());
    body.append(&title);
    let cards = card_grid(DRIVE_GRID);
    for share in &places.saved_shares {
        cards.append(&saved_share_card(share, locations));
    }
    body.append(&cards);
    if places.saved_shares.is_empty() {
        let empty = gtk::Label::builder()
            .label(
                "No network locations saved. Use “Map network location” to connect to a share and add it to \
                 the sidebar.",
            )
            .xalign(0.0)
            .wrap(true)
            .css_classes(["notice"])
            .build();
        body.append(&empty);
    }
}

fn page_header(body: &gtk::Box, page: Page) {
    let title = gtk::Label::builder()
        .label(page.title())
        .xalign(0.0)
        .css_classes(["page-title"])
        .build();
    let subtitle = gtk::Label::builder()
        .label(page.subtitle())
        .xalign(0.0)
        .wrap(true)
        .css_classes(["page-subtitle"])
        .build();
    body.append(&title);
    body.append(&subtitle);
}

/// Draws `page` into `body`, replacing what it showed; the Network page
/// shows `discovery`.
pub(super) fn render(
    body: &gtk::Box,
    page: Page,
    places: &Places,
    locations: &LocationContext,
    discovery: &DiscoveryState,
) {
    remove_children(body);
    match page {
        Page::ThisPc => {
            page_header(body, page);
            quick_access(body, places);
            devices_and_drives(body, places, locations);
            saved_shares(body, places, locations);
        }
        Page::Network => {
            page_header(body, page);
            network_page::render(body, places, locations, discovery);
        }
        // The Settings page replaces the whole browsing area, landing
        // pages included (`super::settings_tab`), so it leaves this empty.
        Page::Settings => {}
    }
}

/// The section titles `body` shows, for tests.
#[cfg(test)]
pub(super) fn section_titles(body: &gtk::Box) -> Vec<String> {
    super::widget_tree::children(body)
        .filter(|child| child.has_css_class("section-title"))
        .filter_map(|title| section_title_text(&title))
        .collect()
}

/// The text of a [`section_title`]: its glyph, then its label.
#[cfg(test)]
fn section_title_text(title: &gtk::Widget) -> Option<String> {
    let glyph = title.first_child()?;
    let label = glyph.next_sibling().and_downcast::<gtk::Label>()?;
    Some(label.text().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::harness::descendants;
    use crate::test_support::mounted_volume;

    /// The texts of `card`, in order.
    fn texts_of(card: &impl IsA<gtk::Widget>) -> Vec<String> {
        let labels = descendants::<gtk::Label>(card);
        labels.iter().map(|label| label.text().to_string()).collect()
    }

    /// Local Disk comes first; a drive shows where it is mounted, a phone
    /// "Connected device", and a volume "Click to connect", which mounts
    /// it when clicked.
    ///
    /// parity: HOME-003
    #[gtk::test]
    fn drive_cards_say_where_each_drive_is_or_that_it_connects_on_click() {
        let locations = LocationContext::default();
        let volume = VolumeRow {
            label: "Backup".into(),
            kind: VolumeKind::Drive,
            state: VolumeState::Mountable { id: "uuid-1".into() },
        };
        let places = Places {
            quick_access: Vec::new(),
            drives: vec![
                mounted_volume("USB", "file:///media/u/USB", VolumeKind::Drive),
                mounted_volume("Pixel 7", "mtp://[usb:001,010]/", VolumeKind::Device),
                volume,
            ],
            saved_shares: Vec::new(),
            network: Vec::new(),
        };
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);

        devices_and_drives(&body, &places, &locations);

        let cards = descendants::<gtk::Button>(&body);
        let first_lines: Vec<Vec<String>> = cards.iter().map(|card| texts_of(card)[..2].to_vec()).collect();
        assert_eq!(
            first_lines,
            [
                ["Local Disk", "/"],
                ["USB", "/media/u/USB"],
                ["Pixel 7", "Connected device"],
                ["Backup", "Click to connect"],
            ]
        );
        let mount = cards.last().expect("the volume's card");
        assert_eq!(
            mount.action_name().as_deref(),
            Some(WindowAction::MountVolume.detailed_name().as_str())
        );
        let target = mount
            .action_target_value()
            .and_then(|target| target.get::<String>());
        assert_eq!(target.as_deref(), Some("uuid-1"));
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
