// SPDX-License-Identifier: AGPL-3.0-only
//! More of Windows 11 File Explorer's keys (CMD-039): Ctrl+1 to Ctrl+9 for
//! the tabs, Ctrl+Shift+E and the keypad's plus, minus and asterisk in the
//! navigation pane, Ctrl+F5 for Refresh, Ctrl+Insert and Shift+Insert for
//! Copy and Paste, and Ctrl+Shift+1 to 8 as a keyboard sends them, with the
//! digit key's shifted symbol.
//!
//! GTK has no public way to synthesise key events, so these tests run the
//! window's shortcut controllers and key handlers for a key, as
//! [`press_shortcut_where_focused`] does; the real-input checks with
//! xdotool are in the job's status report.

use std::fs;

use gtk::glib::translate::IntoGlib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use ox_core::location::{file_uri, same_location};

use super::file_ops_support::{press_shortcut, press_shortcut_where_focused, select_names};
use super::sidebar::row_named;
use crate::test_support::harness::{application, settle, wait_until, Fixture, TestWindow};
use crate::window::folder_pane::FolderView;
use crate::window::window_action::WindowAction;

/// What the first key handler of `widget` does with `key`, no modifier
/// held, as if it were pressed where the focus is.
fn press_on(widget: &impl IsA<gtk::Widget>, key: gdk::Key) -> bool {
    let controller = widget
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("the widget takes keys");
    let no_keycode = 0_u32;
    let modifiers = gdk::ModifierType::empty();
    controller.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &no_keycode, &modifiers])
}

/// The folder tree's list of rows.
fn tree_view(test: &TestWindow) -> gtk::ListView {
    crate::test_support::harness::descendants::<gtk::ListView>(test.window.folder_tree())
        .into_iter()
        .next()
        .expect("the tree has its list")
}

/// A window on `Example projects/Documents/Reports`, whose folders below
/// the fixture are `Documents/Reports/2026` and `Documents/Archive`, with
/// the folder tree shown and following it.
fn window_with_tree(fixture: &Fixture) -> (TestWindow, String) {
    let reports = fixture.path("Documents").join("Reports");
    fs::create_dir_all(reports.join("2026")).expect("fixture subfolders");
    fs::create_dir(fixture.path("Documents").join("Archive")).expect("fixture subfolder");
    let test = TestWindow::open(&file_uri(&reports));
    // The fixture's temporary folder plays the home folder, so the tree
    // starts there and never lists the real disk.
    test.window.imp().locations.borrow_mut().home = fixture.root().parent().map(Into::into);
    test.activate("folder-tree", None);
    let reports = file_uri(&reports);
    let tree = test.window.folder_tree();
    wait_until("the tree selects the folder shown", || {
        tree.selected_uri()
            .is_some_and(|uri| same_location(&uri, &reports))
    });
    (test, reports)
}

/// Ctrl+1 to Ctrl+9 show the tab of that number from the file list, as
/// in Windows 11 File Explorer, beside Alt+1 to Alt+9; a text field keeps
/// them, and Ctrl+0 stays the text size's reset.
///
/// parity: CMD-039
#[gtk::test]
fn ctrl_and_a_number_show_the_tab_of_that_number() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.activate("new-tab", None);
    test.activate("new-tab", None);
    settle();
    let ids: Vec<_> = test
        .window
        .imp()
        .session
        .borrow()
        .tabs()
        .iter()
        .map(|tab| tab.id)
        .collect();
    assert_eq!(ids.len(), 3);
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::_1,
        gdk::ModifierType::CONTROL_MASK
    ));
    assert_eq!(test.active_tab(), Some(ids[0]), "Ctrl+1 shows the first tab");
    test.window.folder_pane().focus_view();
    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::_2,
        gdk::ModifierType::CONTROL_MASK
    ));
    assert_eq!(test.active_tab(), Some(ids[1]), "Ctrl+2 the second");

    test.window.search_box().focus();
    settle();
    assert!(
        !press_shortcut_where_focused(&test, gdk::Key::_3, gdk::ModifierType::CONTROL_MASK),
        "the search box keeps Ctrl+3"
    );
    assert_eq!(test.active_tab(), Some(ids[1]));
    let app = application();
    assert_eq!(
        app.actions_for_accel("<Primary>0"),
        [WindowAction::TextSize(crate::text_size::Step::Reset).detailed_name()],
        "Ctrl+0 resets the text size"
    );
}

/// Ctrl+F5 refreshes, as F5 and Ctrl+R do, from any focus.
///
/// parity: CMD-039
#[gtk::test]
fn ctrl_f5_refreshes_as_f5_does() {
    let app = application();
    let keys = app.accels_for_action(&WindowAction::Refresh.detailed_name());
    assert_eq!(keys, ["F5", "<Control>r", "<Control>F5"]);
    assert_eq!(
        app.actions_for_accel("<Primary>F5").len(),
        1,
        "Ctrl+F5 runs one action"
    );
}

/// Ctrl+Insert copies the selection and Shift+Insert pastes it, as Ctrl+C
/// and Ctrl+V do, in the file list; a text field keeps both keys.
///
/// parity: CMD-039
#[gtk::test]
fn ctrl_insert_copies_and_shift_insert_pastes() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);

    press_shortcut(&test, gdk::Key::Insert, gdk::ModifierType::CONTROL_MASK);
    press_shortcut(&test, gdk::Key::Insert, gdk::ModifierType::SHIFT_MASK);

    wait_until("the copy to be pasted", || {
        fixture.path("Résumé - Copy.txt").is_file()
    });
    test.window.search_box().focus();
    settle();
    assert!(
        !press_shortcut_where_focused(&test, gdk::Key::Insert, gdk::ModifierType::SHIFT_MASK),
        "the search box pastes text itself"
    );
}

