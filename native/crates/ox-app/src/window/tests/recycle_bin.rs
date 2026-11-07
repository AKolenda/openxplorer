// SPDX-License-Identifier: AGPL-3.0-only
//! The Recycle Bin in a real window: listing `trash:///`, its menus,
//! Restore and Empty Recycle Bin. New in the native app (OPS-040 to
//! OPS-042), so no Python test is ported here. Every test checks first
//! that the Recycle Bin is the test run's private one.

use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::TRASH_URI;
use ox_core::ops::{list_recycle_bin, JournalDirection};
use ox_core::settings::{PreferencesUpdate, Settings};
use ox_core::transfer::Cancellation;

use super::command_bar::command_bar_names;
use super::file_ops_support::{
    dialog_over, failure_question, is_enabled, open_dialog, require_private_trash, select_names,
    wait_for_no_dialog,
};
use super::name_conflicts::{button, question_window, texts};
use crate::folder_view::details::shown_cell_text;
use crate::folder_view::sorting::{SortColumn, SortDirection, SortOrder};
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::file_drop::DropAction;
use crate::window::folder_pane::PanePage;

/// Moves `path` to the Trash, as another file manager would.
fn trash(path: &Path) {
    gio::File::for_path(path)
        .trash(None::<&gio::Cancellable>)
        .expect("the private Trash takes the file");
}

/// Panics unless everything in the Recycle Bin came from this test run's
/// temporary folders, so emptying it can delete nothing of the user's
/// (the Trash of another mounted volume is listed too). Returns how many
/// items it holds.
fn require_only_test_items_in_the_recycle_bin() -> usize {
    let items = glib::MainContext::default()
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    let count = items.len();
    let temp = std::env::temp_dir();
    for item in items {
        let from_test = item
            .original_path
            .as_ref()
            .is_some_and(|path| path.starts_with(&temp));
        assert!(
            from_test,
            "the Recycle Bin holds an item from outside the test: {item:?}"
        );
    }
    count
}

/// parity: OPS-040, OPS-041
#[gtk::test]
fn the_recycle_bin_lists_trashed_items_and_restore_puts_them_back() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Budget for restoring.txt");
    let original = fixture.path("Budget for restoring.txt");
    trash(&original);
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        test.names().contains(&"Budget for restoring.txt".to_owned())
    });
    select_names(&test, &["Budget for restoring.txt"]);

    test.window
        .right_click(test.window.folder_model().first_selected());
    let labels = test.window.context_menu().row_labels();
    test.window.context_menu().popdown();
    test.activate("restore", None);

    assert_eq!(labels, ["Restore", "Delete permanently", "-", "Properties"]);
    wait_until("the file to be back", || original.is_file());
    super::file_ops_support::assert_ended_without_a_message(&test);
}

/// The command bar's Restore button.
fn restore_button(test: &TestWindow) -> gtk::Button {
    descendants::<gtk::Button>(test.window.command_bar())
        .into_iter()
        .find(|button| button.has_css_class("restore-command"))
        .expect("the bar has Restore")
}

/// In the Recycle Bin, as in Windows 11, the command bar shows Empty
/// Recycle Bin and Restore all items after View; Restore all items reads
/// Restore the selected items while items are selected, and puts back
/// every item.
///
/// parity: OPS-041, OPS-042
#[gtk::test]
fn the_command_bar_restores_all_items_or_the_selected_ones_in_the_recycle_bin() {
    require_private_trash();
    let fixture = Fixture::standard();
    let names = ["Restore me first.txt", "Restore me too.txt"];
    for name in names {
        fixture.write(name);
        trash(&fixture.path(name));
    }
    let test = TestWindow::open(&fixture.uri());
    test.wait_for_listing("the folder");
    let elsewhere = command_bar_names(&test);
    test.activate("go-to", Some(TRASH_URI));
    wait_until("the trashed files to be listed", || {
        let shown = test.names();
        names.iter().all(|name| shown.contains(&(*name).to_owned()))
    });
    let after_view = |test: &TestWindow| {
        let shown = command_bar_names(test);
        let view = shown
            .iter()
            .position(|name| name == "View")
            .expect("the bar has View");
        shown[view + 1..].to_vec()
    };

    assert!(
        !elsewhere.contains(&"Empty Recycle Bin".to_owned()),
        "{elsewhere:?}"
    );
    assert!(
        !elsewhere.contains(&"Restore all items".to_owned()),
        "{elsewhere:?}"
    );
    assert_eq!(
        after_view(&test)[..4],
        ["|", "Empty Recycle Bin", "Restore all items", "More options"]
    );
    select_names(&test, &["Restore me first.txt"]);
    wait_until("the selected items' Restore", || {
        after_view(&test).contains(&"Restore the selected items".to_owned())
    });
    assert_eq!(
        restore_button(&test).action_name().as_deref(),
        Some("win.restore")
    );
    select_names(&test, &[]);
    wait_until("Restore all items again", || {
        after_view(&test).contains(&"Restore all items".to_owned())
    });
    let restore = restore_button(&test);
    assert_eq!(restore.action_name().as_deref(), Some("win.restore-all-items"));
    assert!(restore.is_sensitive());
    // A search that matches nothing still leaves the items to restore.
    test.window.search_box().entry().set_text("zzz-no-such-item");
    wait_until("the search to match nothing", || {
        !test.window.is_loading() && test.window.folder_pane().page() == Some(PanePage::Empty)
    });
    assert!(restore.is_sensitive(), "the Recycle Bin still holds items");
    test.window.search_box().entry().set_text("");
    wait_until("the items again", || test.names().len() >= names.len());
    restore.emit_clicked();

    wait_until("both files to be back", || {
        names.iter().all(|name| fixture.path(name).is_file())
    });
}

