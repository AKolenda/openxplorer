// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of the drop targets: where a drop at each point goes, what
//! highlights it, and drops onto programs.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use super::highlight::VIEW_DROP_CLASS;
use super::*;
use crate::test_support::harness::{
    capture, capture_popover, wait_for, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::file_drop::{DropAction, DropDestination};

/// Where a drop on the item called `name`, or on blank space for
/// `None`, goes in `test`'s folder view.
fn destination_of(test: &TestWindow, name: Option<&str>) -> Option<DropDestination> {
    let position = name.map(|name| test.position_of(name));
    test.window
        .folder_view_spot(position)
        .map(|spot| spot.destination())
}

/// The middle of `widget` in `ancestor`'s coordinates.
fn middle_of(widget: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let bounds = widget
        .compute_bounds(ancestor)
        .expect("a shown widget has bounds");
    let x = bounds.x() + bounds.width() / 2.0;
    let y = bounds.y() + bounds.height() / 2.0;
    (f64::from(x), f64::from(y))
}

/// A copy of `cp` called "copier" in `fixture`: a program that shows
/// which arguments it got by what it creates.
fn install_copier(fixture: &Fixture) {
    let copier = fixture.path("copier");
    std::fs::copy("/usr/bin/cp", &copier).expect("the test system has cp");
    std::fs::set_permissions(&copier, std::fs::Permissions::from_mode(0o755)).expect("the fixture is ours");
}

/// parity: DND-011
#[gtk::test]
fn a_drop_goes_into_the_folder_under_the_pointer_or_the_folder_shown() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let shown = Some(DropDestination::Folder(fixture.uri()));

    let on_folder = destination_of(&test, Some("Documents"));
    let on_file = destination_of(&test, Some("Notes 2.txt"));
    let on_blank = destination_of(&test, None);
    test.window.search_box().entry().set_text("Notes");
    wait_until("the search to filter", || test.window.is_searching());
    let while_searching = destination_of(&test, None);

    assert_eq!(
        on_folder,
        Some(DropDestination::Folder(fixture.uri_of("Documents")))
    );
    assert_eq!(on_file, shown, "a plain file takes no drop; its folder does");
    assert_eq!(on_blank, shown);
    assert_eq!(while_searching, None, "search results take no drop");
}

/// parity: DND-011
#[gtk::test]
fn only_the_folder_under_a_drag_is_highlighted() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let documents = test.position_of("Documents");
    let owners = || test.window.folder_pane().owners();
    let view = test.window.folder_pane().view_widget();

    let on_folder = test.window.folder_view_spot(Some(documents));
    test.window
        .show_drop_spot(DropZone::FolderView, on_folder.as_ref());
    let row_while_on_folder = owners().is_shown_drop_target(documents);
    let view_while_on_folder = view.has_css_class(VIEW_DROP_CLASS);
    let on_blank = test.window.folder_view_spot(None);
    test.window
        .show_drop_spot(DropZone::FolderView, on_blank.as_ref());
    let row_while_on_blank = owners().is_shown_drop_target(documents);
    let view_while_on_blank = view.has_css_class(VIEW_DROP_CLASS);
    test.window.leave_drop_zone(DropZone::FolderView);

    assert_eq!(row_while_on_folder, Some(true));
    assert!(!view_while_on_folder);
    assert_eq!(row_while_on_blank, Some(false));
    assert!(view_while_on_blank, "blank space means the folder shown");
    assert_eq!(owners().is_shown_drop_target(documents), Some(false));
    assert!(
        !view.has_css_class(VIEW_DROP_CLASS),
        "no highlight once the drag left"
    );
}

