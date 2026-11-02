// SPDX-License-Identifier: AGPL-3.0-only
//! New ▸ Text document in a real window, as Windows Explorer makes one:
//! the file is created at once as "New Text Document.txt", or
//! "New Text Document (2).txt" and so on when that is taken, then
//! selected and renamed in place with only its name before ".txt"
//! selected. The other New items keep their dialogs.

use std::fs;
use std::time::Duration;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};

use super::file_ops_support::{
    dialog_over, is_enabled, is_renaming_in_place, name_editor, open_dialog, require_private_trash,
    select_names, text_field, wait_for_no_dialog,
};
use crate::test_support::harness::{settle, wait_for, wait_until, Fixture, TestWindow};

/// Presses `key` in the in-place name field `editor`, as far as its own
/// key handler goes.
fn press_in(editor: &gtk::Entry, key: gdk::Key) {
    let keys = editor
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .last()
        .expect("the field has its own keys");
    let _: bool = keys.emit_by_name(
        "key-pressed",
        &[&key.into_glib(), &0_u32, &gdk::ModifierType::empty()],
    );
}

/// Runs New ▸ Text document and returns the field that renames the new
/// file.
fn make_new_text_document(test: &TestWindow) -> gtk::Entry {
    test.activate("new-text-document", None);
    let field = name_editor(test);
    settle();
    assert!(dialog_over(test).is_none(), "no dialog asks for a name");
    field
}

/// Escape in `field`, then waits for the rename to end.
fn escape(test: &TestWindow, field: &gtk::Entry) {
    press_in(field, gdk::Key::Escape);
    wait_until("the rename to end", || !is_renaming_in_place(test));
}

/// parity: OPS-002, OPS-010, CMD-004
#[gtk::test]
fn new_text_document_makes_the_file_at_once_and_renames_its_name_in_place() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());

    let field = make_new_text_document(&test);

    let created = fixture.path("New Text Document.txt");
    assert!(created.is_file(), "made before any name is typed");
    assert_eq!(
        fs::read(&created).expect("the new file"),
        ox_core::ops::BuiltinTemplate::Text.contents(),
        "the Text document starter's content"
    );
    assert_eq!(test.selected_names(), ["New Text Document.txt"]);
    assert_eq!(field.text(), "New Text Document.txt");
    assert_eq!(
        field.selection_bounds(),
        Some((0, 17)),
        "only the name before .txt is selected"
    );
    field.set_text("Shopping list.txt");
    field.emit_activate();
    wait_until("the rename", || fixture.path("Shopping list.txt").is_file());
    assert!(!created.exists());
    wait_until("the renamed file to be selected", || {
        test.selected_names() == ["Shopping list.txt"]
    });
    assert!(!is_renaming_in_place(&test));
    assert!(dialog_over(&test).is_none());
}

/// Taken names are never overwritten: the next ones are
/// "New Text Document (2).txt" and "New Text Document (3).txt", as in
/// Windows.
///
/// parity: OPS-002, OPS-008
#[gtk::test]
fn taken_names_give_the_next_numbered_text_documents() {
    let fixture = Fixture::standard();
    fs::write(fixture.path("New Text Document.txt"), b"kept").expect("a taken name");
    let test = TestWindow::open(&fixture.uri());

    let field = make_new_text_document(&test);
    assert_eq!(field.text(), "New Text Document (2).txt");
    assert_eq!(field.selection_bounds(), Some((0, 21)), "the name before .txt");
    escape(&test, &field);
    let field = make_new_text_document(&test);
    assert_eq!(field.text(), "New Text Document (3).txt");
    escape(&test, &field);

    assert!(fixture.path("New Text Document (2).txt").is_file());
    assert!(fixture.path("New Text Document (3).txt").is_file());
    assert_eq!(
        fs::read(fixture.path("New Text Document.txt")).expect("the taken one"),
        b"kept",
        "the taken one is left alone"
    );
}

