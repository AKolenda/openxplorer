// SPDX-License-Identifier: AGPL-3.0-only
//! Home laid out as Windows 11 File Explorer's Home while "Open Quick
//! access on Home" is on; off, the default, Home opens the home folder
//! (NAV-038). Quick access holds the
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

/// Sets "Open Quick access on Home" for `test`'s settings.
fn open_quick_access_on_home(test: &TestWindow, on: bool) {
    let update = PreferencesUpdate {
        home_quick_access: Some(on),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the setting to be read", || {
        test.context.settings_data().preferences.home_quick_access == on
    });
}

/// A window on the fixture showing the Home page, with "Open Quick
/// access on Home" on.
fn home_window(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
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

/// By default Home opens the home folder (the owner's NAV-038). With
/// "Open Quick access on Home" on, the navigation pane's Home row,
/// Alt+Home and a new tab open the Home page, titled Home, with Quick
/// access and the Recent and Favorites switcher.
///
/// parity: NAV-038, HOME-012
#[gtk::test]
fn home_is_the_home_folder_by_default_and_the_setting_opens_windows_home() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("home", None);
    test.wait_for_listing("the home folder");
    let home_folder = test.window.current_uri().expect("a location");
    assert!(home_folder.starts_with("file:///"), "{home_folder}");
    assert_eq!(test.window.title().as_deref(), Some("Home — OpenXplorer"));

    open_quick_access_on_home(&test, true);
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
        home_page::list_names(body(&test)) == ["Notes 2"]
    });
    assert_eq!(
        test.context.settings_data().preferences.favorites,
        std::slice::from_ref(&notes)
    );
    let labels = home_page::menu_labels(body(&test), "Notes 2");
    assert!(labels.contains(&"Open file location".to_owned()), "{labels:?}");
    assert!(labels.contains(&"Remove from Favorites".to_owned()), "{labels:?}");

    home_page::open_item(body(&test), "Notes 2");
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

/// A file's right-click menu offers Add to Favorites only while Home
/// shows Favorites ("Open Quick access on Home"), and Remove from
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
    assert!(!labels(&test).contains(&"Add to Favorites".to_owned()));
    open_quick_access_on_home(&test, true);
    assert!(labels(&test).contains(&"Add to Favorites".to_owned()));
    test.activate("add-to-favorites", Some(&fixture.uri_of("Notes 2.txt")));
    wait_until("the favorite", || {
        !test.context.settings_data().preferences.favorites.is_empty()
    });
    assert!(labels(&test).contains(&"Remove from Favorites".to_owned()));
}

/// A file menu's Add to Favorites, opened while "Open Quick access on
/// Home" was on, changes nothing once the option is off: whether this
/// window already knows it (the command checks) or another window turned
/// it off in the settings file meanwhile (the saved change checks).
///
/// parity: HOME-014
#[gtk::test]
fn a_menu_left_open_adds_no_favorite_once_the_option_is_off() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
    // Waits until every change queued before this one has been saved.
    let settle_changes = |test: &TestWindow| {
        let saved = std::rc::Rc::new(std::cell::Cell::new(false));
        let done = saved.clone();
        test.context
            .update_preferences(PreferencesUpdate::default(), move |result| {
                result.expect("an empty change saves");
                done.set(true);
            });
        wait_until("the queued changes", || saved.get());
    };
    let favorites_on_disk = |test: &TestWindow| {
        Settings::open(test.settings_directory())
            .snapshot()
            .preferences
            .favorites
    };

    // Another window turns the option off; this one has not read it yet.
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    let off = PreferencesUpdate {
        home_quick_access: Some(false),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&off)
        .expect("turned off elsewhere");
    assert!(test.context.settings_data().preferences.home_quick_access);
    menu.row("Add to Favorites").emit_activate();
    settle_changes(&test);
    assert!(favorites_on_disk(&test).is_empty(), "nothing saved");

    // This window has read that the option is off.
    open_quick_access_on_home(&test, true);
    test.window.right_click(Some(position_of(&test, "Notes 2.txt")));
    let menu = test.window.context_menu();
    open_quick_access_on_home(&test, false);
    menu.row("Add to Favorites").emit_activate();
    settle_changes(&test);
    assert!(favorites_on_disk(&test).is_empty(), "nothing saved");
    assert!(test.context.settings_data().preferences.favorites.is_empty());
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
        home_page::list_names(body(&test)).contains(&"plan".to_owned())
    });
    let labels = home_page::menu_labels(body(&test), "plan");
    assert!(labels.contains(&"Open file location".to_owned()), "{labels:?}");
    assert!(labels.contains(&"Add to Favorites".to_owned()), "{labels:?}");

    home_page::open_item(body(&test), "plan");
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), std::slice::from_ref(&plan));

    test.activate("open-file-location-of", Some(&plan));
    test.wait_for_listing("the file's folder");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    assert_eq!(test.selected_names(), ["plan.txt"]);
}

