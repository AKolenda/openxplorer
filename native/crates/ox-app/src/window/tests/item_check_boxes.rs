// SPDX-License-Identifier: AGPL-3.0-only
//! Item check boxes in a real window, as Windows 11's (SEL-014): each item
//! has a check box that is checked while it is selected and selects or
//! deselects it alone; it shows while its row or tile is hovered or
//! selected; the Details header has one that selects all or none; and
//! View > Item check boxes turns them all off, saved for every window.

use gtk::glib;
use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::geometry::laid_out;
use super::icons::pixel_rows;
use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};

/// The selected positions, in order.
fn selected(test: &TestWindow) -> Vec<u32> {
    test.window.folder_model().selected_positions()
}

/// The cell at `position` of the view shown.
fn cell_at(test: &TestWindow, position: u32) -> FileCell {
    let pane = test.window.folder_pane();
    pane.owners()
        .file_cell_at(position, &pane.view_widget())
        .expect("on screen")
}

/// Whether the window's Item check boxes toggle is checked.
fn toggle_is_on(test: &TestWindow) -> Option<bool> {
    test.window
        .lookup_action("item-check-boxes")?
        .state()?
        .get::<bool>()
}

/// The row or tile widget `cell` sits in: the ancestor whose state the
/// stylesheet's `:hover` and `:selected` read.
fn row_of(cell: &FileCell) -> gtk::Widget {
    std::iter::successors(cell.parent(), WidgetExt::parent)
        .find(|widget| matches!(widget.css_name().as_str(), "row" | "child"))
        .expect("a cell sits in a row or a tile")
}

/// How many pixels of `cell`'s check box, as it paints itself with its
/// stylesheet opacity, are covered at least half: none while the box is
/// transparent. (A painting of the whole cell is cropped to what draws,
/// so its pixels cannot be matched to the box's place.)
fn check_ink(cell: &FileCell) -> usize {
    ink_of(&cell.item_check())
}

/// How many pixels of `check`, as it paints itself, are covered at least
/// half.
fn ink_of(check: &gtk::CheckButton) -> usize {
    wait_for_frames(&check.root().expect("on a window"), 3);
    let paintable = gtk::WidgetPaintable::new(Some(check));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(check.width()), f64::from(check.height()));
    let Some(node) = snapshot.to_node() else {
        return 0;
    };
    let renderer = check
        .native()
        .and_then(|native| native.renderer())
        .expect("a drawn window");
    let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
    pixel_rows(&texture)
        .concat()
        .iter()
        .filter(|pixel| pixel[3] >= 128)
        .count()
}

/// Clicking an item's check box selects it and keeps the rest of the
/// selection, as a Ctrl+click does; clicking again deselects it alone.
/// The box is checked exactly while its item is selected, however it was
/// selected.
///
/// parity: SEL-014
#[gtk::test]
fn an_item_check_box_selects_its_item_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    pane.model().select_only(1);
    let check = cell_at(&test, 2).item_check();
    assert!(!check.is_active());
    assert!(
        cell_at(&test, 1).item_check().is_active(),
        "a selected item is checked"
    );

    // A click asks the box for the other state.
    check.set_active(true);
    assert_eq!(selected(&test), [1, 2]);
    assert!(check.is_active());
    check.set_active(false);
    assert_eq!(selected(&test), [1]);
    assert!(!check.is_active());

    // A press on the box is the box's own, before the row sees it, which
    // would open the item on a quick second click; the release toggles.
    let clicks: Vec<gtk::GestureClick> = check
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .filter(|click| click.propagation_phase() == gtk::PropagationPhase::Capture)
        .collect();
    let own = clicks.last().expect("the box takes its own clicks");
    own.emit_by_name::<()>("released", &[&1_i32, &4.0_f64, &4.0_f64]);
    assert_eq!(selected(&test), [1, 2], "a click keeps the rest of the selection");
    own.emit_by_name::<()>("released", &[&1_i32, &4.0_f64, &4.0_f64]);
    assert_eq!(selected(&test), [1]);

    pane.model().select_all();
    assert!(check.is_active(), "Select all checks every box");
    pane.model().select_none();
    assert!(!check.is_active());
}

