// SPDX-License-Identifier: AGPL-3.0-only
//! Screenshots of the file-operation surfaces for visual review.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set (passed through the isolated check
//! environment), these save `native-dialog-delete-*.png`,
//! `native-dialog-conflict-*.png`, `native-transfer-*.png`,
//! `native-rename-in-place-*.png` and `native-menu-compact-*.png` there. Without it, they only prove the
//! surfaces open in both themes.

use std::path::PathBuf;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::settings::{ContextMenu, PreferencesUpdate, Settings};

use super::file_ops_support::{is_enabled, open_dialog, select_names, wait_for_no_dialog};
use crate::test_support::harness::{
    capture, capture_popover, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};
use crate::window::dialog::Dialog;

/// The theme keys, as `win.theme` takes them.
const THEMES: [&str; 2] = ["light", "dark"];

/// Saves `dialog` as `filename` in `$OX_NATIVE_CAPTURE_DIR`, when set.
fn capture_dialog(test: &TestWindow, dialog: &Dialog, filename: &str) {
    let Some(directory) = std::env::var_os("OX_NATIVE_CAPTURE_DIR").map(PathBuf::from) else {
        return;
    };
    wait_for_frames(&test.window, 3);
    let path = directory.join(filename);
    // The dialog's surface without the margin GTK keeps around a window.
    let outer = dialog.compute_bounds(dialog).expect("a shown dialog has bounds");
    let content = dialog
        .child()
        .and_then(|child| child.compute_bounds(dialog))
        .expect("the dialog has content");
    let crop = content.offset_r(-outer.x(), -outer.y());
    wait_until("the dialog to be saved", || {
        crate::snapshot::render_png(dialog, Some(crop), &path).is_ok()
    });
}

#[gtk::test]
fn the_delete_and_conflict_dialogs_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for theme in THEMES {
        test.activate("theme", Some(theme));
        select_names(&test, &["Notes 2.txt"]);
        test.activate("trash", None);
        let delete = open_dialog(&test);
        capture_dialog(&test, &delete, &format!("native-dialog-delete-{theme}.png"));
        delete.press("Cancel");
        wait_for_no_dialog(&test);
        select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
        test.activate("copy", None);
        wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
        test.activate("paste", None);
        let conflict = open_dialog(&test);
        capture_dialog(&test, &conflict, &format!("native-dialog-conflict-{theme}.png"));
        conflict.press("Cancel");
        wait_for_no_dialog(&test);
    }
}

#[gtk::test]
fn the_transfer_panel_and_the_compact_menu_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    let update = PreferencesUpdate {
        context_menu: Some(ContextMenu::Win11),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the compact style", || {
        test.context.settings_data().preferences.context_menu == ContextMenu::Win11
    });
    for theme in THEMES {
        test.activate("theme", Some(theme));
        test.window.begin_operation("Copy: Notes 2.txt (1/3)");
        test.window
            .imp()
            .transfer_panel
            .show_progress("Copy: Notes 2.txt (1/3)", 0.4);
        capture(&test.window, &format!("native-transfer-{theme}.png"));
        test.window.end_operation();
        test.window.folder_model().select_only(1);
        test.activate("rename", None);
        capture(&test.window, &format!("native-rename-in-place-{theme}.png"));
        test.window.folder_pane().focus_view();
        test.window.right_click(Some(0));
        let menu = test.window.context_menu();
        capture_popover(
            &test.window,
            menu.upcast_ref(),
            &format!("native-menu-compact-{theme}.png"),
        );
        menu.popdown();
        wait_until("the menu to close", || !menu.is_mapped());
    }
}