/// Escape ends the rename and keeps the default name.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn escape_keeps_the_name_new_text_document() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    field.set_text("Typed but cancelled.txt");

    escape(&test, &field);

    assert!(fixture.path("New Text Document.txt").is_file());
    assert!(!fixture.path("Typed but cancelled.txt").exists());
    assert_eq!(test.selected_names(), ["New Text Document.txt"]);
}

/// Clicking elsewhere, which takes focus from the field, keeps the typed
/// name, as the existing rename in place does.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn leaving_the_field_keeps_the_typed_name() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    field.set_text("Ideas.txt");

    test.window.folder_pane().focus_view();

    wait_until("the rename", || fixture.path("Ideas.txt").is_file());
    assert!(!fixture.path("New Text Document.txt").exists());
}

/// A listing of the folder that arrives once the rename has started,
/// such as the one the folder's own change notices ask for, leaves the
/// field open with the name as typed.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn a_listing_that_arrives_during_the_rename_keeps_the_field() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    field.set_text("Draft.txt");

    test.activate("refresh", None);
    test.wait_for_listing("the folder listed again");
    wait_for(Duration::from_millis(300));

    assert!(is_renaming_in_place(&test), "the rename goes on");
    let field = name_editor(&test);
    assert_eq!(field.text(), "Draft.txt");
    field.emit_activate();
    wait_until("the rename", || fixture.path("Draft.txt").is_file());
    assert!(!fixture.path("New Text Document.txt").exists());
}

/// Undo takes the new file away again, to the Recycle Bin, as it does a
/// new folder.
///
/// parity: OPS-029
#[gtk::test]
fn undo_takes_the_new_text_document_away() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let field = make_new_text_document(&test);
    escape(&test, &field);

    test.activate("undo", None);

    wait_until("the file to go", || {
        !fixture.path("New Text Document.txt").exists()
    });
}

/// Every other New item still asks first, in the dialog it opened before,
/// with its own suggested name.
///
/// parity: OPS-002
#[gtk::test]
fn the_other_new_items_still_open_their_dialogs() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let before = test.names();

    for (action, target, title, name) in [
        ("new-file", None, "New file", "New file"),
        (
            "new-markdown-document",
            None,
            "New from template",
            "New document.md",
        ),
        ("new-csv-file", None, "New from template", "New spreadsheet.csv"),
        ("new-json-file", None, "New from template", "New file.json"),
        ("new-html-document", None, "New from template", "New page.html"),
        ("new-from-template", None, "New from template", "New document.txt"),
        (
            "new-from-user-template",
            Some("text"),
            "New from template",
            "New document.txt",
        ),
    ] {
        test.activate(action, target);
        let dialog = open_dialog(&test);
        assert_eq!(dialog.title_text(), title, "{action}");
        let field = text_field(&dialog);
        assert_eq!(field.text(), name, "{action}");
        assert!(!is_renaming_in_place(&test), "{action}");
        dialog.press("Cancel");
        wait_for_no_dialog(&test);
    }

    assert_eq!(test.names(), before, "nothing was created");
}

