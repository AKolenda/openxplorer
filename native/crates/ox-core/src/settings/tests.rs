// SPDX-License-Identifier: AGPL-3.0-only
//! Tests of [`Settings`] against real files in temporary directories.

use std::fs;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

use serde_json::json;
use tempfile::TempDir;

mod preferences;

use super::*;
use crate::location::file_uri;

fn temp() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("the path exists").mode() & 0o777
}

fn prefs(values: serde_json::Value) -> PreferencesUpdate {
    PreferencesUpdate::from_json(&values).expect("an object")
}

/// The names of the kept unreadable settings files in `directory`.
fn backups(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("the settings directory exists")
        .map(|entry| entry.expect("a directory entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| name.starts_with("settings.json.unreadable-"))
        .collect();
    names.sort();
    names
}

fn quick() -> Vec<String> {
    [
        "file:///home/test/Desktop",
        "file:///home/test/Downloads",
        "file:///home/test/Documents",
    ]
    .map(String::from)
    .to_vec()
}

/// Ported from `desktop/tests/test_core.py::CoreTests::test_settings_private_and_atomic`
/// parity: SET-012, SET-016, SAFE-018, NET-017
#[test]
fn settings_private_and_atomic() {
    let root = temp();
    let directory = root.path().join("settings");
    let mut store = Settings::open(&directory);
    store
        .bookmark(
            BookmarkAction::Add,
            BookmarkKind::Share,
            r"\\NAS\Projects",
            "Projects (Z:)",
        )
        .unwrap();
    store
        .update_preferences(&prefs(
            json!({"theme": "dark", "password": "not-stored", "view": "bogus"}),
        ))
        .unwrap();
    let reread = Settings::open(&directory);
    assert_eq!(reread.data().shares[0].uri, "smb://nas/Projects");
    assert_eq!(reread.data().preferences.theme, Theme::Dark);
    assert_eq!(reread.data().preferences.view, View::Details);
    assert!(!fs::read_to_string(store.path()).unwrap().contains("not-stored"));
    assert_eq!(mode(&store.path()), 0o600);
    assert_eq!(mode(&directory), 0o700);
    let leftovers = fs::read_dir(&directory)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".settings-")
        })
        .count();
    assert_eq!(leftovers, 0);
}

/// Ported from `desktop/tests/test_core.py::CoreTests::test_credential_bookmark_rejected`
/// parity: SAFE-010
#[test]
fn credential_bookmark_rejected() {
    let root = temp();
    let mut store = Settings::open(root.path());
    let result = store.bookmark(
        BookmarkAction::Add,
        BookmarkKind::Share,
        "smb://u:secret@nas/share",
        "",
    );
    assert!(matches!(result, Err(SettingsError::Invalid(_))));
    assert!(!store.path().exists());
}

/// Ported from `desktop/tests/test_core.py::CoreTests::test_corrupt_settings`
/// parity: SET-013
#[test]
fn corrupt_settings() {
    let root = temp();
    fs::write(root.path().join("settings.json"), "{bad").unwrap();
    let store = Settings::open(root.path());
    assert!(store.warning().is_some());
    assert!(store.data().shares.is_empty());
}

/// Ported from `desktop/tests/test_pins.py::PinTests::test_add_pin_preserves_file_tree`
/// parity: SIDE-007
#[test]
fn add_pin_preserves_file_tree() {
    let root = temp();
    let actual = root.path().join("actual");
    fs::create_dir(&actual).unwrap();
    fs::write(actual.join("data.txt"), "unchanged").unwrap();
    let mut store = Settings::open(&root.path().join("config"));
    let items = [PinRequest::new(file_uri(&actual), "Work")];
    store.pin_many(&items, None, Some(&quick())).unwrap();
    assert_eq!(fs::read_to_string(actual.join("data.txt")).unwrap(), "unchanged");
    assert!(actual.is_dir());
}

/// Ported from `desktop/tests/test_pins.py::PinTests::test_bulk_pin_and_reload`
/// parity: SIDE-007
#[test]
fn bulk_pin_and_reload() {
    let root = temp();
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
    let items: Vec<PinRequest> = pins
        .iter()
        .map(|pin| PinRequest::new(&pin.uri, &pin.label))
        .collect();
    store.pin_many(&items, None, Some(&quick())).unwrap();
    let reread = Settings::open(&directory);
    assert_eq!(reread.data().pins, pins);
    let mut expected = quick();
    expected.extend(pins.iter().map(|pin| pin.uri.clone()));
    assert_eq!(reread.data().quick_order, expected);
}

