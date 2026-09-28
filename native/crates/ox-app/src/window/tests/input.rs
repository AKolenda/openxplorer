// SPDX-License-Identifier: AGPL-3.0-only
//! Keyboard input in the folder views: type-to-select, Escape, Enter and
//! the keyboard context menu.
//!
//! GTK has no public way to synthesise key events, so these tests emit
//! `key-pressed` on the views' capture-phase key controller, which is what
//! a real key press reaches first. Typed text arrives through the input
//! method, whose `commit` handler is [`BrowserWindow::type_text`].
//!
//! [`BrowserWindow::type_text`]: crate::window::BrowserWindow

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::test_support::harness::{descendants, Fixture, TestWindow, ThemeGuard};

/// Presses `key` in the details view, as far as the window's own key
/// handling goes. Returns true when the window handled the key itself.
fn press(test: &TestWindow, key: gdk::Key) -> bool {
    let view = test.window.content().details.column_view();
    let controller = view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.propagation_phase() == gtk::PropagationPhase::Capture)
        .expect("the details view has a capture-phase key controller");
    let no_keycode = 0_u32;
    controller.emit_by_name::<bool>(
        "key-pressed",
        &[&key.into_glib(), &no_keycode, &gdk::ModifierType::empty()],
    )
}

fn hint(test: &TestWindow) -> String {
    test.window.chrome().status.hint.text().to_string()
}

/// Whether the hint is drawn in the light palette's `hex` colour.
fn hint_is_drawn_in(test: &TestWindow, hex: &str) -> bool {
    let expected = gdk::RGBA::parse(hex).expect("a CSS colour");
    let drawn = test.window.chrome().status.hint.color();
    let channels = [
        (drawn.red(), expected.red()),
        (drawn.green(), expected.green()),
        (drawn.blue(), expected.blue()),
    ];
    channels.iter().all(|(a, b)| (a - b).abs() < 0.01)
}

/// parity: SEL-020
#[gtk::test]
fn typing_selects_the_next_matching_name_and_names_it_in_the_hint() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let _theme = ThemeGuard::keep();
    test.activate("theme", Some("light"));
    test.window.content().focus();
    test.window.type_text("n");
    assert_eq!(test.selected_names(), ["Notes 2.txt"]);
    assert_eq!(hint(&test), "Jump to: n — Notes 2.txt");
    assert!(
        hint_is_drawn_in(&test, "#0067c0"),
        "a match is shown in the accent colour"
    );
    test.window.type_text("otes 1");
    assert_eq!(test.selected_names(), ["Notes 10.txt"]);
}

/// parity: SEL-025
#[gtk::test]
fn an_unmatched_prefix_keeps_the_selection_and_says_so() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let _theme = ThemeGuard::keep();
    test.activate("theme", Some("light"));
    test.window.type_text("d");
    test.window.type_text("zz");
    assert_eq!(test.selected_names(), ["Documents"]);
    assert_eq!(hint(&test), "No name starts with “dzz”");
    assert!(
        hint_is_drawn_in(&test, "#616161"),
        "a miss is muted (the light palette's ox_muted), not an error"
    );
}

#[gtk::test]
fn escape_clears_the_typed_prefix_before_the_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.type_text("n");
    assert!(press(&test, gdk::Key::Escape), "Escape is handled by the window");
    assert_eq!(
        test.selected_names(),
        ["Notes 2.txt"],
        "the first Escape keeps the selection"
    );
    assert_eq!(hint(&test), "");
    press(&test, gdk::Key::Escape);
    assert!(
        test.selected_names().is_empty(),
        "the second Escape clears the selection"
    );
}

#[gtk::test]
fn leaving_the_view_starts_a_new_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.content().focus();
    test.window.type_text("n");
    test.window.chrome().search.entry.grab_focus();
    assert_eq!(hint(&test), "", "the prefix ended with the focus");
    test.window.content().focus();
    test.window.type_text("r");
    assert_eq!(test.selected_names(), ["Résumé.txt"]);
}

#[gtk::test]
fn navigation_keys_start_a_new_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.type_text("n");
    let handled = press(&test, gdk::Key::Down);
    assert!(!handled, "the view still moves the cursor");
    assert_eq!(hint(&test), "", "the prefix ended");
    test.window.type_text("r");
    assert_eq!(
        test.selected_names(),
        ["Résumé.txt"],
        "typing starts over instead of extending \"n\""
    );
}

#[gtk::test]
fn modifier_keys_keep_the_typed_prefix() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.type_text("n");
    press(&test, gdk::Key::Shift_L);
    test.window.type_text("otes 1");
    assert_eq!(test.selected_names(), ["Notes 10.txt"]);
}

#[gtk::test]
fn enter_opens_only_a_single_selected_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.content().details.column_view();
    let model = test.window.folder_model();
    model.select_only(1);
    model.selection().select_item(2, false);
    details.emit_by_name::<()>("activate", &[&2_u32]);
    assert!(
        test.context.recorded_launches().is_empty(),
        "several selected items open nothing"
    );
    model.select_only(1);
    details.emit_by_name::<()>("activate", &[&1_u32]);
    let notes = fixture.uri_of("Notes 2.txt");
    assert_eq!(test.context.recorded_launches(), [notes]);
}

/// The triggers of the shortcuts `view` handles itself.
fn view_shortcuts(view: &impl IsA<gtk::Widget>) -> Vec<String> {
    let controllers = view.observe_controllers();
    let shortcut_controllers = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::ShortcutController>().ok());
    let mut triggers = Vec::new();
    for controller in shortcut_controllers {
        // A shortcut controller lists plain objects, all of them shortcuts.
        let shortcuts = controller
            .iter::<glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|object| object.downcast::<gtk::Shortcut>().ok());
        let trigger_texts =
            shortcuts.filter_map(|shortcut| shortcut.trigger().map(|trigger| trigger.to_str()));
        triggers.extend(trigger_texts.map(|text| text.to_string()));
    }
    triggers
}

#[gtk::test]
fn the_menu_key_opens_the_context_menu_with_or_without_a_selection() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let details = test.window.content().details.column_view();
    assert!(view_shortcuts(details).contains(&"Menu|<Shift>F10".to_owned()));
    let menus = descendants::<gtk::PopoverMenu>(details);
    let [menu] = menus.as_slice() else {
        panic!("one context menu per view");
    };
    for selected in [Some(1), None] {
        match selected {
            Some(position) => test.window.folder_model().select_only(position),
            None => test.window.folder_model().select_none(),
        }
        test.activate("context-menu", None);
        assert!(menu.is_visible(), "the menu opens for the selection {selected:?}");
        menu.popdown();
    }
}
