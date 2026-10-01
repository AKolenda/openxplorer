// SPDX-License-Identifier: AGPL-3.0-only
//! Naming items in a real window, beyond `rename` of `v2.0.0:desktop/ui/app.js`:
//! Tab moving an in-place rename on (OPS-012), the question before a name
//! hides an item (OPS-013) and the warnings under a name field while the
//! user types (OPS-007), as Dolphin has them.

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{
    is_renaming_in_place, name_editor, open_dialog, select_names, text_field, wait_for_no_dialog,
};
use crate::test_support::harness::{descendants, wait_until, Fixture, TestWindow};

/// The name the in-place field of `test`'s view shows, if one is open.
fn edited_name(test: &TestWindow) -> Option<String> {
    descendants::<gtk::Entry>(&test.window.folder_pane().view_widget())
        .into_iter()
        .find(|field| field.has_css_class("rename-field"))
        .map(|field| field.text().to_string())
}

/// Presses `key` with `modifiers` in `editor` as far as its own key
/// handler goes; GTK has no public way to synthesise key events.
fn press_in(editor: &gtk::Entry, key: gdk::Key, modifiers: gdk::ModifierType) {
    let keys = editor
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .last()
        .expect("the field has its own keys");
    let handled: bool = keys.emit_by_name("key-pressed", &[&key.into_glib(), &0_u32, &modifiers]);
    assert!(handled, "the field handled {key:?}");
}

/// parity: OPS-012
#[gtk::test]
fn tab_commits_the_name_and_goes_on_to_rename_the_next_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let none = gdk::ModifierType::empty();
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);

    let first = name_editor(&test);
    first.set_text("Notes 3.txt");
    press_in(&first, gdk::Key::Tab, none);
    wait_until("the next item to be renamed in place", || {
        edited_name(&test).as_deref() == Some("Notes 10.txt")
    });
    assert!(fixture.path("Notes 3.txt").is_file(), "the typed name was saved");
    assert!(!fixture.path("Notes 2.txt").exists());

    // An unchanged name moves on too, back with Shift+Tab.
    press_in(
        &name_editor(&test),
        gdk::Key::ISO_Left_Tab,
        gdk::ModifierType::SHIFT_MASK,
    );
    wait_until("the previous item to be renamed in place", || {
        edited_name(&test).as_deref() == Some("Notes 3.txt")
    });
    press_in(&name_editor(&test), gdk::Key::Down, none);
    wait_until("Down to move on in the details view", || {
        edited_name(&test).as_deref() == Some("Notes 10.txt")
    });
    press_in(&name_editor(&test), gdk::Key::Escape, none);
    wait_until("the rename to end", || !is_renaming_in_place(&test));
    assert!(fixture.path("Notes 10.txt").is_file());
}

/// parity: OPS-013
#[gtk::test]
fn cancelling_the_hide_question_keeps_the_name_and_the_field() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);

    field.set_text(".Notes 2.txt");
    field.emit_activate();
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Rename and hide?");
    assert_eq!(
        question.message_text(),
        "Adding a dot to the beginning of this file's name will hide it from view."
    );
    assert_eq!(question.button_labels(), ["Cancel", "Rename and Hide"]);
    question.press("Cancel");
    wait_for_no_dialog(&test);

    wait_until("the field to come back", || field.is_sensitive());
    assert!(is_renaming_in_place(&test), "editing goes on");
    assert_eq!(field.text(), ".Notes 2.txt", "the typed name is kept");
    assert!(fixture.path("Notes 2.txt").is_file());
    assert!(!fixture.path(".Notes 2.txt").exists());
}

/// parity: OPS-007
#[gtk::test]
fn the_new_folder_dialog_warns_about_a_taken_or_hiding_name_while_typing() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("new-folder", None);
    let dialog = open_dialog(&test);
    let field = text_field(&dialog);
    let warning = || {
        descendants::<gtk::Label>(&dialog)
            .into_iter()
            .find(|label| label.has_css_class("dialog-hint") && label.is_visible())
            .map(|label| label.text().to_string())
    };

    field.set_text("Documents");
    wait_until("the taken-name warning", || warning().is_some());
    let taken = warning();
    field.set_text(".cache");
    wait_until("the hidden-name warning", || {
        warning().is_some_and(|text| text.contains("dot"))
    });
    let hidden = warning();
    field.set_text("Plans");
    wait_until("no warning for a plain free name", || warning().is_none());
    dialog.press("Cancel");
    wait_for_no_dialog(&test);

    assert_eq!(
        taken.as_deref(),
        Some("An item named “Documents” already exists here.")
    );
    assert_eq!(
        hidden.as_deref(),
        Some("A name starting with a dot hides the item.")
    );
}
