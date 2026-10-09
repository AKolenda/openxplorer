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

/// Delivers a primary click on `check` to every click handler it has,
/// GTK's and the app's, presses then releases, as a click reaches them.
/// (Only the pointer itself, which tests cannot move, is left out.)
fn click(check: &gtk::CheckButton) {
    let clicks: Vec<gtk::GestureClick> = check
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .collect();
    assert!(clicks.len() >= 2, "GTK's click and the box's own");
    for gesture in &clicks {
        gesture.emit_by_name::<()>("pressed", &[&1_i32, &4.0_f64, &4.0_f64]);
    }
    for gesture in &clicks {
        gesture.emit_by_name::<()>("released", &[&1_i32, &4.0_f64, &4.0_f64]);
    }
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

    // A real click: every click handler on the box hears the press and
    // the release, GTK's own as well as the app's. The box must toggle
    // once, not twice (which selected the item and deselected it again).
    click(&check);
    assert_eq!(selected(&test), [1, 2], "a click keeps the rest of the selection");
    click(&check);
    assert_eq!(selected(&test), [1], "a second click deselects it alone");

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
/// (a minus) with some items selected and checked with all. Clicking it
/// with nothing selected selects every item; clicking it with some or all
/// selected selects none.
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
    // A click on the minus takes the selection away.
    click(&check);
    assert!(selected(&test).is_empty(), "the minus selects none");
    assert!(!check.is_active() && !check.is_inconsistent());
    // A click on the clear box selects every item.
    click(&check);
    assert_eq!(selected(&test).len(), usize::try_from(total).expect("few items"));
    assert!(check.is_active() && !check.is_inconsistent(), "checked with all");
    // A click on the checked box selects none.
    click(&check);
    assert!(selected(&test).is_empty());
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

/// A click a little outside the drawn box still hits it: the box reaches
/// 4px beyond what it draws on every side, without taking room from the
/// row, so what follows it sits as before.
///
/// parity: SEL-014
#[gtk::test]
fn a_click_just_outside_the_box_still_hits_it() {
    /// How far the box reaches past what it draws (folder-views.css).
    const REACH: f32 = 4.0;
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    let cell = cell_at(&test, 2);
    let check = cell.item_check();
    let bounds = check.compute_bounds(&cell).expect("laid out");
    let drawn_right = bounds.x() + bounds.width() - REACH;
    assert!(bounds.width() >= 14.0 + 2.0 * REACH, "{}", bounds.width());

    // What follows the box starts the row's usual gap after what it draws:
    // the reach takes no room.
    let next = check.next_sibling().expect("the box comes first");
    let next_left = next.compute_bounds(&cell).expect("laid out").x();
    #[expect(clippy::cast_precision_loss, reason = "a small spacing")]
    let gap = cell.spacing() as f32;
    assert!(
        (next_left - drawn_right - gap).abs() < 0.5,
        "next at {next_left}, box drawn to {drawn_right}, gap {gap}"
    );

    // Just right of, and just above, the drawn box.
    let middle = bounds.y() + bounds.height() / 2.0;
    let points = [
        (bounds.x() + bounds.width() - 2.0, middle),
        (bounds.x() + bounds.width() / 2.0, bounds.y() + 2.0),
    ];
    for (x, y) in points {
        let picked = cell
            .pick(f64::from(x), f64::from(y), gtk::PickFlags::DEFAULT)
            .expect("something is there");
        assert!(
            picked == check.clone().upcast::<gtk::Widget>() || picked.is_ancestor(&check),
            "({x}, {y}) hits the box, not {}",
            picked.type_().name()
        );
    }
}

/// A checked box is filled solid with the accent, as Explorer's is, not
/// with the lighter gradient GTK's theme lays over it, which made it look
/// washed out: its most painted colour is the light theme's accent.
///
/// parity: SEL-014
#[gtk::test]
fn a_checked_box_is_filled_solid_with_the_accent() {
    /// `ox_accent` in resources/light.css (C14).
    const ACCENT: [u8; 3] = [0x00, 0x67, 0xc0];
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.window.folder_pane().model().select_only(2);
    let check = cell_at(&test, 2).item_check();
    wait_for_frames(&test.window, 3);
    let paintable = gtk::WidgetPaintable::new(Some(&check));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(check.width()), f64::from(check.height()));
    let node = snapshot.to_node().expect("a checked box paints");
    let renderer = check
        .native()
        .and_then(|native| native.renderer())
        .expect("a drawn window");
    let texture = renderer.render_texture(&node, None::<&gtk::graphene::Rect>);
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    for pixel in pixel_rows(&texture).concat() {
        if pixel[3] == 255 {
            let [blue, green, red, _] = pixel;
            *counts.entry([red, green, blue]).or_default() += 1;
        }
    }
    let (most, _) = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .expect("opaque pixels");
    let close = most.iter().zip(ACCENT).all(|(a, b)| a.abs_diff(b) <= 3);
    assert!(close, "the fill is {most:02x?}, not the accent {ACCENT:02x?}");
}