/// The names in `fixture`'s folder on disk, sorted, hidden ones included,
/// so a stage file left behind would show.
fn names_on_disk(fixture: &Fixture) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixture.root())
        .expect("the fixture folder")
        .map(|entry| {
            entry
                .expect("a folder entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Runs New ▸ Text document held before it creates the file, as on a
/// slow share or phone, and waits until it reaches the hold. Dropping or
/// sending on the returned sender lets it go on.
fn start_held_text_document(test: &TestWindow) -> async_channel::Sender<()> {
    let release = test.window.hold_text_documents();
    test.activate("new-text-document", None);
    wait_until("the create to reach the hold", || {
        test.window.reached_text_document_gate()
    });
    release
}

/// While the file is being created, as on a slow share, it is the
/// window's running write: the other file commands wait, closing asks
/// first and cancels it, and the cancelled create leaves nothing behind.
/// No busy panel shows for it.
///
/// parity: OPS-002, TAB-049
#[gtk::test]
fn closing_during_a_slow_create_asks_and_cancels_it() {
    let fixture = Fixture::standard();
    let before = names_on_disk(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);

    let running = test.window.has_running_write();
    let panel_shown = test.window.transfer_panel().is_visible();
    let others_enabled =
        ["new-folder", "new-text-document", "new-file", "undo"].map(|name| is_enabled(&test, name));
    let can_cancel = is_enabled(&test, "cancel-operation");
    test.window.close();
    let question = open_dialog(&test);
    let title = question.title_text();
    question.press("Cancel and close");
    wait_for(Duration::from_millis(200));
    let open_while_running = test.window.is_visible();
    drop(release);
    wait_until("the window to close", || !test.window.is_visible());

    assert!(running, "the create counts as a running write");
    assert!(!panel_shown, "no busy panel");
    assert_eq!(others_enabled, [false; 4], "the other file commands wait");
    assert!(can_cancel);
    assert_eq!(title, "A file operation is running");
    assert!(open_while_running, "the window waits for the create to stop");
    assert_eq!(names_on_disk(&fixture), before, "nothing is left behind");
}

/// Cancel stops a slow create as it stops any file operation: no file, no
/// message, no rename, and the commands come back.
///
/// parity: OPS-002, OPS-022
#[gtk::test]
fn cancelling_a_slow_create_leaves_no_file_and_says_nothing() {
    let fixture = Fixture::standard();
    let before = names_on_disk(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    let running = test.window.has_running_write();

    test.activate("cancel-operation", None);
    drop(release);
    wait_until("the create to stop", || !test.window.has_running_write());
    wait_for(Duration::from_millis(300));

    assert!(running, "the create counts as a running write");
    assert_eq!(names_on_disk(&fixture), before, "nothing is left behind");
    assert_eq!(test.window.shown_message(), "", "a cancel says nothing");
    assert!(!is_renaming_in_place(&test));
    assert!(is_enabled(&test, "new-text-document"));
}

/// A create that ends after the user went to another tab leaves that tab
/// alone, as Windows does: no listing, no selection change, no rename.
/// The file is still made in the folder it was asked for.
///
/// parity: OPS-002
#[gtk::test]
fn a_create_that_ends_in_another_tab_leaves_that_tab_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    test.window.add_tab(&fixture.uri()).expect("a valid folder");
    test.wait_for_listing("the second tab");
    select_names(&test, &["Notes 2.txt"]);
    let shown = test.names();

    drop(release);
    wait_until("the create to end", || {
        fixture.path("New Text Document.txt").is_file() && !test.window.has_running_write()
    });
    wait_for(Duration::from_millis(300));

    assert!(fixture.path("New Text Document.txt").is_file());
    assert_eq!(test.selected_names(), ["Notes 2.txt"], "the selection stays");
    assert!(!is_renaming_in_place(&test), "nothing is renamed there");
    assert_eq!(test.names(), shown, "the tab is not listed again by the create");
}

/// A create that ends after the tab went to another folder leaves that
/// folder alone; the file is still made where it was asked for.
///
/// parity: OPS-002
#[gtk::test]
fn a_create_that_ends_after_navigating_away_leaves_the_new_folder_alone() {
    let fixture = Fixture::standard();
    fixture.write("Documents/Letter.txt");
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    test.window
        .navigate(&fixture.uri_of("Documents"))
        .expect("a valid folder");
    wait_until("the other folder", || test.names() == ["Letter.txt"]);
    select_names(&test, &["Letter.txt"]);

    drop(release);
    wait_until("the create to end", || {
        fixture.path("New Text Document.txt").is_file() && !test.window.has_running_write()
    });
    wait_for(Duration::from_millis(300));

    assert!(fixture.path("New Text Document.txt").is_file());
    assert!(!fixture.path("Documents/New Text Document.txt").exists());
    assert_eq!(test.selected_names(), ["Letter.txt"], "the selection stays");
    assert!(!is_renaming_in_place(&test));
}

/// Back in the tab it was asked in, still on the same folder, the create
/// finishes there: the file is selected and renamed in place.
///
/// parity: OPS-002, OPS-010
#[gtk::test]
fn a_create_that_ends_back_in_its_own_tab_renames_there() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let first = test.active_tab().expect("a tab");
    let release = start_held_text_document(&test);
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a valid folder");
    test.wait_for_listing("the second tab");
    test.activate_tab(first);
    test.wait_for_listing("the first tab");

    drop(release);
    let field = name_editor(&test);

    assert_eq!(field.text(), "New Text Document.txt");
    assert_eq!(test.selected_names(), ["New Text Document.txt"]);
}

/// A create still running after a moment, as on a slow share or phone,
/// shows the transfer panel, whose Cancel stops it: no file is left and
/// nothing is said. Nothing shows at first, so a quick create looks
/// instant.
///
/// parity: OPS-002, OPS-022
#[gtk::test]
fn a_slow_create_shows_the_cancellable_panel_after_a_moment() {
    let fixture = Fixture::standard();
    let before = names_on_disk(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.transfer_panel();
    let release = start_held_text_document(&test);
    let shown_at_once = panel.is_visible();

    wait_until("the panel to show", || panel.is_visible());
    panel.press_cancel();
    drop(release);
    wait_until("the create to stop", || !test.window.has_running_write());
    wait_for(Duration::from_millis(300));

    assert!(!shown_at_once, "nothing shows at first");
    assert!(!panel.is_visible(), "the panel goes once the create stopped");
    assert_eq!(names_on_disk(&fixture), before, "nothing is left behind");
    assert_eq!(test.window.shown_message(), "", "a cancel says nothing");
    assert!(!is_renaming_in_place(&test));
}

/// A create that ends before the moment passes never shows the panel,
/// even once that moment has gone by.
///
/// parity: OPS-002
#[gtk::test]
fn a_quick_create_never_shows_the_panel() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);

    drop(release);
    let field = name_editor(&test);
    wait_for(Duration::from_millis(1500));

    assert!(!test.window.transfer_panel().is_visible());
    assert_eq!(field.text(), "New Text Document.txt");
}

/// A file created while its tab was in the background shows when the
/// user goes back to that tab, even where the folder sends no change
/// notices, as on some shares. The tab in front is not listed again.
///
/// parity: OPS-002
#[gtk::test]
fn going_back_to_the_tab_shows_a_file_created_while_away() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let first = test.active_tab().expect("a tab");
    let release = start_held_text_document(&test);
    // As on a share that sends no change notices.
    if let Some(tab) = test.window.imp().session.borrow_mut().tab_mut(first) {
        tab.watch = None;
    }
    test.window
        .add_tab(&fixture.uri_of("Documents"))
        .expect("a valid folder");
    test.wait_for_listing("the second tab");

    drop(release);
    wait_until("the create to end", || {
        fixture.path("New Text Document.txt").is_file() && !test.window.has_running_write()
    });
    wait_for(Duration::from_millis(300));
    let in_front = test.names();
    test.activate_tab(first);
    test.wait_for_listing("the first tab");
    wait_for(Duration::from_millis(500));

    assert!(in_front.is_empty(), "the tab in front still shows its own folder");
    assert!(
        test.names().contains(&"New Text Document.txt".to_owned()),
        "{:?}",
        test.names()
    );
}

