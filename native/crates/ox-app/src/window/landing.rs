// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC and Network landing pages.
//!
//! Ports the `pc:` branch of `renderLanding` in `desktop/ui/app.js`, and
//! draws the title of every page ([`super::network_page`] draws the rest
//! of Network). This PC lists Quick access (cards in a stretching grid,
//! [`super::card_grid`]), then Devices and drives (Local Disk, drives and
//! devices with a capacity bar, unmounted volumes that connect on click),
//! then the saved network locations with their state.
//!
//! Cards activate `win.go-to` or `win.mount-volume`; a middle-click opens a
//! folder in a background tab.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::location::LocationContext;
use ox_core::places::Place;

use crate::icons::{self, ArtKind, Glyph};
use crate::locations::Page;
use crate::places::{Places, SavedShare};
use crate::volumes::{VolumeKind, VolumeRow, VolumeState};

use super::art_style::ArtStyle;
use super::card_grid::{card_grid, DRIVE_GRID, QUICK_GRID};
use super::{gestures, network_page, unported};

/// The action of the "Map network location" heading button.
const MAP_NETWORK_ACTION: &str = "win.map-network-location";

/// Colour of the server glyph on saved-share cards (`im.style.color`).
const SHARE_GLYPH_COLOR: &str = "#4b96c0";

/// Share of used space from which the capacity bar turns red.
const NEARLY_FULL: f64 = 0.9;

fn label(text: &str, css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes([css_class])
        .build()
}

/// A section heading: a glyph and a bold title.
pub(super) fn section_title(text: &str, glyph: Glyph) -> gtk::Box {
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    title.add_css_class("section-title");
    title.append(&icons::glyph(glyph, 14));
    title.append(&gtk::Label::new(Some(text)));
    title
}

/// A card button that opens `uri`.
pub(super) fn location_card(css_class: &str, uri: &str, content: &gtk::Box) -> gtk::Button {
    let card = gtk::Button::builder()
        .child(content)
        .css_classes([css_class])
        .action_name("win.go-to")
        .action_target(&uri.to_variant())
        .build();
    gestures::open_folder_on_middle_click(&card, uri);
    card
}

/// A card's name above `subtitle`.
fn texts_with(name: &str, subtitle: &gtk::Label) -> gtk::Box {
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    texts.add_css_class("drive-info");
    texts.append(&label(name, "card-name"));
    texts.append(subtitle);
    texts
}

/// A card's name above a subtitle that ends in "…" when it is too long.
pub(super) fn texts(name: &str, subtitle: &str) -> gtk::Box {
    texts_with(name, &label(subtitle, "card-sub"))
}

fn quick_card(place: &Place, style: ArtStyle) -> gtk::Button {
    let network = place.uri.starts_with("smb:");
    let (art, subtitle) = if network {
        (ArtKind::NetworkFolder, "Network folder")
    } else {
        (ArtKind::Folder, "Stored on this PC")
    };
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 15);
    content.append(&style.image(art, 43));
    // "Stored on this PC" is never cut short: in a narrow card it runs
    // into the padding, as `.quick-card .card-sub` lets it.
    let whole_subtitle = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .css_classes(["card-sub"])
        .build();
    content.append(&texts_with(&place.label, &whole_subtitle));
    location_card("quick-card", &place.uri, &content)
}

fn quick_access(body: &gtk::Box, places: &Places, style: ArtStyle) {
    body.append(&section_title("Quick access", Glyph::Pin));
    let cards = card_grid(QUICK_GRID);
    for place in &places.quick_access {
        cards.append(&quick_card(place, style));
    }
    body.append(&cards);
}

/// Adds "N free of M" and a bar under a drive card's texts once GIO has
/// measured the file system. Never blocks: the card is drawn first.
fn show_capacity(info: &gtk::Box, uri: &str) {
    let file = gio::File::for_uri(uri);
    let info = info.downgrade();
    glib::spawn_future_local(async move {
        let attributes = "filesystem::size,filesystem::free";
        let Ok(filesystem) = file
            .query_filesystem_info_future(attributes, glib::Priority::LOW)
            .await
        else {
            return;
        };
        let size = filesystem.attribute_uint64("filesystem::size");
        let free = filesystem.attribute_uint64("filesystem::free").min(size);
        // A card replaced while GIO measured shows nothing; neither does a
        // file system without a size, as `if(m.total)` in app.js.
        let Some(info) = info.upgrade().filter(|_| size > 0) else {
            return;
        };
        #[expect(clippy::cast_precision_loss, reason = "a bar needs no byte precision")]
        let used = (size - free) as f64 / size as f64;
        let bar = gtk::ProgressBar::builder()
            .fraction(used)
            .css_classes(["capacity"])
            .build();
        if used >= NEARLY_FULL {
            bar.add_css_class("full");
        }
        let text = format!(
            "{} free of {}",
            format::pretty_bytes(free),
            format::pretty_bytes(size)
        );
        info.append(&bar);
        info.append(&label(&text, "card-sub"));
    });
}

