// SPDX-License-Identifier: AGPL-3.0-only
//! The file keys while a dialog is in front, as in Windows Explorer: Delete,
//! Shift+Delete, F2, the clipboard keys, Ctrl+A and the other file keys do
//! nothing to the items behind an open dialog, whether it is shown inside
//! the window (Properties, Compress) or in a window of its own (a
//! confirmation), and neither does Alt+Enter. Inside the dialog, a text
//! field keeps its own Delete, and Escape still closes the dialog. Once it
//! closes, the keys work again.

use gtk::gdk;
use gtk::prelude::*;

use super::file_ops_support::{
    dialog_over, is_renaming_in_place, is_triggered_by, open_dialog, press_shortcut,
    press_shortcut_where_focused, select_names, shortcuts_of, wait_for_no_dialog,
};
use super::item_dialogs::press;
use crate::dialog::DialogFrame;
use crate::test_support::harness::{descendants, settle, wait_until, Fixture, TestWindow};
use crate::window::file_ops::file_key_bindings;
use crate::window::WindowAction;

/// Presses `keyval` with exactly `modifiers` where keyboard focus is now,
/// as GTK routes a key through shortcut controllers: in the capture phase
/// from the window down to the focused widget, then in the bubble phase
/// from the focused widget back up. Returns the widget whose shortcut
/// took the key, if any. GTK has no public way to synthesise key events.
fn route_key(test: &TestWindow, keyval: gdk::Key, modifiers: gdk::ModifierType) -> Option<gtk::Widget> {
    let mut path = Vec::new();
    let mut widget = GtkWindowExt::focus(&test.window);
    while let Some(current) = widget {
        widget = current.parent();
        path.push(current);
    }
    if path.last() != Some(test.window.upcast_ref()) {
        path.push(test.window.clone().upcast());
    }
    let runs = |widget: &gtk::Widget, phase| {
        shortcuts_of(widget, phase).into_iter().any(|shortcut| {
            let is_the_key = shortcut
                .trigger()
                .is_some_and(|trigger| is_triggered_by(&trigger, keyval, modifiers));
            let action = shortcut.action().filter(|_| is_the_key);
            action.is_some_and(|action| {
                action.activate(
                    gtk::ShortcutActionFlags::empty(),
                    widget,
                    shortcut.arguments().as_ref(),
                )
            })
        })
    };
    let captured = path
        .iter()
        .rev()
        .find(|widget| runs(widget, gtk::PropagationPhase::Capture));
    captured
        .or_else(|| {
            path.iter()
                .find(|widget| runs(widget, gtk::PropagationPhase::Bubble))
        })
        .cloned()
}

/// Every key of the file commands but Escape, which closes the dialog in
/// front: Delete, Shift+Delete, F2, the clipboard keys, Ctrl+A, Undo,
/// Redo and New folder, each with its modifiers.
fn file_keys() -> Vec<(String, gdk::Key, gdk::ModifierType)> {
    let select_none = WindowAction::SelectNone.detailed_name();
    file_key_bindings()
        .filter(|(action, _)| *action != select_none)
        .flat_map(|(action, keys)| {
            keys.split('|').map(move |accelerator| {
                let (keyval, modifiers) = gtk::accelerator_parse(accelerator).expect("a valid accelerator");
                (format!("{action} {accelerator}"), keyval, modifiers)
            })
        })
        .collect()
}

/// Moves keyboard focus to the button labelled `label` in `frame`.
fn focus_button(test: &TestWindow, frame: &DialogFrame, label: &str) {
    let button = descendants::<gtk::Button>(frame)
        .into_iter()
        .find(|button| button.label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("the dialog has {label}"));
    assert!(button.grab_focus(), "{label} takes focus");
    let focus = GtkWindowExt::focus(&test.window).expect("a focused widget");
    assert!(focus.is_ancestor(frame) || &focus == button.upcast_ref::<gtk::Widget>());
}

/// The text field of `frame` that holds `text`, once it shows, focused,
/// with the cursor at its start. Properties fills its General tab, name
/// field included, only once the item has been read off the main thread.
fn focused_field(frame: &DialogFrame, text: &str) -> gtk::Entry {
    let find = || {
        descendants::<gtk::Entry>(frame)
            .into_iter()
            .find(|field| field.text() == text)
    };
    wait_until("the dialog's text field to be filled", || find().is_some());
    let field = find().expect("wait_until returned only once the field showed");
    assert!(field.grab_focus(), "the field takes focus");
    field.select_region(0, 0);
    field.set_position(0);
    field
}

/// Presses every file key in front of the dialog that has focus and
/// asserts the window ran none of them.
fn assert_window_ignores_file_keys(test: &TestWindow, what: &str) {
    for (name, keyval, modifiers) in file_keys() {
        let taker = route_key(test, keyval, modifiers);
        assert_ne!(
            taker.as_ref(),
            Some(test.window.upcast_ref::<gtk::Widget>()),
            "{name} reaches the files behind {what}"
        );
    }
}