/// Recent and Favorites name files as the file list does: without the
/// extension of a known type while extensions are hidden, as by default
/// in Windows, and whole once View > Show > File name extensions is on.
/// The row still opens the real file.
///
/// parity: HOME-014, HOME-015, VIEW-069
#[gtk::test]
fn recent_and_favorites_follow_the_file_name_extensions_setting() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/plan.txt"), b"Fictional plan\n").expect("fixture file");
    let plan = fixture.uri_of("Documents/plan.txt");
    add_to_desktop_history(&plan, "text/plain");
    let test = home_window(&fixture);
    wait_until("the recent file, without its extension", || {
        home_page::list_names(body(&test)).contains(&"plan".to_owned())
    });
    assert!(
        !home_page::list_names(body(&test)).contains(&"plan.txt".to_owned()),
        "{:?}",
        home_page::list_names(body(&test))
    );
    test.activate("add-to-favorites", Some(&fixture.uri_of("Notes 2.txt")));
    home_page::choose_tab(body(&test), "Favorites");
    wait_until("the favorite, without its extension", || {
        home_page::list_names(body(&test)) == ["Notes 2"]
    });

    test.activate("file-name-extensions", None);
    wait_until("the favorite, whole", || {
        home_page::list_names(body(&test)) == ["Notes 2.txt"]
    });
    home_page::choose_tab(body(&test), "Recent");
    wait_until("the recent file, whole", || {
        home_page::list_names(body(&test)).contains(&"plan.txt".to_owned())
    });
    home_page::open_item(body(&test), "plan.txt");
    wait_until("the file to be opened", || {
        !test.context.recorded_launches().is_empty()
    });
    assert_eq!(test.context.recorded_launches(), std::slice::from_ref(&plan));
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
    assert!(
        home_page::tile_has_pin(body(&test), "Projects"),
        "a pinned folder shows the pin"
    );
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
        home_page::list_names(body(&test)).contains(&"plan".to_owned())
    });

    assert!(home_page::tab_has_icon(body(&test), "Recent"));
    assert!(home_page::tab_has_icon(body(&test), "Favorites"));
    assert_eq!(
        home_page::list_headers(body(&test)),
        ["Name", "Date accessed", "File location"]
    );
    let cells = home_page::row_cells(body(&test), "plan");
    assert_eq!(cells.len(), 3, "{cells:?}");
    assert_eq!(cells[0], "plan");
    assert!(!cells[1].is_empty() && cells[1] != "—", "a date: {cells:?}");
    assert!(cells[2].ends_with("Example projects/Documents"), "{cells:?}");
}

/// Clicks the navigation pane's Home row with the real pointer, then
/// waits for the Home page with the Home row selected.
fn click_the_home_row(test: &TestWindow) {
    use crate::test_support::pointer::{screen_point, Button, Pointer};

    let mut pointer = Pointer::connect();
    let row = test
        .window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("Home comes first");
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    let middle = (f64::from(row.width()) / 2.0, f64::from(row.height()) / 2.0);
    let (x, y) = screen_point(&mut pointer, &row, middle, (8, 8));
    pointer.move_to(x, y);
    pointer.press(Button::Primary);
    pointer.release(Button::Primary);
    wait_until("the Home page", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });
    let selected = test.window.sidebar().list().selected_row();
    assert_eq!(
        selected.map(|row| row.index()),
        Some(0),
        "the Home row is selected"
    );
    assert_eq!(
        home_page::section_titles(body(test)),
        ["Quick access", "Recent", "Favorites"]
    );
}

/// A real click on the navigation pane's Home row opens the Home page
/// while "Open Quick access on Home" is on, whatever else the settings
/// file holds (This PC as the startup folder among other choices). With
/// no settings file, or an empty one, it opens the home folder.
///
/// parity: NAV-038, HOME-012
#[gtk::test]
fn a_real_click_on_the_home_row_opens_the_home_page_whatever_else_the_settings_hold() {
    let fixture = Fixture::standard();

    let test = TestWindow::open(&fixture.uri());
    fs::write(test.settings_directory().join("settings.json"), b"").expect("an empty settings file");
    test.context.reload_settings();
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    test.window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("Home comes first")
        .activate();
    test.wait_for_listing("the home folder");
    let home_folder = test.window.current_uri().expect("a location");
    assert!(home_folder.starts_with("file:///"), "{home_folder}");

    let test = TestWindow::open(&fixture.uri());
    let others = PreferencesUpdate {
        home_quick_access: Some(true),
        startup_folder: Some(ox_core::location::PC_URI.to_owned()),
        show_hidden: Some(true),
        show_full_path: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&others)
        .expect("the settings file takes the choices");
    test.context.reload_settings();
    wait_until("the choices to be read", || {
        test.context.settings_data().preferences.show_hidden
    });
    assert!(test.context.settings_data().preferences.home_quick_access);
    click_the_home_row(&test);
}

/// The address bar leads to the same Home as the navigation pane: with
/// "Open Quick access on Home" on, the
/// chevron's "Home" and the Home page's own address, "Home", typed or
/// left as it is and entered, open the Home page.
///
/// parity: HOME-012
#[gtk::test]
fn the_address_bars_home_opens_the_home_page() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
    test.window.submit_address("Home");
    wait_until("the Home page from a typed Home", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });

    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    assert_eq!(
        test.window.top_level_places_targets().first().map(String::as_str),
        Some(HOME_PAGE),
        "the chevron's Home"
    );
}

