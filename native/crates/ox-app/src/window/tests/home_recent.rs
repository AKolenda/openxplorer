// SPDX-License-Identifier: AGPL-3.0-only
//! Home's Recent and Favorites rows act on the files they name, as the
//! rows of Windows 11's Home do: a Recent row offers Copy and Share, and
//! no Cut, Rename or Delete, by its menu, its keys or the command bar;
//! Remove from list takes a file off the desktop's recently used list
//! and leaves the file; a
//! renamed file stays in Recent and Favorites under its new name, and one
//! found gone leaves Recent. A click on blank space selects nothing, and
//! the Quick access tiles are dragged into a new order. Clicks, keys and drags are
//! real (XTEST, `crate::test_support::pointer`). Every name below is
//! fictional.

use std::fs;

use gtk::prelude::*;

use crate::app_context::add_to_desktop_history;
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::home_page;
use crate::window::menu_popover::MenuPopover;

use super::file_ops_support::{
    dialog_over, open_dialog, require_private_trash, text_field, wait_for_no_dialog,
};
use super::home_page::open_quick_access_on_home;

/// The Home page's address.
const HOME_PAGE: &str = "ox:home";

/// A window showing Home, with "Open Quick access on Home" on, whose
/// Recent lists `Documents/<name>` of a new fixture for each of `names`,
/// each just added to the desktop's history.
fn recent_home(names: &[&str]) -> (Fixture, TestWindow) {
    require_private_trash();
    let schemes = gtk::gio::Vfs::default().supported_uri_schemes();
    assert!(
        schemes.iter().any(|scheme| scheme == "recent"),
        "this test needs GVfs with its Recent backend; found {schemes:?}"
    );
    let fixture = Fixture::standard();
    for name in names {
        let relative = format!("Documents/{name}");
        fs::write(fixture.path(&relative), b"Synthetic notes\n").expect("fixture file");
        add_to_desktop_history(&fixture.uri_of(&relative), "text/plain");
    }
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
    test.activate("home", None);
    wait_until("the Home page", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });
    for name in names {
        let shown = stem(name);
        wait_until("the recent file", || {
            home_page::list_names(body(&test)).contains(&shown)
        });
    }
    // The desktop's list may still announce the additions, which list
    // Recent again: let that happen before the test acts.
    wait_for(std::time::Duration::from_millis(1200));
    (fixture, test)
}

/// `name` as Home shows it while extensions are hidden.
fn stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_owned()
}

/// The Home page's body in the active pane.
fn body(test: &TestWindow) -> &gtk::Box {
    test.window.folder_pane().landing()
}

/// The names Recent lists.
fn recent_names(test: &TestWindow) -> Vec<String> {
    home_page::list_names(body(test))
}

/// Moves the real pointer to the middle of `widget`, or to `at` in it,
/// and clicks `button`.
fn click(
    pointer: &mut Pointer,
    test: &TestWindow,
    widget: &impl IsA<gtk::Widget>,
    at: Option<(f64, f64)>,
    button: Button,
) {
    wait_for_frames(&test.window, 2);
    let at = at.unwrap_or((f64::from(widget.width()) / 2.0, f64::from(widget.height()) / 2.0));
    let (x, y) = screen_point(pointer, widget, at, (8, 8));
    pointer.move_to(x, y);
    pointer.press(button);
    pointer.release(button);
}

/// The menu open in the folder pane.
fn open_menu(test: &TestWindow) -> MenuPopover {
    let open = || {
        descendants::<MenuPopover>(test.window.folder_pane())
            .into_iter()
            .find(gtk::prelude::WidgetExt::is_mapped)
    };
    wait_until("a menu", || open().is_some());
    let menu = open().expect("the menu is open");
    wait_for_frames(&test.window, 2);
    menu
}

/// Selects the row named `name` with a real click.
fn click_row(pointer: &mut Pointer, test: &TestWindow, name: &str) {
    wait_until("the row", || {
        home_page::list_names(body(test)).contains(&name.to_owned())
    });
    let row = home_page::item_named(body(test), name);
    click(pointer, test, &row, None, Button::Primary);
    wait_until("the row to be selected", || row.is_selected() && row.has_focus());
}

/// Remove from list takes the file off the desktop's recently used list
/// and off Home's Recent; the file itself stays.
///
/// parity: HOME-036
#[gtk::test]
fn remove_from_recent_drops_the_row_and_keeps_the_file() {
    let (fixture, test) = recent_home(&["plan.txt", "budget.txt"]);
    let plan = fixture.uri_of("Documents/plan.txt");
    let mut pointer = Pointer::connect();
    let row = home_page::item_named(body(&test), "plan");
    click(&mut pointer, &test, &row, None, Button::Secondary);
    let menu = open_menu(&test);

    menu.click_row("Remove from list");

    wait_until("the row to leave Recent", || {
        !recent_names(&test).contains(&"plan".to_owned())
    });
    assert!(recent_names(&test).contains(&"budget".to_owned()));
    assert!(fixture.path("Documents/plan.txt").is_file(), "the file stays");
    assert!(!gtk::RecentManager::default().has_item(&plan));
}

