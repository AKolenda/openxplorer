// SPDX-License-Identifier: AGPL-3.0-only
//! Naming items in a real window, beyond `rename` of `v2.0.0:desktop/ui/app.js`:
//! Tab moving an in-place rename on (OPS-012), the question before a name
//! hides an item (OPS-013) and the warnings under a name field while the
//! user types (OPS-007), as Dolphin has them.

use std::fs;
use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{
    dialog_over, is_renaming_in_place, name_editor, open_dialog, press_shortcut, select_names, text_field,
    wait_for_no_dialog,
};
use crate::test_support::harness::{descendants, wait_for, wait_until, Fixture, TestWindow};
use crate::test_support::pointer::{screen_point, Pointer};
use crate::window::session::PaneSide;
use crate::window::slow_click_rename::NamePress;

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

/// A slow second click on the name of the only selected item renames it
/// in place once a double-click interval has passed; the second press of
/// a double-click cancels it.
///
/// parity: OPS-011
#[gtk::test]
fn a_slow_second_click_on_the_selected_name_renames_it_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    let position = test.window.folder_model().first_selected();
    let press = |clicks| NamePress {
        on_name_of: position,
        clicks,
        modifiers: gdk::ModifierType::empty(),
    };

    test.window.name_pressed(press(1));
    test.window.name_pressed(press(2));
    wait_for(Duration::from_millis(600));
    let after_double_click = edited_name(&test);
    test.window.name_pressed(press(1));
    wait_until("the rename in place", || edited_name(&test).is_some());

    assert_eq!(after_double_click, None, "a double-click does not rename");
    assert_eq!(edited_name(&test).as_deref(), Some("Notes 2.txt"));
}

/// The names the in-place field showed while `test`'s window ran for
/// `duration`.
fn names_edited_during(test: &TestWindow, duration: Duration) -> Vec<String> {
    let mut edited = Vec::new();
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        wait_for(Duration::from_millis(20));
        if let Some(name) = edited_name(test) {
            edited.push(name);
        }
    }
    edited
}

/// A slow second click schedules a rename of the item clicked, never of
/// whatever item comes to stand at its position before the rename
/// starts, as when the clicked item is deleted and the next one is
/// selected in its place.
///
/// parity: OPS-011
#[gtk::test]
fn a_slow_click_rename_never_starts_on_another_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    select_names(&test, &["Notes 2.txt"]);
    let model = test.window.folder_model();
    let position = model.first_selected().expect("Notes 2.txt is selected");

    test.window.name_pressed(NamePress {
        on_name_of: Some(position),
        clicks: 1,
        modifiers: gdk::ModifierType::empty(),
    });
    test.activate("direction", Some("descending"));
    model.select_only(position);
    let now_there = model.name_at(position).expect("an item at the position");

    assert_ne!(now_there, "Notes 2.txt", "another item stands there now");
    assert_eq!(
        names_edited_during(&test, Duration::from_millis(1200)),
        Vec::<String>::new()
    );
}

/// A command run before the rename starts cancels it, as Delete, Ctrl+D
/// or a refresh does in Windows File Explorer: nothing is renamed in
/// place, also when the item stays the only one selected.
///
/// parity: OPS-011
#[gtk::test]
fn a_command_cancels_a_pending_slow_click_rename() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    for command in ["refresh", "delete-permanently"] {
        select_names(&test, &["Notes 2.txt"]);
        test.window.name_pressed(NamePress {
            on_name_of: test.window.folder_model().first_selected(),
            clicks: 1,
            modifiers: gdk::ModifierType::empty(),
        });
        test.activate(command, None);
        let edited = names_edited_during(&test, Duration::from_millis(1200));
        if let Some(question) = dialog_over(&test) {
            question.press("No");
            wait_for_no_dialog(&test);
        }

        assert_eq!(edited, Vec::<String>::new(), "{command} cancels the rename");
    }
    assert!(fixture.path("Notes 2.txt").is_file());
}