/// Home's spacing, measured on Windows 11's Home (1920x1080, 1x, normal
/// view): Recent rows 36 pixels apart; 51 pixels from the pills' middle
/// to the column headers' and 50 from there to the first row's; Name,
/// Date accessed and File location 484 and 298 pixels apart from the
/// left, not stretched to the edge; Quick access tiles 60 pixels high,
/// about 37 pixels from the heading's middle to the first tile's name and
/// 45 from the last tiles to the pills; the chevrons about 30 pixels in
/// from the pane's edge.
///
/// parity: HOME-013, HOME-015
#[gtk::test]
fn home_is_spaced_as_windows_11_home() {
    use super::geometry::bounds;

    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/plan.txt"), b"Fictional plan\n").expect("fixture file");
    fs::write(fixture.path("Documents/list.txt"), b"Fictional list\n").expect("fixture file");
    add_to_desktop_history(&fixture.uri_of("Documents/plan.txt"), "text/plain");
    add_to_desktop_history(&fixture.uri_of("Documents/list.txt"), "text/plain");
    let test = home_window(&fixture);
    test.window.set_default_size(1600, 1000);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    wait_until("the tiles and two recent files", || {
        !home_page::tile_names(body(&test)).is_empty() && home_page::list_names(body(&test)).len() == 2
    });
    crate::test_support::harness::wait_for_frames(&test.window, 4);
    let metrics = home_page::metrics(body(&test));
    let page = bounds(&test, body(&test));
    let misses = std::cell::RefCell::new(Vec::new());
    let near = |what: &str, got: i32, want: i32, slack: i32| {
        if (got - want).abs() > slack {
            misses
                .borrow_mut()
                .push(format!("{what}: {got}, want {want}±{slack}"));
        }
    };

    near(
        "row pitch",
        bounds(&test, &metrics.rows[1]).y - bounds(&test, &metrics.rows[0]).y,
        36,
        0,
    );
    let middle = |widget: &gtk::Widget| {
        let place = bounds(&test, widget);
        place.y + place.height / 2
    };
    near(
        "pills to headers",
        middle(&metrics.header) - middle(&metrics.pills),
        51,
        3,
    );
    near(
        "headers to the first row",
        middle(&metrics.rows[0]) - middle(&metrics.header),
        36,
        1,
    );
    let (name, date, location) = (
        bounds(&test, &metrics.columns[0]).x,
        bounds(&test, &metrics.columns[1]).x,
        bounds(&test, &metrics.columns[2]).x,
    );
    near("Name to Date accessed", date - name, 484, 2);
    near("Date accessed to File location", location - date, 298, 2);
    near("Name from the pane's edge", name - page.x, 90, 4);
    let cells = home_page::row_cell_widgets(body(&test), "plan");
    let icon = home_page::row_icon(body(&test), "plan");
    near("a row's icon under Name", bounds(&test, &icon).x, name, 1);
    near(
        "a row's name right of its icon",
        bounds(&test, &cells[0]).x - name,
        36,
        4,
    );
    near(
        "a row's date under Date accessed",
        bounds(&test, &cells[1]).x,
        date,
        1,
    );
    near(
        "a row's location under File location",
        bounds(&test, &cells[2]).x,
        location,
        1,
    );
    near("tile height", bounds(&test, &metrics.tiles[0]).height, 60, 0);
    near(
        "heading to the first tile's name",
        middle(&metrics.first_tile_name) - middle(&metrics.heading),
        37,
        4,
    );
    let last_tile = metrics.tiles.last().expect("a tile");
    near(
        "tiles to the pills",
        bounds(&test, &metrics.pills).y - bounds(&test, last_tile).bottom(),
        45,
        4,
    );
    near(
        "chevron from the pane's edge",
        bounds(&test, &metrics.chevron).x - page.x,
        30,
        4,
    );
    let folder = home_page::tile_folder(body(&test), "Documents");
    near(
        "a tile's folder from the pane's edge",
        bounds(&test, &folder).x - page.x,
        92,
        4,
    );
    assert!(misses.borrow().is_empty(), "{:#?}", misses.borrow());
    let shown = home_page::tile_names(body(&test)).len() + home_page::list_names(body(&test)).len();
    assert_eq!(
        test.window.status_bar().texts().0,
        format!("{shown} items"),
        "Home counts its tiles and rows"
    );
}

/// Home is laid out evenly in the normal and the Compact view: the
/// list's first row comes one row under the column titles, with no empty
/// band between them, and a Quick access tile's pin never touches the
/// name of the tile below it.
///
/// parity: HOME-013, HOME-015, HOME-016
#[gtk::test]
fn home_has_no_band_under_the_titles_and_its_tiles_do_not_touch() {
    use super::geometry::bounds;

    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/plan.txt"), b"Fictional plan\n").expect("fixture file");
    add_to_desktop_history(&fixture.uri_of("Documents/plan.txt"), "text/plain");
    let test = home_window(&fixture);
    // Narrow enough for one tile a row, so the tiles stand one above the
    // other.
    test.window.set_default_size(700, 900);
    for folder in ["Documents", "Music", "Pictures"] {
        test.activate("pin-location", Some(&fixture.uri_of(folder)));
    }
    wait_until("the tiles and the recent file", || {
        home_page::tile_names(body(&test)).len() >= 2 && !home_page::list_names(body(&test)).is_empty()
    });
    for (compact, pitch) in [(false, 36), (true, 26)] {
        if compact {
            test.activate("compact-density", None);
            wait_until("Compact view", || test.window.shows_compact_density());
            wait_until("the recent file again", || {
                !home_page::list_names(body(&test)).is_empty()
            });
        }
        let view = if compact { "Compact" } else { "normal" };
        // Home is drawn again for the Compact view: the rows take their
        // new pitch once it is laid out.
        wait_until("Home laid out", || {
            crate::test_support::harness::wait_for_frames(&test.window, 2);
            let rows = home_page::metrics(body(&test)).rows;
            rows.first()
                .is_some_and(|row| (bounds(&test, row).height - pitch).abs() <= 1)
        });
        crate::test_support::harness::wait_for_frames(&test.window, 4);
        let metrics = home_page::metrics(body(&test));
        let middle = |widget: &gtk::Widget| {
            let place = bounds(&test, widget);
            place.y + place.height / 2
        };
        let below = middle(&metrics.rows[0]) - middle(&metrics.columns[0]);
        assert!(
            below <= pitch + 1,
            "{view}: the first row is {below} px under the titles, want one row ({pitch})"
        );
        let tiles = &metrics.tiles;
        let first = bounds(&test, &tiles[0]);
        let next = tiles
            .iter()
            .find(|tile| bounds(&test, *tile).y > first.y)
            .expect("a tile under the first");
        let pin = home_page::tile_pin(&tiles[0]);
        let name = home_page::tile_name_label(next);
        let gap = bounds(&test, &name).y - bounds(&test, &pin).bottom();
        assert!(gap >= 6, "{view}: the pin is {gap} px above the next tile's name");
    }
}

