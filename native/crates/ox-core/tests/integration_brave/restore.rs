// SPDX-License-Identifier: AGPL-3.0-only
//! Putting the previous download folders, and the prompt a sync set,
//! back. Ports the restore cases of `BraveTests`.

use std::fs;

use ox_core::integration::{BraveError, Confirmation, DownloadPreference};
use serde_json::json;

use super::{
    original_preferences, read_json, write_json, Fixture, PREFERENCES_WITH_ZOOM_LEVEL, PROFILE_ID, ZOOM_LEVEL,
};

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_restore_only_changed_keys`
/// parity: INT-021
#[test]
fn restoring_puts_back_only_the_download_folders() {
    let fixture = Fixture::new();
    fixture.sync(Confirmation::Confirmed).expect("sync");
    let mut preferences = fixture.preferences_json();
    preferences["unrelated"] = json!({"new": true});
    write_json(&fixture.preferences, &preferences);

    fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    let restored = fixture.preferences_json();
    assert_eq!(restored["download"], original_preferences()["download"]);
    assert_eq!(restored["savefile"], original_preferences()["savefile"]);
    assert_eq!(restored["unrelated"], json!({"new": true}));
}

/// parity: INT-021
#[test]
fn restoring_keeps_unrelated_decimal_preferences_exactly() {
    let fixture = Fixture::new();
    fs::write(&fixture.preferences, PREFERENCES_WITH_ZOOM_LEVEL).expect("preferences");
    fixture.sync(Confirmation::Confirmed).expect("sync");

    fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    let written = fs::read_to_string(&fixture.preferences).expect("read");
    assert!(written.contains(ZOOM_LEVEL), "{written}");
    assert_eq!(
        fixture.preferences_json()["download"]["default_directory"],
        json!("/old/downloads")
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_restore_does_not_overwrite_later_preference`
/// parity: INT-021
#[test]
fn restoring_keeps_a_folder_the_user_changed_later() {
    let fixture = Fixture::new();
    fixture.sync(Confirmation::Confirmed).expect("sync");
    let mut preferences = fixture.preferences_json();
    preferences["download"]["default_directory"] = json!("/manual");
    write_json(&fixture.preferences, &preferences);

    let restored = fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    assert_eq!(restored.folders, [DownloadPreference::SaveFile]);
    assert!(!restored.download_prompt);
    assert_eq!(
        fixture.preferences_json()["download"]["default_directory"],
        json!("/manual")
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_restore_consent_required`
/// parity: INT-021
#[test]
fn restoring_needs_explicit_consent() {
    let fixture = Fixture::new();
    fixture.sync(Confirmation::Confirmed).expect("sync");

    let refused = fixture.brave().restore(PROFILE_ID, Confirmation::NotConfirmed);

    assert!(
        matches!(refused, Err(BraveError::RestoreNotConfirmed)),
        "{refused:?}"
    );
}

/// parity: INT-021
#[test]
fn restoring_without_a_record_changes_nothing() {
    let fixture = Fixture::new();

    let refused = fixture.brave().restore(PROFILE_ID, Confirmation::Confirmed);

    assert!(matches!(refused, Err(BraveError::NoRecord)), "{refused:?}");
    assert_eq!(fixture.preferences_json(), original_preferences());
}

/// parity: INT-021
#[test]
fn restoring_removes_the_undo_record() {
    let fixture = Fixture::new();
    fixture.sync(Confirmation::Confirmed).expect("sync");

    fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    assert!(fixture.backups_ending_in(".json").is_empty());
    let again = fixture.brave().restore(PROFILE_ID, Confirmation::Confirmed);
    assert!(matches!(again, Err(BraveError::NoRecord)), "{again:?}");
}

/// A restore puts back the prompt the sync set: unset or `false`, as it
/// was.
///
/// parity: INT-034
#[test]
fn restoring_puts_back_the_prompt_the_sync_set() {
    for previous in [None, Some(false)] {
        let fixture = Fixture::new();
        fixture.set_download_prompt(previous);
        let before = fixture.preferences_json();
        fixture.sync_asking().expect("sync");

        let restored = fixture
            .brave()
            .restore(PROFILE_ID, Confirmation::Confirmed)
            .expect("restore");

        assert!(restored.download_prompt, "{previous:?}");
        assert_eq!(fixture.preferences_json(), before, "{previous:?}");
    }
}

/// A prompt the user turned off since the sync stays off; the folders
/// are still put back.
///
/// parity: INT-034
#[test]
fn restoring_keeps_a_prompt_the_user_changed_later() {
    let fixture = Fixture::new();
    fixture.set_download_prompt(None);
    fixture.sync_asking().expect("sync");
    fixture.set_download_prompt(Some(false));

    let restored = fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    assert!(!restored.download_prompt);
    assert_eq!(restored.folders, DownloadPreference::ALL);
    let preferences = fixture.preferences_json();
    assert_eq!(preferences["download"]["prompt_for_download"], json!(false));
    assert_eq!(
        preferences["download"]["default_directory"],
        json!("/old/downloads")
    );
}

/// The prompt alone is enough to restore when the user changed both
/// folders since.
///
/// parity: INT-034
#[test]
fn restoring_puts_back_the_prompt_when_both_folders_changed() {
    let fixture = Fixture::new();
    fixture.set_download_prompt(Some(false));
    fixture.sync_asking().expect("sync");
    let mut preferences = fixture.preferences_json();
    preferences["download"]["default_directory"] = json!("/manual");
    preferences["savefile"]["default_directory"] = json!("/manual");
    write_json(&fixture.preferences, &preferences);

    let restored = fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    assert!(restored.folders.is_empty());
    assert!(restored.download_prompt);
    let preferences = fixture.preferences_json();
    assert_eq!(preferences["download"]["prompt_for_download"], json!(false));
    assert_eq!(preferences["download"]["default_directory"], json!("/manual"));
}

/// A record in the format of the Python app and earlier versions, without
/// the prompt, still restores the folders exactly and leaves the prompt
/// alone.
///
/// parity: INT-021, INT-034
#[test]
fn a_record_without_the_prompt_restores_the_folders_only() {
    let fixture = Fixture::new();
    fixture.set_download_prompt(None);
    fixture.sync_asking().expect("sync");
    let record_path = fixture.undo_record();
    let mut record = read_json(&record_path);
    record
        .as_object_mut()
        .expect("the record is an object")
        .remove("prompt_for_download");
    let keys: Vec<&String> = record.as_object().expect("an object").keys().collect();
    assert_eq!(keys, ["applied", "backup", "previous", "profile"]);
    write_json(&record_path, &record);

    let restored = fixture
        .brave()
        .restore(PROFILE_ID, Confirmation::Confirmed)
        .expect("restore");

    assert_eq!(restored.folders, DownloadPreference::ALL);
    assert!(!restored.download_prompt);
    let preferences = fixture.preferences_json();
    assert_eq!(
        preferences["download"]["default_directory"],
        json!("/old/downloads")
    );
    assert_eq!(preferences["savefile"]["default_directory"], json!("/old/save"));
    assert_eq!(preferences["download"]["prompt_for_download"], json!(true));
}