/// Every file key pressed before the rename starts cancels it, as in
/// Windows File Explorer, also Ctrl+C, Ctrl+X and Ctrl+V, which run their
/// commands directly rather than as window actions.
///
/// parity: OPS-011
#[gtk::test]
fn a_file_key_cancels_a_pending_slow_click_rename() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("view", Some("details"));
    let control = gdk::ModifierType::CONTROL_MASK;
    for key in [gdk::Key::c, gdk::Key::x, gdk::Key::v] {
        select_names(&test, &["Notes 2.txt"]);
        test.window.name_pressed(NamePress {
            on_name_of: test.window.folder_model().first_selected(),
            clicks: 1,
            modifiers: gdk::ModifierType::empty(),
        });
        press_shortcut(&test, key, control);
        let edited = names_edited_during(&test, Duration::from_millis(1200));
        if let Some(question) = dialog_over(&test) {
            question.press("Cancel");
            wait_for_no_dialog(&test);
        }

        assert_eq!(edited, Vec::<String>::new(), "Ctrl+{key:?} cancels the rename");
    }
}

/// A rename a slow click scheduled is for the pane clicked: making the
/// other pane of a split tab active cancels it, also when that pane
/// shows the same folder with the same item selected.
///
/// parity: OPS-011
#[gtk::test]
fn activating_the_other_pane_cancels_a_pending_slow_click_rename() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("split-view", None);
    test.wait_for_listing("the folder in the new pane");
    select_names(&test, &["Notes 2.txt"]);
    test.window.activate_pane(PaneSide::Start);
    select_names(&test, &["Notes 2.txt"]);

    test.window.name_pressed(NamePress {
        on_name_of: test.window.folder_model().first_selected(),
        clicks: 1,
        modifiers: gdk::ModifierType::empty(),
    });
    test.window.activate_pane(PaneSide::End);
    let selected_beside = test.selected_names();

    assert_eq!(
        selected_beside,
        ["Notes 2.txt"],
        "the same item is selected there"
    );
    assert_eq!(
        names_edited_during(&test, Duration::from_millis(1200)),
        Vec::<String>::new()
    );
    assert!(
        descendants::<gtk::Entry>(&test.window.pane_on(PaneSide::Start).view_widget())
            .iter()
            .all(|field| !field.has_css_class("rename-field"))
    );
}

/// The field's own right-click menu takes the keyboard and gives it back,
/// and GTK then says focus left the field: editing goes on with the typed
/// name, and the name is saved only when focus really moves away.
///
/// parity: OPS-010
#[gtk::test]
fn the_name_fields_own_menu_keeps_the_rename_going() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);
    field.set_text("Plans.txt");
    let text = descendants::<gtk::Text>(&field)
        .into_iter()
        .next()
        .expect("the field edits in a text");
    let has_keyboard = || gtk::prelude::RootExt::focus(&test.window).is_some_and(|focus| focus == text);
    wait_until("the field to take the keyboard", has_keyboard);

    text.activate_action("menu.popup", None)
        .expect("the text has its menu");
    let menu = || {
        descendants::<gtk::PopoverMenu>(&text)
            .into_iter()
            .find(WidgetExt::is_visible)
    };
    wait_until("the field's menu to open", || menu().is_some());
    menu().expect("the menu is open").popdown();
    text.activate_action("selection.select-all", None)
        .expect("the text selects all");
    wait_until("the keyboard to come back", has_keyboard);
    wait_for(Duration::from_millis(300));

    assert!(is_renaming_in_place(&test), "editing goes on");
    assert_eq!(field.text(), "Plans.txt");
    assert!(fixture.path("Notes 2.txt").is_file());
    test.window.folder_pane().focus_view();
    wait_until("the typed name to be saved", || {
        fixture.path("Plans.txt").is_file()
    });
    assert!(!is_renaming_in_place(&test));
}

/// parity: OPS-012
#[gtk::test]
fn tab_commits_the_name_and_goes_on_to_rename_the_next_item() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
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