/// Ctrl+Shift+E opens the folder tree down to the folder shown and its
/// subfolders, selects it and puts keyboard focus on it, as Windows 11's
/// navigation pane does; a text field keeps the key.
///
/// parity: CMD-039
#[gtk::test]
fn ctrl_shift_e_opens_the_navigation_pane_down_to_the_folder_shown() {
    let fixture = Fixture::standard();
    let (test, reports) = window_with_tree(&fixture);
    let tree = test.window.folder_tree();
    assert!(
        !tree.outline().iter().any(|line| line.trim() == "2026"),
        "the folder shown is not open yet"
    );
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::e,
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
    ));

    wait_until("the folder shown to show its subfolders", || {
        tree.outline().iter().any(|line| line.trim() == "2026")
    });
    assert!(tree
        .selected_uri()
        .is_some_and(|uri| same_location(&uri, &reports)));
    let focus = GtkWindowExt::focus(&test.window).expect("something has focus");
    assert!(focus.is_ancestor(tree), "keyboard focus is on the tree");

    test.window.search_box().focus();
    settle();
    assert!(!press_shortcut_where_focused(
        &test,
        gdk::Key::e,
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
    ));
}

/// Without the folder tree, Ctrl+Shift+E opens the collapsed section that
/// holds the location and puts keyboard focus on its row.
///
/// parity: CMD-039
#[gtk::test]
fn ctrl_shift_e_without_the_tree_focuses_the_places_row() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::e,
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK
    ));

    let focus = GtkWindowExt::focus(&test.window).expect("something has focus");
    assert!(
        focus.is_ancestor(test.window.sidebar().list()),
        "keyboard focus is in the navigation pane"
    );
}

/// In the folder tree the keypad's minus closes the selected folder, its
/// plus opens it, and its asterisk opens every folder below it, as Num
/// Lock with them does in Windows; the keys are not typed into the file
/// list.
///
/// parity: CMD-039
#[gtk::test]
fn the_keypad_keys_open_and_close_folders_in_the_folder_tree() {
    let fixture = Fixture::standard();
    let (test, _) = window_with_tree(&fixture);
    let tree = test.window.folder_tree();
    let view = tree_view(&test);
    let shows = |name: &str| tree.outline().iter().any(|line| line.trim() == name);
    assert!(shows("Reports") && !shows("2026"));

    assert!(press_on(&view, gdk::Key::KP_Add), "plus opens the folder");
    wait_until("Reports to show 2026", || shows("2026"));
    assert!(press_on(&view, gdk::Key::KP_Subtract), "minus closes it");
    assert!(!shows("2026"));

    // Documents, opened from the tree, closed, then the asterisk opens
    // every folder below it.
    let documents = tree
        .outline()
        .iter()
        .position(|line| line.trim() == "Documents")
        .expect("Documents is listed");
    tree.activate_row(u32::try_from(documents).expect("a short tree"));
    let documents = fixture.uri_of("Documents");
    wait_until("the tree to select Documents", || {
        tree.selected_uri()
            .is_some_and(|uri| same_location(&uri, &documents))
    });
    assert!(press_on(&view, gdk::Key::KP_Subtract));
    assert!(!shows("Reports"), "Documents is closed");
    assert!(press_on(&view, gdk::Key::KP_Multiply), "asterisk opens all");
    wait_until("every folder below Documents to show", || {
        shows("Reports") && shows("Archive") && shows("2026")
    });
}

/// On This PC's head in the navigation pane, the keypad's minus collapses
/// the section and its plus or asterisk expands it again; on any row the
/// keys stay in the navigation pane.
///
/// parity: CMD-039
#[gtk::test]
fn the_keypad_keys_collapse_and_expand_a_section_of_the_navigation_pane() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let sidebar = test.window.sidebar();
    row_named(&test, "This PC").grab_focus();

    assert!(press_on(sidebar.list(), gdk::Key::KP_Subtract));
    assert!(sidebar.section_is_collapsed("thisPc"));
    assert!(press_on(sidebar.list(), gdk::Key::KP_Add));
    assert!(!sidebar.section_is_collapsed("thisPc"));
    assert!(press_on(sidebar.list(), gdk::Key::KP_Subtract));
    assert!(press_on(sidebar.list(), gdk::Key::KP_Multiply));
    assert!(!sidebar.section_is_collapsed("thisPc"));

    row_named(&test, "Home").grab_focus();
    assert!(
        press_on(sidebar.list(), gdk::Key::KP_Subtract),
        "the key stays in the navigation pane"
    );
}

/// A keyboard sends Ctrl+Shift+5 as Ctrl+Shift with the 5 key's shifted
/// symbol (`%` on a US layout); that switches the view as Ctrl+Shift+5
/// does, as do the other digits of the views.
///
/// parity: CMD-039, VIEW-009
#[gtk::test]
fn ctrl_shift_and_a_digit_key_switch_the_view_as_a_keyboard_sends_them() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let both = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
    test.window.folder_pane().focus_view();

    for (key, view) in [
        (gdk::Key::percent, FolderView::Compact),
        (gdk::Key::ampersand, FolderView::Tiles),
        (gdk::Key::asterisk, FolderView::Content),
        (gdk::Key::asciicircum, FolderView::Details),
    ] {
        assert!(
            press_shortcut_where_focused(&test, key, both),
            "{key:?} is a view key"
        );
        settle();
        assert_eq!(test.window.folder_pane().view(), view, "{key:?}");
    }
}