/// Recent follows the desktop's privacy settings as the navigation
/// pane's Recent locations does: with recent files turned off it lists
/// nothing, though `GVfs` still lists the earlier files.
///
/// parity: HOME-015
#[gtk::test]
fn recent_lists_nothing_while_the_desktop_remembers_no_recent_files() {
    super::file_ops_support::require_private_trash();
    let fixture = Fixture::standard();
    fs::write(fixture.path("Documents/plan.txt"), b"Fictional plan\n").expect("fixture file");
    add_to_desktop_history(&fixture.uri_of("Documents/plan.txt"), "text/plain");
    let test = home_window(&fixture);
    wait_until("the recent file", || {
        home_page::list_names(body(&test)).contains(&"plan".to_owned())
    });

    // GNOME's privacy switch, which leaves the desktop's list, and so
    // what `GVfs` lists, as it was.
    let schema = gtk::gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.privacy", true))
        .expect("this test needs GNOME's privacy settings schema");
    assert!(schema.has_key("remember-recent-files"));
    let privacy = gtk::gio::Settings::new("org.gnome.desktop.privacy");
    let remembered = privacy.boolean("remember-recent-files");
    privacy
        .set_boolean("remember-recent-files", false)
        .expect("the setting is writable");
    test.activate("refresh", None);
    let emptied = (0..200).any(|_| {
        crate::test_support::harness::wait_for_frames(&test.window, 1);
        home_page::list_names(body(&test)).is_empty() && home_page::empty_text(body(&test)).is_some()
    });
    privacy
        .set_boolean("remember-recent-files", remembered)
        .expect("the setting is writable");
    assert!(emptied, "Recent lists nothing while recent files are off");
}

/// Home leaves the window idle once drawn: nothing asks for a frame on
/// every frame to watch the width; a narrower window still gets fewer
/// tiles per row.
///
/// parity: HOME-013
#[gtk::test]
fn home_leaves_the_window_idle_and_still_follows_its_width() {
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.window.set_default_size(1600, 1000);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    wait_until("a tile", || !home_page::tile_names(body(&test)).is_empty());
    crate::test_support::harness::wait_for_frames(&test.window, 5);
    let wide = home_page::tiles_per_row(body(&test));

    let clock = test
        .window
        .frame_clock()
        .expect("a shown window has a frame clock");
    let pause = |milliseconds: u64| {
        let over = std::rc::Rc::new(std::cell::Cell::new(false));
        let done = over.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(milliseconds), move || {
            done.set(true);
        });
        wait_until("a pause", || over.get());
    };
    // The lists fill as `GVfs` and the settings answer; then nothing.
    pause(800);
    let before = clock.frame_counter();
    pause(600);
    let frames = clock.frame_counter() - before;
    assert!(frames < 6, "{frames} frames drawn while nothing changed");

    test.window.set_default_size(700, 800);
    wait_until("fewer tiles per row", || {
        home_page::tiles_per_row(body(&test)) < wide
    });
}

/// A Favorites row opens as a double-click opens the item in its
/// folder, through the same steps: a desktop link to a folder opens the
/// folder rather than being handed to an application.
///
/// parity: HOME-014
#[gtk::test]
fn a_favorite_opens_as_a_double_click_in_its_folder_does() {
    let fixture = Fixture::standard();
    let link = format!(
        "[Desktop Entry]\nType=Link\nName=Documents\nURL={}\n",
        fixture.uri_of("Documents")
    );
    fs::write(fixture.path("Documents link.desktop"), link).expect("the fixture is writable");
    let test = home_window(&fixture);
    test.activate(
        "add-to-favorites",
        Some(&fixture.uri_of("Documents link.desktop")),
    );
    home_page::choose_tab(body(&test), "Favorites");
    wait_until("the favorite", || {
        home_page::list_names(body(&test)) == ["Documents link"]
    });

    home_page::open_item(body(&test), "Documents link");

    wait_until("the linked folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    assert!(
        test.context.recorded_launches().is_empty(),
        "the link file is not opened"
    );
}

/// Folding both sections at once saves both: each fold changes only its
/// own key of the settings file as it then is.
///
/// parity: HOME-016
#[gtk::test]
fn folding_both_sections_at_once_saves_both() {
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    wait_until("the Documents tile", || {
        !home_page::tile_names(body(&test)).is_empty()
    });
    let folds = home_page::fold_buttons(body(&test));
    folds[0].emit_clicked();
    folds[1].emit_clicked();

    wait_until("both folds saved", || {
        let mut saved = Settings::open(test.settings_directory())
            .data()
            .preferences
            .home_folded
            .clone();
        saved.sort();
        saved == ["files", "quickAccess"]
    });
}