/// With Properties open, Delete, Shift+Delete, Alt+Enter and the other
/// file keys leave the items behind it alone, Escape closes it, and then
/// Delete asks to move the selection to the Trash again.
///
/// parity: ACC-004, CMD-017
#[gtk::test]
fn the_file_keys_leave_the_items_behind_properties_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);
    test.activate("properties", None);
    let frame = test.wait_for_dialog("Properties");
    focus_button(&test, &frame, "Close");

    assert!(
        !test.window.file_keys_apply(),
        "the dialog in front keeps the file keys"
    );
    assert_window_ignores_file_keys(&test, "Properties");
    assert!(dialog_over(&test).is_none(), "nothing asks to delete");
    assert!(fixture.path("Résumé.txt").is_file());
    assert_eq!(
        test.selected_names(),
        ["Résumé.txt"],
        "Ctrl+A selected nothing more"
    );
    assert!(!is_renaming_in_place(&test), "F2 renamed nothing");
    assert_eq!(test.shown_dialog(), Some(frame.clone()), "Properties stays open");
    test.activate("properties", None);
    settle();
    assert_eq!(
        test.shown_dialog(),
        Some(frame.clone()),
        "Alt+Enter opens no other Properties"
    );

    let taker = route_key(&test, gdk::Key::Escape, gdk::ModifierType::empty());
    assert!(taker.is_some_and(|taker| taker.is::<crate::dialog::DialogLayer>()));
    wait_until("Properties to close", || test.shown_dialog().is_none());
    assert_eq!(
        test.selected_names(),
        ["Résumé.txt"],
        "Escape closed only the dialog"
    );

    press_shortcut(&test, gdk::Key::Delete, gdk::ModifierType::empty());
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Move to Trash?");
    question.press("Cancel");
    wait_for_no_dialog(&test);
    assert!(fixture.path("Résumé.txt").is_file());
}

/// In Properties' name field, Delete and Shift+Delete edit the name, as
/// in any text field, and nothing behind the dialog is deleted.
///
/// parity: CMD-017
#[gtk::test]
fn delete_in_a_dialogs_text_field_edits_the_text() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);
    test.activate("properties", None);
    let frame = test.wait_for_dialog("Properties");
    let field = focused_field(&frame, "Résumé.txt");

    let taker = route_key(&test, gdk::Key::Delete, gdk::ModifierType::empty());
    assert!(
        taker.is_some_and(|taker| taker.is::<gtk::Text>()),
        "the field takes Delete"
    );
    assert_eq!(field.text(), "ésumé.txt");
    field.select_region(0, 1);
    let taker = route_key(&test, gdk::Key::Delete, gdk::ModifierType::SHIFT_MASK);
    assert!(
        taker.is_some_and(|taker| taker.is::<gtk::Text>()),
        "the field takes Shift+Delete"
    );
    assert_eq!(field.text(), "sumé.txt", "Shift+Delete cuts the selected text");

    assert!(dialog_over(&test).is_none(), "nothing asks to delete");
    assert!(fixture.path("Résumé.txt").is_file());
    assert_eq!(test.shown_dialog(), Some(frame));
}

/// Another dialog inside the window, Compress, keeps the file keys from
/// the items behind it too, and its name field keeps Delete.
///
/// parity: CMD-017
#[gtk::test]
fn the_file_keys_leave_the_items_behind_another_in_window_dialog_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("compress-to", None);
    let frame = test.wait_for_dialog("the Compress dialog");
    focus_button(&test, &frame, "Cancel");

    assert_window_ignores_file_keys(&test, "Compress");
    let field = focused_field(&frame, "Notes 2");
    let taker = route_key(&test, gdk::Key::Delete, gdk::ModifierType::empty());
    assert!(
        taker.is_some_and(|taker| taker.is::<gtk::Text>()),
        "the field takes Delete"
    );
    assert_eq!(field.text(), "otes 2");
    assert!(dialog_over(&test).is_none(), "nothing asks to delete");
    assert!(fixture.path("Notes 2.txt").is_file());
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    test.activate("properties", None);
    settle();
    assert_eq!(
        test.shown_dialog(),
        Some(frame.clone()),
        "Alt+Enter leaves Compress in front"
    );

    press(&frame, "Cancel");
    wait_until("Compress to close", || test.shown_dialog().is_none());
    assert!(
        test.window.file_keys_apply(),
        "the keys work again once it closed"
    );
}

/// While a dialog window of its own is open over the window, such as the
/// Delete confirmation, the window's file keys do nothing: a second
/// Delete asks no second question.
///
/// parity: CMD-017
#[gtk::test]
fn the_file_keys_wait_for_a_dialog_window_in_front() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    press_shortcut(&test, gdk::Key::Delete, gdk::ModifierType::empty());
    let question = open_dialog(&test);

    assert!(!test.window.file_keys_apply());
    for (keyval, modifiers) in [
        (gdk::Key::Delete, gdk::ModifierType::empty()),
        (gdk::Key::Delete, gdk::ModifierType::SHIFT_MASK),
    ] {
        let taken = press_shortcut_where_focused(&test, keyval, modifiers);
        assert!(!taken, "{keyval:?} with {modifiers:?} waits for the question");
    }
    let questions = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|window| window.downcast::<crate::dialog::Dialog>().ok())
        .filter(WidgetExt::is_visible)
        .count();
    assert_eq!(questions, 1, "one question");

    question.press("Cancel");
    wait_for_no_dialog(&test);
    test.window.folder_pane().focus_view();
    assert!(
        test.window.file_keys_apply(),
        "the keys work again once it closed"
    );
    assert!(fixture.path("Notes 2.txt").is_file());
}

/// Properties withdrawn with its tab leaves the other tab's file keys
/// working; back on its tab, they wait for it again (PROP-008).
///
/// parity: PROP-008
#[gtk::test]
fn properties_in_another_tab_leaves_this_tabs_file_keys_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);
    test.activate("properties", None);
    test.wait_for_dialog("Properties");

    test.activate("new-tab", None);
    wait_until("Properties to step aside", || test.shown_dialog().is_none());
    test.window.folder_pane().focus_view();
    assert!(test.window.file_keys_apply(), "the other tab has no dialog");

    test.activate("previous-tab", None);
    let frame = test.wait_for_dialog("Properties to come back");
    focus_button(&test, &frame, "Close");
    assert!(!test.window.file_keys_apply());
}
