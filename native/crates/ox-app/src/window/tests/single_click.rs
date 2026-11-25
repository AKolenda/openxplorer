// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options' single-click mode with a real pointer on check.py's
//! private display (SEL-018): GTK's own gestures decide which press opens,
//! selects or draws a band, so only real clicks show that a plain click
//! opens an item in every view, that pointing selects it, and that a Ctrl
//! click, a check box, a rubber band and the second click of a
//! double-click open nothing.

use gtk::gdk;
use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::right_drag::{blank_on_screen, item_on_screen, HOLD, PROBE};
use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Button, Pointer};
use crate::window::single_click::{ClickMode, HOVER_SELECT_DELAY, POINTED_CLASS};

/// Saves `update` as another process would, and has the window read it.
fn save(test: &TestWindow, update: &PreferencesUpdate) {
    Settings::open(test.settings_directory())
        .update_preferences(update)
        .expect("the settings file takes the change");
    test.context.reload_settings();
}

/// Turns Folder Options' single-click mode on, as OK there would, and
/// waits until the window follows it.
fn single_click(test: &TestWindow, extra: PreferencesUpdate) {
    save(
        test,
        &PreferencesUpdate {
            single_click_open: Some(true),
            ..extra
        },
    );
    wait_until("the window to follow single-click", || {
        matches!(test.window.click_mode(), ClickMode::Single { .. })
    });
}

/// The icon-and-name cell of `name` in the view shown.
fn cell_of(test: &TestWindow, name: &str) -> FileCell {
    let pane = test.window.folder_pane();
    pane.owners()
        .file_cell_at(test.position_of(name), &pane.view_widget())
        .expect("the item is shown")
}

/// How long to wait so that a click is not the second of a double-click,
/// and pointing has had its time to select.
fn pause() {
    wait_for(HOVER_SELECT_DELAY * 2);
}

/// With single-click on, in Details, List, Small and Large icons, Tiles
/// and Content: the pointer resting on an item selects it, is a hand and
/// underlines its name; a Ctrl click on a folder selects it and opens
/// nothing; a plain click opens it.
///
/// parity: SEL-018
#[gtk::test]
fn a_single_click_opens_an_item_and_pointing_selects_it_in_every_view() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    single_click(&test, PreferencesUpdate::default());
    for view_name in ["details", "compact", "small", "large", "tiles", "content"] {
        test.activate("view", Some(view_name));
        wait_for_frames(&test.window, 3);
        test.window.folder_model().select_none();
        let view = test.window.folder_pane().view_widget();
        assert!(view.has_css_class("single-click"), "{view_name}: the view knows");

        let notes = item_on_screen(&mut pointer, &test, "Notes 2.txt");
        pointer.move_to(notes.0, notes.1);
        wait_until(&format!("{view_name}: pointing selects"), || {
            test.selected_names() == ["Notes 2.txt"]
        });
        assert_eq!(
            view.cursor().and_then(|cursor| cursor.name()).as_deref(),
            Some("pointer"),
            "{view_name}: a hand over an item"
        );
        assert!(
            cell_of(&test, "Notes 2.txt").has_css_class(POINTED_CLASS),
            "{view_name}: its name is underlined"
        );

        let documents = item_on_screen(&mut pointer, &test, "Documents");
        pointer.move_to(documents.0, documents.1);
        assert!(
            !cell_of(&test, "Notes 2.txt").has_css_class(POINTED_CLASS),
            "{view_name}: only the item pointed at is underlined"
        );
        pointer.hold_key(gdk::Key::Control_L);
        pointer.tap(Button::Primary);
        pointer.let_go_key(gdk::Key::Control_L);
        pause();
        assert_eq!(
            test.window.current_uri(),
            Some(fixture.uri()),
            "{view_name}: a Ctrl click opens nothing"
        );
        assert_eq!(
            test.window.tab_count(),
            1,
            "{view_name}: not even in a tab, as a Ctrl double-click would"
        );
        assert!(
            test.selected_names().contains(&"Documents".to_owned()),
            "{view_name}: it selects"
        );

        pointer.tap(Button::Primary);
        wait_until(&format!("{view_name}: a single click opens"), || {
            test.window.current_uri() == Some(fixture.uri_of("Documents"))
        });
        test.activate("back", None);
        wait_until("the folder again", || {
            test.window.current_uri() == Some(fixture.uri()) && !test.window.is_loading()
        });
        pause();
    }
    // Back to the screen's middle, as the other real-pointer tests end.
    pointer.move_to(720, 500);
}

/// With single-click on, a click on an item's check box selects it and
/// another deselects it, a rubber band released on an item selects it,
/// and neither opens anything; the second click of a quick double-click
/// on a file opens nothing more: the file opens once.
///
/// parity: SEL-018, SEL-014, SEL-012
#[gtk::test]
fn check_boxes_bands_and_double_clicks_open_nothing_more_with_single_click() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    single_click(
        &test,
        PreferencesUpdate {
            selection_marker: Some(true),
            item_checks_on_ctrl: Some(false),
            ..PreferencesUpdate::default()
        },
    );
    wait_until("the check boxes", || {
        test.context.settings_data().preferences.selection_marker
    });
    test.activate("view", Some("details"));
    wait_for_frames(&test.window, 3);
    test.window.folder_model().select_none();

    let check = cell_of(&test, "Documents").item_check();
    let middle = (f64::from(check.width()) / 2.0, f64::from(check.height()) / 2.0);
    let (x, y) = screen_point(&mut pointer, &check, middle, PROBE);
    pointer.move_to(x, y);
    pointer.tap(Button::Primary);
    pause();
    assert_eq!(test.selected_names(), ["Documents"], "the check box selects");
    pointer.tap(Button::Primary);
    pause();
    assert!(test.selected_names().is_empty(), "and deselects");
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "a check box opens nothing"
    );

    let from = blank_on_screen(&mut pointer, &test);
    let onto = item_on_screen(&mut pointer, &test, "Documents");
    pointer.move_to(from.0, from.1);
    pointer.press(Button::Primary);
    pointer.glide(from, onto, 12);
    wait_for(HOLD);
    pointer.release(Button::Primary);
    pause();
    assert!(
        test.selected_names().contains(&"Documents".to_owned()),
        "the band selects"
    );
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "a band released on an item opens nothing"
    );

    let notes = item_on_screen(&mut pointer, &test, "Notes 2.txt");
    pointer.move_to(notes.0, notes.1);
    pause();
    pointer.tap(Button::Primary);
    wait_for(std::time::Duration::from_millis(60));
    pointer.tap(Button::Primary);
    wait_until("the file clicked to open", || {
        !test.context.recorded_launches().is_empty()
    });
    pause();
    assert_eq!(
        test.context.recorded_launches(),
        [fixture.uri_of("Notes 2.txt")],
        "the second click opens nothing more"
    );
    assert_eq!(test.window.current_uri(), Some(fixture.uri()));
    pointer.move_to(720, 500);
}