/// parity: OPS-049
#[gtk::test]
fn changing_the_extension_in_place_asks_first_as_windows_explorer_does() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);

    field.set_text("Notes 2.md");
    field.emit_activate();
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Change the extension?");
    assert_eq!(
        question.message_text(),
        "If you change a file name extension, the file might become unusable. Are you sure you \
         want to change it?"
    );
    assert_eq!(question.button_labels(), ["Cancel", "Change extension"]);
    question.press("Cancel");
    wait_for_no_dialog(&test);
    wait_until("the field to come back", || field.is_sensitive());
    assert!(is_renaming_in_place(&test), "editing goes on");
    assert_eq!(field.text(), "Notes 2.md", "the typed name is kept");
    assert!(fixture.path("Notes 2.txt").is_file());

    field.emit_activate();
    open_dialog(&test).press("Change extension");

    wait_until("the rename", || fixture.path("Notes 2.md").is_file());
    assert!(!fixture.path("Notes 2.txt").exists());
}

/// A name longer than 255 bytes is refused before the extension question,
/// with the message that says why, and the field stays open to fix it.
///
/// parity: OPS-049
#[gtk::test]
fn a_name_too_long_is_refused_in_place_before_the_extension_question() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);
    let too_long = format!("{}.md", "N".repeat(260));

    field.set_text(&too_long);
    field.emit_activate();

    wait_until("the refusal", || {
        test.window.shown_message_text() == "This name is longer than 255 bytes."
    });
    assert!(dialog_over(&test).is_none(), "no extension question");
    wait_until("the field to come back", || field.is_sensitive());
    assert!(is_renaming_in_place(&test), "editing goes on");
    assert_eq!(field.text(), too_long, "the typed name is kept");
    assert!(fixture.path("Notes 2.txt").is_file());
}

/// parity: OPS-049
#[gtk::test]
fn renaming_a_folder_with_a_dot_or_keeping_the_extension_never_asks() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Documents"]);
    test.activate("rename", None);
    let field = name_editor(&test);

    field.set_text("Documents.old");
    field.emit_activate();

    wait_until("the rename", || fixture.path("Documents.old").is_dir());
    assert!(dialog_over(&test).is_none(), "nothing is asked");
}

/// parity: OPS-007
#[gtk::test]
fn the_rename_dialog_warns_about_a_taken_or_hiding_name_while_typing() {
    // An item not on screen is renamed with the dialog (OPS-009).
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["file 0399.txt"]);
    test.activate("rename", None);
    let dialog = open_dialog(&test);
    let field = text_field(&dialog);
    let warning = || {
        descendants::<gtk::Label>(&dialog)
            .into_iter()
            .find(|label| label.has_css_class("dialog-hint") && label.is_visible())
            .map(|label| label.text().to_string())
    };

    field.set_text("file 0001.txt");
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
        Some("An item named “file 0001.txt” already exists here.")
    );
    assert_eq!(
        hidden.as_deref(),
        Some("A name starting with a dot hides the item.")
    );
}

