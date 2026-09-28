// SPDX-License-Identifier: AGPL-3.0-only
//! The This PC and Network pages against `renderLanding` and
//! `renderNetwork` in `desktop/ui/app.js`: cards in stretching grids, the
//! Network page's banner, address field and notes, and Open address.

use gtk::prelude::*;

use super::geometry::{bounds, laid_out, Bounds};
use crate::locations::Page;
use crate::test_support::harness::{descendants, wait_for_frames, Fixture};

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
    let landing = &test.window.content().landing;
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

#[gtk::test]
fn the_network_page_has_the_banner_address_field_and_notes() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window
        .navigate(Page::Network.uri())
        .expect("the Network page");
    test.wait_for_listing("the Network page");
    wait_for_frames(&test.window, 2);
    let texts = texts_in(&test.window.content().landing);
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
    let entries = descendants::<gtk::Entry>(&test.window.content().landing);
    let address = entries.first().expect("the page has an address field");
    assert_eq!(
        address.placeholder_text().as_deref(),
        Some("\\\\server or \\\\archive-nas")
    );
}

#[gtk::test]
fn open_address_refuses_anything_but_an_smb_server() {
    let fixture = Fixture::standard();
    let test = laid_out(Page::Network.uri());
    test.activate("open-server-address", Some(&fixture.uri()));
    let message = test.window.chrome().toast.text();
    assert_eq!(message.as_str(), "Enter an SMB server or share.");
    assert_eq!(test.window.current_uri().as_deref(), Some(Page::Network.uri()));
}
