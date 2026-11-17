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

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{is_triggered_by, open_dialog, select_names, shortcuts_of, wait_for_no_dialog};
use super::view_styles::open_folder_options;
use crate::dialog::Dialog;
use crate::test_support::harness::{descendants, settle, wait_for_frames, wait_until, Fixture, TestWindow};

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

    let forward: Vec<String> = tab_through(dialog.upcast_ref(), 4, false)
        .iter()
        .map(|stop| describe(stop.as_ref()))
        .collect();
    assert_eq!(forward, ["No", "Yes", "No", "Yes"], "Tab goes round Yes and No");
    let backward: Vec<String> = tab_through(dialog.upcast_ref(), 4, true)
        .iter()
        .map(|stop| describe(stop.as_ref()))
        .collect();
    assert_eq!(
        backward,
        ["No", "Yes", "No", "Yes"],
        "Shift+Tab goes round them too"
    );

    dialog.close();
    wait_for_no_dialog(&test);
    glib::MainContext::default().iteration(false);
}

/// Presses F4 in `window` where its keyboard is, as GTK runs a window's
/// shortcut controllers: in the capture phase first, then in the bubble
/// phase. Returns whether a shortcut took the key.
fn press_f4(window: &gtk::Window) -> bool {
    [gtk::PropagationPhase::Capture, gtk::PropagationPhase::Bubble]
        .into_iter()
        .flat_map(|phase| shortcuts_of(window.upcast_ref(), phase))
        .filter(|shortcut| {
            shortcut
                .trigger()
                .is_some_and(|trigger| is_triggered_by(&trigger, gdk::Key::F4, gdk::ModifierType::empty()))
        })
        .any(|shortcut| {
            shortcut.action().is_some_and(|action| {
                action.activate(
                    gtk::ShortcutActionFlags::empty(),
                    window,
                    shortcut.arguments().as_ref(),
                )
            })
        })
}

/// Whether the list of `drop_down` shows.
fn shows_its_list(drop_down: &gtk::DropDown) -> bool {
    descendants::<gtk::Popover>(drop_down)
        .iter()
        .any(gtk::Popover::is_visible)
}

/// F4 on a drop-down list shows its items, as Microsoft lists for dialog
/// boxes ("F4: Display the items in the active list",
/// <https://support.microsoft.com/en-us/accessibility/windows/keyboard-shortcuts-in-windows>):
/// in Folder Options, a window of its own, on "Open `OpenXplorer` to:".
///
/// parity: ACC-004
#[gtk::test]
fn f4_shows_the_items_of_a_drop_down_in_folder_options() {
    let fixture = Fixture::empty();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    wait_for_frames(&options, 2);
    let drop_down = descendants::<gtk::DropDown>(&options)
        .into_iter()
        .find(gtk::DropDown::is_mapped)
        .expect("General shows Open OpenXplorer to:");
    drop_down.grab_focus();
    assert!(!shows_its_list(&drop_down));
    assert!(press_f4(options.upcast_ref()), "F4 is taken on the drop-down");
    wait_until("the list to show", || shows_its_list(&drop_down));
    options.close();
}

/// F4 in a dialog of the app's own (Map network drive) shows the drive
/// letters while the keyboard is on them, and is no key of the dialog's
/// elsewhere.
///
/// parity: ACC-004
#[gtk::test]
fn f4_shows_the_items_of_a_drop_down_in_a_dialog() {
    let fixture = Fixture::empty();
    let test = TestWindow::open(&fixture.uri());
    let dialog = crate::dialogs::map_network_dialog(&test.window, &[], |_, _| {});
    dialog.open();
    wait_for_frames(&dialog, 2);
    let drop_down = descendants::<gtk::DropDown>(&dialog)
        .into_iter()
        .find(gtk::DropDown::is_mapped)
        .expect("the drive letters");
    let elsewhere = descendants::<gtk::Button>(&dialog)
        .into_iter()
        .find(|button| button.label().as_deref() == Some("Cancel"))
        .expect("Cancel");
    elsewhere.grab_focus();
    assert!(!press_f4(dialog.upcast_ref()), "F4 off a list is left alone");
    drop_down.grab_focus();
    assert!(press_f4(dialog.upcast_ref()), "F4 is taken on the drop-down");
    wait_until("the list to show", || shows_its_list(&drop_down));
    dialog.close();
}

/// A dialog shown inside the window, as Create archive is, keeps Tab and
/// Shift+Tab within itself, as its own window would (ACC-004): they went
/// on to the tab strip and the title bar's Minimize, Maximize and Close,
/// which the window's own Tab order never visits (ACC-016).
///
/// parity: ACC-004
#[gtk::test]
fn tab_stays_in_a_dialog_shown_inside_the_window() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    test.select_named("Notes 2.txt");
    test.activate("compress-to", None);
    let frame = test.wait_for_dialog("the Create archive dialog");
    wait_for_frames(&test.window, 3);
    for direction in [gtk::DirectionType::TabForward, gtk::DirectionType::TabBackward] {
        let mut outside = Vec::new();
        for _ in 0..24 {
            test.window.emit_move_focus(direction);
            settle();
            let focus = GtkWindowExt::focus(&test.window);
            if !focus.as_ref().is_some_and(|focus| focus.is_ancestor(&frame)) {
                outside.push(describe(focus.as_ref()));
            }
        }
        assert!(
            outside.is_empty(),
            "{direction:?} left the dialog for {outside:?}"
        );
    }
}
