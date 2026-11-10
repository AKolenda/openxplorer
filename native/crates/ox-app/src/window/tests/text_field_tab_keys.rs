// SPDX-License-Identifier: AGPL-3.0-only
//! Ctrl+T and Ctrl+W from the address bar, while it is edited, and from
//! the search box, as Windows 11 File Explorer takes them there: Ctrl+T
//! opens a new tab and Ctrl+W closes the tab, or the window with its last
//! tab. Ctrl+N does nothing in those fields, as in Windows, and the
//! field's text is left as it was. The keys are pressed with check.py's
//! private display's real keyboard (CMD-017, TAB-001, TAB-002).

use gtk::gdk;
use gtk::prelude::*;

use crate::test_support::harness::{
    wait_for, wait_for_frames, wait_until, windows_besides, Fixture, TestWindow,
};
use crate::test_support::pointer::{screen_point, Pointer};

/// How long a key gets to take effect before its absence is checked.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(400);

/// The text field a key is pressed in.
#[derive(Debug, Clone, Copy)]
enum Field {
    /// The address bar in edit mode (Ctrl+L), showing the folder's path.
    Address,
    /// The search box, empty.
    EmptySearch,
    /// The search box with text typed into it.
    SearchWithText,
}

impl Field {
    /// Puts the keyboard in the field, with its text, and returns the
    /// field's text.
    fn focus(self, test: &TestWindow) -> String {
        match self {
            Field::Address => {
                test.activate("location", None);
                let entry = test.window.address_bar().entry();
                wait_until("the address bar to be edited", || {
                    entry.has_focus() || has_focus_within(&entry)
                });
                entry.text().to_string()
            }
            Field::EmptySearch | Field::SearchWithText => {
                test.window.search_box().focus();
                let entry = test.window.search_box().entry();
                wait_until("the search box to take the keyboard", || has_focus_within(&entry));
                if matches!(self, Field::SearchWithText) {
                    entry.set_text("Plans");
                    entry.set_position(-1);
                }
                entry.text().to_string()
            }
        }
    }

    /// The field's text now.
    fn text(self, test: &TestWindow) -> String {
        match self {
            Field::Address => test.window.address_bar().entry().text().to_string(),
            Field::EmptySearch | Field::SearchWithText => test.window.search_box().entry().text().to_string(),
        }
    }

    /// Whether the keyboard is in the field.
    fn has_keyboard(self, test: &TestWindow) -> bool {
        match self {
            Field::Address => has_focus_within(&test.window.address_bar().entry()),
            Field::EmptySearch | Field::SearchWithText => has_focus_within(&test.window.search_box().entry()),
        }
    }
}

/// Whether the window's focus is `widget` or inside it.
fn has_focus_within(widget: &impl IsA<gtk::Widget>) -> bool {
    let Some(window) = widget.root().and_downcast::<gtk::Window>() else {
        return false;
    };
    gtk::prelude::GtkWindowExt::focus(&window)
        .is_some_and(|focus| &focus == widget.upcast_ref::<gtk::Widget>() || focus.is_ancestor(widget))
}

/// Points into `test`'s window, where the private display sends the
/// keyboard.
fn point_into(pointer: &mut Pointer, test: &TestWindow) {
    let inside = screen_point(pointer, &test.window, (300.0, 300.0), (720, 500));
    pointer.move_to(inside.0, inside.1);
}

/// Presses Ctrl+`key` on the real keyboard.
fn press_ctrl(pointer: &mut Pointer, key: gdk::Key) {
    pointer.hold_key(gdk::Key::Control_L);
    pointer.tap_key(key);
    pointer.let_go_key(gdk::Key::Control_L);
}

/// A window on the fixture with `tabs` tabs, the first in front.
fn window_with_tabs(fixture: &Fixture, tabs: usize) -> TestWindow {
    let test = TestWindow::open(&fixture.uri());
    for _ in 1..tabs {
        test.window
            .add_tab(&fixture.uri_of("Documents"))
            .expect("a folder");
        test.wait_for_listing("the added tab");
    }
    wait_for_frames(&test.window, 3);
    test
}

/// Ctrl+T in `field` opens a new tab, of Home as the New tab button
/// does, with 1 and with 3 tabs open.
fn ctrl_t_opens_a_tab_from(field: Field) {
    for tabs in [1, 3] {
        let fixture = Fixture::standard();
        let mut pointer = Pointer::connect();
        let test = window_with_tabs(&fixture, tabs);
        point_into(&mut pointer, &test);
        field.focus(&test);

        press_ctrl(&mut pointer, gdk::Key::t);
        wait_until(
            &format!("Ctrl+T in {field:?} with {tabs} tabs to open a tab"),
            || test.window.tab_count() == tabs + 1,
        );
        test.wait_for_listing("the new tab");
        // Home as the New tab button opens it: the Home page, or the home
        // folder with "Open the home folder on Home" (HOME-012).
        assert_eq!(
            test.window.current_uri(),
            Some(test.window.home_location()),
            "{field:?}: the new tab shows Home"
        );
        assert!(
            windows_besides(&[&test.window]).is_empty(),
            "{field:?}: no other window"
        );
    }
}