/// The box keeps its room but shows only while its row is hovered,
/// anywhere on it, or its item is selected; on a tile it sits on the
/// icon's corner and shows the same way.
///
/// parity: SEL-014
#[gtk::test]
fn an_item_check_box_shows_on_hover_and_while_selected() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    for view in ["details", "large"] {
        test.activate("view", Some(view));
        wait_for_frames(&test.window, 3);
        let cell = cell_at(&test, 2);
        let row = row_of(&cell);
        assert!(
            cell.item_check().is_visible(),
            "{view}: on by default, as in Windows 11"
        );
        assert_eq!(
            check_ink(&cell),
            0,
            "{view}: hidden while not hovered or selected"
        );

        row.set_state_flags(gtk::StateFlags::PRELIGHT, false);
        assert!(check_ink(&cell) > 0, "{view}: shown while its row is hovered");
        row.unset_state_flags(gtk::StateFlags::PRELIGHT);
        assert_eq!(check_ink(&cell), 0, "{view}: hidden again");

        test.window.folder_pane().model().select_only(2);
        assert!(check_ink(&cell) > 0, "{view}: shown, checked, while selected");
        test.window.folder_pane().model().select_none();
    }
    // In Details the box comes first, before the icon.
    test.activate("view", Some("details"));
    wait_for_frames(&test.window, 3);
    let cell = cell_at(&test, 2);
    assert_eq!(cell.first_child(), Some(cell.item_check().upcast()));
}

/// The box before the Name title is clear with nothing selected, mixed
/// with some items selected and checked with all; clicking it selects
/// every item, or none when every item is selected.
///
/// parity: SEL-014
#[gtk::test]
fn the_header_check_box_selects_all_or_none() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let model = pane.model();
    let check = pane.details().select_all_check().expect("a select-all box");
    wait_for_frames(&test.window, 3);
    let total = model.selection().n_items();
    assert!(total > 2);
    let name_title = crate::folder_view::column_titles::title_buttons(pane.details().column_view())
        .into_iter()
        .next()
        .expect("a Name title");
    assert!(check.is_ancestor(&name_title), "before the Name title");
    assert!(!check.is_active() && !check.is_inconsistent());
    assert_eq!(ink_of(&check), 0, "hidden with nothing selected");
    let header = name_title.parent().expect("the title row");
    header.set_state_flags(gtk::StateFlags::PRELIGHT, false);
    assert!(ink_of(&check) > 0, "shown while the pointer is on the titles");
    header.unset_state_flags(gtk::StateFlags::PRELIGHT);

    model.select_only(1);
    assert!(check.is_inconsistent() && !check.is_active(), "mixed with some");
    assert!(ink_of(&check) > 0, "shown, mixed, while some are selected");
    // A click on a mixed box selects every item.
    check.set_active(!check.is_active());
    assert_eq!(selected(&test).len(), usize::try_from(total).expect("few items"));
    assert!(check.is_active() && !check.is_inconsistent(), "checked with all");
    // A click on a checked box selects none.
    check.set_active(!check.is_active());
    assert!(selected(&test).is_empty());
    assert!(!check.is_active() && !check.is_inconsistent());
}

/// View > Item check boxes turns every box off, the header's too, so they
/// take no room, and saves the choice; turned on again, they are back.
///
/// parity: SEL-014
#[gtk::test]
fn view_item_check_boxes_turns_them_off_and_is_saved() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    assert_eq!(toggle_is_on(&test), Some(true), "on by default, as in Windows 11");
    let cell = cell_at(&test, 1);
    let header = test
        .window
        .folder_pane()
        .details()
        .select_all_check()
        .expect("a select-all box");

    test.activate("item-check-boxes", None);
    assert_eq!(toggle_is_on(&test), Some(false));
    assert!(!cell.item_check().is_visible() && !header.is_visible());
    wait_until("the choice to be saved", || {
        !test.context.settings_data().preferences.selection_marker
    });

    test.activate("item-check-boxes", None);
    assert!(cell.item_check().is_visible() && header.is_visible());
    wait_until("the choice to be saved", || {
        test.context.settings_data().preferences.selection_marker
    });
}

/// Item check boxes turned off in Settings, or by another window, reach
/// an open window at once: its toggle clears and its boxes go.
///
/// parity: SEL-014
#[gtk::test]
fn item_check_boxes_turned_off_elsewhere_go_at_once() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let cell = cell_at(&test, 1);
    let off = PreferencesUpdate {
        selection_marker: Some(false),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&off)
        .expect("the settings file takes the change");
    test.context.reload_settings();
    wait_until("the window to follow", || toggle_is_on(&test) == Some(false));
    assert!(!cell.item_check().is_visible());
}
