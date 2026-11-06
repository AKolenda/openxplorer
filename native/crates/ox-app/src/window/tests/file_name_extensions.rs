// SPDX-License-Identifier: AGPL-3.0-only
//! File name extensions (VIEW-069): hidden for known types by default, as
//! in Windows, shown with View > Show > File name extensions or Folder
//! Options, and kept by every rename and new file.

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::Settings;

use super::file_ops_support::{is_renaming_in_place, name_editor};
use super::geometry::laid_out;
use super::view_styles::{close_folder_options, open_folder_options, option_check};
use crate::folder_view::cells::FileCell;
use crate::test_support::harness::{descendants, wait_for_frames, wait_until, Fixture, TestWindow};
use crate::window::menu_popover::MenuPopover;

/// The names the file list shows now.
fn shown_names(test: &TestWindow) -> Vec<String> {
    wait_for_frames(&test.window, 2);
    let cells = descendants::<FileCell>(&test.window);
    let mut names: Vec<String> = cells
        .iter()
        .filter(|cell| cell.is_mapped())
        .map(FileCell::name)
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Whether the saved settings show extensions, as another window reads
/// them.
fn saved_shown(test: &TestWindow) -> bool {
    Settings::open(test.settings_directory())
        .data()
        .preferences
        .show_extensions
}

/// Whether the app's settings hold `shown` for extensions, which every
/// window takes up after each save.
fn holds_shown(test: &TestWindow, shown: bool) -> bool {
    test.context.settings_data().preferences.show_extensions == shown
}

/// The View menu's Show submenu, opened, and whether it checks File name
/// extensions.
fn show_menu_checks_extensions(test: &TestWindow) -> bool {
    let button = super::support::menu_button_with_class(test, "view-command");
    let menu = button
        .popover()
        .and_downcast::<MenuPopover>()
        .expect("View's menu");
    menu.popup();
    wait_until("the View menu", || menu.is_visible());
    menu.click_row("Show");
    let labels = menu.row_labels();
    let at = labels.iter().position(|label| label == "File name extensions");
    assert_eq!(
        labels[at.expect("Show has File name extensions") - 1..][..3],
        ["Item check boxes", "File name extensions", "Hidden items"],
        "{labels:?}"
    );
    let checked = menu.checked_labels().contains(&"File name extensions".to_owned());
    menu.popdown();
    wait_until("the View menu to close", || !menu.is_visible());
    checked
}

/// A fresh window hides the extensions of known types, as Windows does by
/// default: "Notes 2.txt" shows as "Notes 2", a folder keeps its name, and
/// the tile, its accessible name and the details pane follow. View > Show
/// > File name extensions, unchecked, shows them in every view at once
/// and saves the choice.
///
/// parity: VIEW-069
#[gtk::test]
fn known_extensions_are_hidden_by_default_and_view_show_shows_them() {
    let fixture = Fixture::standard();
    std::fs::create_dir(fixture.path("Album.txt")).expect("a folder with a dot");
    let test = laid_out(&fixture.uri());
    assert!(
        !test.context.settings_data().preferences.show_extensions,
        "off by default"
    );
    let names = shown_names(&test);
    assert!(names.contains(&"Notes 2".to_owned()), "{names:?}");
    assert!(names.contains(&"Résumé".to_owned()), "{names:?}");
    assert!(
        names.contains(&"Album.txt".to_owned()),
        "a folder keeps its name: {names:?}"
    );
    assert!(!names.iter().any(|name| name == "Notes 2.txt"), "{names:?}");
    assert!(!show_menu_checks_extensions(&test));
    test.select_named("Notes 2.txt");
    test.activate("details-pane", None);
    wait_until("the details pane to name the file", || {
        test.window.imp().details_pane.shown_name() == "Notes 2"
    });
    test.activate("view", Some("large"));
    let names = shown_names(&test);
    assert!(names.contains(&"Notes 2".to_owned()), "the icons too: {names:?}");

    test.activate("file-name-extensions", None);

    // Every finished save has the windows take up the saved preferences,
    // so one that finished before the toggle's own (View's, here) can show
    // the old choice for a moment. Once the choice is held, none can.
    wait_until("the choice to be held", || holds_shown(&test, true));
    wait_until("the names to show their extensions", || {
        shown_names(&test).contains(&"Notes 2.txt".to_owned())
    });
    assert!(show_menu_checks_extensions(&test));
    wait_until("the choice to be saved", || saved_shown(&test));
    wait_until("the details pane to follow", || {
        test.window.imp().details_pane.shown_name() == "Notes 2.txt"
    });
}

/// File name extensions, toggled while the rows are bound, renames them
/// for a screen reader too: each list item's accessible label follows the
/// name it shows, not the name it had when it was bound.
///
/// parity: VIEW-069
#[gtk::test]
fn toggling_extensions_renames_the_rows_for_a_screen_reader() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("view", Some("large"));
    let labels = || {
        wait_for_frames(&test.window, 2);
        test.window.folder_pane().owners().bound_accessible_labels()
    };
    let before = labels();
    assert!(before.contains(&"Notes 2".to_owned()), "{before:?}");

    test.activate("file-name-extensions", None);

    // As above, an earlier save can show the old names for a moment.
    wait_until("the choice to be held", || holds_shown(&test, true));
    wait_until("the labels to show extensions", || {
        let after = labels();
        after.contains(&"Notes 2.txt".to_owned()) && !after.contains(&"Notes 2".to_owned())
    });
    test.activate("file-name-extensions", None);
    wait_until("the choice to be held", || holds_shown(&test, false));
    wait_until("the labels to hide them again", || {
        labels().contains(&"Notes 2".to_owned())
    });
}

