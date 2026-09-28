// SPDX-License-Identifier: AGPL-3.0-only
//! Putting the previous download folders back. Ports the restore cases of
//! `BraveTests`.

use std::fs;

use ox_core::integration::{BraveError, Confirmation, DownloadPreference};
use serde_json::json;

use super::{original_preferences, write_json, Fixture, PREFERENCES_WITH_ZOOM_LEVEL, PROFILE_ID, ZOOM_LEVEL};

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_restore_only_changed_keys`
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

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_restore_does_not_overwrite_later_preference`
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

    assert_eq!(restored, [DownloadPreference::SaveFile]);
    assert_eq!(
        fixture.preferences_json()["download"]["default_directory"],
        json!("/manual")
    );
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_restore_consent_required`
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