/// Ported from `desktop/tests/test_pins.py::PinTests::test_invalid_batch_has_no_partial_writes`
/// parity: SIDE-007
#[test]
fn invalid_batch_writes_nothing() {
    let root = temp();
    let mut store = Settings::open(&root.path().join("config"));
    let before = store.snapshot();
    let items = [
        PinRequest::new("smb://nas/work", ""),
        PinRequest::new("smb://u:secret@nas/work", ""),
    ];
    assert!(store.pin_many(&items, None, None).is_err());
    assert_eq!(store.snapshot(), before);
    assert!(!store.path().exists());
}

/// Ported from `desktop/tests/test_pins.py::PinTests::test_save_error_rolls_back_in_memory`
/// parity: SIDE-007
#[test]
fn save_error_rolls_back_in_memory() {
    let root = temp();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    store
        .update_preferences(&prefs(json!({"theme": "dark"})))
        .unwrap();
    let before = store.snapshot();
    // A directory in place of settings.json makes the save fail.
    fs::remove_file(store.path()).unwrap();
    fs::create_dir(store.path()).unwrap();
    let result = store.pin_many(&[PinRequest::new("smb://nas/work", "")], None, None);
    assert!(result.is_err());
    assert_eq!(store.data(), &before);
}

/// Ported from `desktop/tests/test_pins.py::PinTests::test_system_theme_and_legacy_migration`
/// parity: SET-017, LOOK-003
#[test]
fn system_theme_and_legacy_migration() {
    let root = temp();
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
    store
        .update_preferences(&prefs(json!({"theme": "system"})))
        .unwrap();
    assert_eq!(Settings::open(&directory).data().preferences.theme, Theme::System);
}

/// Ported from `desktop/tests/test_v05.py::SettingsWindowsTests::test_two_windows_preserve_each_others_preferences`
/// parity: SET-014
#[test]
fn two_windows_preserve_each_others_preferences() {
    let root = temp();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    first
        .update_preferences(&prefs(json!({"theme": "dark"})))
        .unwrap();
    second
        .update_preferences(&prefs(json!({"details": false})))
        .unwrap();
    let merged = first.snapshot().preferences;
    assert_eq!(merged.theme, Theme::Dark);
    assert!(!merged.details);
}

/// Ported from `desktop/tests/test_v05.py::SettingsWindowsTests::test_two_windows_pins_not_lost`
/// parity: SIDE-022
#[test]
fn two_windows_pins_not_lost() {
    let root = temp();
    let mut first = Settings::open(root.path());
    let mut second = Settings::open(root.path());
    first
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, "file:///home/demo/A", "")
        .unwrap();
    second
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, "file:///home/demo/B", "")
        .unwrap();
    assert_eq!(first.snapshot().pins.len(), 2);
}

/// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_lock_symlink_refused`
/// parity: SAFE-009
#[test]
fn settings_lock_symlink_refused() {
    let root = temp();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    fs::create_dir(&directory).unwrap();
    let target = root.path().join("target");
    fs::write(&target, "unchanged").unwrap();
    symlink(&target, directory.join("settings.lock")).unwrap();
    let result = store.update_preferences(&prefs(json!({"theme": "dark"})));
    assert!(matches!(result, Err(SettingsError::Io { .. })));
    assert_eq!(fs::read_to_string(&target).unwrap(), "unchanged");
}

/// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_json_symlink_not_overwritten`
///
/// Python calls `save()`; the native app writes only through changes, so
/// the write goes through one. It must not move the link aside either.
/// parity: SAFE-009, SET-013
#[test]
fn settings_json_symlink_not_overwritten() {
    let root = temp();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let target = root.path().join("target");
    fs::write(&target, "{}").unwrap();
    symlink(&target, directory.join("settings.json")).unwrap();
    let mut store = Settings::open(&directory);
    assert!(store.warning().is_some());
    let result = store.update_preferences(&PreferencesUpdate::default());
    assert!(matches!(result, Err(SettingsError::Io { .. })));
    assert!(store.path().is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
    assert_eq!(backups(&directory), Vec::<String>::new());
}

/// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_still_persist`
/// parity: SET-012, SAFE-009
#[test]
fn settings_still_persist() {
    let root = temp();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    store
        .update_preferences(&prefs(json!({"textSize": 125})))
        .unwrap();
    assert_eq!(Settings::open(&directory).snapshot().preferences.text_size, 125);
}

/// parity: SAFE-009
#[test]
fn reading_makes_an_existing_directory_and_file_private() {
    let root = temp();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(directory.join("settings.json"), "{}").unwrap();
    let store = Settings::open(&directory);
    assert!(store.warning().is_none());
    assert_eq!(mode(&directory), 0o700);
    assert_eq!(mode(&store.path()), 0o600);
}

#[test]
fn opening_a_missing_directory_does_not_create_it() {
    let root = temp();
    let directory = root.path().join("missing");
    let store = Settings::open(&directory);
    assert!(store.warning().is_none());
    assert_eq!(store.data(), &SettingsData::default());
    assert!(!directory.exists());
}

/// parity: SAFE-009, SET-013
#[test]
fn a_hard_linked_settings_file_is_refused_with_a_warning() {
    let root = temp();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let original = root.path().join("original.json");
    fs::write(&original, r#"{"preferences": {"theme": "dark"}}"#).unwrap();
    fs::hard_link(&original, directory.join("settings.json")).unwrap();
    let store = Settings::open(&directory);
    assert!(store.warning().is_some());
    assert_eq!(store.data().preferences.theme, Theme::System);
}

/// parity: SAFE-018
#[test]
fn unknown_keys_are_dropped_and_whitelisted_ones_survive_a_rust_write() {
    let root = temp();
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
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, "/tmp/Rust", "")
        .unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
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
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"theme": "dark"})))
        .unwrap();
    fs::remove_file(store.path()).unwrap();
    assert_eq!(store.snapshot().preferences.theme, Theme::Dark);
    store.update_preferences(&prefs(json!({"view": "grid"}))).unwrap();
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
    let root = temp();
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