/// Shift+F10 on a Home item opens the menu the file list's Shift+F10
/// opens: the right-click menu's style, or the classic menu with the
/// Settings choice "Shift+F10 opens the classic menu".
///
/// parity: HOME-013
#[gtk::test]
fn shift_f10_on_a_home_item_opens_the_file_lists_menu() {
    use crate::window::menu_popover::MenuStyle;

    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    wait_until("the Documents tile", || {
        home_page::tile_names(body(&test)).contains(&"Documents".to_owned())
    });

    let menu = home_page::item_menu(body(&test), "Documents", "<Shift>F10");
    let style = menu.style();
    menu.popdown();
    assert_eq!(
        style,
        test.window.preferred_menu_style(),
        "the right-click menu's style"
    );
    assert_ne!(style, MenuStyle::Classic);

    let classic = PreferencesUpdate {
        shift_f10_classic: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&classic)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the choice to be read", || {
        test.context.settings_data().preferences.shift_f10_classic
    });
    let menu = home_page::item_menu(body(&test), "Documents", "<Shift>F10");
    assert_eq!(menu.style(), MenuStyle::Classic);
    menu.popdown();
}

/// A file whose GIO address is not in the settings' canonical form (here
/// with `(` and `!`) is known as a favorite once added: its menu then
/// offers Remove from Favorites, Home lists it once, and removing it by
/// that address takes it away.
///
/// parity: HOME-014
#[gtk::test]
fn a_favorite_with_parentheses_is_known_and_removed_by_its_gio_address() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("Notes (1)!.txt"), b"Fictional notes\n").expect("fixture file");
    let address = gtk::gio::File::for_path(fixture.path("Notes (1)!.txt"))
        .uri()
        .to_string();
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
    test.activate("add-to-favorites", Some(&address));
    wait_until("the favorite", || {
        !test.context.settings_data().preferences.favorites.is_empty()
    });
    test.window
        .right_click(Some(position_of(&test, "Notes (1)!.txt")));
    let menu = test.window.context_menu();
    let labels = menu.row_labels();
    menu.popdown();
    assert!(labels.contains(&"Remove from Favorites".to_owned()), "{labels:?}");

    test.activate("remove-from-favorites", Some(&address));
    wait_until("the favorite to go", || {
        test.context.settings_data().preferences.favorites.is_empty()
    });
}

/// Favorites lists its rows as soon as Home is drawn, from their
/// addresses alone, and reads no file for their dates while Recent is
/// shown; once Favorites is shown, the dates follow.
///
/// parity: HOME-014
#[gtk::test]
fn favorites_rows_come_at_once_and_their_dates_only_when_shown() {
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.activate("add-to-favorites", Some(&fixture.uri_of("Notes 2.txt")));
    wait_until("the favorite saved", || {
        !test.context.settings_data().preferences.favorites.is_empty()
    });
    crate::test_support::harness::wait_for_frames(&test.window, 3);

    test.window.render_landing();
    home_page::choose_tab(body(&test), "Favorites");
    assert_eq!(
        home_page::list_names(body(&test)),
        ["Notes 2"],
        "listed at once, before any file is read"
    );
    home_page::choose_tab(body(&test), "Recent");
    test.window.render_landing();
    let before = home_page::favorite_date_reads();
    crate::test_support::harness::wait_for_frames(&test.window, 5);
    assert_eq!(
        home_page::favorite_date_reads(),
        before,
        "no date read while Recent is shown"
    );

    home_page::choose_tab(body(&test), "Favorites");
    wait_until("the date", || {
        home_page::row_cells(body(&test), "Notes 2")
            .get(1)
            .is_some_and(|date| !date.is_empty() && date != "—")
    });
}

/// Compact view draws Quick access's tiles closer too, as it does the
/// list's rows.
///
/// parity: HOME-016
#[gtk::test]
fn compact_view_draws_quick_access_tiles_closer() {
    use super::geometry::bounds;

    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    wait_until("a tile", || !home_page::tile_names(body(&test)).is_empty());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let tile = |test: &TestWindow| bounds(test, &home_page::tile_named(body(test), "Documents")).height;
    assert_eq!(tile(&test), 60);

    test.activate("compact-density", None);
    wait_until("Compact view", || test.window.shows_compact_density());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    assert_eq!(tile(&test), 54, "a compact tile");
}

/// The shortcut text `menu`'s rows show, in order.
fn shortcut_texts(menu: &crate::window::menu_popover::MenuPopover) -> Vec<String> {
    menu.rows()
        .iter()
        .flat_map(crate::test_support::harness::descendants::<gtk::Label>)
        .filter(|label| label.has_css_class("shortcut"))
        .map(|label| label.text().to_string())
        .collect()
}