/// Something the user does in the window, as its own input controllers
/// see it.
#[derive(Clone, Copy)]
enum Input {
    /// A key press.
    Key,
    /// A mouse button press.
    Press,
    /// A turn of the mouse wheel.
    Scroll,
}

/// Plays `input` to the window's input watch, as GTK delivers it: the
/// isolated session cannot press keys or move a mouse.
fn touch(test: &TestWindow, input: Input) {
    let controllers = test.window.observe_controllers();
    let watching = controllers
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventController>().ok())
        .filter(|controller| controller.name().as_deref() == Some("input-generation"));
    for controller in watching {
        match input {
            Input::Key => {
                if let Some(keys) = controller.downcast_ref::<gtk::EventControllerKey>() {
                    let _: bool = keys.emit_by_name(
                        "key-pressed",
                        &[&gdk::Key::a.into_glib(), &0_u32, &gdk::ModifierType::empty()],
                    );
                }
            }
            Input::Press => {
                if let Some(press) = controller.downcast_ref::<gtk::GestureClick>() {
                    press.emit_by_name::<()>("pressed", &[&1_i32, &10.0_f64, &10.0_f64]);
                }
            }
            Input::Scroll => {
                if let Some(wheel) = controller.downcast_ref::<gtk::EventControllerScroll>() {
                    let _: bool = wheel.emit_by_name("scroll", &[&0.0_f64, &3.0_f64]);
                }
            }
        }
    }
}

