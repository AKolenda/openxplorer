// SPDX-License-Identifier: AGPL-3.0-only
//! Addresses typed as Windows 11 File Explorer takes them: a path between
//! double quotes, as "Copy as path" copies it, environment variables, Tab
//! through the suggestions, and one typed history shared by every window
//! and kept across restarts.

use std::fs;
use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};
use ox_core::typed_addresses::{TypedAddresses, TYPED_ADDRESSES_FILE};

use crate::test_support::harness::{wait_for, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::address_bar::AddressMode;

/// A fixture with a folder whose name has a space, as Copy as path quotes
/// it.
fn fixture_with_team_files() -> Fixture {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Team files")).expect("fixture subfolder");
    fs::write(fixture.path("Team files/Tone sample.mp3"), b"ID3").expect("fixture file");
    fixture
}

/// Submits `typed` in the address bar and waits until the tab shows
/// `uri`.
fn go_to(test: &TestWindow, typed: &str, uri: &str) {
    test.activate("location", None);
    test.window.submit_address(typed);
    wait_until(&format!("the address {typed}"), || {
        test.window.current_uri().as_deref() == Some(uri)
    });
}

/// The capture-phase key controller of the address entry, which the
/// suggestions list listens with.
fn list_keys(test: &TestWindow) -> gtk::EventControllerKey {
    let entry = test.window.address_bar().entry();
    entry
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|keys| keys.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the list's keys")
}

/// Presses `key` with `modifiers` on the list's keys; whether it was
/// taken.
fn press(keys: &gtk::EventControllerKey, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
    let no_keycode = 0_u32;
    keys.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &modifiers])
}

/// A path between double quotes, as Copy as path puts it on the
/// clipboard, opens like the path itself; so does a place's name in
/// quotes. Windows 11 File Explorer's address bar takes both.
///
/// parity: NAV-033
#[gtk::test]
fn a_quoted_path_as_copy_as_path_gives_it_opens_the_folder() {
    let fixture = fixture_with_team_files();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the fixture");
    let team_files = fixture.path("Team files").display().to_string();

    go_to(&test, &format!("\"{team_files}\""), &fixture.uri_of("Team files"));
    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
    go_to(&test, "\"Recycle Bin\"", ox_core::location::TRASH_URI);
    go_to(
        &test,
        &format!("  \"{team_files}/\"  "),
        &fixture.uri_of("Team files"),
    );

    let history = test.window.address_bar().typed_history();
    assert_eq!(
        history.first().map(String::as_str),
        Some(format!("\"{team_files}/\"").as_str()),
        "the history keeps what was typed"
    );
}

/// `%NAME%`, as in Windows, and `$NAME` or `${NAME}` stand for the
/// environment variable's value; an unset one is looked up as typed.
///
/// parity: NAV-033
#[gtk::test]
fn environment_variables_in_a_typed_address_are_expanded() {
    let fixture = fixture_with_team_files();
    std::env::set_var("OX_TEST_TYPED_ROOT", fixture.root());
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.wait_for_listing("Documents");
    let team_files = fixture.uri_of("Team files");

    go_to(&test, "%OX_TEST_TYPED_ROOT%/Team files", &team_files);
    go_to(&test, "~", &ox_core::location::file_uri(&glib::home_dir()));
    go_to(&test, "$OX_TEST_TYPED_ROOT/Team files", &team_files);
    go_to(&test, "~", &ox_core::location::file_uri(&glib::home_dir()));
    go_to(&test, "\"${OX_TEST_TYPED_ROOT}/Team files\"", &team_files);

    test.activate("location", None);
    test.window.submit_address("%OX_TEST_NOT_SET%/Team files");
    wait_until("the not-found box", || test.window.has_open_dialog());
    assert_eq!(test.window.current_uri(), Some(team_files));
}

