// SPDX-License-Identifier: AGPL-3.0-only
//! The Network landing page.
//!
//! Ports `renderNetwork` in `desktop/ui/app.js`: the "Computers & network
//! storage" banner with Discover servers, the server address field with
//! Open address and Map location, the discovered servers, the note on how
//! discovery works, and every connected, saved and visited network
//! location. Server discovery and the connect dialog arrive with the
//! Network and devices milestone, so their buttons are disabled until then
//! ([`super::unported`]); Open address works now.

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::{self, LocationContext};
use ox_core::places::NetworkLocation;

use crate::icons::{self, Icon};
use crate::places::Places;

use super::button_style::ButtonStyle;
use super::card_grid::{card_grid, DRIVE_GRID};
use super::landing::{card_texts, location_card, section_title, CARD_ICON_GAP};
use super::location_kind::is_smb_location;
use super::window_action::WindowAction;
use super::{unported, BrowserWindow};

/// The network glyph of the banner (`icon('network',38)` in
/// `renderNetwork`).
const BANNER_GLYPH: i32 = 38;
/// Pixels between the banner's glyph, words and button
/// (`.network-banner{gap:18px}`).
const BANNER_GAP: i32 = 18;
/// The "+" before "Map location".
const MAP_GLYPH: i32 = 14;
/// Pixels between the "+" and "Map location".
const MAP_GLYPH_GAP: i32 = 7;
/// Pixels between the address field and its buttons
/// (`.network-manual{gap:9px}`).
const ADDRESS_FIELD_GAP: i32 = 9;
/// The network glyph of a connected or saved location's card
/// (`icon('network',34)`).
const LOCATION_CARD_GLYPH: i32 = 34;

/// The note under the discovered servers (`.discovery-note`).
const DISCOVERY_NOTE: &str = "Discovery depends on devices advertising themselves and on local \
firewall/network settings. It does not guarantee a list of every host.";

/// A button for a command that may not be ported yet.
fn command_button(label: &str, action: WindowAction, style: ButtonStyle) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .action_name(action.detailed_name())
        .tooltip_text(unported::tooltip(action, label))
        .valign(gtk::Align::Center)
        .css_classes([style.css_class()])
        .build()
}

/// "Computers & network storage" with Discover servers.
fn banner() -> gtk::Box {
    let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
    words.set_hexpand(true);
    words.append(
        &gtk::Label::builder()
            .label("Computers & network storage")
            .xalign(0.0)
            .build(),
    );
    let hint = gtk::Label::builder()
        .label("Discover devices without scanning their files.")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["banner-hint"])
        .build();
    words.append(&hint);
    let glyph = icons::image(Icon::Organization, BANNER_GLYPH);
    glyph.add_css_class("banner-glyph");
    let banner = gtk::Box::builder()
        .spacing(BANNER_GAP)
        .css_classes(["network-banner"])
        .build();
    banner.append(&glyph);
    banner.append(&words);
    // Starts looking for advertised SMB servers (`discoverNetwork`).
    banner.append(&command_button(
        "Discover servers",
        WindowAction::DiscoverServers,
        ButtonStyle::Accent,
    ));
    banner
}

/// The server address field with Open address and Map location
/// (`.network-manual`).
fn server_address_field() -> gtk::Box {
    let address = gtk::Entry::builder()
        .placeholder_text("\\\\server or \\\\archive-nas")
        .hexpand(true)
        .build();
    address.update_property(&[gtk::accessible::Property::Label("SMB server address")]);
    address.connect_activate(open_typed_address);
    let open = gtk::Button::builder()
        .label("Open address")
        .valign(gtk::Align::Center)
        .css_classes([ButtonStyle::Bordered.css_class()])
        .build();
    open.connect_clicked(glib::clone!(
        #[weak]
        address,
        move |_| open_typed_address(&address)
    ));
    let map_content = gtk::Box::new(gtk::Orientation::Horizontal, MAP_GLYPH_GAP);
    map_content.append(&icons::image(Icon::Add, MAP_GLYPH));
    map_content.append(&gtk::Label::new(Some("Map location")));
    // Opens the connect dialog (`connectDialog`).
    let map = command_button(
        "Map location",
        WindowAction::MapNetworkLocation,
        ButtonStyle::Bordered,
    );
    map.set_child(Some(&map_content));
    let field = gtk::Box::builder()
        .spacing(ADDRESS_FIELD_GAP)
        .css_classes(["network-manual"])
        .build();
    field.append(&address);
    field.append(&open);
    field.append(&map);
    field
}

/// Runs Open address for the text in `entry`.
fn open_typed_address(entry: &gtk::Entry) {
    let typed = entry.text().to_variant();
    WindowAction::OpenServerAddress.activate_from(entry, Some(&typed));
}

/// "Discovered servers" with their count, and the notice while there are
/// none. Discovery is not ported, so no server is ever found yet.
fn discovered_servers(body: &gtk::Box) {
    let title = section_title("Discovered servers", Icon::Desktop);
    let count = gtk::Label::builder()
        .label("0")
        .hexpand(true)
        .xalign(1.0)
        .css_classes(["network-count"])
        .build();
    title.append(&count);
    body.append(&title);
    let notice = gtk::Label::builder()
        .label("No advertised SMB servers found yet. Discover again or enter an address above.")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["notice"])
        .build();
    body.append(&notice);
    let note = gtk::Label::builder()
        .label(DISCOVERY_NOTE)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["discovery-note"])
        .build();
    body.append(&note);
}

fn network_card(location: &NetworkLocation, locations: &LocationContext) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, CARD_ICON_GAP);
    content.append(&icons::image(Icon::Organization, LOCATION_CARD_GLYPH));
    let address = locations.display_location(&location.uri);
    content.append(&card_texts(&location.label, &address));
    location_card("drive-card", &location.uri, &content)
}

/// Every connected, saved and visited network location.
fn connected_and_saved(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    body.append(&section_title("Connected & saved locations", Icon::Pin));
    let cards = card_grid(DRIVE_GRID);
    for location in &places.network {
        cards.append(&network_card(location, locations));
    }
    body.append(&cards);
}

/// Draws the Network page's sections into `body`, below its title.
pub(super) fn render(body: &gtk::Box, places: &Places, locations: &LocationContext) {
    body.append(&banner());
    body.append(&server_address_field());
    discovered_servers(body);
    connected_and_saved(body, places, locations);
}

impl BrowserWindow {
    /// Open address: opens the SMB server or share `typed` names, and
    /// refuses anything else as the Network page's field does.
    pub(super) fn open_server_address(&self, typed: &str) {
        match location::normalise_location(typed, None, &glib::home_dir()) {
            Ok(uri) if is_smb_location(&uri) => self.navigate_or_report(&uri),
            Ok(_) => self.show_message("Enter an SMB server or share."),
            Err(error) => self.show_message(error.message()),
        }
    }
}
