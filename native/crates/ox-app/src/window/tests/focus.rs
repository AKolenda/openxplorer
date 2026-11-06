// SPDX-License-Identifier: AGPL-3.0-only
//! Where the keyboard goes around the folder list: typing to select works
//! after the user clicks a place, a command, a crumb or the Details pane, not
//! only while the list itself has the keyboard, and no page switch or address
//! edit leaves it on a control GTK picked (such as the Details pane's close
//! button).
//!
//! GTK has no public way to synthesise input, so these tests emit the
//! `pressed` and `key-pressed` signals of the controllers a real click or key
//! reaches: the sidebar's rows, the window's own bubble-phase key handling and
//! the views' capture-phase one.

use std::path::PathBuf;

use gtk::gdk;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::file_uri;
use ox_core::places::FolderLocations;

use crate::folder_view::grid::IconSize;

use crate::test_support::harness::{descendants, settle, wait_until, Fixture, TestWindow};

/// A home with the standard folders the sidebar lists, so a row of it opens a
/// folder the test filled: the test home of the harness is the machine's
/// temporary one, whose folders the test does not set up.
struct TestHome {
    _root: tempfile::TempDir,
    home: PathBuf,
    config: PathBuf,
}

impl TestHome {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("the test home has room");
        let base = std::fs::canonicalize(root.path()).expect("the folder resolves");
        let home = base.join("home");
        let config = base.join("config");
        std::fs::create_dir_all(home.join("Documents")).expect("a folder");
        std::fs::create_dir_all(&config).expect("a folder");
        std::fs::write(home.join("Documents").join("Notes 2.txt"), "x").expect("a file");
        Self {
            _root: root,
            home,
            config,
        }
    }

    fn locations(&self) -> FolderLocations {
        FolderLocations::new(self.home.clone(), &self.config)
    }

    /// The URI of `name` in the home.
    fn uri_of(&self, name: &str) -> String {
        file_uri(&self.home.join(name))
    }
}

/// The sidebar row whose name is `label`.
fn place_row(test: &TestWindow, label: &str) -> gtk::ListBoxRow {
    let rows = descendants::<gtk::ListBoxRow>(test.window.sidebar().list());
    rows.into_iter()
        .find(|row| {
            descendants::<gtk::Label>(row)
                .into_iter()
                .any(|text| text.text() == label)
        })
        .unwrap_or_else(|| panic!("the sidebar lists {label}"))
}

/// Presses `key` as GTK delivers it to the window's own bubble-phase key
/// handling, which runs after the focused widget had its turn. Returns true
/// when the window took the key.
fn press_after_focus(test: &TestWindow, key: gdk::Key) -> bool {
    let no_keycode = 0_u32;
    let none = gdk::ModifierType::empty();
    let controllers: Vec<gtk::EventControllerKey> = test
        .window
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .filter(|controller| controller.propagation_phase() == gtk::PropagationPhase::Bubble)
        .collect();
    controllers.iter().any(|controller| {
        controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &none])
    })
}

/// The widget holding the keyboard, if any.
fn focused(test: &TestWindow) -> Option<gtk::Widget> {
    GtkWindowExt::focus(&test.window)
}

/// The Details pane's close button, the control the keyboard used to end up on.
fn details_close(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(test.window.details_pane())
        .into_iter()
        .find(|button| button.has_css_class("detail-close"))
        .expect("the Details pane has a close button")
}

/// The Back button of the navigation row.
fn back_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(&*test.window.imp().navigation_buttons)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.back"))
        .expect("the navigation row shows Back")
}

/// The Refresh button of the navigation row, which is always enabled.
fn refresh_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(&*test.window.imp().navigation_buttons)
        .into_iter()
        .find(|button| button.action_name().as_deref() == Some("win.refresh"))
        .expect("the navigation row shows Refresh")
}

/// parity: SIDE-002, SEL-020
#[gtk::test]
fn clicking_a_place_puts_the_keyboard_in_the_folder() {
    let home = TestHome::new();
    let folders = home.locations();
    let test = TestWindow::open_with_standard_folders(&file_uri(&home.home), folders, |_| {});
    test.wait_for_listing("the home folder");
    let documents = place_row(&test, "Documents");

    documents.grab_focus();
    documents.activate();
    test.wait_for_listing("the place");
    settle();

    assert_eq!(
        test.window.current_uri(),
        Some(home.uri_of("Documents")),
        "the row opened its folder"
    );
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the keyboard is in the list, not on the row clicked"
    );
    test.window.type_text("n");
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
}

/// parity: SEL-020
#[gtk::test]
fn a_typed_address_leaves_the_keyboard_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let path = fixture.path("Documents").display().to_string();

    test.activate("location", None);
    let entry = test.window.address_bar().entry();
    entry.set_text(&path);
    entry.emit_by_name::<()>("activate", &[]);
    wait_until("the typed folder", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    test.wait_for_listing("Documents");
    settle();

    assert!(
        test.window.folder_pane().view_has_focus(),
        "the keyboard followed the address into the list"
    );
}

/// parity: SEL-020
#[gtk::test]
fn ending_address_editing_moves_the_keyboard_out_of_the_entry_first() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");

    test.activate("location", None);
    assert_eq!(
        focused(&test)
            .map(|focus| focus.type_().name().to_string())
            .as_deref(),
        Some("GtkText"),
        "the address entry has the keyboard"
    );

    test.window.finish_address();

    assert_eq!(
        test.window.address_bar().mode(),
        crate::window::address_bar::AddressMode::Crumbs
    );
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the entry hands the keyboard to the list, not to the next control"
    );
}

