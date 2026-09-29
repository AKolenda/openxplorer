// SPDX-License-Identifier: AGPL-3.0-only
//! The search strip's options: searching names and contents, narrowing
//! the results by kind, and saving a search to the navigation pane.

use std::fs;

use gtk::prelude::*;
use ox_core::search::{KindFacet, SearchIn};

use crate::test_support::harness::{wait_until, Fixture, TestWindow};

/// "Names and contents" finds a file by its text, even in an indexed
/// folder, whose cache holds names only.
///
/// parity: SRCH-036
#[gtk::test]
fn names_and_contents_finds_files_by_their_text() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("minutes.txt"), "The budget was approved.").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("budget");
    assert!(test.names().is_empty(), "no name holds the word");

    test.window
        .search_strip()
        .choose_search_in(SearchIn::NamesAndContents);

    wait_until("the search of contents", || test.names() == ["minutes.txt"]);
    let strip = test.window.search_strip();
    assert_eq!(strip.caption(), "Current folder + subfolders");
    assert!(
        !strip.offers_to_cache_folder(),
        "a cached folder is not offered for caching, which would switch it off"
    );
}

/// The kind option narrows the results at once, and ending the search
/// forgets it.
///
/// parity: SRCH-037
#[gtk::test]
fn the_kind_option_narrows_the_results_until_the_search_ends() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("notes.png"), b"\x89PNG\r\n\x1a\n").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt", "notes.png"]);

    test.window.search_strip().choose_kind(KindFacet::Images);

    assert_eq!(test.names(), ["notes.png"]);
    test.window.search_strip().choose_kind(KindFacet::Documents);
    assert_eq!(test.names(), ["Notes 2.txt", "Notes 10.txt"]);
    test.window.search_box().clear();
    wait_until("the listing", || test.names().len() > 3);
    assert!(test.names().contains(&"notes.png".to_owned()));
    test.search_for("notes");
    assert_eq!(
        test.names(),
        ["Notes 2.txt", "Notes 10.txt", "notes.png"],
        "clearing the box forgot the kind"
    );

    test.window.search_strip().choose_kind(KindFacet::Images);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the other folder");
    test.search_for("report");

    assert!(test.window.search_strip().is_visible(), "the window searches again");
}

/// The kind option also narrows the cache's results, whose type is
/// guessed from their names.
///
/// parity: SRCH-037
#[gtk::test]
fn the_kind_option_narrows_cached_results() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/scan notes.png"), b"\x89PNG\r\n\x1a\n").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    test.index_folder(&fixture.uri());
    test.search_for("notes");
    assert_eq!(test.window.search_strip().caption(), "Cached names & paths");

    test.window.search_strip().choose_kind(KindFacet::Images);

    assert_eq!(test.names(), ["scan notes.png"]);
}

/// "Save search" adds the search to the navigation pane; opening it from
/// another folder opens its folder and searches it again.
///
/// parity: SRCH-038
#[gtk::test]
fn a_saved_search_opens_its_folder_and_searches_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    let label = "Search for notes in Example projects";

    test.window.search_strip().click_save();

    wait_until("the saved search row", || {
        test.window.sidebar().labels().contains(&label.to_owned())
    });
    test.show(&fixture.uri_of("Documents"));
    let target = (fixture.uri(), "notes".to_owned()).to_variant();
    WidgetExt::activate_action(&test.window, "win.open-saved-search", Some(&target))
        .expect("the window has the action");
    wait_until("the saved search to run", || {
        test.names() == ["Notes 2.txt", "Notes 10.txt"]
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}
