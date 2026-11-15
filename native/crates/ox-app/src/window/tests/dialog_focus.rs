// SPDX-License-Identifier: AGPL-3.0-only
//! Tab and Shift+Tab in the app's dialogs: every stop is a control the
//! keyboard can see and use, and the keys go round them, as Microsoft
//! describes dialog boxes: "Tab: Move forward through options", "Shift+Tab:
//! Move back through options"
//! (<https://support.microsoft.com/en-us/accessibility/windows/keyboard-shortcuts-in-windows>,
//! Dialog box keyboard shortcuts). GTK 4.14 let Tab past a dialog's last
//! button into its message, a label that claimed the key without taking
//! the keyboard, and from there no Tab or Shift+Tab brought it back; a
//! body that needs no scrolling was a stop that showed nothing.

use gtk::glib;
use gtk::prelude::*;

use super::file_ops_support::{open_dialog, select_names, wait_for_no_dialog};
use crate::dialog::Dialog;
use crate::test_support::harness::{settle, Fixture, TestWindow};

/// Presses Tab (Shift+Tab `backward`) in `dialog` `count` times, as GTK
/// runs the key once no widget took it: the window's `move-focus`.
/// Returns the widget focused after each press, `None` where none was.
fn tab_through(dialog: &gtk::Window, count: usize, backward: bool) -> Vec<Option<gtk::Widget>> {
    let direction = if backward {
        gtk::DirectionType::TabBackward
    } else {
        gtk::DirectionType::TabForward
    };
    (0..count)
        .map(|_| {
            dialog.emit_move_focus(direction);
            settle();
            GtkWindowExt::focus(dialog)
        })
        .collect()
}

/// What a stop is, for a failure message: a button's label, else its type.
fn describe(stop: Option<&gtk::Widget>) -> String {
    match stop {
        None => "nothing".to_owned(),
        Some(widget) => widget
            .downcast_ref::<gtk::Button>()
            .and_then(gtk::Button::label)
            .map_or_else(|| widget.type_().name().to_owned(), |label| label.to_string()),
    }
}

/// Shift+Delete's question about one file: a body of plain lines under
/// the message, then Yes and No.
fn permanent_delete_question(test: &TestWindow) -> Dialog {
    select_names(test, &["notes.txt"]);
    test.activate("delete-permanently", None);
    open_dialog(test)
}

/// parity: ACC-004
#[gtk::test]
fn tab_goes_round_a_questions_buttons_and_never_into_its_text() {
    let fixture = Fixture::empty();
    fixture.write("notes.txt");
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    let dialog = permanent_delete_question(&test);
    let first = GtkWindowExt::focus(&dialog);
    assert_eq!(describe(first.as_ref()), "Yes", "the question opens on Yes");

    let forward: Vec<String> = tab_through(dialog.upcast_ref(), 4, false).iter().map(|stop| describe(stop.as_ref())).collect();
    assert_eq!(forward, ["No", "Yes", "No", "Yes"], "Tab goes round Yes and No");
    let backward: Vec<String> = tab_through(dialog.upcast_ref(), 4, true).iter().map(|stop| describe(stop.as_ref())).collect();
    assert_eq!(backward, ["No", "Yes", "No", "Yes"], "Shift+Tab goes round them too");

    dialog.close();
    wait_for_no_dialog(&test);
    glib::MainContext::default().iteration(false);
}