/// The Recycle Bin row counts a file trashed while the window is open:
/// its watch, now made off the GTK thread so a stalled trash backend
/// cannot hold a new window (and an open file dialog with it), still
/// follows the Recycle Bin.
///
/// parity: INT-032
#[gtk::test]
fn the_recycle_bin_row_follows_files_trashed_while_a_window_is_open() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    // The first count, made as the window opens.
    wait_until("the first count", || {
        test.window.recycle_bin_row_count().is_some()
    });
    let before = test.window.recycle_bin_row_count().expect("counted");
    fixture.write("Trashed while open.txt");
    trash(&fixture.path("Trashed while open.txt"));
    wait_until("the row to count it", || {
        test.window.recycle_bin_row_count() == Some(before + 1)
    });
}

/// The details view of the Recycle Bin shows where each item was
/// deleted from and when, in place of Date modified, and sorts by them.
///
/// parity: VIEW-062, OPS-040
#[gtk::test]
fn the_recycle_bin_shows_and_sorts_by_original_location_and_date_deleted() {
    require_private_trash();
    let fixture = Fixture::empty();
    for (folder, name) in [("Zeta", "Agenda.txt"), ("Alpha", "Budget.txt")] {
        std::fs::create_dir(fixture.path(folder)).expect("fixture subfolder");
        fixture.write(&format!("{folder}/{name}"));
        trash(&fixture.path(&format!("{folder}/{name}")));
    }
    let test = TestWindow::open(TRASH_URI);
    let ours = || {
        let names = test.names().into_iter();
        names
            .filter(|name| name == "Agenda.txt" || name == "Budget.txt")
            .collect::<Vec<_>>()
    };
    wait_until("both trashed files to be listed", || ours().len() == 2);
    let details = test.window.folder_pane().details();
    let shown = |column| details.column(column).is_some_and(|column| column.is_visible());

    details.sort_by(SortOrder {
        column: SortColumn::OriginalLocation,
        direction: SortDirection::Ascending,
    });

    assert!(shown(SortColumn::OriginalLocation) && shown(SortColumn::Deleted));
    assert!(!shown(SortColumn::Modified), "Date deleted takes its place");
    assert_eq!(ours(), ["Budget.txt", "Agenda.txt"], "Alpha before Zeta");
    let model = test.window.folder_model();
    let budget = (0..model.n_items())
        .filter_map(|position| model.item(position))
        .find(|item| item.entry().name == "Budget.txt")
        .expect("Budget.txt is listed");
    let alpha = fixture.path("Alpha").display().to_string();
    assert_eq!(
        shown_cell_text(
            SortColumn::OriginalLocation,
            &budget,
            ox_core::format::DateStyle::Absolute,
            true
        ),
        alpha
    );
    let unknown = ox_core::format::date_short_time_text(None);
    assert_ne!(
        shown_cell_text(
            SortColumn::Deleted,
            &budget,
            ox_core::format::DateStyle::Absolute,
            true
        ),
        unknown,
        "the deletion date is known"
    );
}

