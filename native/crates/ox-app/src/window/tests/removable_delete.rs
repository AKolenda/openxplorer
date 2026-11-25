// SPDX-License-Identifier: AGPL-3.0-only
//! Delete on a removable drive, with and without the setting "Delete files
//! on removable drives permanently (like Windows)" (OPS-018). By default
//! the items go to the drive's own Recycle Bin, as on any folder with a
//! Trash; with the setting on, Delete asks Windows 11's question and
//! deletes them for good. The test run's private Recycle Bin stands in for
//! the drive's, and the fixture's folder for the drive.

use gtk::subclass::prelude::*;
use ox_core::settings::{PreferencesUpdate, Settings};

use super::file_ops_support::{open_dialog, require_private_trash, select_names, wait_for_no_dialog};
use crate::test_support::harness::{wait_until, Fixture, TestWindow};
use crate::volumes::{MountControls, VolumeKind, VolumeRow, VolumeState};

/// A USB stick mounted at the fixture's folder, as the volume monitor
/// would list it.
fn stick_at(test: &TestWindow, fixture: &Fixture) {
    test.window.imp().volumes.replace(vec![VolumeRow {
        label: "STICK".to_owned(),
        kind: VolumeKind::Drive,
        state: VolumeState::Mounted {
            uri: fixture.uri(),
            controls: MountControls {
                can_unmount: true,
                can_eject: true,
                can_stop: true,
                can_open_in_disks: false,
                can_format: true,
            },
        },
    }]);
}

/// The names in the private Recycle Bin.
fn recycled_names() -> Vec<String> {
    let files = gtk::glib::user_data_dir().join("Trash").join("files");
    std::fs::read_dir(files)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Turns "Delete files on removable drives permanently" on in `test`'s
/// settings.
fn delete_removable_permanently(test: &TestWindow) {
    let update = PreferencesUpdate {
        delete_removable_permanently: Some(true),
        ..PreferencesUpdate::default()
    };
    Settings::open(test.settings_directory())
        .update_preferences(&update)
        .expect("the settings file takes the choice");
    test.context.reload_settings();
    wait_until("the window to read the choice", || {
        test.context
            .settings_data()
            .preferences
            .delete_removable_permanently
    });
}

/// By default, Delete on a USB stick moves the file to the Recycle Bin
/// without asking, as on any drive with a Trash (OPS-018).
///
/// parity: OPS-018
#[gtk::test]
fn delete_on_a_removable_drive_uses_the_recycle_bin_by_default() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    stick_at(&test, &fixture);
    select_names(&test, &["Résumé.txt"]);

    test.activate("trash", None);

    wait_until("the file to go to the Recycle Bin", || {
        !fixture.path("Résumé.txt").exists()
    });
    assert!(
        recycled_names().iter().any(|name| name.starts_with("Résumé")),
        "in the Recycle Bin: {:?}",
        recycled_names()
    );
}

/// With "Delete files on removable drives permanently (like Windows)" on,
/// Delete on a USB stick asks Windows 11's question, "Are you sure you want
/// to permanently delete this file?" (for several items, "these N items"),
/// with Yes and No; No keeps the items, and Yes deletes them for good,
/// with nothing in the Recycle Bin.
///
/// parity: OPS-018
#[gtk::test]
fn delete_on_a_removable_drive_can_delete_permanently_as_windows_does() {
    require_private_trash();
    let fixture = Fixture::standard();
    let test = TestWindow::open(&fixture.uri());
    stick_at(&test, &fixture);
    delete_removable_permanently(&test);
    let recycled_before = recycled_names();

    select_names(&test, &["Résumé.txt"]);
    test.activate("trash", None);
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Delete File");
    assert_eq!(
        dialog.message_text(),
        "Are you sure you want to permanently delete this file?"
    );
    assert_eq!(dialog.button_labels(), ["Yes", "No"]);
    dialog.press("No");
    wait_for_no_dialog(&test);
    assert!(fixture.path("Résumé.txt").is_file(), "No keeps the file");

    test.activate("trash", None);
    open_dialog(&test).press("Yes");
    wait_until("the file to be deleted", || !fixture.path("Résumé.txt").exists());

    select_names(&test, &["Notes 2.txt", "Notes 10.txt"]);
    test.activate("trash", None);
    let dialog = open_dialog(&test);
    assert_eq!(dialog.title_text(), "Delete Multiple Items");
    assert_eq!(
        dialog.message_text(),
        "Are you sure you want to permanently delete these 2 items?"
    );
    dialog.press("Yes");
    wait_until("the files to be deleted", || {
        !fixture.path("Notes 2.txt").exists() && !fixture.path("Notes 10.txt").exists()
    });
    assert_eq!(
        recycled_names(),
        recycled_before,
        "nothing went to the Recycle Bin"
    );
}