/// Releases a held create and waits until the file is made and the create
/// has ended.
fn finish_held_text_document(test: &TestWindow, fixture: &Fixture, release: async_channel::Sender<()>) {
    drop(release);
    wait_until("the create to end", || {
        fixture.path("New Text Document.txt").is_file() && !test.window.has_running_write()
    });
    wait_for(Duration::from_millis(500));
}

/// A search started and a result selected while the create ran is left
/// as it is: no rename steals the keyboard and the selection stays.
///
/// parity: OPS-002
#[gtk::test]
fn a_search_started_during_the_create_is_left_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    touch(&test, Input::Key);
    test.search_for("Notes");
    touch(&test, Input::Press);
    select_names(&test, &["Notes 2.txt"]);

    finish_held_text_document(&test, &fixture, release);

    assert_eq!(test.window.search_box().entry().text().as_str(), "Notes");
    assert_eq!(test.selected_names(), ["Notes 2.txt"], "the selection stays");
    assert!(!is_renaming_in_place(&test), "nothing is renamed");
}

/// An item the user selected in the same folder while the create ran
/// stays selected, nothing is renamed, and the folder shows the new file.
///
/// parity: OPS-002
#[gtk::test]
fn a_selection_made_during_the_create_is_left_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    touch(&test, Input::Press);
    select_names(&test, &["Résumé.txt"]);

    finish_held_text_document(&test, &fixture, release);

    assert_eq!(test.selected_names(), ["Résumé.txt"], "the selection stays");
    assert!(!is_renaming_in_place(&test), "nothing is renamed");
    wait_until("the new file to be listed", || {
        test.names().contains(&"New Text Document.txt".to_owned())
    });
}

/// Typing in the address bar, which the user started while the create
/// ran, keeps the keyboard: nothing is renamed and the typed address stays.
///
/// parity: OPS-002
#[gtk::test]
fn typing_an_address_during_the_create_keeps_the_keyboard() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    let address = test.window.address_bar();
    touch(&test, Input::Key);
    address.edit("/example/typed");
    let entry = address.entry();
    let focused = || entry.has_focus() || entry.focus_child().is_some();
    assert!(focused(), "the address is being typed");

    finish_held_text_document(&test, &fixture, release);

    assert!(!is_renaming_in_place(&test), "nothing is renamed");
    assert!(focused(), "the keyboard stays in the address bar");
    assert_eq!(entry.text().as_str(), "/example/typed");
}

/// Stands in for an extraction or compression, which runs on the
/// transfer panel without being a file operation (`archive_actions.rs`).
fn start_archive_on_panel(test: &TestWindow) -> ox_core::transfer::Cancellation {
    let cancel = ox_core::transfer::Cancellation::new();
    test.window.transfer_panel().start(
        crate::window::transfer_panel::TransferKind::Archive,
        "Preparing extraction…",
        cancel.clone(),
    );
    cancel
}