/// parity: DND-009, DND-011, DND-014
#[gtk::test]
fn sidebar_places_take_drops_and_quick_access_pins_where_the_line_shows() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    let home = sidebar.middle_of("Home");
    let documents = sidebar.middle_of("Documents");
    let this_pc = sidebar.middle_of("This PC");

    let on_home = test.window.sidebar_spot(home);
    let above_documents = test.window.sidebar_spot(documents - 5.0);
    test.window
        .show_drop_spot(DropZone::Sidebar, above_documents.as_ref());
    let line = sidebar.drop_highlight_of("Documents");
    test.window.leave_drop_zone(DropZone::Sidebar);

    let home_uri = test.window.imp().locations.borrow().home_uri();
    let on_home = on_home.map(|spot| spot.destination());
    assert_eq!(on_home, Some(DropDestination::Folder(home_uri)));
    let pin_spot = above_documents.map(|spot| spot.destination());
    assert!(
        matches!(&pin_spot, Some(DropDestination::QuickAccess { before: Some(before) }) if before.ends_with("/Documents")),
        "{pin_spot:?}"
    );
    assert_eq!(line, Some("drop-before"));
    assert_eq!(sidebar.drop_highlight_of("Documents"), None);
    assert_eq!(test.window.sidebar_spot(this_pc), None, "a page takes no drop");
}

/// parity: DND-014
#[gtk::test]
fn folders_dropped_on_quick_access_are_pinned_and_files_are_not() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let destination = Some(DropDestination::QuickAccess { before: None });

    let folder_taken = test
        .window
        .complete_drop(&[fixture.uri()], destination.clone(), DropAction::Copy);
    wait_until("the pin", || {
        test.window
            .sidebar()
            .labels()
            .contains(&"Example projects".to_owned())
    });
    let pinned_message = test.window.shown_message();
    let file_taken =
        test.window
            .complete_drop(&[fixture.uri_of("Notes 2.txt")], destination, DropAction::Copy);
    wait_until("the refusal", || {
        test.window.shown_message().starts_with("Could not pin")
    });

    assert!(folder_taken && file_taken, "both are checked off the main thread");
    assert_eq!(pinned_message, "Pinned to Quick access. No files were moved.");
    assert!(!test.window.sidebar().labels().contains(&"Notes 2.txt".to_owned()));
}

/// parity: DND-011, DND-016
#[gtk::test]
fn a_crumb_takes_drops_for_its_folder() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let address_bar = test.window.address_bar();
    let crumbs = address_bar.crumb_buttons();
    let parent = crumbs.iter().rev().nth(1).expect("the folder has a parent crumb");
    let (x, y) = middle_of(parent, address_bar);

    let spot = test
        .window
        .drop_spot(DropZone::Breadcrumbs, address_bar.upcast_ref(), x, y);
    test.window.show_drop_spot(DropZone::Breadcrumbs, spot.as_ref());
    let highlighted = parent.has_css_class("file-drop-active");
    test.window.leave_drop_zone(DropZone::Breadcrumbs);

    assert_eq!(
        spot.map(|spot| spot.destination()),
        Some(DropDestination::Folder(fixture.uri()))
    );
    assert!(highlighted);
    assert!(!parent.has_css_class("file-drop-active"));
}

/// parity: TAB-018, DND-016
#[gtk::test]
fn a_drop_on_a_tab_goes_into_its_folder_and_hovering_shows_the_tab() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a folder");
    test.wait_for_listing("the second tab");
    test.activate("previous-tab", None);
    // Showing a tab draws the strip's tabs anew.
    wait_for_frames(&test.window, 3);
    let strip = test.window.tab_strip();
    let second_tab = strip.tab_list().last_child().expect("two tabs");
    let (x, y) = middle_of(&second_tab, strip);

    let spot = test.window.drop_spot(DropZone::Tabs, strip.upcast_ref(), x, y);
    test.window.show_drop_spot(DropZone::Tabs, spot.as_ref());
    let is_highlighted = second_tab.has_css_class("file-drop-active");
    let before_the_delay = test.window.current_uri();
    wait_until("the hovered tab to show", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
    test.window.leave_drop_zone(DropZone::Tabs);

    assert_eq!(
        spot.map(|spot| spot.destination()),
        Some(DropDestination::Folder(fixture.uri_of("Documents")))
    );
    assert!(is_highlighted);
    assert_eq!(
        before_the_delay,
        Some(fixture.uri()),
        "a tab shows only after the hover delay"
    );
}

