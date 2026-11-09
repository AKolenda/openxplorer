// SPDX-License-Identifier: AGPL-3.0-only
//! Home laid out as Windows 11 File Explorer's Home, by default; the
//! setting "Open the home folder on Home" (off by default) brings back
//! the home folder on Home (NAV-038). Quick access holds the
//! navigation pane's pinned folders as tiles, and a Recent / Favorites
//! switcher lists the desktop's recently used files (`GVfs` `recent:///`)
//! and the files added to Favorites. Every name below is fictional.

use std::fs;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::context_menus::position_of;
use crate::app_context::add_to_desktop_history;
use crate::test_support::harness::{wait_until, Fixture, TestWindow};
use crate::window::home_page;

/// The Home page's address.
const HOME_PAGE: &str = "ox:home";

/// Sets "Open the home folder on Home" for `test`'s settings.
fn open_the_home_folder_on_home(test: &TestWindow, on: bool) {
    let update = PreferencesUpdate {
        home_opens_folder: Some(on),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the setting to be read", || {
        test.context.settings_data().preferences.home_opens_folder == on
    });
}

/// A window on the fixture showing the Home page, as Home opens by
/// default.
fn home_window(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    test.activate("home", None);
    wait_until("the Home page", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });
    test
}

/// The Home page's body in the active pane.
fn body(test: &TestWindow) -> &gtk::Box {
    test.window.folder_pane().landing()
}

/// Emits `key-pressed` with `key` on the first key controller of
/// `widget`; whether a handler took it.
fn press_key(widget: &impl IsA<gtk::Widget>, key: gtk::gdk::Key) -> bool {
    let controller = widget
        .observe_controllers()
        .into_iter()
        .find_map(|controller| controller.ok().and_downcast::<gtk::EventControllerKey>())
        .expect("the widget has a key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>(
        "key-pressed",
        &[&key.into_glib(), &no_keycode, &gtk::gdk::ModifierType::empty()],
    )
}

/// By default the navigation pane's Home row, Alt+Home and a new tab
/// open the Home page, titled Home, with Quick access and the Recent and
/// Favorites switcher; "Open the home folder on Home" brings back the
/// home folder there (the owner's NAV-038).
///
/// parity: NAV-038, HOME-012
#[gtk::test]
fn home_is_windows_home_by_default_and_the_setting_opens_the_home_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let home_row = test
        .window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("Home comes first");
    home_row.activate();
    wait_until("the Home page", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });

    assert_eq!(test.window.title().as_deref(), Some("Home — OpenXplorer"));
    assert_eq!(
        home_page::section_titles(body(&test)),
        ["Quick access", "Recent", "Favorites"]
    );
    assert_eq!(home_page::shown_tab_title(body(&test)).as_deref(), Some("Recent"));
    test.activate("new-tab", None);
    wait_until("a new tab on Home", || {
        test.window.tab_count() == 2 && test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });

    open_the_home_folder_on_home(&test, true);
    test.activate("home", None);
    test.wait_for_listing("the home folder");
    let home_folder = test.window.current_uri().expect("a location");
    assert!(home_folder.starts_with("file:///"), "{home_folder}");
    assert_eq!(test.window.title().as_deref(), Some("Home — OpenXplorer"));
}

/// Quick access on Home is the navigation pane's list: a folder pinned
/// from its menu gets a tile, opening the tile opens the folder, its
/// menu is the pin's (with Unpin from Quick access), and unpinning
/// removes the tile.
///
/// parity: HOME-013
#[gtk::test]
fn a_pinned_folder_has_a_tile_that_opens_it_until_it_is_unpinned() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let test = home_window(&fixture);
    let documents = fixture.uri_of("Projects");
    assert!(!home_page::tile_names(body(&test)).contains(&"Projects".to_owned()));

    test.activate("pin-location", Some(&documents));
    wait_until("the Projects tile", || {
        home_page::tile_names(body(&test)).contains(&"Projects".to_owned())
    });
    let labels = home_page::menu_labels(body(&test), "Projects");
    assert!(labels.contains(&"Open".to_owned()), "{labels:?}");
    assert!(
        labels.contains(&"Unpin from Quick access".to_owned()),
        "{labels:?}"
    );

    home_page::open_item(body(&test), "Projects");
    test.wait_for_listing("Projects");
    assert_eq!(test.window.current_uri(), Some(documents.clone()));
    test.activate("back", None);
    wait_until("Back to Home", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });

    test.activate("unpin", Some(&documents));
    wait_until("the tile to go", || {
        !home_page::tile_names(body(&test)).contains(&"Projects".to_owned())
    });
}