/// parity: OPS-043
#[gtk::test]
fn delete_in_the_recycle_bin_deletes_only_the_chosen_items_for_good() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Delete me for good.txt");
    fixture.write("Keep me trashed.txt");
    trash(&fixture.path("Delete me for good.txt"));
    trash(&fixture.path("Keep me trashed.txt"));
    let test = TestWindow::open(TRASH_URI);
    let is_listed = |name: &str| test.names().contains(&name.to_owned());
    wait_until("the trashed files to be listed", || {
        is_listed("Delete me for good.txt") && is_listed("Keep me trashed.txt")
    });
    select_names(&test, &["Delete me for good.txt"]);

    test.activate("trash", None);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Delete File");
    assert_eq!(
        dialog.message_text(),
        "Are you sure you want to permanently delete this file?"
    );
    dialog.press("Yes");
    wait_until("the item to leave the Recycle Bin", || {
        !is_listed("Delete me for good.txt")
    });
    assert!(is_listed("Keep me trashed.txt"));
    assert!(
        !fixture.path("Delete me for good.txt").exists(),
        "nothing was restored"
    );
}

/// Deleting a Recycle Bin item that another program removed while the
/// question showed asks "Item Not Found" in the progress window, as
/// Windows does, and Skip ends the delete without a word.
///
/// parity: OPS-043, OPS-047, OPS-023
#[gtk::test]
fn deleting_an_item_gone_from_the_recycle_bin_asks_item_not_found() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Gone meanwhile.txt");
    trash(&fixture.path("Gone meanwhile.txt"));
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        test.names().contains(&"Gone meanwhile.txt".to_owned())
    });
    select_names(&test, &["Gone meanwhile.txt"]);
    let item = test.window.folder_model().selected_uris();

    test.activate("trash", None);
    let dialog = open_dialog(&test);
    // Another program empties it out of the Recycle Bin meanwhile.
    gio::File::for_uri(&item[0])
        .delete(None::<&gio::Cancellable>)
        .expect("the item leaves the Recycle Bin");
    dialog.press("Yes");
    let failed = failure_question(&test);

    assert_eq!(failed.caption_shown(), "Item Not Found");
    let texts = failed.failure_texts();
    assert!(
        texts[0].starts_with("Could not find this item\n\nThis is no longer located in Recycle Bin."),
        "{texts:?}"
    );
    assert!(texts.contains(&"Gone meanwhile.txt".to_owned()), "{texts:?}");
    assert_eq!(failed.failure_button_labels(), ["Try Again", "Skip", "Cancel"]);
    failed.press_failure("Skip");
    wait_until("the delete to end", || !test.window.is_writing_files());
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    assert!(dialog_over(&test).is_none(), "nothing is said afterwards");
}

/// Empty Recycle Bin asks Windows' question, "Delete Multiple Items",
/// with Yes and No, and the empty Recycle Bin then says "This folder is
/// empty." as any empty folder does.
///
/// parity: OPS-040, OPS-042
#[gtk::test]
fn empty_recycle_bin_asks_then_deletes_everything_in_it() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Old draft.txt");
    fixture.write("Old notes.txt");
    trash(&fixture.path("Old draft.txt"));
    trash(&fixture.path("Old notes.txt"));
    let count = require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed files to be listed", || test.names().len() == count);

    test.window.right_click(None);
    let labels = test.window.context_menu().row_labels();
    test.window.context_menu().popdown();
    test.activate("empty-recycle-bin", None);
    let dialog = open_dialog(&test);

    assert_eq!(labels, ["Empty Recycle Bin", "Refresh"]);
    assert_eq!(dialog.title_text(), "Delete Multiple Items");
    assert_eq!(
        dialog.message_text(),
        format!("Are you sure you want to permanently delete these {count} items?")
    );
    assert_eq!(dialog.button_labels(), ["Yes", "No"]);
    dialog.press("Yes");
    wait_until("the Recycle Bin to be empty", || test.names().is_empty());
    wait_until("the empty folder note", || {
        test.window.folder_pane().shows_empty_note()
    });
    assert_eq!(
        test.window.folder_pane().page(),
        Some(PanePage::Listing),
        "no page of its own"
    );
    let left = glib::MainContext::default()
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    assert!(left.is_empty(), "{left:?}");
}

/// Empty Recycle Bin deletes only the items it asked about: one deleted
/// while the question shows stays in the Recycle Bin.
///
/// parity: OPS-042
#[gtk::test]
fn empty_recycle_bin_keeps_an_item_deleted_while_it_asks() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Asked about.txt");
    fixture.write("Arrived later.txt");
    trash(&fixture.path("Asked about.txt"));
    require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        test.names().contains(&"Asked about.txt".to_owned())
    });

    test.activate("empty-recycle-bin", None);
    let dialog = open_dialog(&test);
    trash(&fixture.path("Arrived later.txt"));
    dialog.press("Yes");

    wait_until("only the later item to be left", || {
        test.names() == ["Arrived later.txt"]
    });
    let left = glib::MainContext::default()
        .block_on(list_recycle_bin(&Cancellation::new()))
        .expect("a readable Recycle Bin");
    let left: Vec<_> = left.into_iter().map(|item| item.name).collect();
    assert_eq!(left, ["Arrived later.txt"]);
}