/// A drive card: Local Disk, a drive, a device, or a volume to connect.
fn drive_card(row: &VolumeRow, locations: &LocationContext) -> gtk::Button {
    let glyph = match row.kind {
        VolumeKind::Device => Glyph::Phone,
        VolumeKind::Drive => Glyph::Drive,
    };
    let subtitle = match (&row.state, row.kind) {
        (VolumeState::Mounted { .. }, VolumeKind::Device) => "Connected device".to_owned(),
        (VolumeState::Mounted { uri, .. }, VolumeKind::Drive) => locations.display_location(uri),
        (VolumeState::Mountable { .. }, _) => "Click to connect".to_owned(),
    };
    let info = texts(&row.label, &subtitle);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 15);
    content.append(&icons::glyph(glyph, 46));
    content.append(&info);
    match &row.state {
        VolumeState::Mounted { uri, .. } => {
            show_capacity(&info, uri);
            location_card("drive-card", uri, &content)
        }
        VolumeState::Mountable { id } => gtk::Button::builder()
            .child(&content)
            .css_classes(["drive-card"])
            .action_name("win.mount-volume")
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
            can_unmount: false,
        },
    }
}

fn devices_and_drives(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    body.append(&section_title("Devices and drives", Glyph::Drive));
    let cards = card_grid(DRIVE_GRID);
    let drives = std::iter::once(local_disk()).chain(places.drives.iter().cloned());
    for row in drives {
        cards.append(&drive_card(&row, locations));
    }
    body.append(&cards);
}

fn saved_share_card(share: &SavedShare, locations: &LocationContext) -> gtk::Button {
    let bookmark = &share.bookmark;
    let glyph_color = gtk::gdk::RGBA::parse(SHARE_GLYPH_COLOR).expect("a valid CSS colour literal");
    let info = texts(&bookmark.label, &locations.display_location(&bookmark.uri));
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    state.add_css_class("connected");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    dot.set_valign(gtk::Align::Center);
    let state_text = if share.connected {
        "Mounted in this session"
    } else {
        dot.add_css_class("offline");
        "Connect on open"
    };
    state.append(&dot);
    state.append(&gtk::Label::new(Some(state_text)));
    info.append(&state);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 15);
    content.append(&icons::colored_glyph(Glyph::Server, 46, glyph_color));
    content.append(&info);
    location_card("drive-card", &bookmark.uri, &content)
}

/// "Map network location" at the right of the Network locations heading,
/// disabled until the connect dialog is ported.
fn map_network_button() -> gtk::Button {
    gtk::Button::builder()
        .label("Map network location")
        .action_name(MAP_NETWORK_ACTION)
        .tooltip_text(unported::tooltip(MAP_NETWORK_ACTION, "Map network location"))
        .hexpand(true)
        .halign(gtk::Align::End)
        .build()
}

/// The saved network locations with their state (`shares()` in app.js).
fn saved_shares(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    let title = section_title("Network locations", Glyph::Network);
    title.append(&map_network_button());
    body.append(&title);
    let cards = card_grid(DRIVE_GRID);
    for share in &places.saved_shares {
        cards.append(&saved_share_card(share, locations));
    }
    body.append(&cards);
    if places.saved_shares.is_empty() {
        let empty = gtk::Label::builder()
            .label("No saved network locations. Enter a share's address in the location bar to open it.")
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

/// Draws `page` into `body`, replacing what it showed.
pub(super) fn render(
    body: &gtk::Box,
    page: Page,
    places: &Places,
    locations: &LocationContext,
    style: ArtStyle,
) {
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    page_header(body, page);
    match page {
        Page::ThisPc => {
            quick_access(body, places, style);
            devices_and_drives(body, places, locations);
            saved_shares(body, places, locations);
        }
        Page::Network => network_page::render(body, places, locations),
    }
}

/// The section titles `body` shows, for tests.
#[cfg(test)]
pub(super) fn section_titles(body: &gtk::Box) -> Vec<String> {
    let mut titles = Vec::new();
    let mut child = body.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("section-title") {
            // The glyph, then the title's label.
            let text = widget
                .first_child()
                .and_then(|glyph| glyph.next_sibling())
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_string());
            titles.extend(text);
        }
        child = widget.next_sibling();
    }
    titles
}
