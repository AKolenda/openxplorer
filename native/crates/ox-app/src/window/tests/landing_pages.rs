// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC and Network pages against `renderLanding` and
//! `renderNetwork` in `desktop/ui/app.js`: cards in stretching grids, the
//! Network page's banner, address field and notes, Open address, and the
//! network locations' art.

use gtk::prelude::*;
use ox_core::places::NetworkKind;

use super::address_input::click_gesture;
use super::file_ops_support::is_enabled;
use super::geometry::{bounds, laid_out, Bounds};
use super::support::{art_image_showing, arts_in, menu_button_with_class};
use crate::icons::{Art, Connection, Icon};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::landing;

/// The labels `widget` shows, in order.
fn texts_in(widget: &impl IsA<gtk::Widget>) -> Vec<String> {
    descendants::<gtk::Label>(widget)
        .iter()
        .map(|label| label.text().to_string())
        .collect()
}

#[gtk::test]
fn quick_access_cards_stretch_across_the_page_in_equal_columns() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("pin-folder", None);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("Documents");
    test.activate("pin-folder", None);
    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    wait_for_frames(&test.window, 3);
    let landing = test.window.folder_pane().landing();
    let grid = descendants::<gtk::Box>(landing)
        .into_iter()
        .find(|widget| widget.has_css_class("card-grid"))
        .expect("This PC has card grids");
    let cards: Vec<Bounds> = descendants::<gtk::Button>(&grid)
        .iter()
        .map(|card| bounds(&test, card))
        .collect();
    assert!(cards.len() >= 2, "the two pins at least");
    let first = &cards[0];
    let grid_place = bounds(&test, &grid);
    assert_eq!(first.x, grid_place.x, "the first card starts at the grid's edge");
    assert!(
        cards.iter().all(|card| card.width == first.width),
        "equal columns: {cards:?}"
    );
    assert!(first.width >= 170, "columns never narrower than 170 pixels");
    let first_row: Vec<&Bounds> = cards.iter().filter(|card| card.y == first.y).collect();
    let last = first_row.last().expect("the first row has a card");
    let columns = i32::try_from(first_row.len()).expect("a few columns");
    let room_for_another = last.right() + 12 + 170 <= grid_place.right();
    let cards_left = cards.len() > first_row.len();
    assert!(
        !(room_for_another && cards_left),
        "as many columns as fit: {columns}"
    );
}

/// parity: HOME-001
#[gtk::test]
fn this_pc_has_its_heading_three_sections_and_nothing_to_search_or_create() {
    let test = TestWindow::open(Page::ThisPc.uri());
    let landing = test.window.folder_pane().landing();

    let texts = texts_in(landing);

    assert_eq!(
        texts[..2],
        ["This PC", "Folders, devices, and connected storage."]
    );
    assert_eq!(
        landing::section_titles(landing),
        ["Quick access", "Devices and drives", "Network locations"]
    );
    assert_eq!(test.window.status_bar().texts().0, "Ready");
    assert!(!test.window.search_box().is_sensitive(), "Search");
    assert!(!is_enabled(&test, "up"), "Up");
    assert!(
        !menu_button_with_class(&test, "new-command").is_sensitive(),
        "New"
    );
}