/// With "Ask before emptying the Recycle Bin" off, Empty Recycle Bin
/// empties it at once.
///
/// parity: SET-010
#[gtk::test]
fn empty_recycle_bin_asks_nothing_when_the_settings_say_so() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Old note.txt");
    trash(&fixture.path("Old note.txt"));
    require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(TRASH_URI);
    let update = PreferencesUpdate {
        confirm_empty_trash: Some(false),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        !test.context.settings_data().preferences.confirm_empty_trash
    });
    wait_until("the trashed file to be listed", || !test.names().is_empty());

    test.activate("empty-recycle-bin", None);
    wait_until("the Recycle Bin to be empty", || test.names().is_empty());
    assert!(dialog_over(&test).is_none(), "nothing asked");
}

/// The sidebar's Recycle Bin shows whether it is full and empties it
/// from any folder; Recent files sits beside it.
///
/// parity: SIDE-025, SIDE-026, OPS-042
#[gtk::test]
fn the_sidebar_recycle_bin_shows_it_is_full_and_empties_from_anywhere() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Old draft.txt");
    trash(&fixture.path("Old draft.txt"));
    require_only_test_items_in_the_recycle_bin();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    let bin_tooltip = || {
        let index = sidebar.labels().iter().position(|label| label == "Recycle Bin");
        let index = i32::try_from(index.expect("the sidebar shows the Recycle Bin")).unwrap_or(0);
        let row = sidebar.list().row_at_index(index).expect("its row");
        row.tooltip_text().map(String::from).unwrap_or_default()
    };
    assert!(sidebar.labels().contains(&"Recent files".to_owned()));
    // Earlier tests in this run may have left items of their own.
    wait_until("the full Recycle Bin", || {
        bin_tooltip().ends_with(" item") || bin_tooltip().ends_with(" items")
    });

    wait_for_frames(&test.window, 3);
    let menu = sidebar.right_click_row("Recycle Bin");
    let labels = menu.row_labels();
    let empty = menu.row("Empty Recycle Bin");
    menu.popdown();
    assert_eq!(labels[..3], ["Open", "Open in new tab", "Open in new window"]);
    assert!(empty.is_sensitive(), "something to empty");
    test.activate("empty-trash", None);
    open_dialog(&test).press("Yes");

    wait_until("the empty Recycle Bin", || bin_tooltip() == "Recycle Bin · Empty");
    assert_eq!(test.window.current_uri(), Some(fixture.uri()), "the folder stays");
}

/// parity: OPS-045
#[gtk::test]
fn items_dropped_on_the_recycle_bin_go_to_the_trash() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Drop me in the bin.txt");
    let test = TestWindow::open(TRASH_URI);
    test.wait_for_listing("the Recycle Bin");

    let taken = test.window.drop_files(
        &[fixture.uri_of("Drop me in the bin.txt")],
        None,
        DropAction::Copy,
    );
    assert!(taken, "{}", test.window.shown_message());

    wait_until("the dropped file to be listed", || {
        test.names().contains(&"Drop me in the bin.txt".to_owned())
    });
    assert!(!fixture.path("Drop me in the bin.txt").exists());
}

/// parity: OPS-046, DND-018
#[gtk::test]
fn items_dragged_out_of_the_recycle_bin_are_moved_into_the_folder() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Bring me back here.txt");
    trash(&fixture.path("Bring me back here.txt"));
    let bin = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        bin.names().contains(&"Bring me back here.txt".to_owned())
    });
    select_names(&bin, &["Bring me back here.txt"]);
    let position = bin.window.folder_model().first_selected().expect("selected");
    assert!(
        bin.window.drag_content_for(position).is_some(),
        "Recycle Bin items can be dragged"
    );
    let trashed = bin.window.folder_model().selected_uris();
    assert!(
        !bin.window.drop_files(&trashed, None, DropAction::Copy),
        "not back into the Recycle Bin"
    );

    let documents = TestWindow::open(&fixture.uri_of("Documents"));
    assert!(documents.window.drop_files(&trashed, None, DropAction::Copy));

    let moved = fixture.path("Documents").join("Bring me back here.txt");
    wait_until("the item to be moved into Documents", || moved.is_file());
    assert!(
        !fixture.path("Bring me back here.txt").exists(),
        "moved, not restored"
    );
}