/// Tab with the suggestions shown puts the next one into the address, as
/// Tab completes a path in Windows' address bar, and Shift+Tab the one
/// before; without suggestions Tab moves focus as usual.
///
/// parity: NAV-030
#[gtk::test]
fn tab_takes_the_next_suggestion_and_shift_tab_the_one_before() {
    let fixture = Fixture::standard();
    fixture.write("Notes 3.txt");
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the fixture");
    let address = test.window.address_bar();
    let keys = list_keys(&test);
    test.activate("location", None);
    assert!(
        !press(&keys, gdk::Key::Tab, gdk::ModifierType::empty()),
        "Tab moves on without a list"
    );
    let base = format!("{}/", fixture.root().display());
    let typed = format!("{base}No");
    address.entry().set_text(&typed);
    address.entry().set_position(-1);
    wait_until("the suggestions", || address.suggestions_shown());
    let first = format!("{base}Notes 2.txt");
    let second = format!("{base}Notes 3.txt");
    assert_eq!(address.suggestion_rows()[..2], [first.clone(), second.clone()]);

    assert!(press(&keys, gdk::Key::Tab, gdk::ModifierType::empty()));
    assert_eq!(address.entry().text(), first);
    assert!(press(&keys, gdk::Key::Tab, gdk::ModifierType::empty()));
    assert_eq!(address.entry().text(), second);
    assert!(press(
        &keys,
        gdk::Key::ISO_Left_Tab,
        gdk::ModifierType::SHIFT_MASK
    ));
    assert_eq!(address.entry().text(), first, "Shift+Tab goes back");
    assert!(address.suggestions_shown(), "the list stays open");
    assert!(address.entry().has_focus() || address.entry().focus_child().is_some());
    address.entry().emit_by_name::<()>("activate", &[]);
    wait_until("the chosen file opened", || {
        test.context.recorded_launches() == [fixture.uri_of("Notes 2.txt")]
    });
}

/// Every window lists the same typed addresses, most recent first, and a
/// new window, as after a restart, still lists them: Windows keeps them
/// for the user. Clear history forgets them everywhere and on disk.
///
/// parity: NAV-043
#[gtk::test]
fn typed_addresses_are_shared_by_every_window_and_kept_for_the_next_start() {
    let fixture = fixture_with_team_files();
    let first = TestWindow::open(&fixture.uri());
    first.wait_for_listing("the fixture");
    let second = first.open_beside(&fixture.uri());
    let team_files = fixture.path("Team files").display().to_string();
    let documents = fixture.path("Documents").display().to_string();

    go_to(&first, &team_files, &fixture.uri_of("Team files"));
    go_to(&second, &documents, &fixture.uri_of("Documents"));

    for window in [&first, &second] {
        assert_eq!(
            window.window.address_bar().typed_history(),
            [documents.clone(), team_files.clone()]
        );
    }
    let kept = TypedAddresses::open(first.settings_directory());
    assert_eq!(kept.addresses(), [documents.clone(), team_files.clone()]);

    let stored = first.settings_directory().join(TYPED_ADDRESSES_FILE);
    let restarted = TestWindow::open_prepared(&fixture.uri(), |context| {
        fs::copy(&stored, context.settings_directory().join(TYPED_ADDRESSES_FILE))
            .expect("the history is copied into the new settings");
    });
    restarted.activate("address-history", None);
    assert_eq!(
        restarted.window.address_bar().suggestion_rows(),
        [documents, team_files],
        "a new start lists them"
    );

    first.window.clear_explorer_history();
    assert!(second.window.address_bar().typed_history().is_empty());
    assert!(TypedAddresses::open(first.settings_directory())
        .addresses()
        .is_empty());
}

/// The screen point of the middle of `widget`.
fn on_screen(pointer: &mut Pointer, widget: &impl IsA<gtk::Widget>) -> (i32, i32) {
    let middle = (f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0);
    screen_point(pointer, widget, middle, (8, 8))
}

