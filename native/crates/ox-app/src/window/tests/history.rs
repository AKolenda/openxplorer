// SPDX-License-Identifier: AGPL-3.0-only
//! Back, Forward, Up, Refresh and Home: the navigation buttons, their
//! keys and the mouse's side buttons, against `goHistory`, `onKey` and
//! `renderNavigation` in `desktop/ui/app.js` and the mouse-button
//! handling of `desktop/winspace.py`.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{press_shortcut, press_shortcut_where_focused};
use super::support::middle_click_at;
use crate::locations::Page;
use crate::search::SearchScope;
use crate::test_support::harness::{application, descendants, wait_until, Fixture, TestWindow};
use crate::window::address_bar::AddressMode;
use crate::window::session::Direction;
use crate::window::type_to_select::monotonic_now;

/// The home folder's URI, as tabs store it.
fn home_uri() -> String {
    ox_core::location::file_uri(&glib::home_dir())
}

/// A window on the fixture's Documents folder, reached from the fixture
/// folder, so Back has one step to take.
fn window_one_step_in(fixture: &Fixture) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("valid folder");
    test.wait_for_listing("the subfolder");
    test
}

/// The button before the address bar that runs `action` (`win.` omitted).
fn navigation_button(test: &TestWindow, action: &str) -> gtk::Button {
    let detailed = format!("win.{action}");
    descendants::<gtk::Button>(&test.window)
        .into_iter()
        .find(|button| {
            button.action_name().as_deref() == Some(detailed.as_str()) && button.tooltip_text().is_some()
        })
        .unwrap_or_else(|| panic!("the window has a {action} button"))
}

const ALT: gdk::ModifierType = gdk::ModifierType::ALT_MASK;

/// parity: NAV-001
#[gtk::test]
fn back_and_forward_say_their_keys_and_follow_each_tabs_own_history() {
    let fixture = Fixture::standard();
    let test = window_one_step_in(&fixture);
    let back = navigation_button(&test, "back");
    let forward = navigation_button(&test, "forward");
    assert_eq!(back.tooltip_text().as_deref(), Some("Back (Alt+Left)"));
    assert_eq!(forward.tooltip_text().as_deref(), Some("Forward (Alt+Right)"));
    assert!(back.is_sensitive() && !forward.is_sensitive());

    test.window.add_tab(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the second tab");

    assert!(!back.is_sensitive(), "the new tab has its own, empty history");
    test.activate("previous-tab", None);
    assert!(back.is_sensitive(), "the first tab kept its history");
    back.emit_clicked();
    test.wait_for_listing("the folder before");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    assert!(!back.is_sensitive() && forward.is_sensitive());
}

/// parity: NAV-002
#[gtk::test]
fn alt_left_and_alt_right_walk_the_history_from_the_file_list() {
    let fixture = Fixture::standard();
    let test = window_one_step_in(&fixture);
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(&test, gdk::Key::Left, ALT));
    test.wait_for_listing("the folder before");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    press_shortcut_where_focused(&test, gdk::Key::Left, ALT);
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "no step before the first"
    );

    assert!(press_shortcut_where_focused(&test, gdk::Key::Right, ALT));
    test.wait_for_listing("the subfolder again");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// parity: NAV-002
#[gtk::test]
fn the_history_keys_leave_text_fields_and_dialogs_alone() {
    let fixture = Fixture::standard();
    let test = window_one_step_in(&fixture);
    test.window.search_box().entry().grab_focus();

    let in_search_box = press_shortcut_where_focused(&test, gdk::Key::Left, ALT);

    assert!(!in_search_box, "the search box keeps Alt+Left");
    test.activate("location", None);
    assert!(
        !press_shortcut_where_focused(&test, gdk::Key::Left, ALT),
        "so does the address"
    );
    test.window.folder_pane().focus_view();
    test.activate("properties", None);
    wait_until("the Properties dialog", || {
        test.window.dialog_layer().shown().is_some()
    });

    let in_dialog = press_shortcut_where_focused(&test, gdk::Key::Left, ALT);

    assert!(!in_dialog, "a dialog is open");
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
}

