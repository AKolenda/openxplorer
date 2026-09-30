// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of the drop targets: where a drop at each point goes, what
//! highlights it, and drops onto programs.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use super::highlight::VIEW_DROP_CLASS;
use super::*;
use crate::locations::Page;
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

/// A server's share list and a page take no drop; a share and a local
/// folder do.
///
/// parity: OPS-036
#[gtk::test]
fn a_server_listing_takes_no_drops() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    assert!(!test.window.takes_drops("smb://nas/"));
    assert!(!test.window.takes_drops(Page::Network.uri()));
    assert!(test.window.takes_drops("smb://nas/share/"));
    assert!(test.window.takes_drops(&fixture.uri()));
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

/// A drive still to be mounted takes a drop, which mounts it first
/// and then copies the items into its root; a volume that cannot be
/// mounted says so and copies nothing.
///
/// parity: DEV-010
#[gtk::test]
fn a_drop_on_an_unmounted_drive_mounts_it_first() {
    use crate::volumes::{VolumeKind, VolumeRow, VolumeState};

    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.imp().volumes.replace(vec![VolumeRow {
        label: "USB stick".to_owned(),
        kind: VolumeKind::Drive,
        state: VolumeState::Mountable {
            id: "gone-volume".to_owned(),
        },
    }]);
    test.window.render_places();
    wait_for_frames(&test.window, 2);
    let drive = test.window.sidebar().middle_of("USB stick");

    let spot = test.window.sidebar_spot(drive).map(|spot| spot.destination());
    assert_eq!(spot, Some(DropDestination::Volume("gone-volume".to_owned())));
    let window = test.window.clone();
    let dropped = vec![fixture.uri_of("Notes 2.txt")];
    glib::spawn_future_local(async move {
        let destination = DropDestination::Volume("gone-volume".to_owned());
        window.deliver_drop(&dropped, destination, DropAction::Copy).await;
    });
    let says_so = |window: gtk::Window| {
        crate::test_support::harness::descendants::<gtk::Label>(&window)
            .iter()
            .any(|label| label.text() == "Could not mount device")
    };
    let failure = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|window| window.downcast::<gtk::Window>().ok())
            .find(|window| window.is_visible() && says_so(window.clone()))
    };
    wait_until("the mount failure", || failure().is_some());
    // A drop waits while a dialog is open (DND-006): OK dismisses it.
    let ok = failure().and_then(|dialog| {
        crate::test_support::harness::descendants::<gtk::Button>(&dialog)
            .into_iter()
            .find(|button| button.label().as_deref() == Some("OK"))
    });
    ok.expect("the failure has OK").emit_clicked();
    wait_until("the failure to close", || failure().is_none());

    // A drive that mounts receives the items in its root.
    std::fs::create_dir(fixture.path("USB")).expect("the drive's root");
    let root = fixture.uri_of("USB");
    test.window
        .imp()
        .test_volume
        .replace(Some(("usb-volume".to_owned(), root)));
    let window = test.window.clone();
    let dropped = vec![fixture.uri_of("Notes 2.txt")];
    glib::spawn_future_local(async move {
        let destination = DropDestination::Volume("usb-volume".to_owned());
        window.deliver_drop(&dropped, destination, DropAction::Copy).await;
    });
    wait_until("the copy on the drive", || {
        fixture.path("USB/Notes 2.txt").exists()
    });
    assert!(fixture.path("Notes 2.txt").exists(), "a copy keeps the original");
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

/// parity: DND-020
#[gtk::test]
fn items_dropped_on_a_trusted_launcher_start_its_application() {
    let fixture = Fixture::standard();
    let launched = fixture.path("launched.txt");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Copier\nExec=cp %f \"{}\"\n",
        launched.display()
    );
    let launcher = fixture.path("copier.desktop");
    std::fs::write(&launcher, entry).expect("the fixture is ours");
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).expect("the fixture is ours");
    let test = TestWindow::open(&fixture.uri());
    let position = test.position_of("copier.desktop");

    test.window.item_destination(position);
    wait_until("GIO's answer", || {
        test.window.item_destination(position).is_some()
    });
    let Some(DropDestination::Program(program)) = test.window.item_destination(position) else {
        panic!("a trusted launcher takes drops");
    };
    let name = program.name.clone();
    test.window
        .open_with_program(program, vec![fixture.uri_of("Notes 2.txt")]);

    assert_eq!(name, "Copier", "named after its application");
    wait_until("the application to run", || launched.is_file());
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