/// A file added to Favorites is listed under Favorites, opens the real
/// file, and goes once removed; an empty list says so.
///
/// parity: HOME-014
#[gtk::test]
fn a_favorite_file_is_listed_opened_and_removed() {
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    home_page::choose_tab(body(&test), "Favorites");
    assert!(home_page::list_names(body(&test)).is_empty());
    assert!(
        home_page::empty_text(body(&test)).is_some(),
        "an empty Favorites says so"
    );

    let notes = fixture.uri_of("Notes 2.txt");
    test.activate("add-to-favorites", Some(&notes));
    wait_until("the favorite", || {
        home_page::list_names(body(&test)) == ["Notes 2.txt"]
    });
    assert_eq!(
        test.context.settings_data().preferences.favorites,
        std::slice::from_ref(&notes)
    );
    let labels = home_page::menu_labels(body(&test), "Notes 2.txt");
    assert!(labels.contains(&"Open file location".to_owned()), "{labels:?}");
    assert!(labels.contains(&"Remove from Favorites".to_owned()), "{labels:?}");

    home_page::open_item(body(&test), "Notes 2.txt");
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), std::slice::from_ref(&notes));

    test.activate("remove-from-favorites", Some(&notes));
    wait_until("the favorite to go", || {
        home_page::list_names(body(&test)).is_empty()
    });
    assert!(test.context.settings_data().preferences.favorites.is_empty());
}

/// A file's right-click menu offers Add to Favorites while Home shows
/// Favorites, not with "Open the home folder on Home", and Remove from
/// Favorites once it is one.
///
/// parity: HOME-014
#[gtk::test]
fn a_files_menu_adds_it_to_favorites_while_home_shows_them() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let labels = |test: &TestWindow| {
        test.window.right_click(Some(position_of(test, "Notes 2.txt")));
        let menu = test.window.context_menu();
        let labels = menu.row_labels();
        menu.popdown();
        labels
    };
    assert!(labels(&test).contains(&"Add to Favorites".to_owned()));
    open_the_home_folder_on_home(&test, true);
    assert!(!labels(&test).contains(&"Add to Favorites".to_owned()));
    open_the_home_folder_on_home(&test, false);
    test.activate("add-to-favorites", Some(&fixture.uri_of("Notes 2.txt")));
    wait_until("the favorite", || {
        !test.context.settings_data().preferences.favorites.is_empty()
    });
    assert!(labels(&test).contains(&"Remove from Favorites".to_owned()));
}

/// Recent lists the desktop's recently used files from `GVfs`, as the
/// navigation pane's Recent files does; a row opens the real file, and
/// Open file location opens its folder with it selected.
///
/// parity: HOME-015
#[gtk::test]
fn a_recent_file_opens_the_real_file_and_its_location() {
    super::file_ops_support::require_private_trash();
    let schemes = gtk::gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "recent"),
        "this test needs GVfs with its Recent backend; found {schemes:?}"
    );
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/plan.txt"), b"Fictional plan\n").expect("fixture file");
    let plan = fixture.uri_of("Documents/plan.txt");
    add_to_desktop_history(&plan, "text/plain");
    let test = home_window(&fixture);

    wait_until("the recent file", || {
        home_page::list_names(body(&test)).contains(&"plan.txt".to_owned())
    });
    let labels = home_page::menu_labels(body(&test), "plan.txt");
    assert!(labels.contains(&"Open file location".to_owned()), "{labels:?}");
    assert!(labels.contains(&"Add to Favorites".to_owned()), "{labels:?}");

    home_page::open_item(body(&test), "plan.txt");
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), std::slice::from_ref(&plan));

    test.activate("open-file-location-of", Some(&plan));
    test.wait_for_listing("the file's folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.selected_names(), ["plan.txt"]);
}

