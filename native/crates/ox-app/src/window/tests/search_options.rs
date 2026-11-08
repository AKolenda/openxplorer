// SPDX-License-Identifier: AGPL-3.0-only
//! The search strip's options: searching names and contents, narrowing
//! the results by kind, and saving a search to the navigation pane.

use std::fs;

use gtk::prelude::*;
use std::time::{Duration, SystemTime};

use ox_core::search::{DateFacet, KindFacet, SearchIn, SizeFacet};

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

/// A file found by its text stays among the results when its folder is
/// listed again, as when the folder changes while they show: its name
/// alone does not match. It goes once its text no longer matches, or once
/// it is deleted.
///
/// parity: SRCH-036
#[gtk::test]
fn a_file_found_by_its_text_stays_until_its_text_changes_or_it_goes() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("minutes.txt"), "Notes from the meeting").expect("fixture file");
    fs::write(fixture.path("agenda.txt"), "Meeting notes").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    test.window
        .search_strip()
        .choose_search_in(SearchIn::NamesAndContents);
    let shows = |name: &str| test.names().contains(&name.to_owned());
    wait_until("the search of contents to finish", || {
        shows("minutes.txt") && shows("agenda.txt") && !test.status_count().starts_with("Searching")
    });

    fixture.write("More notes.txt");
    wait_until("the folder to be listed again", || shows("More notes.txt"));
    assert!(shows("minutes.txt") && shows("agenda.txt"), "{:?}", test.names());

    fs::write(fixture.path("minutes.txt"), "The budget was approved.").expect("fixture file");
    wait_until("the changed file to go", || !shows("minutes.txt"));
    assert!(shows("agenda.txt"), "the unchanged file stays");

    fs::remove_file(fixture.path("agenda.txt")).expect("fixture file");
    wait_until("the deleted file to go", || !shows("agenda.txt"));
}

/// A file whose text stopped matching comes back once it matches again,
/// and a new file is added once its text matches, while the results of
/// a search of names and contents show: neither name holds the word.
///
/// parity: SRCH-036
#[gtk::test]
fn a_file_whose_text_matches_again_comes_back() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("minutes.txt"), "Notes from the meeting").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    test.window
        .search_strip()
        .choose_search_in(SearchIn::NamesAndContents);
    let shows = |name: &str| test.names().contains(&name.to_owned());
    wait_until("the search of contents to finish", || {
        shows("minutes.txt") && !test.status_count().starts_with("Searching")
    });

    fs::write(fixture.path("minutes.txt"), "The budget was approved.").expect("fixture file");
    wait_until("the changed file to go", || !shows("minutes.txt"));
    fs::write(fixture.path("minutes.txt"), "Notes from the meeting").expect("fixture file");
    wait_until("the file to come back", || shows("minutes.txt"));

    fs::write(fixture.path("memo.txt"), "Notes for later").expect("fixture file");
    wait_until("the new file found by its text", || shows("memo.txt"));
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

    assert!(
        test.window.search_strip().is_visible(),
        "the window searches again"
    );
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

/// Writes `name` with `size` bytes, last modified `days_ago` days ago.
fn write_file(fixture: &Fixture, name: &str, size: usize, days_ago: u64) {
    let path = fixture.path(name);
    fs::write(&path, vec![b'x'; size]).expect("fixture file");
    let modified = SystemTime::now() - Duration::from_secs(days_ago * 24 * 60 * 60);
    fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(modified))
        .expect("fixture time");
}

/// A fictional set of files with known sizes, kinds and dates.
fn sized_fixture() -> Fixture {
    let fixture = Fixture::empty();
    fs::create_dir(fixture.path("plan folder")).expect("fixture folder");
    write_file(&fixture, "plan empty.txt", 0, 0);
    write_file(&fixture, "plan tiny.txt", 2_000, 0);
    write_file(&fixture, "plan small.txt", 40_000, 0);
    write_file(&fixture, "plan medium.txt", 2_000_000, 0);
    write_file(&fixture, "plan old tiny.txt", 2_000, 400);
    write_file(&fixture, "plan photo.png", 2_000, 0);
    fixture
}