/// parity: SEL-020, SEL-030
#[gtk::test]
fn typing_on_a_command_or_the_details_pane_selects_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.open_details_pane();
    let refresh = refresh_button(&test);
    let close = details_close(&test);

    for button in [refresh, close] {
        test.window.folder_pane().model().select_none();
        assert!(button.grab_focus(), "the control takes the keyboard");
        assert!(!test.window.folder_pane().view_has_focus());

        assert!(
            press_after_focus(&test, gdk::Key::n),
            "the window takes the typed letter"
        );

        assert_eq!(
            test.selected_names(),
            ["Notes 2.txt"],
            "the letter selected the name it starts"
        );
        assert!(
            test.window.folder_pane().view_has_focus(),
            "the keyboard moved from the control into the list"
        );
    }
}

/// parity: SEL-020
#[gtk::test]
fn a_landing_page_never_keeps_the_keyboard_on_the_hidden_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");

    test.window.navigate("This PC").expect("a page");
    settle();

    assert!(!test.window.folder_pane().view_has_focus());
    let focus = focused(&test).expect("a landing page card takes the keyboard");
    assert!(
        focus.is_ancestor(test.window.folder_pane().landing()),
        "the keyboard is on the page shown, not on the list it replaced"
    );
}

/// parity: NAV-002
#[gtk::test]
fn going_back_puts_the_keyboard_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("Documents");
    let back = back_button(&test);
    back.grab_focus();

    back.emit_clicked();

    wait_until("the folder left", || {
        test.window.current_uri() == Some(fixture.uri())
    });
    test.wait_for_listing("the first folder");
    settle();
    assert!(
        test.window.folder_pane().view_has_focus(),
        "Back returns the keyboard to the list"
    );
}

/// parity: SEL-020
#[gtk::test]
fn switching_views_keeps_the_keyboard_in_the_list() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    let notes = test.position_of("Notes 2.txt");
    test.window.folder_pane().focus_view();
    test.window.folder_model().select_only(notes);
    test.window.folder_pane().focus_item(notes);
    assert!(test.window.folder_pane().view_has_focus());

    test.activate("view", Some("large"));
    crate::test_support::harness::wait_for_frames(&test.window, 3);

    assert!(
        test.window.folder_pane().view_has_focus(),
        "the view that replaces the list takes the keyboard"
    );
    assert_eq!(test.selected_names(), ["Notes 2.txt"], "the selection stays");
}

/// parity: VIEW-027
#[gtk::test]
fn closing_the_details_pane_puts_the_keyboard_in_the_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.open_details_pane();
    let close = details_close(&test);
    assert!(close.grab_focus(), "the pane's close button takes the keyboard");

    test.activate("details-pane", None);

    wait_until("the pane to hide", || !test.window.details_pane().is_visible());
    assert!(
        test.window.folder_pane().view_has_focus(),
        "the folder takes the keyboard the hidden pane had"
    );
}

/// parity: SEL-020
#[gtk::test]
fn switching_views_on_the_unavailable_page_keeps_try_again() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("go-to", Some(&fixture.uri_of("Gone")));
    wait_until("the unavailable page", || {
        !test.window.is_loading()
            && test.window.folder_pane().page() == Some(crate::window::folder_pane::PanePage::Empty)
    });
    let retry = descendants::<gtk::Button>(&test.window.folder_pane().empty_area())
        .into_iter()
        .find(WidgetExt::is_visible)
        .expect("Try again shows");
    retry.grab_focus();

    test.window
        .show_view(crate::window::FolderView::Icons(IconSize::LARGE));

    assert!(
        retry.has_focus(),
        "switching the hidden list leaves the page's own control alone"
    );
}

/// A letter no row of an open menu takes stays with the menu: typing on the
/// window's chrome goes to the folder, typing in its menus does not.
///
/// parity: SEL-030
#[gtk::test]
fn typing_in_a_menu_leaves_the_folder_alone() {
    let fixture = Fixture::standard();
    let test = super::geometry::laid_out(&fixture.uri());
    test.wait_for_listing("the first folder");
    let view = super::command_bar::menu_of(&test, "View");
    view.popup();
    crate::test_support::harness::wait_for_frames(&test.window, 2);
    test.window.folder_pane().model().select_none();
    assert!(view.row("Terminal").grab_focus(), "a menu row takes the keyboard");

    assert!(
        !press_after_focus(&test, gdk::Key::n),
        "the window leaves the letter to the menu"
    );
    assert!(view.is_visible(), "the menu stays open");
    assert!(
        test.selected_names().is_empty(),
        "nothing was typed into the folder"
    );
    view.popdown();
}

/// A letter typed on a tab stays with the tab strip.
///
/// parity: SEL-030
#[gtk::test]
fn typing_on_a_tab_leaves_the_folder_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.window.folder_pane().model().select_none();
    let tab = descendants::<gtk::Widget>(test.window.tab_strip())
        .into_iter()
        .find(|widget| widget.is_focusable() && widget.is_visible())
        .expect("a tab takes the keyboard");
    assert!(tab.grab_focus(), "the tab takes the keyboard");

    assert!(
        !press_after_focus(&test, gdk::Key::n),
        "the window leaves the letter to the tab strip"
    );
    assert!(
        test.selected_names().is_empty(),
        "nothing was typed into the folder"
    );
}

/// While "Working on it..." hides the previous folder's rows, a letter typed
/// on a command selects none of them.
///
/// parity: SEL-030
#[gtk::test]
fn typing_while_a_location_opens_selects_nothing_hidden() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the first folder");
    test.window.folder_pane().model().select_none();
    test.window.folder_pane().show_working_note(true);
    let refresh = refresh_button(&test);
    assert!(refresh.grab_focus(), "the command takes the keyboard");

    assert!(
        !press_after_focus(&test, gdk::Key::n),
        "the window leaves the letter alone"
    );
    assert!(test.selected_names().is_empty(), "no hidden row was selected");
}