/// Delete, Shift+Delete, F2 and Ctrl+X do nothing on a Recent row, as
/// in Windows 11's Home (checked 2026-11-22): no file goes to the Recycle
/// Bin, no dialog opens, the clipboard keeps what it had, and the row
/// stays.
///
/// parity: HOME-036
#[gtk::test]
fn delete_f2_and_ctrl_x_do_nothing_on_a_recent_row() {
    use super::recent_rows::copied_uris;

    let (fixture, test) = recent_home(&["plan.txt", "budget.txt"]);
    test.window.clipboard().set_text("Synthetic text");
    let mut pointer = Pointer::connect();
    click_row(&mut pointer, &test, "plan");

    pointer.tap_key(gtk::gdk::Key::Delete);
    pointer.hold_key(gtk::gdk::Key::Shift_L);
    pointer.tap_key(gtk::gdk::Key::Delete);
    pointer.let_go_key(gtk::gdk::Key::Shift_L);
    pointer.tap_key(gtk::gdk::Key::F2);
    pointer.hold_key(gtk::gdk::Key::Control_L);
    pointer.tap_key(gtk::gdk::Key::x);
    pointer.let_go_key(gtk::gdk::Key::Control_L);
    wait_for(std::time::Duration::from_millis(800));

    assert!(dialog_over(&test).is_none(), "no dialog");
    assert!(fixture.path("Documents/plan.txt").is_file(), "the file stays");
    assert_eq!(copied_uris(&test), None, "nothing cut");
    assert!(recent_names(&test).contains(&"plan".to_owned()));
    assert_eq!(test.window.current_uri().as_deref(), Some(HOME_PAGE));
}

/// A rename anywhere in the app keeps the file in Recent and Favorites
/// under its new name, as the desktop's list follows the rename.
///
/// parity: HOME-036
#[gtk::test]
fn a_renamed_file_stays_in_recent_and_favorites() {
    let (fixture, test) = recent_home(&["plan.txt"]);
    let plan = fixture.uri_of("Documents/plan.txt");
    test.activate("add-to-favorites", Some(&plan));
    wait_until("the favorite", || {
        test.context.settings_data().preferences.favorites.len() == 1
    });
    wait_for(std::time::Duration::from_millis(600));

    test.activate("rename-folder", Some(&plan));
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Rename");
    text_field(&dialog).set_text("final plan.txt");
    dialog.press("Save");
    wait_for_no_dialog(&test);

    wait_until("the file to be renamed", || {
        fixture.path("Documents/final plan.txt").is_file()
    });
    assert!(!fixture.path("Documents/plan.txt").exists());
    wait_until("Recent to list the new name", || {
        recent_names(&test) == ["final plan"]
    });
    let renamed = fixture.uri_of("Documents/final plan.txt");
    assert!(test.window.is_favorite(&renamed), "Favorites follows the rename");
    assert!(!test.window.is_favorite(&plan));
}

/// Ctrl+C on a Recent row puts the file on the clipboard, as its menu's
/// Copy does.
///
/// parity: HOME-036
#[gtk::test]
fn ctrl_c_on_a_recent_row_copies_the_file() {
    use super::recent_rows::copied_uris;

    let (fixture, test) = recent_home(&["plan.txt"]);
    test.window.clipboard().set_text("Synthetic text");
    let mut pointer = Pointer::connect();
    click_row(&mut pointer, &test, "plan");

    pointer.hold_key(gtk::gdk::Key::Control_L);
    pointer.tap_key(gtk::gdk::Key::c);
    pointer.let_go_key(gtk::gdk::Key::Control_L);

    let plan = fixture.uri_of("Documents/plan.txt");
    wait_until("the copied file", || {
        copied_uris(&test).as_deref() == Some(plan.as_str())
    });
}

/// Opening a Recent file that is gone says "Item Not Found", as Windows
/// does, and the row then leaves Recent.
///
/// parity: HOME-015, HOME-036
#[gtk::test]
fn a_recent_file_found_gone_leaves_recent() {
    let (fixture, test) = recent_home(&["plan.txt", "budget.txt"]);
    fs::remove_file(fixture.path("Documents/plan.txt")).expect("the file is removable");

    home_page::open_item(body(&test), "plan");
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Item Not Found");
    dialog.press("OK");
    wait_for_no_dialog(&test);

    wait_until("the gone file to leave Recent", || {
        !recent_names(&test).contains(&"plan".to_owned())
    });
    assert!(recent_names(&test).contains(&"budget".to_owned()));
}