/// parity: DND-021
#[gtk::test]
fn a_drag_that_stays_over_a_folder_opens_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let documents = test
        .window
        .folder_view_spot(Some(test.position_of("Documents")))
        .expect("a folder takes drops");
    let blank = test
        .window
        .folder_view_spot(None)
        .expect("the folder shown takes drops");
    assert_eq!(
        super::spring::folder_to_open(&blank),
        None,
        "the folder shown is open already"
    );

    test.window.show_drop_spot(DropZone::FolderView, Some(&documents));
    test.window.show_drop_spot(DropZone::FolderView, None);
    wait_for(Duration::from_millis(900));
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "leaving stops the wait"
    );

    test.window.show_drop_spot(DropZone::FolderView, Some(&documents));
    wait_until("the hovered folder to open", || {
        test.window.current_uri() == Some(fixture.uri_of("Documents"))
    });
}

/// A drag held still near the bottom of the file list scrolls it, and
/// the folder that was under the pointer does not open meanwhile;
/// leaving stops the scroll.
///
/// parity: DND-025, DND-021
#[gtk::test]
fn a_drag_near_the_bottom_edge_scrolls_without_opening_the_folder_under_it() {
    let fixture = Fixture::standard();
    for number in 0..80 {
        std::fs::create_dir(fixture.path(&format!("Folder {number:02}"))).expect("the fixture is ours");
    }
    let test = TestWindow::open(&fixture.uri());
    let view = test.window.folder_pane().view_widget();
    let scroller: gtk::ScrolledWindow = view
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast()
        .expect("the view scrolls");
    let adjustment = scroller.vadjustment();
    let near_bottom = f64::from(scroller.height()) - 5.0;

    let spot = test
        .window
        .hover_drop_at(DropZone::FolderView, &view, 40.0, near_bottom);
    wait_until("the list to scroll", || adjustment.value() > 100.0);
    wait_for(Duration::from_millis(900));
    let shown_while_scrolling = test.window.current_uri();
    test.window.leave_drop_zone(DropZone::FolderView);
    let stopped_at = adjustment.value();
    wait_for(Duration::from_millis(100));

    assert!(
        matches!(
            spot,
            Some(spot::DropSpot::FolderView {
                destination: DropDestination::Folder(_),
                row: Some(_)
            })
        ),
        "the pointer is over a folder"
    );
    assert_eq!(shown_while_scrolling, Some(fixture.uri()), "no folder opened");
    assert!(
        (adjustment.value() - stopped_at).abs() < f64::EPSILON,
        "leaving stops the scroll"
    );
}

/// A folder dropped on the dashed row of an empty Quick access is pinned.
///
/// parity: DND-014
#[gtk::test]
fn a_folder_dropped_on_the_pin_row_of_an_empty_quick_access_is_pinned() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    for place in test.window.places().quick_access {
        test.window.unpin(&place.uri);
    }
    wait_until("Quick access to empty", || {
        sidebar.labels().contains(&"Pin to Quick access".to_owned())
    });
    // The new rows are laid out on the next frames; before that the pin
    // row has no place to find.
    wait_for_frames(&test.window, 3);

    let spot = test.window.sidebar_spot(sidebar.middle_of("Pin to Quick access"));
    let destination = spot.map(|spot| spot.destination());
    assert_eq!(destination, Some(DropDestination::QuickAccess { before: None }));
    let taken = test
        .window
        .complete_drop(&[fixture.uri()], destination, DropAction::Copy);

    assert!(taken);
    wait_until("the pin", || {
        sidebar.labels().contains(&"Example projects".to_owned())
    });
}