/// The side buttons cannot be pressed without a real pointer, so this
/// checks the window listens to every button before its widgets do, then
/// runs what a side button runs.
///
/// parity: NAV-003
#[gtk::test]
fn the_mouse_side_buttons_step_through_history_unless_a_dialog_is_open() {
    let fixture = Fixture::standard();
    let test = window_one_step_in(&fixture);
    let listens_first = test
        .window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .any(|gesture| {
            gesture.button() == 0 && gesture.propagation_phase() == gtk::PropagationPhase::Capture
        });
    assert!(listens_first, "the window hears every mouse button first");
    test.window.folder_pane().focus_view();
    test.activate("properties", None);
    wait_until("the Properties dialog", || {
        test.window.dialog_layer().shown().is_some()
    });

    test.window.go_history_from_mouse(Direction::Backward);

    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    let dialog = test.window.dialog_layer().shown().expect("the dialog is open");
    dialog.close();
    wait_until("the dialog to close", || {
        test.window.dialog_layer().shown().is_none()
    });
    test.window.go_history_from_mouse(Direction::Backward);
    test.wait_for_listing("the folder before");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
}

/// parity: NAV-010
#[gtk::test]
fn up_and_alt_up_open_the_parent_and_stop_at_pages_and_roots() {
    let fixture = Fixture::standard();
    let test = window_one_step_in(&fixture);
    let up = navigation_button(&test, "up");
    assert_eq!(up.tooltip_text().as_deref(), Some("Up (Alt+Up)"));
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(&test, gdk::Key::Up, ALT));
    test.wait_for_listing("the parent folder");

    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    for page in [Page::ThisPc, Page::Network] {
        test.window.navigate(page.uri()).expect("a landing page");
        test.wait_for_listing(page.title());
        assert!(!up.is_sensitive(), "{} has no parent", page.title());
    }
    test.window.navigate("file:///").expect("the root folder");
    test.wait_for_listing("the root folder");
    assert!(!up.is_sensitive(), "/ has no parent");
}

/// parity: NAV-011
#[gtk::test]
fn going_up_or_to_an_ancestor_crumb_selects_the_folder_left() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("Documents/Letters")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri_of("Documents/Letters"));

    test.activate("up", None);
    test.wait_for_listing("Documents");
    assert_eq!(test.selected_names(), ["Letters"]);
    test.window
        .navigate(&fixture.uri_of("Documents/Letters"))
        .expect("valid folder");
    test.wait_for_listing("Letters again");
    test.activate("go-to", Some(&fixture.uri()));
    test.wait_for_listing("the crumb's folder");

    assert_eq!(test.selected_names(), ["Documents"]);
}

/// parity: NAV-012
#[gtk::test]
fn alt_home_and_the_homepage_key_open_the_home_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(&test, gdk::Key::Home, ALT));
    test.wait_for_listing("the home folder");
    assert_eq!(test.window.current_uri(), Some(home_uri()));
    test.window.navigate(&fixture.uri()).expect("valid folder");
    test.wait_for_listing("the fixture folder");
    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::HomePage,
        gdk::ModifierType::empty()
    ));
    test.wait_for_listing("the home folder again");

    assert_eq!(test.window.current_uri(), Some(home_uri()));
}

/// parity: NAV-006
#[gtk::test]
fn the_back_menu_lists_the_history_and_jumps_straight_to_an_entry() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("Documents/Letters")).expect("fixture subfolder");
    let test = TestWindow::open(&fixture.uri());
    for folder in ["Documents", "Documents/Letters"] {
        test.window
            .navigate(&fixture.uri_of(folder))
            .expect("valid folder");
        test.wait_for_listing(folder);
    }
    let back = navigation_button(&test, "back");

    let menu = test
        .window
        .popup_history_menu(back.upcast_ref(), Direction::Backward)
        .expect("Back has a menu");

    let nearest_first = [fixture.path("Documents"), fixture.root().to_path_buf()];
    assert_eq!(
        menu.row_labels(),
        nearest_first.map(|path| path.display().to_string())
    );
    menu.popdown();
    assert!(test
        .window
        .popup_history_menu(back.upcast_ref(), Direction::Forward)
        .is_none());
    let two_back = (-2_i32).to_variant();
    WidgetExt::activate_action(&test.window, "win.go-history", Some(&two_back)).expect("the action");
    test.wait_for_listing("two steps back");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    let forward = test.window.history_menu_entries(Direction::Forward);
    assert_eq!(forward.len(), 2, "both steps are ahead now");
}

