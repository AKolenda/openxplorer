// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC and Network landing pages.
//!
//! Ports the `pc:` branch of `renderLanding` and `renderNetwork` in
//! `desktop/ui/app.js`. This PC lists Quick access, then Devices and drives
//! (Local Disk, drives and devices with a capacity bar, unmounted volumes
//! that connect on click), then the saved network locations with their
//! state. Network lists every connected and saved location. Server
//! discovery and the manual address box arrive with the discovery service.
//!
//! Cards activate `win.go-to` or `win.mount-volume`; a middle-click opens a
//! folder in a background tab.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use ox_core::format;
use ox_core::location::LocationContext;
use ox_core::places::{NetworkLocation, Place};

use crate::icons::{self, ArtKind, Glyph};
use crate::locations::Page;
use crate::places::{Places, SavedShare};
use crate::theme::Appearance;
use crate::volumes::{VolumeKind, VolumeRow, VolumeState};

use super::gestures;

/// Colour of the server glyph on saved-share cards (`im.style.color`).
const SHARE_GLYPH_COLOR: &str = "#4b96c0";

/// Share of used space from which the capacity bar turns red.
const NEARLY_FULL: f64 = 0.9;

/// What a page needs to draw its art.
#[derive(Debug, Clone, Copy)]
pub(super) struct Drawing {
    pub appearance: Appearance,
    pub scale: i32,
}

fn label(text: &str, css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes([css_class])
        .build()
}

fn section_title(text: &str, glyph: Glyph) -> gtk::Box {
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    title.add_css_class("section-title");
    title.append(&icons::glyph(glyph, 14));
    title.append(&gtk::Label::new(Some(text)));
    title
}

fn card_grid(max_per_line: u32) -> gtk::FlowBox {
    gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(12)
        .row_spacing(10)
        .min_children_per_line(1)
        .max_children_per_line(max_per_line)
        .homogeneous(true)
        .build()
}

/// A card button that opens `uri`.
fn location_card(css_class: &str, uri: &str, content: &gtk::Box) -> gtk::Button {
    let card = gtk::Button::builder()
        .child(content)
        .css_classes([css_class])
        .action_name("win.go-to")
        .action_target(&uri.to_variant())
        .build();
    gestures::open_folder_on_middle_click(&card, uri);
    card
}

fn texts(name: &str, subtitle: &str) -> gtk::Box {
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    texts.add_css_class("drive-info");
    texts.append(&label(name, "card-name"));
    texts.append(&label(subtitle, "card-sub"));
    texts
}

fn quick_card(place: &Place, drawing: Drawing) -> gtk::Button {
    let network = place.uri.starts_with("smb:");
    let (art, subtitle) = if network {
        (ArtKind::NetworkFolder, "Network folder")
    } else {
        (ArtKind::Folder, "Stored on this PC")
    };
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 15);
    content.append(&icons::art_image(art, 43, drawing.appearance, drawing.scale));
    content.append(&texts(&place.label, subtitle));
    location_card("quick-card", &place.uri, &content)
}

fn quick_access(body: &gtk::Box, places: &Places, drawing: Drawing) {
    body.append(&section_title("Quick access", Glyph::Pin));
    let cards = card_grid(4);
    for place in &places.quick_access {
        cards.insert(&quick_card(place, drawing), -1);
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
    let cards = card_grid(3);
    let drives = std::iter::once(local_disk()).chain(places.drives.iter().cloned());
    for row in drives {
        cards.insert(&drive_card(&row, locations), -1);
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

/// The saved network locations with their state (`shares()` in app.js).
fn saved_shares(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    body.append(&section_title("Network locations", Glyph::Network));
    let cards = card_grid(3);
    for share in &places.saved_shares {
        cards.insert(&saved_share_card(share, locations), -1);
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

fn network_card(location: &NetworkLocation, locations: &LocationContext) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 15);
    content.append(&icons::glyph(Glyph::Network, 34));
    content.append(&texts(
        &location.label,
        &locations.display_location(&location.uri),
    ));
    location_card("drive-card", &location.uri, &content)
}

/// Every connected, saved and visited network location (`renderNetwork`).
fn connected_and_saved(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    body.append(&section_title("Connected & saved locations", Glyph::Pin));
    let cards = card_grid(3);
    for location in &places.network {
        cards.insert(&network_card(location, locations), -1);
    }
    body.append(&cards);
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
    drawing: Drawing,
) {
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    page_header(body, page);
    match page {
        Page::ThisPc => {
            quick_access(body, places, drawing);
            devices_and_drives(body, places, locations);
            saved_shares(body, places, locations);
        }
        Page::Network => connected_and_saved(body, places, locations),
    }
}

/// The section titles `body` shows, for tests.
#[cfg(test)]
pub(super) fn section_titles(body: &gtk::Box) -> Vec<String> {
    let mut titles = Vec::new();
    let mut child = body.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("section-title") {
            let text = widget
                .last_child()
                .and_downcast::<gtk::Label>()
                .map(|label| label.text().to_string());
            titles.extend(text);
        }
        child = widget.next_sibling();
    }
    titles
}
