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

/// A real click on the navigation pane's Home row opens the Home page,
/// whatever the settings file holds besides "Open the home folder on
/// Home": nothing at all (no file, or an empty file) or other choices,
/// This PC as the startup folder among them.
///
/// parity: HOME-012
#[gtk::test]
fn a_real_click_on_the_home_row_opens_the_home_page_whatever_else_the_settings_hold() {
    let fixture = Fixture::standard();

    let test = TestWindow::open(&fixture.uri());
    click_the_home_row(&test);

    let test = TestWindow::open(&fixture.uri());
    fs::write(test.settings_directory().join("settings.json"), b"").expect("an empty settings file");
    test.context.reload_settings();
    test.window.navigate(&fixture.uri()).expect("the fixture folder");
    test.wait_for_listing("the fixture folder");
    click_the_home_row(&test);

    let test = TestWindow::open(&fixture.uri());
    let others = PreferencesUpdate {
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
    assert!(!test.context.settings_data().preferences.home_opens_folder);
    click_the_home_row(&test);
}

/// The address bar leads to the same Home as the navigation pane: the
/// chevron's "Home" and the Home page's own address, "Home", typed or
/// left as it is and entered, open the Home page.
///
/// parity: HOME-012
#[gtk::test]
fn the_address_bars_home_opens_the_home_page() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
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
        50,
        3,
    );
    let (name, date, location) = (
        bounds(&test, &metrics.columns[0]).x,
        bounds(&test, &metrics.columns[1]).x,
        bounds(&test, &metrics.columns[2]).x,
    );
    near("Name to Date accessed", date - name, 484, 2);
    near("Date accessed to File location", location - date, 298, 2);
    near("Name from the pane's edge", name - page.x, 90, 4);
    let cells = home_page::row_cell_widgets(body(&test), "plan.txt");
    let icon = home_page::row_icon(body(&test), "plan.txt");
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
        home_page::list_names(body(&test)).contains(&"plan.txt".to_owned())
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
        home_page::list_names(body(&test)) == ["Documents link.desktop"]
    });

    home_page::open_item(body(&test), "Documents link.desktop");

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
    let address = gtk::gio::File::for_path(fixture.path("Notes (1)!.txt")).uri().to_string();
    let test = TestWindow::open(&fixture.uri());
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
    let before = home_page::favorite_date_reads();

    test.window.render_landing();
    home_page::choose_tab(body(&test), "Favorites");
    assert_eq!(
        home_page::list_names(body(&test)),
        ["Notes 2.txt"],
        "listed at once, before any file is read"
    );
    home_page::choose_tab(body(&test), "Recent");
    test.window.render_landing();
    crate::test_support::harness::wait_for_frames(&test.window, 5);
    assert_eq!(home_page::favorite_date_reads(), before, "no date read while Recent is shown");

    home_page::choose_tab(body(&test), "Favorites");
    wait_until("the date", || {
        home_page::row_cells(body(&test), "Notes 2.txt")
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
    let tile = |test: &TestWindow| {
        let metrics = home_page::metrics(body(test));
        bounds(test, &metrics.tiles[0]).height
    };
    assert_eq!(tile(&test), 60);

    test.activate("compact-density", None);
    wait_until("Compact view", || test.window.shows_compact_density());
    crate::test_support::harness::wait_for_frames(&test.window, 3);
    assert_eq!(tile(&test), 48, "a compact tile");
}