/// parity: NAV-007
#[gtk::test]
fn a_middle_click_on_back_or_up_opens_its_target_in_a_background_tab() {
    let fixture = Fixture::standard();
    let test = window_one_step_in(&fixture);
    let middle_click = |action: &str| {
        let button = navigation_button(&test, action);
        middle_click_at(&button, (1.0, 1.0));
    };

    middle_click("back");
    middle_click("up");
    let back = navigation_button(&test, "back");
    let menu = test
        .window
        .popup_history_menu(back.upcast_ref(), Direction::Backward)
        .expect("Back has a menu");
    wait_until("the menu to show", || {
        menu.rows().iter().all(|row| row.height() > 0)
    });
    menu.middle_click_row(&fixture.root().display().to_string());

    assert_eq!(test.window.tab_count(), 4);
    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    let session = test.window.imp().session.borrow();
    let uris: Vec<&str> = session
        .tabs()
        .iter()
        .map(super::super::session::Tab::uri)
        .collect();
    assert_eq!(uris[1..], [fixture.uri(), fixture.uri(), fixture.uri()]);
}

/// parity: NAV-013
#[gtk::test]
fn refresh_says_f5_and_f5_and_ctrl_r_work_even_from_text_fields() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let refresh = navigation_button(&test, "refresh");
    assert_eq!(refresh.tooltip_text().as_deref(), Some("Refresh (F5)"));

    let keys = application().accels_for_action("win.refresh");

    // Application accelerators run before the focused widget, so a text
    // field never keeps them (`onKey` handles F5 before its input check).
    assert_eq!(keys, ["F5", "<Control>r"]);
}

/// parity: NAV-015
#[gtk::test]
fn navigating_ends_the_search_prefix_and_address_editing_and_updates_the_frame() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.search_box().entry().set_text("Notes");
    wait_until("the filter", || test.names().len() == 2);
    test.window
        .imp()
        .search
        .borrow_mut()
        .set_scope(SearchScope::AllCachedFolders);
    test.window.folder_pane().focus_view();
    test.window.type_text("n");
    test.activate("location", None);

    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the subfolder");

    assert_eq!(test.window.search_box().entry().text().as_str(), "");
    assert_eq!(test.window.imp().search.borrow().scope(), SearchScope::ThisFolder);
    let typing = test.window.imp().typeahead.borrow().is_active(monotonic_now());
    assert!(!typing, "the typed prefix is gone");
    assert_eq!(test.window.address_bar().mode(), AddressMode::Crumbs);
    assert_eq!(test.window.title().as_deref(), Some("Documents — OpenXplorer"));
    let crumbs = test.window.address_bar().crumb_buttons();
    let last = crumbs.last().and_then(ButtonExt::label);
    assert_eq!(last.as_deref(), Some("Documents"));
}

/// parity: NAV-038
#[gtk::test]
fn home_new_tabs_and_ctrl_t_open_the_real_home_folder_titled_home() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let home_row = test
        .window
        .sidebar()
        .list()
        .row_at_index(0)
        .expect("Home comes first");

    home_row.activate();
    test.wait_for_listing("the home folder");

    assert_eq!(test.window.current_uri(), Some(home_uri()));
    assert_eq!(test.window.title().as_deref(), Some("Home — OpenXplorer"));
    press_shortcut(&test, gdk::Key::t, gdk::ModifierType::CONTROL_MASK);
    test.wait_for_listing("the new tab");
    assert_eq!(test.window.tab_count(), 2);
    assert_eq!(test.window.current_uri(), Some(home_uri()));
    wait_until("a listed home folder", || test.window.load_error().is_none());
}

/// A location that turns out to be a file is replaced by its folder in
/// the history, so Back skips it; a share's target opens directly.
///
/// parity: NAV-009
#[gtk::test]
fn a_location_that_redirects_is_not_kept_as_its_own_history_step() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));

    test.window
        .navigate(&fixture.uri_of("Notes 2.txt"))
        .expect("valid location");
    wait_until("the file's folder", || {
        test.window.current_uri() == Some(fixture.uri())
    });
    test.wait_for_listing("the file's folder");
    test.window.go_history(Direction::Backward);
    test.wait_for_listing("the folder before");

    assert_eq!(test.window.current_uri(), Some(fixture.uri_of("Documents")));
    test.window.go_history(Direction::Forward);
    wait_until("forward to the folder", || {
        test.window.current_uri() == Some(fixture.uri())
    });
}