/// Clear File Explorer history (Folder Options > General > Privacy)
/// empties Recent on Home at once; Favorites and the pinned folders stay,
/// as Microsoft's answer says they do.
///
/// parity: HOME-015, HOME-036
#[gtk::test]
fn clearing_history_empties_recent_and_keeps_favorites() {
    let (fixture, test) = recent_home(&["plan.txt"]);
    test.activate("add-to-favorites", Some(&fixture.uri_of("Documents/plan.txt")));
    wait_until("the favorite", || {
        test.context.settings_data().preferences.favorites.len() == 1
    });

    test.window.clear_explorer_history();

    wait_until("Recent to empty", || {
        home_page::empty_text(body(&test)).as_deref() == Some("Files you open recently show here.")
    });
    assert_eq!(test.context.settings_data().preferences.favorites.len(), 1);
    home_page::choose_tab(body(&test), "Favorites");
    wait_until("the favorite still listed", || {
        home_page::list_names(body(&test)) == ["plan"]
    });
}

/// A Quick access tile dragged onto the right half of another goes after
/// it, and onto the left half before it, as an insertion mark shows in
/// Windows; the navigation pane takes the same order, which is saved.
///
/// parity: HOME-037
#[gtk::test]
fn a_tile_dragged_onto_another_moves_there() {
    let fixture = Fixture::standard();
    for name in ["Alpha", "Beta", "Gamma"] {
        fs::create_dir(fixture.path(name)).expect("fixture folder");
    }
    let test = TestWindow::open(&fixture.uri());
    open_quick_access_on_home(&test, true);
    for name in ["Alpha", "Beta", "Gamma"] {
        test.activate("pin-location", Some(&fixture.uri_of(name)));
        wait_until("the pin", || home_tiles(&test).contains(&name.to_owned()));
    }
    test.activate("home", None);
    wait_until("the Home page", || {
        test.window.current_uri().as_deref() == Some(HOME_PAGE)
    });
    wait_until("the tiles", || {
        pins_in(&home_tiles(&test)) == ["Alpha", "Beta", "Gamma"]
    });

    let mut pointer = Pointer::connect();
    drag_tile(&mut pointer, &test, "Alpha", "Gamma", 0.8);
    wait_until("Alpha after Gamma", || {
        pins_in(&home_tiles(&test)) == ["Beta", "Gamma", "Alpha"]
    });
    drag_tile(&mut pointer, &test, "Alpha", "Beta", 0.2);
    wait_until("Alpha before Beta", || {
        pins_in(&home_tiles(&test)) == ["Alpha", "Beta", "Gamma"]
    });
    let order = test.context.settings_data().quick_order;
    let saved: Vec<String> = ["Alpha", "Beta", "Gamma"]
        .iter()
        .map(|name| fixture.uri_of(name))
        .collect();
    let positions: Vec<usize> = saved
        .iter()
        .map(|uri| {
            order
                .iter()
                .position(|kept| kept == uri)
                .expect("saved in the order")
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
}

/// The Quick access tiles Home shows, in order.
fn home_tiles(test: &TestWindow) -> Vec<String> {
    if test.window.current_uri().as_deref() != Some(HOME_PAGE) {
        test.activate("home", None);
        wait_until("the Home page", || {
            test.window.current_uri().as_deref() == Some(HOME_PAGE)
        });
    }
    home_page::tile_names(body(test))
}

/// The fixture's pins among `tiles`, in order.
fn pins_in(tiles: &[String]) -> Vec<String> {
    tiles
        .iter()
        .filter(|name| ["Alpha", "Beta", "Gamma"].contains(&name.as_str()))
        .cloned()
        .collect()
}

/// Drags the tile named `from` with the real pointer and drops it at
/// `across` (0 to 1) of the width of the tile named `onto`.
fn drag_tile(pointer: &mut Pointer, test: &TestWindow, from: &str, onto: &str, across: f64) {
    wait_for_frames(&test.window, 2);
    let source = home_page::tile_named(body(test), from);
    let target = home_page::tile_named(body(test), onto);
    let start = screen_point(pointer, &source, (40.0, f64::from(source.height()) / 2.0), (8, 8));
    let end = screen_point(
        pointer,
        &target,
        (
            f64::from(target.width()) * across,
            f64::from(target.height()) / 2.0,
        ),
        (8, 8),
    );
    pointer.move_to(start.0, start.1);
    pointer.press(Button::Primary);
    pointer.glide(start, (start.0 + 12, start.1 + 4), 4);
    pointer.glide((start.0 + 12, start.1 + 4), end, 12);
    wait_for_frames(&test.window, 3);
    pointer.release(Button::Primary);
    wait_for_frames(&test.window, 3);
}
