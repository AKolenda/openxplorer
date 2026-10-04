// SPDX-License-Identifier: AGPL-3.0-only
//! Collapsing This PC and Network in a real window (SIDE-033): with the
//! chevron, with Left and Right on the section's head, the head
//! highlighted while the open place is inside the collapsed section, and
//! a click on a chevron never taken for a click on its row.

use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, graphene};

use super::sidebar::{row_named, section_chevron};
use super::support::middle_click_at;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};

/// The labels of the sidebar rows shown, hidden ones left out.
fn shown_labels(test: &TestWindow) -> Vec<String> {
    let sidebar = test.window.sidebar();
    let labels = sidebar.labels();
    descendants::<gtk::ListBoxRow>(sidebar.list())
        .into_iter()
        .zip(labels)
        .filter(|(row, _)| row.is_visible())
        .map(|(_, label)| label)
        .collect()
}

/// Whether a row labelled `label` is shown.
fn shows(test: &TestWindow, label: &str) -> bool {
    shown_labels(test).iter().any(|shown| shown == label)
}

/// What the sidebar list's key controller does with `key`, no modifier
/// held, as if it were pressed where the focus is.
fn press(test: &TestWindow, key: gdk::Key) -> bool {
    let controller = test
        .window
        .sidebar()
        .list()
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("the sidebar list takes keys");
    let no_keycode = 0_u32;
    let modifiers = gdk::ModifierType::empty();
    controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &modifiers])
}

/// The middle of `widget` in the sidebar list's coordinates.
fn in_list(test: &TestWindow, widget: &impl IsA<gtk::Widget>) -> (f64, f64) {
    let list = test.window.sidebar().list();
    let bounds = widget.compute_bounds(list).unwrap_or_else(graphene::Rect::zero);
    let centre = bounds.center();
    (f64::from(centre.x()), f64::from(centre.y()))
}

/// Clicking This PC's chevron collapses the section, as Windows
/// Explorer's navigation pane does: its drives are hidden, the chevron
/// points right and offers to expand it, and the window stays where it
/// was. Clicking it again shows them. The collapse holds when the
/// sidebar's rows are rebuilt.
///
/// parity: SIDE-033
#[gtk::test]
fn the_this_pc_chevron_collapses_and_expands_its_section() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert!(shows(&test, "Local Disk"));
    let folder = test.window.current_uri();
    let chevron = section_chevron(&test, "This PC");
    assert_eq!(chevron.tooltip_text().as_deref(), Some("Collapse This PC"));
    assert!(gtk::test_accessible_has_state(
        &row_named(&test, "This PC"),
        gtk::AccessibleState::Expanded
    ));

    chevron.emit_clicked();
    wait_until("This PC to collapse", || !shows(&test, "Local Disk"));
    assert!(test.window.sidebar().section_is_collapsed("thisPc"));
    let chevron = section_chevron(&test, "This PC");
    assert_eq!(chevron.tooltip_text().as_deref(), Some("Expand This PC"));
    assert!(shows(&test, "This PC"), "the head stays");
    assert!(shows(&test, "Network"), "other sections stay");
    assert_eq!(test.window.current_uri(), folder, "the chevron opens nothing");

    // Rows rebuilt (a drive coming or going) keep the section collapsed.
    test.window.render_places();
    wait_for_frames(&test.window, 2);
    assert!(!shows(&test, "Local Disk"));
    assert_eq!(
        section_chevron(&test, "This PC").tooltip_text().as_deref(),
        Some("Expand This PC")
    );

    section_chevron(&test, "This PC").emit_clicked();
    wait_until("This PC to expand", || shows(&test, "Local Disk"));
    assert!(!test.window.sidebar().section_is_collapsed("thisPc"));
}

/// Left on This PC's row collapses the section and Right expands it
/// again, as in Explorer's navigation pane; a key that changes nothing,
/// or one pressed on a row that heads no section, is left to the list.
///
/// parity: SIDE-033, ACC-006
#[gtk::test]
fn left_and_right_on_a_section_head_collapse_and_expand_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    let this_pc = row_named(&test, "This PC");
    this_pc.grab_focus();

    assert!(press(&test, gdk::Key::Left), "Left collapses This PC");
    assert!(sidebar.section_is_collapsed("thisPc"));
    assert!(!shows(&test, "Local Disk"));
    assert!(!press(&test, gdk::Key::Left), "already collapsed");
    assert!(press(&test, gdk::Key::Right), "Right expands it");
    assert!(!sidebar.section_is_collapsed("thisPc"));
    assert!(shows(&test, "Local Disk"));
    assert!(!press(&test, gdk::Key::Right), "already expanded");

    row_named(&test, "Local Disk").grab_focus();
    assert!(!press(&test, gdk::Key::Left), "Local Disk heads no section");
    assert!(!sidebar.section_is_collapsed("thisPc"));
}

/// While the open place is inside a collapsed section, the section's head
/// is highlighted, as Explorer highlights the collapsed parent; expanded
/// again, the place's own row is.
///
/// parity: SIDE-033, SIDE-004
#[gtk::test]
fn a_collapsed_section_highlights_its_head_for_the_place_inside() {
    let test = TestWindow::open("file:///");
    let local_disk = row_named(&test, "Local Disk");
    let this_pc = row_named(&test, "This PC");
    assert!(local_disk.is_selected(), "the open drive is highlighted");

    section_chevron(&test, "This PC").emit_clicked();
    wait_until("This PC to be highlighted", || this_pc.is_selected());
    assert!(!local_disk.is_visible());

    section_chevron(&test, "This PC").emit_clicked();
    wait_until("Local Disk to be highlighted again", || local_disk.is_selected());
    assert!(!this_pc.is_selected());
}

/// A click on a chevron belongs to the chevron: the list's Ctrl+click and
/// middle-click, which open the row's place in a tab, leave it alone,
/// while a click on the name still finds the place.
///
/// parity: SIDE-033, SIDE-015
#[gtk::test]
fn a_click_on_a_chevron_opens_no_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    wait_for_frames(&test.window, 3);
    let sidebar = test.window.sidebar();
    let chevron = in_list(&test, &section_chevron(&test, "This PC"));
    let (_, name) = in_list(&test, &row_named(&test, "This PC"));
    let name = (chevron.0 + 60.0, name);
    assert_eq!(sidebar.clicked_location(chevron.0, chevron.1), None);
    assert!(
        sidebar.clicked_location(name.0, name.1).is_some(),
        "the name opens This PC"
    );

    let tabs = test.tab_listing_needs().len();
    middle_click_at(sidebar.list(), chevron);
    wait_for_frames(&test.window, 2);
    assert_eq!(test.tab_listing_needs().len(), tabs, "no tab opened");
}