/// The size option narrows the results to Windows' size groups, as
/// "Search options" > "Size" does in Windows 11 File Explorer.
///
/// parity: SRCH-043
#[gtk::test]
fn the_size_option_narrows_the_results() {
    let fixture = sized_fixture();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("plan");
    assert_eq!(test.names().len(), 7);
    let strip = test.window.search_strip();

    strip.choose_size(SizeFacet::Empty);
    assert_eq!(test.names(), ["plan empty.txt"]);
    strip.choose_size(SizeFacet::Tiny);
    assert_eq!(
        test.names(),
        ["plan old tiny.txt", "plan photo.png", "plan tiny.txt"]
    );
    strip.choose_size(SizeFacet::Small);
    assert_eq!(test.names(), ["plan small.txt"]);
    strip.choose_size(SizeFacet::Medium);
    assert_eq!(test.names(), ["plan medium.txt"]);
    strip.choose_size(SizeFacet::Large);
    assert!(test.names().is_empty());
    strip.choose_size(SizeFacet::Any);
    assert_eq!(test.names().len(), 7);
}

/// Date, kind and size chosen together narrow the results together.
///
/// parity: SRCH-043
#[gtk::test]
fn date_kind_and_size_combine() {
    let fixture = sized_fixture();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("plan");
    let strip = test.window.search_strip();

    strip.choose_date(DateFacet::Today);
    assert_eq!(test.names().len(), 6, "{:?}", test.names());
    strip.choose_kind(KindFacet::Documents);
    strip.choose_size(SizeFacet::Tiny);
    assert_eq!(test.names(), ["plan tiny.txt"]);
    strip.choose_date(DateFacet::Any);
    assert_eq!(test.names(), ["plan old tiny.txt", "plan tiny.txt"]);
    strip.choose_date(DateFacet::LastWeek);
    assert!(test.names().is_empty());
}

/// `kind:`, `datemodified:` and `size:` typed into the box narrow the
/// results as the lists do, and an option alone lists everything of it.
///
/// parity: SRCH-044
#[gtk::test]
fn typed_options_match_the_lists() {
    let fixture = sized_fixture();
    let test = TestWindow::open(&fixture.uri());
    let strip = test.window.search_strip();
    test.search_for("plan");
    strip.choose_kind(KindFacet::Documents);
    strip.choose_size(SizeFacet::Tiny);
    strip.choose_date(DateFacet::Today);
    let chosen = test.names();
    assert_eq!(chosen, ["plan tiny.txt"]);
    test.window.search_box().clear();
    wait_until("the listing", || test.names().len() == 7);

    test.search_for("plan kind:document size:tiny datemodified:today");
    wait_until("the typed search", || test.names() == chosen);
}

/// An option typed alone lists everything of it; what is not a known
/// option is searched for as text.
///
/// parity: SRCH-044
#[gtk::test]
fn a_typed_option_alone_lists_everything_of_it() {
    let fixture = sized_fixture();
    let test = TestWindow::open(&fixture.uri());
    test.search_for("kind:folder");
    wait_until("folders alone", || test.names() == ["plan folder"]);
    test.search_for("size:medium");
    wait_until("the medium file", || test.names() == ["plan medium.txt"]);
    test.search_for("kind:of");
    wait_until("an unknown option searched as text", || test.names().is_empty());
}

/// A typed option alone in a big folder finds its items past the first
/// 500 names, and only the newest query's results show.
///
/// parity: SRCH-044, SRCH-002
#[gtk::test]
fn a_typed_option_finds_items_in_a_big_folder() {
    let fixture = Fixture::with_files(2_000);
    fs::write(fixture.path("zz holiday.png"), b"\x89PNG\r\n\x1a\n").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.search_for("file 1");
    test.search_for("kind:picture");
    wait_until("the picture", || {
        test.names() == ["zz holiday.png"] && !test.status_count().starts_with("Searching")
    });
}

/// "Save search" adds the search to the navigation pane; opening it from
/// another folder opens its folder and searches it again.
///
/// parity: SRCH-038
#[gtk::test]
fn a_saved_search_opens_its_folder_and_searches_again() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("minutes.txt"), "Notes from the meeting").expect("fixture file");
    let test = TestWindow::open(&fixture.uri());
    test.search_for("notes");
    test.window
        .search_strip()
        .choose_search_in(SearchIn::NamesAndContents);
    wait_until("the search of contents", || test.names().len() == 3);
    let label = "Search for notes in Example projects";

    test.window.search_strip().click_save();

    wait_until("the saved search row", || {
        test.window.sidebar().labels().contains(&label.to_owned())
    });
    test.show(&fixture.uri_of("Documents"));
    test.window.search_strip().choose_search_in(SearchIn::Names);
    let target = (fixture.uri(), "notes".to_owned()).to_variant();
    WidgetExt::activate_action(&test.window, "win.open-saved-search", Some(&target))
        .expect("the window has the action");
    wait_until("the saved search to run with its options", || {
        test.names() == ["minutes.txt", "Notes 2.txt", "Notes 10.txt"]
    });
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}