/// While an extraction or compression runs, New ▸ Text document does
/// nothing, as the other file operations do then: no file, no rename, and
/// the archive keeps its panel and stays cancellable.
///
/// parity: OPS-002, OPS-024
#[gtk::test]
fn new_text_document_waits_while_an_archive_runs() {
    let fixture = Fixture::standard();
    let before = names_on_disk(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let archive = start_archive_on_panel(&test);

    test.activate("new-text-document", None);
    wait_for(Duration::from_millis(1500));
    let panel = test.window.transfer_panel();
    let status = panel.status_text();
    let still_writing = test.window.is_writing_files();
    panel.press_cancel();
    let cancelled = archive.is_cancelled();
    test.window.finish_archive_operation();

    assert_eq!(names_on_disk(&fixture), before, "nothing is created");
    assert!(!is_renaming_in_place(&test));
    assert_eq!(status, "Preparing extraction…", "the archive keeps its panel");
    assert!(still_writing, "the window still writes");
    assert!(cancelled, "the panel's Cancel still stops the archive");
}

/// When the panel is taken by another write while a slow create runs, the
/// create neither replaces that panel when its moment passes nor hides it
/// when it ends.
///
/// parity: OPS-002, OPS-024
#[gtk::test]
fn a_slow_create_never_takes_or_ends_another_writes_panel() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.transfer_panel();
    let release = start_held_text_document(&test);
    let archive = start_archive_on_panel(&test);

    wait_for(Duration::from_millis(1500));
    let status_after_the_moment = panel.status_text();
    drop(release);
    // The other write still runs, so only the create's own state ends.
    wait_until("the create to end", || {
        fixture.path("New Text Document.txt").is_file()
            && test.window.imp().file_operations.borrow().is_idle()
    });
    wait_for(Duration::from_millis(300));
    let busy_after_the_create = panel.is_busy() && panel.is_visible();
    panel.press_cancel();
    let cancelled = archive.is_cancelled();
    test.window.finish_archive_operation();

    assert_eq!(status_after_the_moment, "Preparing extraction…");
    assert!(busy_after_the_create, "the other write keeps its panel");
    assert!(cancelled, "the panel's Cancel still stops the other write");
}

/// The create's own panel, once shown, is not hidden by another write's
/// end, and the create still hides it when it ends.
///
/// parity: OPS-002
#[gtk::test]
fn a_slow_creates_panel_is_its_own_until_it_ends() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.transfer_panel();
    let release = start_held_text_document(&test);
    wait_until("the panel to show", || panel.is_visible());
    let status = panel.status_text();

    finish_held_text_document(&test, &fixture, release);

    assert_eq!(status, "Creating a text document…");
    assert!(
        !panel.is_visible() && !panel.is_busy(),
        "the create hid its panel"
    );
}

/// Properties opened over the items while the create ran stays in front:
/// nothing is renamed underneath it and the selection is left as it was.
///
/// parity: OPS-002
#[gtk::test]
fn properties_opened_during_the_create_is_left_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);
    let release = start_held_text_document(&test);
    test.activate("properties", None);
    test.wait_for_dialog("Properties");

    finish_held_text_document(&test, &fixture, release);

    assert!(test.window.dialog_layer().shown().is_some(), "Properties stays");
    assert!(!is_renaming_in_place(&test), "nothing is renamed underneath");
    assert_eq!(test.selected_names(), ["Résumé.txt"]);
}

/// Scrolling through a long folder while the create ran is not undone:
/// the view stays where the user scrolled and nothing is renamed.
///
/// parity: OPS-002
#[gtk::test]
fn scrolling_during_the_create_is_left_alone() {
    let fixture = Fixture::with_files(400);
    let test = TestWindow::open(&fixture.uri());
    let pane = test.window.folder_pane();
    let release = start_held_text_document(&test);
    touch(&test, Input::Scroll);
    pane.restore_scroll_position(2000.0);
    wait_until("the view to scroll", || pane.scroll_position() > 1000.0);
    let scrolled = pane.scroll_position();

    finish_held_text_document(&test, &fixture, release);

    assert!(!is_renaming_in_place(&test), "nothing is renamed");
    assert!(
        (pane.scroll_position() - scrolled).abs() < 1.0,
        "the view stays where it was scrolled"
    );
}