/// A Recycle Bin item dragged into a folder that has its name asks
/// "Replace or Skip Files" in the progress window, as Windows does, and
/// Skip leaves it in the Recycle Bin without a word.
///
/// parity: OPS-046, OPS-026, OPS-023
#[gtk::test]
fn an_item_dragged_out_onto_a_taken_name_asks_replace_or_skip_files() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Bring me back here.txt");
    trash(&fixture.path("Bring me back here.txt"));
    fixture.write("Documents/Bring me back here.txt");
    let bin = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        bin.names().contains(&"Bring me back here.txt".to_owned())
    });
    select_names(&bin, &["Bring me back here.txt"]);
    let trashed = bin.window.folder_model().selected_uris();

    let documents = TestWindow::open(&fixture.uri_of("Documents"));
    assert!(documents.window.drop_files(&trashed, None, DropAction::Copy));
    let window = question_window(&documents);
    let shown = texts(&window);

    assert!(
        shown.contains(&"The destination already has a file named \"Bring me back here.txt\"".to_owned()),
        "{shown:?}"
    );
    button(&window, "Skip this file").emit_clicked();
    wait_until("the move to end", || !documents.window.is_writing_files());
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    assert!(dialog_over(&documents).is_none(), "nothing is said afterwards");
    assert!(
        gio::File::for_uri(&trashed[0]).query_exists(None::<&gio::Cancellable>),
        "still in the Recycle Bin"
    );
}

/// A right-button drop of Recycle Bin items asks before anything moves,
/// with Move here (bold) and Cancel only, as items can only be moved out
/// of the Recycle Bin; Cancel leaves them there and Move here moves them.
///
/// parity: OPS-046, DND-027
#[gtk::test]
fn a_right_drop_of_recycle_bin_items_asks_before_moving_them() {
    require_private_trash();
    let fixture = Fixture::standard();
    fixture.write("Ask before moving.txt");
    trash(&fixture.path("Ask before moving.txt"));
    let bin = TestWindow::open(TRASH_URI);
    wait_until("the trashed file to be listed", || {
        bin.names().contains(&"Ask before moving.txt".to_owned())
    });
    select_names(&bin, &["Ask before moving.txt"]);
    let trashed = bin.window.folder_model().selected_uris();
    let documents = TestWindow::open(&fixture.uri_of("Documents"));
    let moved = fixture.path("Documents").join("Ask before moving.txt");
    let ask = DropAction::Ask(crate::window::file_drop::DragOrigin::ThisApp);

    assert!(documents.window.drop_files(&trashed, None, ask));
    let menu = documents.window.drop_menu();
    wait_until("the drop menu", || menu.is_mapped());
    let labels = menu.row_labels();
    let bold = menu.emphasised_labels();
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    let moved_before_the_answer = moved.exists();
    menu.popdown();
    wait_until("the menu to close", || !menu.is_mapped());
    crate::test_support::harness::wait_for(std::time::Duration::from_millis(300));
    let moved_after_cancel = moved.exists();
    assert!(documents.window.drop_files(&trashed, None, ask));
    wait_until("the drop menu", || menu.is_mapped());
    menu.click_row("Move here");

    assert_eq!(labels, ["Move here", "-", "Cancel"]);
    assert_eq!(bold, ["Move here"]);
    assert!(!moved_before_the_answer, "nothing moves before the answer");
    assert!(!moved_after_cancel, "Cancel leaves the item in the Recycle Bin");
    wait_until("the item to be moved into Documents", || moved.is_file());
}

/// Undo asks before it moves a copy that changed after the copy to the
/// Recycle Bin; Cancel keeps the copy and the Undo step.
///
/// parity: OPS-030
#[gtk::test]
fn undoing_a_copy_that_changed_since_asks_and_cancel_keeps_it() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Documents")));
    test.wait_for_listing("the Documents folder");
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    test.activate("paste", None);
    let copy = fixture.path("Documents/Notes 2.txt");
    wait_until("the copy", || copy.is_file());
    super::file_ops_support::assert_ended_without_a_message(&test);
    let edited = std::fs::File::options()
        .write(true)
        .open(&copy)
        .expect("the copy exists");
    let a_minute_ahead = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    edited.set_modified(a_minute_ahead).expect("the copy is ours");

    test.activate("undo", None);
    let question = open_dialog(&test);
    assert_eq!(question.title_text(), "Undo copy?");
    assert_eq!(
        question.message_text(),
        "“Notes 2.txt” may have changed after it was copied. Undo moves it to the Recycle Bin anyway?"
    );
    question.press("Cancel");
    wait_for_no_dialog(&test);

    assert!(copy.is_file(), "the copy stays");
    assert_eq!(test.window.journal_label(JournalDirection::Undo), "Undo Copy");
}
