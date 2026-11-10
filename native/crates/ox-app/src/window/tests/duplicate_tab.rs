// SPDX-License-Identifier: AGPL-3.0-only
//! Duplicate tab, as Windows 11 File Explorer does it: the copy goes right
//! after the tab it copies and comes to the front, with the same folder,
//! view and Back and Forward history, from the tab menu's "Duplicate tab"
//! (opened with a right-click or from the keyboard) and from a
//! double-click on a tab (TAB-013, TAB-014). Right-clicking a tab behind
//! does not bring it to the front (TAB-012).

use std::fs;

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::shift_f10_keys::press_shift_f10;
use super::support::{click_gesture, middle_of};
use crate::test_support::harness::{wait_for_frames, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::Pointer;
use crate::window::folder_pane::FolderView;
use crate::window::session::TabId;

/// The folders the three tabs went to, left to right.
const FOLDERS: [&str; 3] = ["Alpha", "Beta", "Gamma"];

/// The open tabs, left to right.
fn tab_ids(test: &TestWindow) -> Vec<TabId> {
    let session = test.window.imp().session.borrow();
    session.tabs().iter().map(|tab| tab.id).collect()
}

/// The widget of the tab at `index` in the strip, counted from the left.
fn tab_widget(test: &TestWindow, index: usize) -> gtk::Widget {
    let list = test.window.tab_strip().tab_list();
    std::iter::successors(list.first_child(), WidgetExt::next_sibling)
        .nth(index)
        .unwrap_or_else(|| panic!("tab {index} is shown"))
}

/// A window with three tabs, each of which went from `fixture` to one of
/// [`FOLDERS`], so each has a step to go Back to; the last is in front.
fn three_tabs_with_history(fixture: &Fixture) -> TestWindow {
    for folder in FOLDERS {
        fs::create_dir(fixture.path(folder)).expect("fixture subfolder");
    }
    let test = TestWindow::open(&fixture.uri());
    for (index, folder) in FOLDERS.iter().enumerate() {
        if index > 0 {
            test.window.add_tab(&fixture.uri()).expect("a folder");
            test.wait_for_listing("the new tab");
        }
        test.window.navigate(&fixture.uri_of(folder)).expect("a folder");
        test.wait_for_listing(folder);
    }
    test
}

/// Checks that the tab at `original` was copied right after it and the
/// copy is in front, showing the same folder in the same view with the
/// same history, and that the other tabs kept their places.
fn assert_copied_after(
    test: &TestWindow,
    fixture: &Fixture,
    before: &[TabId],
    original: usize,
    view: FolderView,
    case: &str,
) {
    wait_until(&format!("{case}: the copy"), || test.window.tab_count() == 4);
    test.wait_for_listing("the copy");
    let after = tab_ids(test);
    let copy = after[original + 1];
    assert!(
        !before.contains(&copy),
        "{case}: a new tab right after the original ({after:?})"
    );
    let mut kept = after.clone();
    kept.remove(original + 1);
    assert_eq!(kept, before, "{case}: the other tabs keep their places");
    assert_eq!(test.active_tab(), Some(copy), "{case}: the copy is in front");
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri_of(FOLDERS[original])),
        "{case}: the same folder"
    );
    assert_eq!(test.window.folder_pane().view(), view, "{case}: the same view");
    {
        let session = test.window.imp().session.borrow();
        let history = |id| session.tab(id).map(|tab| tab.history.clone());
        assert_eq!(
            history(copy),
            history(before[original]),
            "{case}: the same history"
        );
    }
    assert!(
        test.window.is_action_enabled("back"),
        "{case}: Back works in the copy"
    );
    test.activate("back", None);
    test.wait_for_listing("Back in the copy");
    assert_eq!(
        test.window.current_uri(),
        Some(fixture.uri()),
        "{case}: Back goes where the original came from"
    );
}

