// SPDX-License-Identifier: AGPL-3.0-only
//! Screenshots of the file-operation surfaces for visual review.
//!
//! With `OX_NATIVE_CAPTURE_DIR` set (passed through the isolated check
//! environment), these save `native-dialog-delete-*.png`,
//! `native-conflict-question-*.png`, `native-file-conflict-*.png`,
//! `native-transfer-*.png`,
//! `native-rename-in-place-*.png`, `native-menu-compact-*.png` and
//! `native-cut-details-*.png` / `native-cut-icons-*.png` there. Without
//! it, they only prove the surfaces open in both themes.

use std::path::PathBuf;

use gtk::prelude::*;
use ox_core::settings::{ContextMenu, PreferencesUpdate, Settings};
use ox_core::transfer::{Progress, ProgressScope};

use super::file_ops_support::{is_enabled, open_dialog, select_names, wait_for_no_dialog};
use crate::dialog::Dialog;
use crate::test_support::harness::{
    capture, capture_popover, descendants, wait_for_frames, wait_until, Fixture, TestWindow, ThemeGuard,
};

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
fn the_delete_dialog_is_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for theme in THEMES {
        test.activate("theme", Some(theme));
        select_names(&test, &["Notes 2.txt"]);
        test.activate("delete-permanently", None);
        let delete = open_dialog(&test);
        capture_dialog(&test, &delete, &format!("native-dialog-delete-{theme}.png"));
        delete.press("No");
        wait_for_no_dialog(&test);
    }
}

/// A page-like picture, taller than wide, as a scanned document is: the
/// File Conflict window shows its thumbnail.
fn write_page_picture(path: &std::path::Path) {
    let page = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 600, 1100)
        .expect("a picture");
    page.fill(0xf4_f4_f4_ff);
    for line in 0..18 {
        let stripe = page.new_subpixbuf(60, 80 + line * 52, if line % 3 == 2 { 300 } else { 480 }, 14);
        stripe.fill(0x9a_a4_b0_ff);
    }
    page.savev(path, "png", &[]).expect("the fixture is ours");
}

/// "Replace or Skip Files" and the File Conflict window in the situation
/// of the Windows screenshots they are compared with: one picture,
/// `report.png`, copied from Downloads to Desktop, which already has one
/// of the same date and size. Saves `native-conflict-question-*.png` and
/// `native-file-conflict-*.png`.
#[gtk::test]
fn the_conflict_questions_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::empty();
    for folder in ["Downloads", "Desktop"] {
        std::fs::create_dir(fixture.path(folder)).expect("the fixture is ours");
    }
    write_page_picture(&fixture.path("Downloads/report.png"));
    std::fs::copy(
        fixture.path("Downloads/report.png"),
        fixture.path("Desktop/report.png"),
    )
    .expect("the fixture is ours");
    let time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_793_459_580);
    for name in ["Downloads/report.png", "Desktop/report.png"] {
        std::fs::File::options()
            .write(true)
            .open(fixture.path(name))
            .and_then(|file| file.set_modified(time))
            .expect("the fixture is ours");
    }
    let test = TestWindow::open(&fixture.uri_of("Downloads"));
    select_names(&test, &["report.png"]);
    test.activate("copy", None);
    test.activate("go-to", Some(&fixture.uri_of("Desktop")));
    wait_until("Paste to be enabled", || {
        !test.window.is_loading() && is_enabled(&test, "paste")
    });
    for theme in THEMES {
        test.activate("theme", Some(theme));
        test.activate("paste", None);
        wait_until("the Replace or Skip Files question", || {
            test.context.progress_window().is_some_and(|window| {
                window.is_visible()
                    && window
                        .actions()
                        .iter()
                        .any(crate::window::transfer_panel::TransferPanel::is_asking)
            })
        });
        let progress = test.context.progress_window().expect("it asks");
        crate::test_support::harness::capture_dialog(
            &progress,
            &format!("native-conflict-question-{theme}.png"),
        );
        let compare = descendants::<gtk::Button>(&progress)
            .into_iter()
            .find(|button| {
                descendants::<gtk::Label>(button)
                    .iter()
                    .any(|label| label.text() == "Compare info for both files")
            })
            .expect("the row");
        compare.emit_clicked();
        wait_until("the File Conflict window", || {
            progress.conflict_window().is_some()
        });
        let conflicts = progress.conflict_window().expect("it shows");
        crate::test_support::harness::wait_for(std::time::Duration::from_millis(1500));
        crate::test_support::harness::capture_dialog(
            &conflicts,
            &format!("native-file-conflict-{theme}.png"),
        );
        // Closing it goes back to the question, as in Windows 11.
        conflicts.close();
        wait_until("the question again", || {
            progress.conflict_window().is_none() && progress.actions()[0].waits_for_a_row()
        });
        super::file_ops_support::answer_conflicts(&test, super::file_ops_support::ConflictAnswer::Skip);
        wait_until("the action to end", || test.context.progress_window().is_none());
    }
}