/// A Quick access tile's menu is Windows 11's (checked on Windows 11 on
/// 2026-11-14): Copy alone in the strip, then Open (Enter), Open in new
/// tab, Open in new window, Unpin from Quick access, Compress to, Copy as
/// path (Ctrl+Shift+C), Properties (Alt+Enter), Open in Terminal and Show
/// more options, which opens the classic menu with the app's own items
/// (Open folder with…, Previous versions). Pin to Start is Windows' own.
/// The Menu key and Shift+F10 open the same menu, and Escape gives the
/// keyboard back to the tile.
///
/// parity: HOME-013, SIDE-014
#[gtk::test]
fn a_quick_access_tiles_menu_is_windows_11s() {
    use crate::test_support::harness::descendants;
    use crate::window::context_menu::MENU_KEY;
    use crate::window::menu_popover::{MenuPopover, MenuStyle};

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Projects")));
    wait_until("the Projects tile", || {
        home_page::tile_names(body(&test)).contains(&"Projects".to_owned())
    });

    let mut seen = Vec::new();
    for key in [MENU_KEY, "<Shift>F10"] {
        let tile = home_page::tile_named(body(&test), "Projects");
        tile.grab_focus();
        let menu = home_page::item_menu(body(&test), "Projects", key);
        let rows: Vec<String> = menu.row_labels().into_iter().filter(|row| row != "-").collect();
        seen.push((
            key,
            menu.style(),
            menu.strip_labels(),
            rows,
            shortcut_texts(&menu),
        ));
        // As Escape closes it.
        menu.popdown();
        wait_until("the menu to close", || !menu.is_visible());
        wait_until("the keyboard back on the tile", || {
            gtk::prelude::GtkWindowExt::focus(&test.window).as_ref() == Some(tile.upcast_ref::<gtk::Widget>())
        });
    }
    let windows: Vec<String> = [
        "Open",
        "Open in new tab",
        "Open in new window",
        "Unpin from Quick access",
        "Compress to",
        "Copy as path",
        "Properties",
        "Open in Terminal",
        "Show more options",
    ]
    .map(str::to_owned)
    .to_vec();
    let shortcuts: Vec<String> = ["Enter", "Ctrl+Shift+C", "Alt+Enter"].map(str::to_owned).to_vec();
    for (key, style, strip, rows, keys) in seen {
        assert_eq!(style, MenuStyle::Compact, "{key}");
        assert_eq!(strip, ["Copy"], "{key}");
        assert_eq!(rows, windows, "{key}");
        assert_eq!(keys, shortcuts, "{key}");
    }

    let menu = home_page::item_menu(body(&test), "Projects", MENU_KEY);
    menu.click_row("Show more options");
    let open_classic = || {
        descendants::<MenuPopover>(test.window.folder_pane())
            .into_iter()
            .find(|menu| menu.is_visible() && menu.style() == MenuStyle::Classic)
    };
    wait_until("the classic menu", || open_classic().is_some());
    let classic = open_classic().expect("the classic menu is open");
    let rows = classic.row_labels();
    for extra in [
        "Open folder with…",
        "Previous versions",
        "Copy",
        "Unpin from Quick access",
    ] {
        assert!(rows.iter().any(|row| row == extra), "{extra} in {rows:?}");
    }
    assert!(classic.strip_labels().is_empty());
    classic.popdown();
}

/// Ctrl+C on a Quick access tile copies its folder, as the menu's Copy,
/// which shows Ctrl+C, does.
///
/// parity: HOME-013
#[gtk::test]
fn ctrl_c_on_a_quick_access_tile_copies_its_folder() {
    use super::recent_rows::copied_uris;

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Projects")));
    wait_until("the Projects tile", || {
        home_page::tile_names(body(&test)).contains(&"Projects".to_owned())
    });
    test.window.clipboard().set_text("Synthetic text");

    let handled = home_page::press_on_item(body(&test), "Projects", "<Control>c");

    assert!(handled, "the tile handles Ctrl+C");
    let projects = fixture.uri_of("Projects");
    wait_until("the copied folder", || {
        copied_uris(&test).as_deref() == Some(projects.as_str())
    });
}

/// The open menu in the folder pane, once it has the keyboard.
fn open_tile_menu(test: &TestWindow) -> crate::window::menu_popover::MenuPopover {
    use crate::test_support::harness::descendants;
    use crate::window::menu_popover::MenuPopover;

    let open = || {
        descendants::<MenuPopover>(test.window.folder_pane())
            .into_iter()
            .find(gtk::prelude::WidgetExt::is_mapped)
    };
    wait_until("the tile's menu", || open().is_some());
    let menu = open().expect("the menu is open");
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    menu
}

/// Where the keyboard is in `menu`, as `(on a row, on the list of rows
/// or the scroller around it)`: a list or a scroller with the keyboard
/// draws its ring around every row, as GTK draws a focused widget's.
fn keyboard_in_menu(test: &TestWindow, menu: &crate::window::menu_popover::MenuPopover) -> (bool, bool) {
    let list = menu.row_list();
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window);
    let on_a_row = focus
        .as_ref()
        .is_some_and(|focus| focus.is::<gtk::ListBoxRow>() && focus.is_ancestor(&list));
    let around_the_rows = focus.is_some_and(|focus| {
        focus == *list.upcast_ref::<gtk::Widget>()
            || (focus.is::<gtk::ScrolledWindow>() && list.is_ancestor(&focus))
    });
    (on_a_row, around_the_rows)
}

/// A Quick access tile's menu draws no ring around its list of rows, as
/// Windows 11's menus and the file list's own menu do not, however it was
/// opened: by a right-click, by Shift+F10, or by Tab and then the Menu
/// key. The keyboard is on its first row, which shows it with the row's
/// own highlight. Real keys and clicks, with XTEST.
///
/// parity: HOME-013, ACC-009
#[gtk::test]
fn a_quick_access_tiles_menu_draws_no_ring_around_its_rows() {
    use crate::test_support::pointer::{screen_point, Button, Pointer};

    let fixture = Fixture::standard();
    fs::create_dir(fixture.path("Projects")).expect("fixture subfolder");
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Projects")));
    wait_until("the Projects tile", || {
        home_page::tile_names(body(&test)).contains(&"Projects".to_owned())
    });
    test.window.present();
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    let mut pointer = Pointer::connect();
    let tile = home_page::tile_named(body(&test), "Projects");
    let middle = (f64::from(tile.width()) / 2.0, f64::from(tile.height()) / 2.0);
    let (x, y) = screen_point(&mut pointer, &tile, middle, (720, 500));
    let settle = || {
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
        crate::test_support::harness::wait_for_frames(&test.window, 2);
    };
    // The pointer leaves the menu before it closes, onto the tile left of
    // where the menu opened: GTK 4.14 can crash (gdk_surface_request_motion)
    // when a popover closes under the tests' resting pointer (see
    // tests/focus.rs).
    let off_the_menu = (x - 40, y);
    let close = |pointer: &mut Pointer, menu: &crate::window::menu_popover::MenuPopover| {
        pointer.move_to(off_the_menu.0, off_the_menu.1);
        menu.popdown();
        wait_until("the menu to close", || !menu.is_mapped());
    };
    let mut seen = Vec::new();

    pointer.move_to(x, y);
    pointer.press(Button::Secondary);
    pointer.release(Button::Secondary);
    let menu = open_tile_menu(&test);
    seen.push(("right-click", keyboard_in_menu(&test, &menu)));
    close(&mut pointer, &menu);
    settle();

    pointer.hold_key(gtk::gdk::Key::Shift_L);
    pointer.tap_key(gtk::gdk::Key::F10);
    pointer.let_go_key(gtk::gdk::Key::Shift_L);
    let menu = open_tile_menu(&test);
    seen.push(("Shift+F10", keyboard_in_menu(&test, &menu)));
    close(&mut pointer, &menu);
    settle();

    home_page::tile_named(body(&test), "Desktop").grab_focus();
    pointer.tap_key(gtk::gdk::Key::Tab);
    settle();
    pointer.tap_key(gtk::gdk::Key::Menu);
    let menu = open_tile_menu(&test);
    seen.push(("Tab, then Menu", keyboard_in_menu(&test, &menu)));
    close(&mut pointer, &menu);
    // Back to where Xvfb starts the pointer (see tests/focus.rs).
    pointer.move_to(720, 500);

    assert_eq!(
        seen,
        [
            ("right-click", (true, false)),
            ("Shift+F10", (true, false)),
            ("Tab, then Menu", (true, false)),
        ]
    );
}