/// What F2 shows and selects of each name, as Windows 11 does: a file's
/// name before its last extension, and the whole name of a shortcut (a
/// symbolic link named as Explorer names one, whose `.lnk` Windows
/// hides), a folder, a name with no extension and a name that starts
/// with its only dot. With extensions hidden, the field shows the name
/// without its extension and all of it is selected.
///
/// parity: OPS-010, VIEW-069
#[gtk::test]
fn rename_selects_the_name_windows_selects_with_extensions_shown_and_hidden() {
    let fixture = Fixture::empty();
    for name in ["Report.txt", "archive.tar.gz", "my.notes.txt", ".hidden"] {
        fixture.write(name);
    }
    std::os::unix::fs::symlink(fixture.path("Report.txt"), fixture.path("Report.txt - Shortcut"))
        .expect("the shortcut is made");
    std::fs::create_dir(fixture.path("v1.2 release")).expect("the folder is made");
    let test = TestWindow::open(&fixture.uri());
    test.activate("hidden", None);
    wait_until("the hidden file to be listed", || {
        test.names().contains(&".hidden".to_owned())
    });
    let shown_and_selected = |name: &str| {
        select_names(&test, &[name]);
        test.activate("rename", None);
        let field = name_editor(&test);
        let selected = field.selection_bounds();
        let shown = field.text().to_string();
        press_in(&field, gdk::Key::Escape, gdk::ModifierType::empty());
        wait_until("the rename to end", || !is_renaming_in_place(&test));
        (shown, selected)
    };
    let cases_hidden = [
        ("Report.txt - Shortcut", "Report.txt - Shortcut", 21),
        ("archive.tar.gz", "archive.tar", 11),
        ("my.notes.txt", "my.notes", 8),
        ("v1.2 release", "v1.2 release", 12),
        (".hidden", ".hidden", 7),
    ];
    for (name, shown, selected) in cases_hidden {
        assert_eq!(
            shown_and_selected(name),
            (shown.to_owned(), Some((0, selected))),
            "{name}, extensions hidden"
        );
    }

    super::support::show_file_name_extensions(&test);
    let cases_shown = [
        ("Report.txt - Shortcut", 21),
        ("archive.tar.gz", 11),
        ("my.notes.txt", 8),
        ("v1.2 release", 12),
        (".hidden", 7),
    ];
    for (name, selected) in cases_shown {
        assert_eq!(
            shown_and_selected(name),
            (name.to_owned(), Some((0, selected))),
            "{name}, extensions shown"
        );
    }

    // A shortcut's name has no extension to change: typing over all of
    // it renames it without asking, as Explorer does.
    select_names(&test, &["Report.txt - Shortcut"]);
    test.activate("rename", None);
    let field = name_editor(&test);
    field.set_text("Q3 figures");
    field.emit_activate();
    wait_until("the shortcut to be renamed", || {
        test.selected_names() == ["Q3 figures"]
    });
    assert!(dialog_over(&test).is_none(), "no question about the extension");
    assert!(fixture.path("Q3 figures").is_symlink());
}

/// The lines of the balloon under `editor`, while it shows.
fn balloon_lines(editor: &gtk::Entry) -> Option<Vec<String>> {
    descendants::<gtk::Popover>(editor)
        .into_iter()
        .find(|balloon| balloon.has_css_class("name-balloon") && balloon.is_visible())
        .map(|balloon| {
            let lines = descendants::<gtk::Label>(&balloon);
            lines.iter().map(|line| line.text().to_string()).collect()
        })
}

/// The balloon's lines, in Windows' words.
fn windows_balloon() -> Vec<String> {
    vec![
        "A file name can't contain any of the following characters:".to_owned(),
        "\\ / : * ? \" < > |".to_owned(),
    ]
}

/// A character Windows forbids in names is refused as it is typed, with
/// Windows' balloon under the field; the name and its selection stay,
/// the rename goes on, and the next key hides the balloon. The key is a
/// real one on the private display, so it goes through the field's input
/// method as a typed key does, and Compose sequences keep working.
///
/// parity: OPS-010
#[gtk::test]
fn a_character_windows_forbids_is_refused_with_its_balloon() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);
    let mut pointer = Pointer::connect();
    let middle = (f64::from(field.width()) / 2.0, f64::from(field.height()) / 2.0);
    let (x, y) = screen_point(&mut pointer, &field, middle, (200, 200));
    pointer.move_to(x, y);
    let name_back = || field.text() == "Notes 2.txt" && field.selection_bounds() == Some((0, 7));

    pointer.tap_key(gdk::Key::slash);
    wait_until("the balloon and the name back", || {
        balloon_lines(&field).is_some() && name_back()
    });
    let shown = balloon_lines(&field);
    let mut position = 5;
    field.insert_text("10:30", &mut position);
    wait_for(Duration::from_millis(300));
    let unchanged = name_back();
    let still_renaming = is_renaming_in_place(&test);
    pointer.tap_key(gdk::Key::End);
    wait_until("the balloon to go", || balloon_lines(&field).is_none());

    assert_eq!(shown, Some(windows_balloon()));
    assert!(unchanged, "nothing was inserted and the stem stays selected");
    assert!(still_renaming, "the balloon leaves the keyboard in the field");
    press_in(&field, gdk::Key::Escape, gdk::ModifierType::empty());
    wait_until("the rename to end", || !is_renaming_in_place(&test));
}