/// The tab menu's "Duplicate tab" on the first, middle and last of three
/// tabs puts the copy right after that tab and brings it to the front,
/// with the same folder, view and history, as in Windows 11 File
/// Explorer. The right-click itself leaves the tab in front as it was.
///
/// parity: TAB-012, TAB-013
#[gtk::test]
fn duplicate_tab_puts_the_copy_right_after_its_tab_in_front() {
    for original in 0..3 {
        let fixture = Fixture::standard();
        let test = three_tabs_with_history(&fixture);
        let before = tab_ids(&test);
        // The original shows its folder as tiles.
        test.window.switch_tab(before[original]);
        test.wait_for_listing("the original");
        test.activate("view", Some("tiles"));
        test.wait_for_listing("tiles");
        // Another tab is in front, so the menu is opened on a tab behind.
        let front = before[(original + 1) % 3];
        test.window.switch_tab(front);
        test.wait_for_listing("the tab in front");
        wait_for_frames(&test.window, 2);
        let case = format!("tab {} of 3", original + 1);

        let tab = tab_widget(&test, original);
        let point = middle_of(&tab, &tab);
        click_gesture(&tab, gdk::BUTTON_SECONDARY)
            .emit_by_name::<()>("pressed", &[&1_i32, &point.0, &point.1]);
        let menu = test.window.tab_strip().menu();
        assert!(menu.is_visible(), "{case}: the tab's menu");
        assert_eq!(
            test.active_tab(),
            Some(front),
            "{case}: right-clicking a tab behind leaves it behind"
        );
        menu.row("Duplicate tab").emit_activate();

        assert_copied_after(&test, &fixture, &before, original, FolderView::Tiles, &case);
    }
}

/// From the keyboard: Shift+F10 on the tab in front opens its menu, and
/// Down to "Duplicate tab" and Enter copy it right after itself, in front,
/// as the tab menu's item does. Pressed with check.py's display's real
/// keyboard.
///
/// parity: TAB-012, TAB-013
#[gtk::test]
fn duplicate_tab_from_the_keyboard_puts_the_copy_right_after_its_tab() {
    let fixture = Fixture::standard();
    let mut keyboard = Pointer::connect();
    let test = three_tabs_with_history(&fixture);
    let before = tab_ids(&test);
    // The middle tab, in front, with the keyboard on it.
    let original = 1;
    test.window.switch_tab(before[original]);
    test.wait_for_listing("the middle tab");
    wait_for_frames(&test.window, 2);
    let tab = tab_widget(&test, original);
    assert!(tab.grab_focus(), "the tab in front takes the keyboard");
    wait_for_frames(&test.window, 2);

    press_shift_f10(&mut keyboard);
    let menu = test.window.tab_strip().menu();
    wait_until("Shift+F10 to open the tab's menu", || menu.is_visible());
    let duplicate = menu.row("Duplicate tab");
    for _ in 0..6 {
        wait_for_frames(&test.window, 2);
        if duplicate.has_focus() {
            break;
        }
        keyboard.tap_key(gdk::Key::Down);
    }
    assert!(duplicate.has_focus(), "Down reaches Duplicate tab");
    keyboard.tap_key(gdk::Key::Return);

    assert_copied_after(
        &test,
        &fixture,
        &before,
        original,
        FolderView::Details,
        "Shift+F10, Duplicate tab",
    );
}

/// A double-click on a tab copies it the same way, right after itself.
///
/// parity: TAB-014
#[gtk::test]
fn double_clicking_a_tab_puts_the_copy_right_after_it() {
    let fixture = Fixture::standard();
    let test = three_tabs_with_history(&fixture);
    let before = tab_ids(&test);
    let original = 0;
    let strip = test.window.tab_strip();
    wait_until("the tabs to be laid out", || tab_widget(&test, 2).width() > 0);
    let point = middle_of(&tab_widget(&test, original), strip);
    let strip_click = click_gesture(strip, gdk::BUTTON_PRIMARY);
    click_gesture(&tab_widget(&test, original), gdk::BUTTON_PRIMARY)
        .emit_by_name::<()>("pressed", &[&1_i32, &point.0, &point.1]);
    strip_click.emit_by_name::<()>("pressed", &[&1_i32, &point.0, &point.1]);
    strip_click.emit_by_name::<()>("pressed", &[&2_i32, &point.0, &point.1]);

    assert_copied_after(
        &test,
        &fixture,
        &before,
        original,
        FolderView::Details,
        "double-click",
    );
}