/// A folder in the test's own home with a file only it holds, deleted
/// with the guard. Only check.py's private session has a disposable
/// home folder to write to.
fn file_in_home_folder(name: &str) -> tempfile::TempDir {
    assert_eq!(
        std::env::var("OX_ISOLATED_SESSION").as_deref(),
        Ok("1"),
        "the home folder is written to only in check.py's private session"
    );
    let folder = tempfile::Builder::new()
        .prefix("Home search ")
        .tempdir_in(gtk::glib::home_dir())
        .expect("the test home has room");
    fs::write(folder.path().join(name), b"Fictional plan\n").expect("fixture file");
    folder
}

/// Whether the active pane shows the Home page rather than a listing.
fn shows_home_page(test: &TestWindow) -> bool {
    test.window.current_uri().as_deref() == Some(HOME_PAGE)
        && test.window.folder_pane().page() == Some(crate::window::folder_pane::PanePage::Landing)
}

/// Search works from Home, as Windows 11 says: "To find relevant files
/// from your PC and the cloud, search from File Explorer Home". The box
/// reads "Search Home", Ctrl+E and F3 reach it, and typing searches the
/// home folder and its subfolders with the usual results: "<query> -
/// Search Results in Home". Back and Close search return to Home.
///
/// parity: HOME-035
#[gtk::test]
fn searching_from_home_searches_the_home_folder_and_its_subfolders() {
    let _home_files = file_in_home_folder("Quarterly zebra plan.txt");
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    let search = test.window.search_box();

    assert!(search.is_sensitive(), "the search box works on Home");
    assert_eq!(search.entry().placeholder_text().as_deref(), Some("Search Home"));
    // The Search action, which Ctrl+F, Ctrl+E and F3 run.
    test.window.folder_pane().focus_view();
    test.activate("search", None);
    let focus = gtk::prelude::GtkWindowExt::focus(&test.window);
    assert!(
        focus.is_some_and(|focus| focus.is_ancestor(search)),
        "Search puts the keyboard in the search box"
    );

    test.search_for("zebra");
    wait_until("the result from a subfolder of home", || {
        test.names() == ["Quarterly zebra plan.txt"]
    });
    assert_eq!(test.active_tab_title(), "zebra - Search Results in Home");
    assert!(!shows_home_page(&test), "the results show, not Home");
    let count = test.status_count();
    assert!(
        count.starts_with("1 result"),
        "the status bar counts the results: {count}"
    );

    test.activate("back", None);
    wait_until("Back to Home", || shows_home_page(&test));
    assert_eq!(test.search_text(), "");

    test.search_for("zebra");
    wait_until("the result again", || {
        test.names() == ["Quarterly zebra plan.txt"]
    });
    test.activate("close-search", None);
    wait_until("Close search returns to Home", || shows_home_page(&test));
}

/// With "Open Quick access on Home" off, Home is the home folder, which
/// is searched as any folder is, with its subfolders.
///
/// parity: HOME-035, NAV-038
#[gtk::test]
fn searching_the_home_folder_finds_files_in_its_subfolders() {
    let _home_files = file_in_home_folder("Quarterly zebra plan.txt");
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("home", None);
    test.wait_for_listing("the home folder");
    assert!(test.window.search_box().is_sensitive());

    test.search_for("zebra");
    wait_until("the result from a subfolder of home", || {
        test.names() == ["Quarterly zebra plan.txt"]
    });
    assert_eq!(test.active_tab_title(), "zebra - Search Results in Home");
}