/// Pasting text with a character Windows forbids pastes nothing and
/// shows the balloon, so the selected name is not lost; other text is
/// pasted over the selection as usual.
///
/// parity: OPS-010
#[gtk::test]
fn pasting_a_forbidden_character_leaves_the_name_as_it_was() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);
    let text = field
        .delegate()
        .and_downcast::<gtk::Text>()
        .expect("the field edits through a text");

    field.clipboard().set_text("Plan: 2027");
    text.emit_paste_clipboard();
    wait_until("the balloon and the name back", || {
        balloon_lines(&field).is_some() && field.text() == "Notes 2.txt"
    });
    let after_refusal = (field.text(), field.selection_bounds());
    field.clipboard().set_text("Plan 2027");
    text.emit_paste_clipboard();
    wait_until("the paste", || field.text() == "Plan 2027.txt");

    assert_eq!(after_refusal, ("Notes 2.txt".into(), Some((0, 7))));
    press_in(&field, gdk::Key::Escape, gdk::ModifierType::empty());
    wait_until("the rename to end", || !is_renaming_in_place(&test));
    assert!(fixture.path("Notes 2.txt").is_file(), "Escape renamed nothing");
}

/// A file renamed to the name of another file asks Windows' "Rename
/// File" question: No goes back to editing the typed name, Yes gives the
/// file the first free numbered name, and nothing is overwritten.
///
/// parity: OPS-008, OPS-010
#[gtk::test]
fn a_file_renamed_to_a_taken_name_is_offered_the_next_free_number() {
    let fixture = Fixture::standard();
    fixture.write("Notes 10 (2).txt");
    fs::write(fixture.path("Notes 2.txt"), b"renamed").expect("the renamed file");
    let test = TestWindow::open(&fixture.uri());
    super::support::show_file_name_extensions(&test);
    select_names(&test, &["Notes 2.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);

    field.set_text("Notes 10.txt");
    field.emit_activate();
    let question = open_dialog(&test);
    let (title, message) = (question.title_text(), question.message_text());
    let (texts, buttons) = (question.texts(), question.button_labels());
    question.press("No");
    wait_for_no_dialog(&test);
    wait_until("the field to come back", || field.is_sensitive());
    let typed = field.text();
    field.emit_activate();
    open_dialog(&test).press("Yes");
    wait_until("the numbered rename", || {
        fixture.path("Notes 10 (3).txt").is_file() && !is_renaming_in_place(&test)
    });

    assert_eq!(title, "Rename File");
    assert_eq!(
        message,
        "There is already a file with the same name in this location."
    );
    let asks = "Would you like to rename \"Notes 10.txt\" to \"Notes 10 (3).txt\"?";
    assert!(texts.iter().any(|text| text == asks), "{texts:?}");
    let sizes = texts.iter().filter(|text| text.starts_with("Size: ")).count();
    let dates = texts
        .iter()
        .filter(|text| text.starts_with("Date modified: "))
        .count();
    assert_eq!((sizes, dates), (2, 2), "both files are described: {texts:?}");
    assert_eq!(buttons, ["Yes", "No"]);
    assert_eq!(typed, "Notes 10.txt", "No keeps the typed name for editing");
    assert!(!fixture.path("Notes 2.txt").exists());
    assert_eq!(
        fs::read(fixture.path("Notes 10 (3).txt")).ok(),
        Some(b"renamed".to_vec())
    );
    for kept in ["Notes 10.txt", "Notes 10 (2).txt"] {
        let content = fs::read(fixture.path(kept)).ok();
        assert_eq!(content, Some(b"Synthetic test data\n".to_vec()), "{kept} stays");
    }
}