/// With extensions hidden, F2 edits the name without its extension and
/// the rename keeps the extension, as in Windows; with them shown it edits
/// the whole name.
///
/// parity: VIEW-069, OPS-010
#[gtk::test]
fn renaming_with_extensions_hidden_keeps_the_extension() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.select_named("Notes 2.txt");
    test.activate("rename", None);
    let editor = name_editor(&test);
    assert_eq!(editor.text(), "Notes 2");
    editor.set_text("Minutes");
    editor.emit_activate();
    wait_until("the rename", || fixture.path("Minutes.txt").is_file());
    assert!(!fixture.path("Minutes").exists(), "the extension stays");
    wait_until("the rename to end", || !is_renaming_in_place(&test));

    test.activate("file-name-extensions", None);
    wait_until("the choice to be held", || holds_shown(&test, true));
    wait_until("the listing to have the new name", || {
        test.names().contains(&"Minutes.txt".to_owned())
    });
    test.select_named("Minutes.txt");
    test.activate("rename", None);
    assert_eq!(name_editor(&test).text(), "Minutes.txt");
}

/// A new text document is named in place without its extension while
/// extensions are hidden, and keeps ".txt" whatever name is typed.
///
/// parity: VIEW-069, OPS-003
#[gtk::test]
fn a_new_text_document_keeps_its_extension_while_extensions_are_hidden() {
    let fixture = Fixture::standard();
    let test = laid_out(&fixture.uri());
    test.activate("new-text-document", None);
    let editor = name_editor(&test);
    assert_eq!(editor.text(), "New Text Document");
    editor.set_text("Plan");
    editor.emit_activate();
    wait_until("the new file's name", || fixture.path("Plan.txt").is_file());
    assert!(!fixture.path("New Text Document.txt").exists());
}

/// Folder Options > View has Windows' "Hide extensions for known file
/// types", checked by default; unchecking it shows the extensions.
///
/// parity: VIEW-069, VIEW-020
#[gtk::test]
fn folder_options_hides_extensions_for_known_file_types() {
    const HIDE: &str = "Hide extensions for known file types";
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let options = open_folder_options(&test);
    let hide = option_check(&options, "View", HIDE);
    assert!(hide.is_active(), "checked by default, as in Windows");
    hide.set_active(false);
    close_folder_options(&test, &options, "OK");
    wait_until("the choice to be saved", || saved_shown(&test));
    wait_until("the names to show their extensions", || {
        shown_names(&test).contains(&"Notes 2.txt".to_owned())
    });
}