/// A slow create whose panel was taken by another write when its moment
/// came shows its own panel once that write ends, so it can still be
/// cancelled without closing the window.
///
/// parity: OPS-002, OPS-022
#[gtk::test]
fn a_slow_create_shows_its_panel_once_another_write_lets_it_go() {
    let fixture = Fixture::standard();
    let before = names_on_disk(&fixture);
    let test = TestWindow::open(&fixture.uri());
    let panel = test.window.transfer_panel();
    let release = start_held_text_document(&test);
    let _archive = start_archive_on_panel(&test);
    wait_for(Duration::from_millis(1500));

    test.window.finish_archive_operation();
    wait_until("the create's panel", || {
        panel.is_visible() && panel.status_text() == "Creating a text document…"
    });
    panel.press_cancel();
    drop(release);
    wait_until("the create to stop", || !test.window.has_running_write());

    assert_eq!(names_on_disk(&fixture), before, "nothing is left behind");
    assert!(!panel.is_visible());
}

/// Another window that took the focus while the create ran, such as a
/// Properties window or another application's, keeps it: nothing is
/// renamed in the browser window behind it, and the folder still shows
/// the new file.
///
/// parity: OPS-002
#[gtk::test]
fn another_window_taking_focus_during_the_create_keeps_it() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let release = start_held_text_document(&test);
    // Not added to the application: it stands in for another window.
    let other = gtk::Window::new();
    other.present();
    wait_until("the other window has focus", || {
        other.is_active() && !test.window.is_active()
    });

    finish_held_text_document(&test, &fixture, release);

    let focus_kept = other.is_active();
    let renaming = is_renaming_in_place(&test);
    let selected = test.selected_names();
    wait_until("the new file to be listed", || {
        test.names().contains(&"New Text Document.txt".to_owned())
    });
    other.destroy();

    assert!(focus_kept, "the other window keeps the focus");
    assert!(!renaming, "nothing is renamed behind it");
    assert!(selected.is_empty(), "nothing is selected: {selected:?}");
}

/// The archive commands of the tab the user went to while a slow create
/// ran, which wait while it writes, come back once it ends, without the
/// selection changing again.
///
/// parity: OPS-002, ARC-009
#[gtk::test]
fn the_archive_commands_come_back_once_a_background_create_ends() {
    let fixture = super::archives::fixture_with_zip();
    let test = TestWindow::open(&fixture.uri_of("Documents"));
    let release = start_held_text_document(&test);
    test.window.add_tab(&fixture.uri()).expect("a valid folder");
    test.wait_for_listing("the second tab");
    select_names(&test, &["Bundle.zip"]);
    let waited = !is_enabled(&test, "extract-all");

    drop(release);
    wait_until("the create to end", || {
        fixture.path("Documents/New Text Document.txt").is_file() && !test.window.has_running_write()
    });
    settle();

    assert!(waited, "Extract all waits while the create writes");
    assert!(is_enabled(&test, "extract-all"), "Extract all is back");
    assert!(is_enabled(&test, "extract-here"), "Extract here is back");
    assert_eq!(test.selected_names(), ["Bundle.zip"]);
}

/// Extract and Compress, enabled for a selected ZIP, wait while a slow
/// create writes, as during any other write, without the selection
/// changing, and come back once it ends.
///
/// parity: OPS-002, ARC-009
#[gtk::test]
fn the_archive_commands_wait_while_a_slow_create_writes() {
    let fixture = super::archives::fixture_with_zip();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Bundle.zip"]);
    let commands = ["extract-all", "extract-here", "compress-to-zip", "compress-to"];
    let before = commands.map(|name| is_enabled(&test, name));

    let release = start_held_text_document(&test);
    let during = commands.map(|name| is_enabled(&test, name));
    drop(release);
    let field = name_editor(&test);
    escape(&test, &field);
    select_names(&test, &["Bundle.zip"]);
    let after = commands.map(|name| is_enabled(&test, name));

    assert_eq!(before, [true; 4], "enabled for the ZIP");
    assert_eq!(during, [false; 4], "they wait while the create writes");
    assert_eq!(after, [true; 4], "and come back");
}
