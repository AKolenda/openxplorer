// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of [`Settings`] against real files in temporary directories.
//!
//! Ports the settings cases of `v2.0.0:desktop/tests/test_core.py`,
//! `test_pins.py`, `test_v05.py` and `test_terminal_security.py`; the
//! preference cases are in `tests/preferences.rs`, and private storage and
//! the backups of unreadable files in `tests/storage.rs`.

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

use serde_json::{json, Value};

mod preferences;
mod storage;

use super::test_support::{names_starting_with, pin, shown_quick_order};
use super::*;
use crate::location::file_uri;
use crate::test_support::{permission_bits, temporary_folder};

/// The update a window sends for `values`, read like any untrusted request.
fn update_from(values: &Value) -> PreferencesUpdate {
    PreferencesUpdate::from_json(values).expect("the values are an object")
}

/// Applies `values` with [`Settings::update_preferences`], which must save.
fn save_preferences(store: &mut Settings, values: &Value) -> Preferences {
    store
        .update_preferences(&update_from(values))
        .expect("the preferences are saved")
}

/// The names of the kept unreadable settings files in `directory`.
fn backups(directory: &Path) -> Vec<String> {
    names_starting_with(directory, "settings.json.unreadable-")
}

/// Ported from `v2.0.0:desktop/tests/test_core.py::CoreTests::test_settings_private_and_atomic`
/// parity: SET-012, SET-016, SAFE-018, NET-017
#[test]
fn settings_private_and_atomic() {
    let root = temporary_folder();
    let directory = root.path().join("settings");
    let mut store = Settings::open(&directory);
    let projects = BookmarkRequest::new(r"\\NAS\Projects", "Projects (Z:)");
    store
        .bookmark(BookmarkAction::Add, BookmarkKind::Share, &projects)
        .unwrap();
    save_preferences(
        &mut store,
        &json!({"theme": "dark", "password": "not-stored", "view": "bogus"}),
    );
    let reread = Settings::open(&directory);
    assert_eq!(reread.data().shares[0].uri, "smb://nas/Projects");
    assert_eq!(reread.data().preferences.theme, Theme::Dark);
    assert_eq!(reread.data().preferences.view, View::Details);
    assert!(!fs::read_to_string(store.path()).unwrap().contains("not-stored"));
    assert_eq!(permission_bits(&store.path()), 0o600);
    assert_eq!(permission_bits(&directory), 0o700);
    let temporary_files = names_starting_with(&directory, ".settings-");
    assert_eq!(temporary_files.len(), 0);
}

/// Ported from `v2.0.0:desktop/tests/test_core.py::CoreTests::test_credential_bookmark_rejected`
/// parity: SAFE-010
#[test]
fn a_bookmark_with_credentials_is_refused_and_nothing_is_saved() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    let with_password = BookmarkRequest::new("smb://u:secret@nas/share", "");
    let result = store.bookmark(BookmarkAction::Add, BookmarkKind::Share, &with_password);
    assert!(matches!(result, Err(SettingsError::Location(_))));
    assert!(!store.path().exists());
}

/// Ported from `v2.0.0:desktop/tests/test_core.py::CoreTests::test_corrupt_settings`
/// parity: SET-013
#[test]
fn corrupt_settings_fall_back_to_defaults_with_a_warning() {
    let root = temporary_folder();
    fs::write(root.path().join("settings.json"), "{bad").unwrap();
    let store = Settings::open(root.path());
    assert!(store.warning().is_some());
    assert!(store.data().shares.is_empty());
}

/// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_add_pin_preserves_file_tree`
/// parity: SIDE-007
#[test]
fn add_pin_preserves_file_tree() {
    let root = temporary_folder();
    let actual = root.path().join("actual");
    fs::create_dir(&actual).unwrap();
    fs::write(actual.join("data.txt"), "unchanged").unwrap();
    let mut store = Settings::open(&root.path().join("config"));
    let items = [BookmarkRequest::new(file_uri(&actual), "Work")];
    store.pin_many(&items, None, Some(&shown_quick_order())).unwrap();
    assert_eq!(fs::read_to_string(actual.join("data.txt")).unwrap(), "unchanged");
    assert!(actual.is_dir());
}

/// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_bulk_pin_and_reload`
/// parity: SIDE-007
#[test]
fn a_pinned_batch_is_read_back_after_the_shown_order() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    let pins = [
        Bookmark {
            uri: "smb://nas/work/Plans".into(),
            label: "Plans".into(),
        },
        Bookmark {
            uri: "smb://nas/work/Invoices".into(),
            label: "Invoices".into(),
        },
    ];
    let items: Vec<BookmarkRequest> = pins
        .iter()
        .map(|pin| BookmarkRequest::new(&pin.uri, &pin.label))
        .collect();
    store.pin_many(&items, None, Some(&shown_quick_order())).unwrap();
    let reread = Settings::open(&directory);
    assert_eq!(reread.data().pins, pins);
    let mut expected = shown_quick_order();
    expected.extend(pins.iter().map(|pin| pin.uri.clone()));
    assert_eq!(reread.data().quick_order, expected);
}

/// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_invalid_batch_has_no_partial_writes`
/// parity: SIDE-007
#[test]
fn invalid_batch_writes_nothing() {
    let root = temporary_folder();
    let mut store = Settings::open(&root.path().join("config"));
    let before = store.snapshot();
    let items = [
        BookmarkRequest::new("smb://nas/work", ""),
        BookmarkRequest::new("smb://u:secret@nas/work", ""),
    ];
    assert!(store.pin_many(&items, None, None).is_err());
    assert_eq!(store.snapshot(), before);
    assert!(!store.path().exists());
}

/// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_save_error_rolls_back_in_memory`
/// parity: SIDE-007
#[test]
fn save_error_rolls_back_in_memory() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    save_preferences(&mut store, &json!({"theme": "dark"}));
    let before = store.snapshot();
    // A directory in place of settings.json makes the save fail.
    fs::remove_file(store.path()).unwrap();
    fs::create_dir(store.path()).unwrap();
    let result = store.pin_many(&[BookmarkRequest::new("smb://nas/work", "")], None, None);
    assert!(result.is_err());
    assert_eq!(store.data(), &before);
}

/// Ported from `v2.0.0:desktop/tests/test_pins.py::PinTests::test_system_theme_and_legacy_migration`
/// parity: SET-017, LOOK-003
#[test]
fn a_version_1_file_is_read_and_can_switch_to_the_system_theme() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let legacy = json!({
        "version": 1,
        "pins": [{"uri": "smb://nas/work", "label": "work"}],
        "preferences": {"theme": "dark"},
        "shares": [{"uri": "smb://nas/work", "label": "Z:"}]
    });
    fs::write(directory.join("settings.json"), legacy.to_string()).unwrap();
    let mut store = Settings::open(&directory);
    assert_eq!(store.data().preferences.theme, Theme::Dark);
    assert_eq!(store.data().shares.len(), 1);
    save_preferences(&mut store, &json!({"theme": "system"}));
    assert_eq!(Settings::open(&directory).data().preferences.theme, Theme::System);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_two_windows_preserve_each_others_preferences`
/// parity: SET-014
#[test]
fn two_windows_preserve_each_others_preferences() {
    let root = temporary_folder();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    save_preferences(&mut first, &json!({"theme": "dark"}));
    save_preferences(&mut second, &json!({"details": false}));
    let merged = first.snapshot().preferences;
    assert_eq!(merged.theme, Theme::Dark);
    assert!(!merged.show_details_pane);
}

/// Ported from `v2.0.0:desktop/tests/test_v05.py::SettingsWindowsTests::test_two_windows_pins_not_lost`
/// parity: SIDE-022
#[test]
fn two_windows_pins_not_lost() {
    let root = temporary_folder();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    let folder_a = pin("file:///home/demo/A");
    let folder_b = pin("file:///home/demo/B");
    first
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, &folder_a)
        .unwrap();
    second
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, &folder_b)
        .unwrap();
    assert_eq!(first.snapshot().pins.len(), 2);
}

/// parity: SAFE-018
#[test]
fn unknown_keys_are_dropped_and_whitelisted_ones_survive_a_rust_write() {
    let root = temporary_folder();
    let python_written = json!({
        "version": 2,
        "pins": [], "shares": [], "hiddenQuick": [], "quickOrder": [], "recent": [],
        "preferences": {
            "theme": "light", "view": "grid", "details": false, "showHidden": true,
            "autoIndex": false, "contextMenu": "win11", "networkInterval": 300, "textSize": 110,
            "sidebarWidth": 222, "columnWidths": {"name": 300, "parentUri": 400},
            "futureSetting": 1
        },
        "futureSection": {"x": 1}
    });
    fs::write(root.path().join("settings.json"), python_written.to_string()).unwrap();
    let mut store = Settings::open(root.path());

    store
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, &pin("/tmp/Rust"))
        .unwrap();

    let written: Value = serde_json::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
    let mut expected_preferences = python_written["preferences"].clone();
    expected_preferences
        .as_object_mut()
        .unwrap()
        .remove("futureSetting");
    assert_eq!(written["preferences"], expected_preferences);
    assert!(written.get("futureSection").is_none());
    assert_eq!(
        written["pins"],
        json!([{"uri": "file:///tmp/Rust", "label": "Rust"}])
    );
}

#[test]
fn a_deleted_file_keeps_the_data_last_read() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"theme": "dark"}));
    fs::remove_file(store.path()).unwrap();
    assert_eq!(store.snapshot().preferences.theme, Theme::Dark);
    save_preferences(&mut store, &json!({"view": "grid"}));
    let reread = Settings::open(root.path()).snapshot().preferences;
    assert_eq!((reread.theme, reread.view), (Theme::Dark, View::Grid));
}

/// A change holds `settings.lock` from the re-read until the file is
/// written, so a Python `settings_mutation` cannot interleave. `File::lock`
/// and Python's `fcntl.flock` conflict (see `tests/settings_interop.rs`),
/// so `WouldBlock` here means Python blocks too.
/// parity: SET-014, SIDE-022
#[test]
fn a_change_holds_the_settings_lock_until_it_is_written() {
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    let lock_path = root.path().join(SettingsLock::FILE_NAME);
    store
        .mutate(|data| {
            let other = fs::File::open(&lock_path).expect("the change created the lock file");
            assert!(matches!(other.try_lock(), Err(fs::TryLockError::WouldBlock)));
            data.preferences.theme = Theme::Dark;
            Ok(())
        })
        .unwrap();
    let other = fs::File::open(&lock_path).unwrap();
    other.try_lock().expect("the lock is released after the change");
}
