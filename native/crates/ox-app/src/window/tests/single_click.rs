// SPDX-License-Identifier: AGPL-3.0-only
//! Folder Options' single-click mode with a real pointer on check.py's
//! private display (SEL-018): GTK's own gestures decide which press opens,
//! selects or draws a band, so only real clicks show that a plain click
//! opens an item in every view, that pointing selects it, and that a Ctrl
//! click, a check box, a rubber band, the second click of a double-click
//! and a release on another item at the place pressed open nothing.

use std::fs;

use gtk::gdk;
use gtk::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::gallery::{gallery_window, write_modified, PicturesHome};
use super::right_drag::{blank_on_screen, item_on_screen, HOLD, PROBE};
use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{descendants, wait_for, wait_for_frames, wait_until, Fixture, TestWindow};
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
        assert_eq!(
            view.cursor().and_then(|cursor| cursor.name()).as_deref(),
            Some("pointer"),
            "{view_name}: still a hand, straight from one item to another"
        );
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

    // A double-click so quick that its second press comes before the
    // window has opened what the first click did: still opened once.
    pointer.double_tap(Button::Primary);
    wait_until("the file double-clicked to open", || {
        test.context.recorded_launches().len() == 2
    });
    pause();
    assert_eq!(
        test.context.recorded_launches(),
        [fixture.uri_of("Notes 2.txt"), fixture.uri_of("Notes 2.txt")],
        "a queued double-click opens the file once"
    );

    // A file listed before it while the button is down puts another item
    // at the place pressed: the release opens neither.
    pointer.press(Button::Primary);
    fs::write(fixture.path("Aaa.txt"), b"Synthetic test data\n").expect("a new file");
    wait_until("the new file listed", || {
        test.names().contains(&"Aaa.txt".to_owned())
    });
    wait_for_frames(&test.window, 3);
    assert!(
        test.position_of("Aaa.txt") < test.position_of("Notes 2.txt"),
        "the new file comes before"
    );
    pointer.release(Button::Primary);
    pause();
    assert_eq!(
        test.context.recorded_launches().len(),
        2,
        "the release opens nothing"
    );
    pointer.move_to(720, 500);
}

/// With single-click on, a key that moves the selection while the
/// pointer rests on an item keeps pointing from selecting that item.
///
/// parity: SEL-018
#[gtk::test]
fn a_key_takes_the_selection_over_from_pointing() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    single_click(&test, PreferencesUpdate::default());
    test.activate("view", Some("details"));
    test.window.present();
    wait_for_frames(&test.window, 3);
    let (x, y) = screen_point(&mut pointer, &test.window, (4.0, 4.0), PROBE);
    pointer.move_to(x, y);
    wait_until("the window to have the keyboard", || test.window.is_active());
    let documents = test.position_of("Documents");
    test.window.folder_pane().select_and_reveal(documents);
    wait_until("the keyboard on Documents", || {
        test.window.folder_pane().focused_position() == Some(documents)
    });
    let next = test.names()[documents as usize + 1].clone();
    assert_ne!(next, "Résumé.txt");

    let resume = item_on_screen(&mut pointer, &test, "Résumé.txt");
    pointer.move_to(resume.0, resume.1);
    pointer.tap_key(gdk::Key::Down);
    pause();
    assert_eq!(
        test.selected_names(),
        [next],
        "the key's selection stays; pointing selects nothing"
    );
    pointer.move_to(720, 500);
}

