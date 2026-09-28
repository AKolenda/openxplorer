// SPDX-License-Identifier: AGPL-3.0-only
//! How the settings file is stored: private storage that never follows a
//! link, and the backup kept of a file that could not be read completely.
//!
//! Ports the settings cases of
//! `desktop/tests/test_terminal_security.py::PrivateStorageTests` and the
//! "never erase unreadable settings" rule of `desktop/core.py`.

use super::*;

/// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_lock_symlink_refused`
/// parity: SAFE-009
#[test]
fn settings_lock_symlink_refused() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    fs::create_dir(&directory).unwrap();
    let target = root.path().join("target");
    fs::write(&target, "unchanged").unwrap();
    symlink(&target, directory.join("settings.lock")).unwrap();
    let result = store.update_preferences(&update_from(&json!({"theme": "dark"})));
    assert!(matches!(
        result,
        Err(SettingsError::Storage(StorageError::Io { .. }))
    ));
    assert_eq!(fs::read_to_string(&target).unwrap(), "unchanged");
}

/// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_json_symlink_not_overwritten`
///
/// Python calls `save()`; the native app writes only through changes, so
/// the write goes through one. It must not move the link aside either.
/// parity: SAFE-009, SET-013
#[test]
fn settings_json_symlink_not_overwritten() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let target = root.path().join("target");
    fs::write(&target, "{}").unwrap();
    symlink(&target, directory.join("settings.json")).unwrap();
    let mut store = Settings::open(&directory);
    assert!(store.warning().is_some());
    let result = store.update_preferences(&PreferencesUpdate::default());
    assert!(matches!(
        result,
        Err(SettingsError::Storage(StorageError::Io { .. }))
    ));
    assert!(store.path().is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
    assert_eq!(backups(&directory), Vec::<String>::new());
}

/// Ported from `desktop/tests/test_terminal_security.py::PrivateStorageTests::test_settings_still_persist`
/// parity: SET-012, SAFE-009
#[test]
fn preferences_still_persist_in_private_storage() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    let mut store = Settings::open(&directory);
    save_preferences(&mut store, &json!({"textSize": 125}));
    assert_eq!(Settings::open(&directory).snapshot().preferences.text_size, 125);
}

/// parity: SAFE-009
#[test]
fn reading_makes_an_existing_directory_and_file_private() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(directory.join("settings.json"), "{}").unwrap();
    let store = Settings::open(&directory);
    assert!(store.warning().is_none());
    assert_eq!(permission_bits(&directory), 0o700);
    assert_eq!(permission_bits(&store.path()), 0o600);
}

#[test]
fn opening_a_missing_directory_does_not_create_it() {
    let root = temporary_folder();
    let directory = root.path().join("missing");
    let store = Settings::open(&directory);
    assert!(store.warning().is_none());
    assert_eq!(store.data(), &SettingsData::default());
    assert!(!directory.exists());
}

/// parity: SAFE-009, SET-013
#[test]
fn a_hard_linked_settings_file_is_refused_with_a_warning() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let original = root.path().join("original.json");
    fs::write(&original, r#"{"preferences": {"theme": "dark"}}"#).unwrap();
    fs::hard_link(&original, directory.join("settings.json")).unwrap();
    let store = Settings::open(&directory);
    let warning = store.warning().expect("the refused file is reported");
    let refused_path = store.path().display().to_string();
    assert!(
        warning.contains(&refused_path),
        "the warning names the refused file: {warning}"
    );
    assert_eq!(store.data().preferences.theme, Theme::System);
}

/// parity: SET-013
#[test]
fn a_change_keeps_an_unreadable_file_as_a_backup() {
    let root = temporary_folder();
    let damaged = r#"{"pins": [{"uri": "file:///tmp/Kept"}],}"#;
    fs::write(root.path().join("settings.json"), damaged).unwrap();
    let mut store = Settings::open(root.path());
    assert!(store.warning().is_some());

    save_preferences(&mut store, &json!({"theme": "dark"}));

    let kept = backups(root.path());
    assert_eq!(kept.len(), 1);
    let backup = root.path().join(&kept[0]);
    assert_eq!(fs::read_to_string(&backup).unwrap(), damaged);
    assert_eq!(permission_bits(&backup), 0o600);
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
    let root = temporary_folder();
    // Python reads pins, then shares: the pin is kept, the hidden entry lost.
    let partial =
        r#"{"pins": [{"uri": "file:///tmp/Kept"}], "shares": 5, "hiddenQuick": ["file:///tmp/Hidden"]}"#;
    fs::write(root.path().join("settings.json"), partial).unwrap();
    let mut store = Settings::open(root.path());
    assert!(store.warning().is_some());

    store
        .bookmark(BookmarkAction::Add, BookmarkKind::Pin, &pin("/tmp/New"))
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
    let root = temporary_folder();
    let mut store = Settings::open(root.path());
    save_preferences(&mut store, &json!({"theme": "dark"}));
    save_preferences(&mut store, &json!({"view": "grid"}));
    assert_eq!(backups(root.path()), Vec::<String>::new());
    assert!(store.warning().is_none());
}

/// Refused files stay refused: moving a hard-linked settings.json aside
/// would undo the refusal. The change would keep an unreadable file as a
/// backup, but the checks before the save refuse this one first.
/// parity: SAFE-009
#[test]
fn a_change_never_moves_a_refused_file() {
    let root = temporary_folder();
    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    let original = root.path().join("original.json");
    fs::write(&original, "{bad").unwrap();
    fs::hard_link(&original, directory.join("settings.json")).unwrap();
    let mut store = Settings::open(&directory);
    assert!(store.warning().is_some());

    let result = store.update_preferences(&update_from(&json!({"theme": "dark"})));

    assert!(matches!(
        result,
        Err(SettingsError::Storage(StorageError::Refused {
            reason: StorageRefusal::NotPrivateFile,
            ..
        }))
    ));
    assert_eq!(
        fs::read_to_string(directory.join("settings.json")).unwrap(),
        "{bad"
    );
    assert_eq!(fs::metadata(&original).unwrap().nlink(), 2);
    assert_eq!(backups(&directory), Vec::<String>::new());
}

/// Safety rule "never erase unreadable settings": only a file that was
/// read completely, or that a change has just written, is replaced
/// without a backup.
/// parity: SET-013
#[test]
fn only_a_completely_read_file_is_replaced_without_a_backup() {
    let unreadable = FileState::Unreadable("Could not fully read settings.".into());
    let backed_up = FileState::BackedUp("Kept as settings.json.unreadable-1-a.".into());

    assert_eq!(FileState::Sound.old_file(), OldFile::Discard);
    assert_eq!(backed_up.old_file(), OldFile::Discard);
    assert_eq!(unreadable.old_file(), OldFile::KeepAsBackup);
}