/// parity: HOME-002
#[gtk::test]
fn a_quick_access_card_says_where_the_folder_is_and_opens_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("pin-folder", None);
    wait_until("the pin", || !test.context.settings_data().pins.is_empty());
    test.window.navigate(Page::ThisPc.uri()).expect("This PC");
    test.wait_for_listing("This PC");
    let cards = descendants::<gtk::Button>(test.window.folder_pane().landing());
    let card = cards
        .into_iter()
        .find(|card| card.has_css_class("quick-card") && texts_in(card)[0] == "Example projects")
        .expect("the pinned folder has a card");

    assert_eq!(texts_in(&card), ["Example projects", "Stored on this PC"]);
    assert_eq!(arts_in(&card), [Art::Folder]);
    let middle = click_gesture(&card, gtk::gdk::BUTTON_MIDDLE);
    middle.emit_by_name::<()>("released", &[&1_i32, &1.0_f64, &1.0_f64]);
    assert_eq!(test.window.tab_count(), 2, "a middle-click opens a tab");
    assert_eq!(test.window.current_uri().as_deref(), Some(Page::ThisPc.uri()));
    card.emit_clicked();
    test.wait_for_listing("the pinned folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// parity: HOME-006, HOME-009
#[gtk::test]
fn the_network_page_has_the_banner_address_field_and_notes() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    test.wait_for_listing("the Network page");
    // The first visit discovers servers; stopping shows the idle page.
    test.activate("stop-discovery", None);
    wait_for_frames(&test.window, 2);
    let texts = texts_in(test.window.folder_pane().landing());
    for expected in [
        "Find shared storage on your local network, or enter an address.",
        "Computers & network storage",
        "Discover devices without scanning their files.",
        "Discover servers",
        "Open address",
        "Map location",
        "No advertised SMB servers found yet. Discover again or enter an address above.",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "{expected} in {texts:?}"
        );
    }
    let entries = descendants::<gtk::Entry>(test.window.folder_pane().landing());
    let address = entries.first().expect("the page has an address field");
    assert_eq!(
        address.placeholder_text().as_deref(),
        Some("\\\\server or \\\\archive-nas")
    );
}

/// parity: HOME-009
#[gtk::test]
fn open_address_refuses_anything_but_an_smb_server() {
    let fixture = Fixture::standard();
    let test = laid_out(Page::Network.uri());
    test.activate("open-server-address", Some(&fixture.uri()));
    let message = test.window.shown_message();
    assert_eq!(message.as_str(), "Enter an SMB server or share.");
    assert_eq!(test.window.current_uri().as_deref(), Some(Page::Network.uri()));
}

/// app.js drew a blue server on every saved share's card; the owner's icon
/// mapping (2026-09-28) shows a network location the same way everywhere,
/// so a mapped drive that is not mounted is the crossed-out drive on the
/// network bar on its card and in the sidebar alike.
///
/// parity: LOOK-016
#[gtk::test]
fn a_saved_share_card_shows_the_art_of_its_sidebar_row() {
    let test = laid_out(Page::ThisPc.uri());
    test.save_share("smb://nas/media", "Media (M:)");
    let crossed_out_drive =
        Art::for_network_location(NetworkKind::Share, "Media (M:)", Connection::Disconnected);
    let landing = test.window.folder_pane().landing();
    wait_until("the saved share's card", || {
        arts_in(landing).contains(&crossed_out_drive)
    });
    let sidebar_arts = arts_in(test.window.sidebar());
    assert!(sidebar_arts.contains(&crossed_out_drive), "{sidebar_arts:?}");
}

/// A server keeps the blue app.js gave a saved share's server glyph
/// (`im.style.color='#4b96c0'`), now on the network bar; the stylesheet
/// colours it, not code.
///
/// parity: LOOK-016
#[gtk::test]
fn a_server_card_shows_the_server_in_the_share_blue_on_the_network_bar() {
    let test = laid_out(Page::Network.uri());
    // Browsing a server lists it under Network for the session; recording
    // it directly mounts nothing.
    test.window.context().remember_network("smb://nas/");
    let server = Art::for_network_location(NetworkKind::Server, "nas", Connection::Disconnected);
    let landing = test.window.folder_pane().landing();
    wait_until("the server's card", || arts_in(landing).contains(&server));
    wait_for_frames(&test.window, 2);
    let card_art = art_image_showing(landing, server).expect("the card is drawn");
    let glyph = descendants::<gtk::Image>(&card_art)
        .into_iter()
        .find(|image| image.has_css_class("server"))
        .expect("the card shows the server glyph");
    assert_eq!(glyph.icon_name().as_deref(), Some(Icon::Server.name()));
    assert_eq!(glyph.color().to_str(), "rgb(75,150,192)");
}