/// Ctrl+W in `field` closes the tab in front, with 3 tabs open.
fn ctrl_w_closes_the_tab_from(field: Field) {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = window_with_tabs(&fixture, 3);
    point_into(&mut pointer, &test);
    field.focus(&test);

    press_ctrl(&mut pointer, gdk::Key::w);
    wait_until(&format!("Ctrl+W in {field:?} to close the tab"), || {
        test.window.tab_count() == 2
    });
    assert!(
        test.window.is_visible(),
        "{field:?}: the window stays with its other tabs"
    );
}

/// Ctrl+W in `field` with the last tab closes the window.
fn ctrl_w_closes_the_window_from(field: Field) {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = window_with_tabs(&fixture, 1);
    point_into(&mut pointer, &test);
    field.focus(&test);

    press_ctrl(&mut pointer, gdk::Key::w);
    wait_until(
        &format!("Ctrl+W in {field:?} on the last tab to close the window"),
        || !test.window.is_visible(),
    );
    pointer.move_to(720, 500);
}

/// Ctrl+N in `field` does nothing, with 1 and with 3 tabs, and leaves the
/// field's text and keyboard as they were.
fn ctrl_n_does_nothing_in(field: Field) {
    for tabs in [1, 3] {
        let fixture = Fixture::standard();
        let mut pointer = Pointer::connect();
        let test = window_with_tabs(&fixture, tabs);
        point_into(&mut pointer, &test);
        let text = field.focus(&test);

        press_ctrl(&mut pointer, gdk::Key::n);
        wait_for(SETTLE);

        assert!(
            windows_besides(&[&test.window]).is_empty(),
            "{field:?}: Ctrl+N opened no window"
        );
        assert_eq!(
            test.window.tab_count(),
            tabs,
            "{field:?}: no tab opened or closed"
        );
        assert_eq!(field.text(&test), text, "{field:?}: the text is as it was");
        assert!(
            field.has_keyboard(&test),
            "{field:?}: the field keeps the keyboard"
        );
    }
}

/// parity: CMD-017, TAB-001
#[gtk::test]
fn ctrl_t_opens_a_tab_from_the_address_bar() {
    ctrl_t_opens_a_tab_from(Field::Address);
}

/// parity: CMD-017, TAB-001
#[gtk::test]
fn ctrl_t_opens_a_tab_from_the_empty_search_box() {
    ctrl_t_opens_a_tab_from(Field::EmptySearch);
}

/// parity: CMD-017, TAB-001
#[gtk::test]
fn ctrl_t_opens_a_tab_from_the_search_box_with_text() {
    ctrl_t_opens_a_tab_from(Field::SearchWithText);
}

/// parity: CMD-017, TAB-002
#[gtk::test]
fn ctrl_w_closes_the_tab_from_the_address_bar() {
    ctrl_w_closes_the_tab_from(Field::Address);
}

/// parity: CMD-017, TAB-002
#[gtk::test]
fn ctrl_w_closes_the_tab_from_the_search_box() {
    ctrl_w_closes_the_tab_from(Field::EmptySearch);
    ctrl_w_closes_the_tab_from(Field::SearchWithText);
}

/// parity: CMD-017, TAB-002
#[gtk::test]
fn ctrl_w_on_the_last_tab_closes_the_window_from_the_address_bar() {
    ctrl_w_closes_the_window_from(Field::Address);
}

/// parity: CMD-017, TAB-002
#[gtk::test]
fn ctrl_w_on_the_last_tab_closes_the_window_from_the_search_box() {
    ctrl_w_closes_the_window_from(Field::SearchWithText);
}

/// parity: CMD-017
#[gtk::test]
fn ctrl_n_does_nothing_in_the_address_bar_or_the_search_box() {
    ctrl_n_does_nothing_in(Field::Address);
    ctrl_n_does_nothing_in(Field::EmptySearch);
    ctrl_n_does_nothing_in(Field::SearchWithText);
}

/// Ctrl+A, Ctrl+C, Ctrl+V and Ctrl+Z still act on the search box's text,
/// not on the files: select the text, copy it, paste it twice over the
/// selection, and undo.
///
/// parity: CMD-017
#[gtk::test]
fn the_text_keys_still_act_on_the_fields_text() {
    let fixture = Fixture::standard();
    let mut pointer = Pointer::connect();
    let test = window_with_tabs(&fixture, 1);
    point_into(&mut pointer, &test);
    let field = Field::SearchWithText;
    field.focus(&test);
    let entry = test.window.search_box().entry();

    press_ctrl(&mut pointer, gdk::Key::a);
    wait_until("Ctrl+A to select the text", || {
        entry.selection_bounds() == Some((0, 5))
    });
    assert!(
        test.window.folder_model().selected_positions().is_empty(),
        "no file selected"
    );
    press_ctrl(&mut pointer, gdk::Key::c);
    entry.set_position(-1);
    press_ctrl(&mut pointer, gdk::Key::v);
    wait_until("Ctrl+V to paste the copied text", || {
        field.text(&test) == "PlansPlans"
    });
    press_ctrl(&mut pointer, gdk::Key::z);
    wait_until("Ctrl+Z to undo the paste", || field.text(&test) == "Plans");
    assert!(fixture.path("Notes 2.txt").is_file(), "the files are untouched");
}