/// With a real pointer, Ctrl held as a crumb is dragged onto another
/// tab copies its folder there and opens no tab, as a Ctrl+drag of a
/// folder does; a Ctrl+click without a drag still opens the crumb in a
/// background tab (NAV-019).
///
/// parity: DND-001, NAV-019
#[gtk::test]
fn a_real_ctrl_drag_of_a_crumb_copies_its_folder_and_opens_no_tab() {
    let fixture = fixture_with_team_files();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    test.window
        .add_tab(&fixture.uri_of("Team files"))
        .expect("a second tab");
    wait_until("Team files", || {
        test.window.current_uri() == Some(fixture.uri_of("Team files"))
    });
    test.wait_for_listing("Team files");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let crumb = test
        .window
        .address_bar()
        .crumb_buttons()
        .pop()
        .expect("the Team files crumb");
    let from = on_screen(&mut pointer, &crumb);
    let documents_tab = crate::window::widget_tree::children(&test.window.tab_strip().tab_list())
        .next()
        .expect("the Documents tab");
    let to = on_screen(&mut pointer, &documents_tab);

    pointer.move_to(from.0, from.1);
    pointer.hold_key(gdk::Key::Control_L);
    pointer.press(Button::Primary);
    pointer.glide(from, (from.0 + 12, from.1 + 12), 4);
    pointer.glide((from.0 + 12, from.1 + 12), to, 12);
    wait_for(Duration::from_millis(600));
    pointer.release(Button::Primary);
    pointer.let_go_key(gdk::Key::Control_L);

    wait_until("the copy", || {
        fixture.path("Documents/Team files/Tone sample.mp3").is_file()
    });
    assert!(
        fixture.path("Team files/Tone sample.mp3").is_file(),
        "copied, not moved"
    );
    wait_for(Duration::from_millis(300));
    assert_eq!(test.window.tab_count(), 2, "the drag opened no tab");

    pointer.move_to(from.0, from.1);
    pointer.hold_key(gdk::Key::Control_L);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    pointer.let_go_key(gdk::Key::Control_L);
    wait_until("the Ctrl+click's tab", || test.window.tab_count() == 3);
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri_of("Team files")),
        "in the background"
    );
}

/// A home with the standard folders, a folder elsewhere holding its own
/// Documents, and its settings folder.
struct KnownFolderHome {
    _root: tempfile::TempDir,
    home: std::path::PathBuf,
    config: std::path::PathBuf,
    work: std::path::PathBuf,
}

impl KnownFolderHome {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room");
        let base = fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        let work = base.join("work");
        for folder in [
            "Desktop",
            "Documents/Letters",
            "Downloads",
            "Music",
            "Pictures",
            "Videos",
        ] {
            fs::create_dir_all(home.join(folder)).expect("a folder");
        }
        fs::create_dir_all(work.join("Documents")).expect("a folder");
        fs::create_dir_all(&config).expect("a folder");
        fs::write(home.join("Music/Tone sample.mp3"), b"ID3").expect("a file");
        Self {
            _root: root,
            home,
            config,
            work,
        }
    }

    fn locations(&self) -> ox_core::places::FolderLocations {
        ox_core::places::FolderLocations::new(self.home.clone(), &self.config)
    }

    fn uri_of(&self, name: &str) -> String {
        ox_core::location::file_uri(&self.home.join(name))
    }
}

/// The crumbs' labels.
fn crumb_labels(test: &TestWindow) -> Vec<String> {
    let crumbs = test.window.address_bar().crumb_buttons();
    crumbs
        .iter()
        .filter_map(|crumb| crumb.label().map(String::from))
        .collect()
}

/// As in Windows 11 File Explorer (checked on build 26200), typing
/// "Documents" in another folder opens the user's Documents, and the
/// address bar shows it as one "Documents" crumb; so for Desktop,
/// Downloads, Music, Pictures and Videos, in any case. The standard
/// folder wins over a folder of that name in the current folder. Its
/// subfolders' crumbs start at it.
///
/// parity: NAV-033, NAV-024
#[gtk::test]
fn typing_a_standard_folder_name_opens_it_and_its_crumbs_start_there() {
    let home = KnownFolderHome::new();
    let work = ox_core::location::file_uri(&home.work);
    let test = TestWindow::open_with_standard_folders(&work, home.locations(), |_| {});
    test.wait_for_listing("work");

    go_to(&test, "Documents", &home.uri_of("Documents"));
    assert_eq!(crumb_labels(&test), ["Documents"]);
    test.show(&home.uri_of("Documents/Letters"));
    assert_eq!(crumb_labels(&test), ["Documents", "Letters"]);
    for (typed, folder) in [
        ("music", "Music"),
        ("  Downloads  ", "Downloads"),
        ("PICTURES", "Pictures"),
        ("Videos", "Videos"),
        ("Desktop", "Desktop"),
    ] {
        go_to(&test, typed, &home.uri_of(folder));
        assert_eq!(crumb_labels(&test), [folder], "{typed}");
    }
    let history = test.window.address_bar().typed_history();
    assert_eq!(history.first().map(String::as_str), Some("Desktop"));
}