/// Quick access and the Recent / Favorites list fold and unfold from
/// their chevrons, by a click and by Left and Right on the keyboard;
/// F5 draws the page again and keeps a folded section folded.
///
/// parity: HOME-016
#[gtk::test]
fn each_section_folds_by_click_and_by_keyboard() {
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    wait_until("the Documents tile", || {
        !home_page::tile_names(body(&test)).is_empty()
    });
    let folds = home_page::fold_buttons(body(&test));
    assert_eq!(folds.len(), 2, "Quick access and the list fold");
    for (index, fold) in folds.iter().enumerate() {
        assert!(!home_page::is_folded(body(&test), index));
        fold.emit_clicked();
        assert!(home_page::is_folded(body(&test), index), "a click folds {index}");
        fold.emit_clicked();
        assert!(
            !home_page::is_folded(body(&test), index),
            "a click unfolds {index}"
        );
        assert!(press_key(fold, gtk::gdk::Key::Left), "Left on {index}");
        assert!(home_page::is_folded(body(&test), index), "Left folds {index}");
        assert!(press_key(fold, gtk::gdk::Key::Right), "Right on {index}");
        assert!(!home_page::is_folded(body(&test), index), "Right unfolds {index}");
    }

    home_page::fold_buttons(body(&test))[0].emit_clicked();
    test.activate("refresh", None);
    wait_until("the page drawn again", || {
        home_page::fold_buttons(body(&test)).len() == 2
    });
    assert!(
        home_page::is_folded(body(&test), 0),
        "F5 keeps Quick access folded"
    );
}

/// Quick access tiles are laid out as Windows 11's: a 48 px folder, then
/// three lines: the name, where the folder is (the parent folder's name,
/// "Home" for a folder in the home folder; never Windows' cloud status)
/// and a pin glyph for a pinned folder.
///
/// parity: HOME-013
#[gtk::test]
fn a_quick_access_tile_shows_its_name_its_location_and_a_pin() {
    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Projects")));
    wait_until("the Projects tile", || {
        home_page::tile_names(body(&test)).contains(&"Projects".to_owned())
    });

    assert_eq!(
        home_page::tile_lines(body(&test), "Projects"),
        ["Projects", "Example projects"]
    );
    assert!(home_page::tile_has_pin(body(&test), "Projects"), "a pinned folder shows the pin");
    assert_eq!(home_page::tile_icon_size(body(&test), "Projects"), 48);
}

/// The Recent and Favorites switcher has a glyph on each of its tabs
/// (a clock for Recent, a star for Favorites), and the list under it is
/// a Details list with Name, Date accessed and File location columns.
///
/// parity: HOME-015
#[gtk::test]
fn the_list_has_name_date_accessed_and_file_location_columns() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/plan.txt"), b"Fictional plan\n").expect("fixture file");
    add_to_desktop_history(&fixture.uri_of("Documents/plan.txt"), "text/plain");
    let test = home_window(&fixture);
    wait_until("the recent file", || {
        home_page::list_names(body(&test)).contains(&"plan.txt".to_owned())
    });

    assert!(home_page::tab_has_icon(body(&test), "Recent"));
    assert!(home_page::tab_has_icon(body(&test), "Favorites"));
    assert_eq!(
        home_page::list_headers(body(&test)),
        ["Name", "Date accessed", "File location"]
    );
    let cells = home_page::row_cells(body(&test), "plan.txt");
    assert_eq!(cells.len(), 3, "{cells:?}");
    assert_eq!(cells[0], "plan.txt");
    assert!(!cells[1].is_empty() && cells[1] != "—", "a date: {cells:?}");
    assert!(cells[2].ends_with("Example projects/Documents"), "{cells:?}");
}
