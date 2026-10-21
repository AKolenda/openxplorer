// SPDX-License-Identifier: AGPL-3.0-only
//! Windows 11 File Explorer's further keys: Ctrl+E for the search box,
//! Ctrl+D for Delete, F11 for full screen and Alt+P for the preview pane,
//! which is the details pane here (CMD-036, VIEW-063).
//!
//! GTK has no public way to synthesise key events, so these tests run the
//! window's shortcut controllers for a key, the application's accelerators
//! among them, as [`press_shortcut_where_focused`] does.

use gtk::gdk;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::file_ops_support::{
    dialog_over, name_editor, open_dialog, press_shortcut, press_shortcut_where_focused,
    require_private_trash, select_names,
};
use crate::test_support::harness::{application, settle, wait_until, Fixture, TestWindow};
use crate::window::window_action::WindowAction;

/// Whether keyboard focus is in the search box's field.
fn focus_is_in_search(test: &TestWindow) -> bool {
    let entry = test.window.search_box().entry();
    GtkWindowExt::focus(&test.window)
        .is_some_and(|focus| focus == *entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&entry))
}

/// The new keys are application accelerators beside the ones they match,
/// and the keys they sit beside keep their own: F3 is still Split view.
///
/// parity: CMD-036, VIEW-063
#[gtk::test]
fn explorers_further_keys_sit_beside_the_ones_they_match() {
    let app = application();
    let keys = |action: WindowAction| app.accels_for_action(&action.detailed_name());

    assert_eq!(keys(WindowAction::Search), ["<Control>f", "<Control>e"]);
    assert_eq!(keys(WindowAction::DetailsPane), ["<Shift><Alt>p", "<Alt>p"]);
    assert_eq!(keys(WindowAction::FullScreen), ["F11"]);
    assert_eq!(keys(WindowAction::SplitView), ["F3"], "F3 keeps Split view");
    for key in ["<Primary>e", "<Alt>p", "F11"] {
        assert_eq!(app.actions_for_accel(key).len(), 1, "{key} runs one action");
    }
}

/// Ctrl+E moves to the search box and selects its text, as Ctrl+F does,
/// from the file list and from the address being typed alike.
///
/// parity: CMD-036
#[gtk::test]
fn ctrl_e_moves_to_the_search_box_and_selects_its_text() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let entry = test.window.search_box().entry();
    entry.set_text("Notes");
    test.window.folder_pane().focus_view();
    settle();

    let took = press_shortcut_where_focused(&test, gdk::Key::e, gdk::ModifierType::CONTROL_MASK);

    assert!(took, "Ctrl+E is handled");
    assert!(focus_is_in_search(&test), "focus moves to the search box");
    assert_eq!(entry.selection_bounds(), Some((0, 5)), "its text is selected");

    test.activate("location", None);
    settle();
    assert!(!focus_is_in_search(&test), "the address is being edited");
    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::e,
        gdk::ModifierType::CONTROL_MASK
    ));
    assert!(focus_is_in_search(&test), "Ctrl+E works from the address bar too");
}

/// Ctrl+D asks and moves the selection to the Recycle Bin exactly as
/// Delete does, and the toast's Undo brings it back.
///
/// parity: CMD-036, OPS-015
#[gtk::test]
fn ctrl_d_moves_the_selection_to_the_recycle_bin_as_delete_does() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);

    press_shortcut(&test, gdk::Key::d, gdk::ModifierType::CONTROL_MASK);
    let dialog = open_dialog(&test);

    assert_eq!(dialog.title_text(), "Move to Trash?");
    assert_eq!(dialog.button_labels(), ["Cancel", "Move to Trash"]);
    dialog.press("Move to Trash");
    wait_until("the file to leave the folder", || {
        !fixture.path("Résumé.txt").exists()
    });
    wait_until("the toast", || {
        test.window.shown_message() == "1 item(s) sent to Trash."
    });
    let toast = test.window.imp().toast.get();
    assert_eq!(toast.action_label().as_deref(), Some("Undo"));
    toast.press_action();
    wait_until("the file to come back", || fixture.path("Résumé.txt").is_file());
}

/// A text field keeps Ctrl+D, as it keeps Delete: the search box, the
/// address being typed and a name being renamed delete nothing, and
/// neither does the Settings page.
///
/// parity: CMD-036, CMD-017
#[gtk::test]
fn ctrl_d_leaves_text_fields_and_the_settings_page_alone() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Résumé.txt"]);
    let ctrl_d =
        |test: &TestWindow| press_shortcut_where_focused(test, gdk::Key::d, gdk::ModifierType::CONTROL_MASK);

    test.window.search_box().focus();
    let in_search = ctrl_d(&test);
    test.activate("location", None);
    settle();
    let in_address = ctrl_d(&test);
    test.window.folder_pane().focus_view();
    select_names(&test, &["Résumé.txt"]);
    test.activate("rename", None);
    let field = name_editor(&test);
    field.grab_focus();
    settle();
    let in_rename = ctrl_d(&test);
    test.activate("settings", None);
    let on_settings = ctrl_d(&test);
    settle();

    assert!(!in_search, "the search box keeps Ctrl+D");
    assert!(!in_address, "the address keeps Ctrl+D");
    assert!(!in_rename, "the name being renamed keeps Ctrl+D");
    assert!(!on_settings, "the Settings page deletes nothing");
    assert!(dialog_over(&test).is_none(), "nothing asked to delete");
    assert!(fixture.path("Résumé.txt").is_file());
}

/// Alt+P toggles the details pane, which also previews the selection
/// here, as Alt+Shift+P does, and saves the choice the same way.
///
/// parity: VIEW-063
#[gtk::test]
fn alt_p_toggles_the_details_pane_as_alt_shift_p_does() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let shown = || {
        test.window
            .window_action_state(WindowAction::DetailsPane)
            .and_then(|state| state.get::<bool>())
            .expect("the details pane action has a state")
    };
    let before = shown();
    test.window.folder_pane().focus_view();

    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::p,
        gdk::ModifierType::ALT_MASK
    ));
    let after_alt_p = shown();
    wait_until("Alt+P's choice to be saved", || {
        test.context.settings_data().preferences.show_details_pane == after_alt_p
    });
    assert!(press_shortcut_where_focused(
        &test,
        gdk::Key::p,
        gdk::ModifierType::ALT_MASK | gdk::ModifierType::SHIFT_MASK
    ));

    assert_ne!(after_alt_p, before, "Alt+P toggles the pane");
    assert_eq!(shown(), before, "Alt+Shift+P toggles it back");
    wait_until("Alt+Shift+P's choice to be saved", || {
        test.context.settings_data().preferences.show_details_pane == before
    });
}

/// F11 runs Full screen, from the file list and from a text field, and
/// the action asks for full screen or leaves it by the window's state.
///
/// parity: CMD-036
#[gtk::test]
fn f11_toggles_full_screen_from_any_focus() {
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    test.window.folder_pane().focus_view();

    let from_list = press_shortcut_where_focused(&test, gdk::Key::F11, gdk::ModifierType::empty());
    let asked = test.window.imp().wants_full_screen.get();
    test.window.search_box().focus();
    let from_search = press_shortcut_where_focused(&test, gdk::Key::F11, gdk::ModifierType::empty());

    assert!(from_list, "F11 is handled in the file list");
    assert!(asked, "F11 asks for full screen");
    assert!(from_search, "and from the search box");
    assert!(
        !test.window.imp().wants_full_screen.get(),
        "a second F11 leaves full screen"
    );
}
