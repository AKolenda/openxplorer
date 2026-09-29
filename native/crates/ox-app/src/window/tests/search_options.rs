// SPDX-License-Identifier: AGPL-3.0-only
//! The search strip's options: searching names and contents, and narrowing
//! the results by kind.

use std::fs;

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
    assert_eq!(
        test.window.search_strip().caption(),
        "Current folder + subfolders"
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
}
