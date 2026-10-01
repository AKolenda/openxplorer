// SPDX-License-Identifier: AGPL-3.0-only
//! The Network landing page.
//!
//! Ports `renderNetwork` in `v2.0.0:desktop/ui/app.js`: the "Computers & network
//! storage" banner with Discover servers (Stop while it runs), the server
//! address field with Open address and Map location, the discovered
//! servers with their count, the note on how discovery works, and every
//! connected, saved and visited network location with its menu. The page
//! draws what the window's [`DiscoveryState`] holds; discovery starts the
//! first time the window shows the page ([`super::network_actions`]).
//! Each location's card shows it on the network bar as its sidebar row
//! does (the owner's icon mapping, 2026-09-28), where app.js drew the
//! network glyph for every one.

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::{self, is_server_location, LocationContext};
use ox_core::network::DiscoveredServer;
use ox_core::places::{NetworkKind, NetworkLocation};

use crate::dialogs::Protocol;
use crate::icons::{self, Art, ArtImage, Connection, Icon};
use crate::network::DiscoveryState;
use crate::places::Places;

use super::button_style::ButtonStyle;
use super::card_grid::{card_grid, DRIVE_GRID};
use super::landing::{card_texts, location_card, section_title, CARD_ICON_GAP};
use super::place_menus::{attach_place_menu, PlaceMenu};
use super::window_action::WindowAction;
use super::BrowserWindow;

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
/// The icon of a connected or saved location's card, as large as app.js
/// drew its network glyph (`icon('network',34)`).
const LOCATION_CARD_ICON: i32 = 34;
/// The icon of a discovered server's card (`icon('server',40)`).
const SERVER_CARD_ICON: i32 = 40;

/// The note under the discovered servers (`.discovery-note`).
const DISCOVERY_NOTE: &str = "Discovery depends on devices advertising themselves and on local \
firewall/network settings. It does not guarantee a list of every host.";

/// A bordered or accent button that runs `action`.
fn command_button(label: &str, action: WindowAction, style: ButtonStyle) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .action_name(action.detailed_name())
        .valign(gtk::Align::Center)
        .css_classes([style.css_class()])
        .build()
}

/// "Computers & network storage" with Discover servers, or Stop while
/// discovery runs.
fn banner(discovery: &DiscoveryState) -> gtk::Box {
    let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
    words.set_hexpand(true);
    words.append(
        &gtk::Label::builder()
            .label("Computers & network storage")
            .xalign(0.0)
            .build(),
    );
    let hint = gtk::Label::builder()
        .label(discovery.banner_hint())
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
    let toggle = if discovery.is_busy {
        command_button("Stop", WindowAction::StopDiscovery, ButtonStyle::Accent)
    } else {
        command_button(
            "Discover servers",
            WindowAction::DiscoverServers,
            ButtonStyle::Accent,
        )
    };
    banner.append(&toggle);
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

/// A discovered server's card: its name, address and "SMB · Discovered".
/// It opens the server's shares; a middle-click opens them in a tab
/// behind the Network page.
fn server_card(server: &DiscoveredServer, locations: &LocationContext) -> gtk::Button {
    let art = Art::for_network_location(NetworkKind::Server, &server.label, Connection::Connected);
    let texts = card_texts(&server.label, &locations.display_location(&server.uri));
    let protocol = gtk::Label::builder()
        .label(format!("{} · Discovered", protocol_name(&server.uri)))
        .xalign(0.0)
        .css_classes(["network-protocol"])
        .build();
    texts.append(&protocol);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, CARD_ICON_GAP);
    content.append(&ArtImage::new(art, SERVER_CARD_ICON));
    content.append(&texts);
    let card = location_card("drive-card", &server.uri, &content);
    card.add_css_class("discovered-server");
    card
}

/// The protocol a discovered server is reached with, as its card names it.
fn protocol_name(uri: &str) -> &'static str {
    let protocol = location::scheme(uri).and_then(|scheme| Protocol::from_scheme(&scheme));
    protocol.unwrap_or(Protocol::Smb).short_name()
}

/// "Discovered servers" with their count, their cards, the notice while
/// there are none, and the note on how discovery works.
fn discovered_servers(body: &gtk::Box, discovery: &DiscoveryState, locations: &LocationContext) {
    let title = section_title("Discovered servers", Icon::Desktop);
    let count = gtk::Label::builder()
        .label(discovery.servers.len().to_string())
        .hexpand(true)
        .xalign(1.0)
        .css_classes(["network-count"])
        .build();
    title.append(&count);
    body.append(&title);
    let cards = card_grid(DRIVE_GRID);
    for server in &discovery.servers {
        cards.append(&server_card(server, locations));
    }
    body.append(&cards);
    if discovery.servers.is_empty() {
        let notice = gtk::Label::builder()
            .label(discovery.empty_notice())
            .xalign(0.0)
            .wrap(true)
            .css_classes(["notice"])
            .build();
        body.append(&notice);
    }
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
    content.append(&ArtImage::new(Art::for_network_row(location), LOCATION_CARD_ICON));
    let address = locations.display_location(&location.uri);
    content.append(&card_texts(&location.label, &address));
    let card = location_card("drive-card", &location.uri, &content);
    attach_place_menu(&card, PlaceMenu::Network(location.clone()));
    card
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
pub(super) fn render(
    body: &gtk::Box,
    places: &Places,
    locations: &LocationContext,
    discovery: &DiscoveryState,
) {
    body.append(&banner(discovery));
    body.append(&server_address_field());
    discovered_servers(body, discovery, locations);
    connected_and_saved(body, places, locations);
}

impl BrowserWindow {
    /// Open address: opens the SMB, SFTP, FTP, WebDAV or NFS location
    /// `typed` names, and refuses anything else as the Network page does.
    pub(super) fn open_server_address(&self, typed: &str) {
        match location::normalise_location(typed, None, &glib::home_dir()) {
            Ok(uri) if is_server_location(&uri) => self.navigate_or_report(&uri),
            Ok(_) => self.show_message("Enter a network server or shared folder."),
            Err(error) => self.show_message(&error.to_string()),
        }
    }
}