/// parity: SET-013
#[test]
fn a_change_keeps_an_unreadable_file_as_a_backup() {
    let root = temp();
    let damaged = r#"{"pins": [{"uri": "file:///tmp/Kept"}],}"#;
    fs::write(root.path().join("settings.json"), damaged).unwrap();
    let mut store = Settings::open(root.path());
    assert!(store.warning().is_some());

    store
        .update_preferences(&prefs(json!({"theme": "dark"})))
        .unwrap();

    let kept = backups(root.path());
    assert_eq!(kept.len(), 1);
    let backup = root.path().join(&kept[0]);
    assert_eq!(fs::read_to_string(&backup).unwrap(), damaged);
    assert_eq!(mode(&backup), 0o600);
    let warning = store.warning().expect("the change reports the backup");
    assert!(warning.contains(&kept[0]), "{warning}");
    let reread = Settings::open(root.path());
    assert!(reread.warning().is_none());
    assert_eq!(reread.data().preferences.theme, Theme::Dark);
}

/// A section of the wrong type stops reading after the sections before it,
/// so writing the result alone would silently drop the rest.
/// parity: SET-013
#[test]
fn a_change_keeps_a_partly_read_file_as_a_backup() {
    let root = temp();
    // Python reads pins, then shares: the pin is kept, the hidden entry lost.
    let partial =
        r#"{"pins": [{"uri": "file:///tmp/Kept"}], "shares": 5, "hiddenQuick": ["file:///tmp/Hidden"]}"#;
    fs::write(root.path().join("settings.json"), partial).unwrap();
    let mut store = Settings::open(root.path());
    assert!(store.warning().is_some());

    store
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, "/tmp/New", "")
        .unwrap();

    let kept = backups(root.path());
    assert_eq!(kept.len(), 1);
    assert_eq!(fs::read_to_string(root.path().join(&kept[0])).unwrap(), partial);
    let reread = Settings::open(root.path());
    assert_eq!(reread.data().pins.len(), 2);
    assert!(reread.data().hidden_quick.is_empty());
}

#[test]
fn a_readable_file_is_replaced_without_a_backup() {
    let root = temp();
    let mut store = Settings::open(root.path());
    store
        .update_preferences(&prefs(json!({"theme": "dark"})))
        .unwrap();
    store.update_preferences(&prefs(json!({"view": "grid"}))).unwrap();
    assert_eq!(backups(root.path()), Vec::<String>::new());
    assert!(store.warning().is_none());
}

/// Refused files stay refused: moving a hard-linked settings.json aside
/// would undo the refusal.
/// parity: SAFE-009
#[test]
fn a_change_never_moves_a_refused_file() {
    let root = temp();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let original = root.path().join("original.json");
    fs::write(&original, "{bad").unwrap();
    fs::hard_link(&original, directory.join("settings.json")).unwrap();
    let mut store = Settings::open(&directory);
    assert!(store.warning().is_some());

    let result = store.update_preferences(&prefs(json!({"theme": "dark"})));

    assert!(matches!(result, Err(SettingsError::Invalid(_))));
    assert_eq!(
        fs::read_to_string(directory.join("settings.json")).unwrap(),
        "{bad"
    );
    assert_eq!(fs::metadata(&original).unwrap().nlink(), 2);
    assert_eq!(backups(&directory), Vec::<String>::new());
}