/// The keyboard focus is on the tile or row whose menu just closed, by
/// Escape, by choosing an item that leaves Home where it is, or by a
/// click away, so the arrows and Enter go on working there, as with the
/// navigation pane's menus.
///
/// parity: HOME-016
#[gtk::test]
fn the_keyboard_comes_back_to_the_item_whose_menu_closed() {
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.activate("pin-location", Some(&fixture.uri_of("Documents")));
    let notes = fixture.uri_of("Notes 2.txt");
    test.activate("add-to-favorites", Some(&notes));
    home_page::choose_tab(body(&test), "Favorites");
    // Known extensions are hidden in Home's list too (VIEW-069).
    wait_until("the tile and the favorite", || {
        !home_page::tile_names(body(&test)).is_empty() && home_page::list_names(body(&test)) == ["Notes 2"]
    });
    let focus_is_on = |item: &gtk::FlowBoxChild| {
        gtk::prelude::GtkWindowExt::focus(&test.window)
            .is_some_and(|focus| focus == *item.upcast_ref::<gtk::Widget>() || focus.is_ancestor(item))
    };

    for (name, accelerator) in [
        ("Documents", "<Shift>F10"),
        ("Documents", "Menu"),
        ("Notes 2", "<Shift>F10"),
        ("Notes 2", "Menu"),
    ] {
        let item = home_page::item_named(body(&test), name);
        item.grab_focus();
        let menu = home_page::item_menu(body(&test), name, accelerator);
        wait_until("the menu", || menu.is_visible());
        menu.popdown();
        wait_until(
            &format!("the keyboard back on {name} after {accelerator}"),
            || !menu.is_visible() && focus_is_on(&item),
        );
    }
}

/// A fictional home laid out as a real one: a cached folder (as a pinned
/// standard folder is), an uncached test tree with the files three levels
/// down, and a bigger folder beside them. Deleted with the guard.
fn home_with_cached_and_uncached_folders() -> tempfile::TempDir {
    let folder = file_in_home_folder("readme.txt");
    let root = folder.path();
    fs::create_dir_all(root.join("Documents/Reports")).expect("fixture folder");
    fs::write(root.join("Documents/Reports/extra_1.txt"), b"x").expect("fixture file");
    fs::create_dir_all(root.join("OX-Test/Deb32/Logs")).expect("fixture folder");
    for name in ["extra_2.txt", "extra_3.txt", "extra_30.txt", "extra_31.txt"] {
        fs::write(root.join("OX-Test/Deb32/Logs").join(name), b"x").expect("fixture file");
    }
    fs::create_dir_all(root.join("checkout/src")).expect("fixture folder");
    for number in 0..400 {
        fs::write(root.join(format!("checkout/src/module_{number}.rs")), b"x").expect("fixture file");
    }
    folder
}

/// Searching from Home finds files anywhere under the home folder, also
/// in folders the search cache does not hold, three levels down, while
/// another folder under home is cached: the Home page has no listing of
/// its own, so its search walks the home folder. The home folder itself
/// finds them too.
///
/// parity: HOME-035
#[gtk::test]
fn searching_from_home_finds_files_deep_in_uncached_folders() {
    let home = home_with_cached_and_uncached_folders();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.start_search_cache();
    let cached = ox_core::location::file_uri(&home.path().join("Documents"));
    test.index_folder(&cached);
    let expected = [
        "extra_1.txt",
        "extra_2.txt",
        "extra_3.txt",
        "extra_30.txt",
        "extra_31.txt",
    ];
    let found = |test: &TestWindow| {
        let mut names = test.names();
        names.sort();
        names == expected
    };

    open_quick_access_on_home(&test, true);
    test.activate("home", None);
    wait_until("the Home page", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });
    test.search_for("extra");
    wait_until("every extra file from Home", || found(&test));

    test.activate("close-search", None);
    open_quick_access_on_home(&test, false);
    test.activate("home", None);
    test.wait_for_listing("the home folder");
    test.search_for("extra");
    wait_until("every extra file from the home folder", || found(&test));
}

/// What the active pane shows of a search from Home, sampled once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchSample {
    rows: u32,
    searching: bool,
    empty_page: bool,
}

/// Searching from Home in a big home folder with a cached folder in it:
/// the files are listed as the walk finds them, and the pane never says "No matching items" while
/// the search still runs or while it has rows; at the end every match is
/// shown. Before, the empty page stayed up until the whole home folder
/// had been walked, which on a real home can take minutes.
///
/// parity: HOME-035, SRCH-035
#[gtk::test]
fn searching_from_a_big_home_shows_matches_as_they_are_found() {
    let home = home_with_cached_and_uncached_folders();
    for folder in 0..1500 {
        let path = home.path().join(format!("snap/part{}/dir{folder}", folder / 50));
        fs::create_dir_all(&path).expect("fixture folder");
        for file in 0..4 {
            fs::write(path.join(format!("item_{file}.dat")), b"").expect("fixture file");
        }
    }
    let fixture = Fixture::standard();
    let test = home_window(&fixture);
    test.start_search_cache();
    test.index_folder(&ox_core::location::file_uri(&home.path().join("Documents")));
    let tooltip = test.window.search_box().entry().tooltip_text();
    assert_eq!(
        tooltip.as_deref(),
        Some("Search names in this folder and its subfolders. Folders cached in Settings → Search are searched faster."),
        "the box does not claim to search cached names only"
    );
    let pane = test.window.folder_pane();
    let sample = || SearchSample {
        rows: test.window.folder_model().n_items(),
        searching: test.status_count().starts_with("Searching"),
        empty_page: pane.page() == Some(crate::window::folder_pane::PanePage::Empty),
    };
    let samples = std::cell::RefCell::new(Vec::new());

    test.window.search_box().entry().set_text("extra");
    wait_until("the search to end with every match", || {
        let now = sample();
        samples.borrow_mut().push(now);
        !now.searching && now.rows == 5
    });
    let samples = samples.into_inner();

    let wrong: Vec<_> = samples
        .iter()
        .filter(|sample| sample.empty_page && (sample.searching || sample.rows > 0))
        .collect();
    assert!(
        wrong.is_empty(),
        "no empty page while searching or with rows: {wrong:?}"
    );
    assert!(
        samples.iter().any(|sample| sample.searching),
        "the search was seen running"
    );
}