/// parity: DND-020, DND-026
#[gtk::test]
fn items_dropped_on_a_program_are_given_to_it_as_arguments() {
    let fixture = Fixture::standard();
    install_copier(&fixture);
    let test = TestWindow::open(&fixture.uri());
    test.window.refresh();
    wait_until("the program to be listed", || {
        test.names().contains(&"copier".to_owned())
    });
    let copier = test.position_of("copier");

    let first_look = test.window.item_destination(copier);
    wait_until("GIO's answer", || test.window.item_destination(copier).is_some());
    let Some(DropDestination::Program(program)) = test.window.item_destination(copier) else {
        panic!("the copier is a program");
    };
    let spot = test.window.folder_view_spot(Some(copier));
    test.window.show_drop_spot(DropZone::FolderView, spot.as_ref());
    let hint = test.window.folder_pane().drag_hint();
    test.window.leave_drop_zone(DropZone::FolderView);
    let copy_name = "Notes 2 (dropped).txt";
    let dropped = vec![fixture.uri_of("Notes 2.txt"), fixture.uri_of(copy_name)];
    test.window.open_with_program(program, dropped);

    assert_eq!(first_look, None, "unknown until GIO answers");
    assert_eq!(hint.as_deref(), Some("Open with copier"));
    wait_until("the program to run", || fixture.path(copy_name).is_file());
    assert_eq!(test.window.folder_pane().drag_hint(), None);
}

/// parity: DND-026
#[gtk::test]
fn a_file_that_is_not_executable_is_no_program() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let notes = test.position_of("Notes 2.txt");

    test.window.item_destination(notes);
    wait_for(Duration::from_millis(200));

    assert_eq!(test.window.item_destination(notes), None);
}

/// With `OX_NATIVE_CAPTURE_DIR` set, saves the drop highlights (a
/// folder row, the Quick access line and a crumb), the program hint
/// and the drop menu in both themes; without it, proves they show.
#[gtk::test]
fn the_drop_highlights_the_program_hint_and_the_drop_menu_are_captured() {
    let _theme = ThemeGuard::keep();
    let source = Fixture::standard();
    let fixture = Fixture::standard();
    install_copier(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let copier = test.position_of("copier");
    let sidebar = test.window.sidebar();
    for theme in ["light", "dark"] {
        test.activate("theme", Some(theme));
        // A new theme draws the sidebar's rows anew; a drag's next
        // motion would mark the new ones.
        wait_for_frames(&test.window, 3);
        let on_folder = test.window.folder_view_spot(Some(test.position_of("Documents")));
        test.window
            .show_drop_spot(DropZone::FolderView, on_folder.as_ref());
        let pin_line = test.window.sidebar_spot(sidebar.middle_of("Documents") - 5.0);
        test.window.show_drop_spot(DropZone::Sidebar, pin_line.as_ref());
        capture(&test.window, &format!("native-drop-targets-{theme}.png"));
        test.window.leave_drop_zone(DropZone::Sidebar);
        // Leaving the view forgot GIO's answers; a new drag asks again.
        wait_until("GIO's answer", || test.window.item_destination(copier).is_some());
        let on_program = test.window.folder_view_spot(Some(copier));
        test.window
            .show_drop_spot(DropZone::FolderView, on_program.as_ref());
        capture(&test.window, &format!("native-drop-program-{theme}.png"));
        test.window.leave_drop_zone(DropZone::FolderView);
        test.window
            .remember_drop_point(test.window.folder_pane().upcast_ref(), 300.0, 200.0);
        test.window
            .drop_files(&[source.uri_of("Notes 2.txt")], None, DropAction::Ask);
        let menu = test.window.drop_menu();
        wait_until("the drop menu", || menu.is_mapped());
        capture_popover(
            &test.window,
            menu.upcast_ref(),
            &format!("native-drop-menu-{theme}.png"),
        );
        menu.popdown();
        wait_until("the menu to close", || !menu.is_mapped());
    }
}