/// Two cut items, one of them selected, in both views.
#[gtk::test]
fn cut_items_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
    test.activate("cut", None);
    wait_until("Paste to be enabled", || is_enabled(&test, "paste"));
    select_names(&test, &["Notes 10.txt"]);
    for theme in THEMES {
        test.activate("theme", Some(theme));
        test.activate("view", Some("details"));
        capture(&test.window, &format!("native-cut-details-{theme}.png"));
        test.activate("view", Some("large"));
        capture(&test.window, &format!("native-cut-icons-{theme}.png"));
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
        test.window.transfer_panel().show_progress(&Progress {
            label: "Copy: Notes 2.txt (1/3)".to_owned(),
            fraction: 0.4,
            scope: ProgressScope::Batch,
            bytes: None,
            items: None,
            name: None,
        });
        let progress = test.context.progress_window().expect("the operation's window");
        progress.set_visible(true);
        crate::test_support::harness::capture_dialog(&progress, &format!("native-transfer-{theme}.png"));
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

/// The progress window with More details open, light and dark: the speed
/// graph, the name and what remains, for visual review.
#[gtk::test]
fn the_progress_windows_details_are_captured_light_and_dark() {
    let _theme = ThemeGuard::keep();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    for theme in THEMES {
        test.activate("theme", Some(theme));
        let context = test
            .window
            .begin_operation("Preparing copy…")
            .expect("the copy starts");
        let panel = test.window.transfer_panel().clone();
        let uris = [format!("{}/Notes 2.txt", fixture.uri())];
        let documents = format!("{}/Documents", fixture.uri());
        let verb = crate::window::transfer_panel::Verb::Copying;
        panel.describe(crate::window::transfer_panel::Action::of(verb, &uris, Some(&documents)).unwrap());
        let progress = test.context.progress_window().expect("the operation's window");
        progress.set_visible(true);
        if !progress.shows_details() {
            progress.press_details();
        }
        let size = 40 * 1024 * 1024;
        for step in 1..=44_u64 {
            let wobble = [3, 5, 2, 6, 4][usize::try_from(step % 5).unwrap()];
            wait_for_frames(&progress, u32::try_from(wobble).unwrap());
            panel.show_progress(&Progress {
                label: "Copying Notes 2.txt".to_owned(),
                fraction: 0.5,
                scope: ProgressScope::File,
                bytes: Some(ox_core::transfer::ByteProgress {
                    file_written: step * size / 100,
                    file_size: size,
                    batch_written: step * size / 100,
                    batch_size: Some(size),
                    file_started: false,
                }),
                items: Some(ox_core::transfer::ItemCount { done: 0, total: 1 }),
                name: Some("Notes 2.txt".to_owned()),
            });
        }
        crate::test_support::harness::capture_dialog(
            &progress,
            &format!("native-progress-details-{theme}.png"),
        );
        context.cancel.cancel();
        test.window.end_operation();
    }
}