/// With single-click on, the list scrolled under a resting pointer within
/// the time pointing takes to select leaves the item scrolled away
/// unselected: the item under the pointer once the time is up is selected
/// instead. The item first pointed at used to be selected though another
/// one was under the pointer by then.
///
/// parity: SEL-018
#[gtk::test]
fn scrolling_under_a_resting_pointer_selects_the_item_now_under_it() {
    let fixture = Fixture::empty();
    for number in 0..80 {
        fixture.write(&format!("Notes {number:02}.txt"));
    }
    let mut pointer = Pointer::connect();
    let test = TestWindow::open(&fixture.uri());
    single_click(&test, PreferencesUpdate::default());
    test.activate("view", Some("details"));
    wait_for_frames(&test.window, 3);
    test.window.folder_model().select_none();
    let pane = test.window.folder_pane();
    let view = pane.input_widget();
    // Where the pointer rests, in the view, as `item_on_screen` aims.
    let first_bounds = pane
        .owners()
        .file_cell_at(test.position_of("Notes 03.txt"), &view)
        .and_then(|cell| cell.compute_bounds(&view))
        .expect("the item is laid out");
    let resting = gtk::graphene::Point::new(
        first_bounds.x() + 30.0,
        first_bounds.y() + first_bounds.height() / 2.0,
    );
    let under_the_pointer = || {
        descendants::<FileCell>(&view)
            .into_iter()
            .filter(|cell| cell.is_mapped() && !cell.name().is_empty())
            .find(|cell| {
                cell.compute_bounds(&view)
                    .is_some_and(|bounds| bounds.contains_point(&resting))
            })
            .map(|cell| cell.name())
    };

    let first = item_on_screen(&mut pointer, &test, "Notes 03.txt");
    pointer.move_to(first.0, first.1);
    // Pointing has started on the item: it is underlined, the delay runs.
    wait_until("the item to be pointed at", || {
        cell_of(&test, "Notes 03.txt").has_css_class(POINTED_CLASS)
    });
    let adjustment = pane.details().vadjustment();
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    wait_until("another item under the pointer", || {
        under_the_pointer().is_some_and(|name| name != "Notes 03")
    });
    let now_under = under_the_pointer().expect("an item under the pointer");
    wait_until("the item now under the pointer to be selected", || {
        test.selected_names() == [format!("{now_under}.txt")]
    });
    pause();

    assert_eq!(
        test.selected_names(),
        [format!("{now_under}.txt")],
        "only the item under the pointer is selected, never the one scrolled away"
    );
    pointer.move_to(720, 500);
}

/// With single-click on, in Gallery: the pointer resting on a photo
/// selects it alone; with Ctrl held by the time pointing selects, the
/// photo joins the selection; with Shift, the photos from the one
/// pointed at first are selected.
///
/// parity: SEL-018
#[gtk::test]
fn pointing_with_ctrl_or_shift_extends_the_selection_in_gallery() {
    let home = PicturesHome::new();
    let pictures = home.pictures();
    for (name, date) in [
        ("a.png", "2026-11-02"),
        ("b.png", "2026-10-02"),
        ("c.png", "2026-09-02"),
        ("d.png", "2026-08-02"),
    ] {
        write_modified(&pictures.join(name), date);
    }
    let mut pointer = Pointer::connect();
    let test = gallery_window(&home);
    single_click(&test, PreferencesUpdate::default());
    let gallery = test.window.folder_pane().gallery().clone();
    wait_until("the photos shown", || gallery.shown_tiles().len() == 4);
    let photo = |pointer: &mut Pointer, name: &str| {
        let (_, bounds, _) = gallery
            .shown_tiles()
            .into_iter()
            .find(|(shown, _, _)| shown == name)
            .expect("the photo is shown");
        let middle = bounds.center();
        screen_point(
            pointer,
            gallery.canvas(),
            (f64::from(middle.x()), f64::from(middle.y())),
            PROBE,
        )
    };

    let a = photo(&mut pointer, "a.png");
    pointer.move_to(a.0, a.1);
    wait_until("pointing selects", || test.selected_names() == ["a.png"]);

    let c = photo(&mut pointer, "c.png");
    pointer.move_to(c.0, c.1);
    pointer.hold_key(gdk::Key::Control_L);
    pause();
    pointer.let_go_key(gdk::Key::Control_L);
    assert_eq!(test.selected_names(), ["a.png", "c.png"], "Ctrl adds");

    let d = photo(&mut pointer, "d.png");
    pointer.move_to(d.0, d.1);
    pointer.hold_key(gdk::Key::Shift_L);
    pause();
    pointer.let_go_key(gdk::Key::Shift_L);
    assert_eq!(
        test.selected_names(),
        ["a.png", "b.png", "c.png", "d.png"],
        "Shift extends"
    );
    pointer.move_to(720, 500);
}
