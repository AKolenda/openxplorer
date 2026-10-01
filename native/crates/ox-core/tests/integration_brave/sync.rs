// SPDX-License-Identifier: AGPL-3.0-only
//! Pointing the selected profiles at the download folder. Ports the sync
//! cases of `BraveTests`.

use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use ox_core::integration::{BraveError, Confirmation};
use serde_json::json;

use super::{
    mode_of, original_preferences, read_json, Fixture, PREFERENCES_WITH_ZOOM_LEVEL, PROFILE_ID, ZOOM_LEVEL,
};

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_explicit_consent`
/// parity: INT-020
#[test]
fn syncing_needs_explicit_consent() {
    let fixture = Fixture::new();

    let refused = fixture.sync(Confirmation::NotConfirmed);

    assert!(
        matches!(refused, Err(BraveError::SyncNotConfirmed)),
        "{refused:?}"
    );
    assert_eq!(fixture.preferences_json(), original_preferences());
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_requires_quit`
/// parity: INT-020, SAFE-020
#[test]
fn syncing_waits_until_brave_is_closed() {
    let fixture = Fixture::new();
    fixture.brave_switch.set_running(true);

    let refused = fixture.sync(Confirmation::Confirmed);

    assert!(
        matches!(refused, Err(BraveError::BraveRunningBeforeSync)),
        "{refused:?}"
    );
    assert_eq!(fixture.preferences_json(), original_preferences());
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_changes_only_directories`
/// parity: INT-020
#[test]
fn syncing_changes_only_the_two_download_folders() {
    let fixture = Fixture::new();

    let outcome = fixture.sync(Confirmation::Confirmed).expect("sync");

    assert_eq!(outcome.updated, [PROFILE_ID]);
    let preferences = fixture.preferences_json();
    let destination = fixture.destination_text();
    assert_eq!(preferences["unrelated"], original_preferences()["unrelated"]);
    assert_eq!(preferences["download"]["prompt_for_download"], json!(true));
    assert_eq!(preferences["savefile"]["default_directory"], json!(destination));
    assert_eq!(preferences["download"]["default_directory"], json!(destination));
}

/// parity: INT-020
#[test]
fn syncing_keeps_unrelated_decimal_preferences_exactly() {
    let fixture = Fixture::new();
    fs::write(&fixture.preferences, PREFERENCES_WITH_ZOOM_LEVEL).expect("preferences");

    fixture.sync(Confirmation::Confirmed).expect("sync");

    let written = fs::read_to_string(&fixture.preferences).expect("read");
    assert!(written.contains(ZOOM_LEVEL), "{written}");
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_private_backups_and_prefs`
/// parity: INT-020, SAFE-020
#[test]
fn backups_records_and_preferences_are_private() {
    let fixture = Fixture::new();

    fixture.sync(Confirmation::Confirmed).expect("sync");

    let backups = fixture.backups_ending_in(".preferences.bak");
    let records = fixture.backups_ending_in(".json");
    assert_eq!(backups.len(), 1);
    assert_eq!(records.len(), 1);
    assert_eq!(read_json(&backups[0]), original_preferences());
    for path in [&backups[0], &fixture.preferences, &records[0]] {
        assert_eq!(mode_of(path), 0o600, "{}", path.display());
    }
    assert_eq!(mode_of(fixture.brave().backup_folder()), 0o700);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_unknown_profile`
/// parity: INT-020
#[test]
fn a_profile_outside_the_detected_ones_is_refused() {
    let fixture = Fixture::new();

    let refused = fixture.brave().sync(
        &["Brave-Browser:../outside".to_owned()],
        &fixture.destination_text(),
        Confirmation::Confirmed,
    );

    assert!(matches!(refused, Err(BraveError::UnknownProfile)), "{refused:?}");
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_missing_directory`
/// parity: INT-020
#[test]
fn a_missing_download_folder_is_refused() {
    let fixture = Fixture::new();
    let missing = fixture.home().join("missing");

    let refused = fixture.sync_to(missing.to_str().expect("UTF-8"));

    assert!(
        matches!(refused, Err(BraveError::UnusableDirectory)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_dedicated_directory_required`
/// parity: INT-020
#[test]
fn the_home_folder_is_not_a_download_folder() {
    let fixture = Fixture::new();

    let refused = fixture.sync_to(fixture.home().to_str().expect("UTF-8"));

    assert!(matches!(refused, Err(BraveError::NotDedicated)), "{refused:?}");
    assert_eq!(fixture.preferences_json(), original_preferences());
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_no_smb_uri_preference`
/// parity: INT-020
#[test]
fn an_smb_address_is_not_a_download_folder() {
    let fixture = Fixture::new();

    let refused = fixture.sync_to("smb://nas/downloads");

    assert!(
        matches!(refused, Err(BraveError::RelativeDirectory)),
        "{refused:?}"
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_symlink_pref_refused`
/// parity: INT-020, SAFE-020
#[test]
fn symlinked_preferences_are_never_written() {
    let fixture = Fixture::new();
    let target = fixture.root.path().join("actual");
    fs::rename(&fixture.preferences, &target).expect("move");
    symlink(&target, &fixture.preferences).expect("symlink");

    let refused = fixture.sync(Confirmation::Confirmed);

    assert!(refused.is_err());
    assert_eq!(read_json(&target), original_preferences());
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_late_running_race_no_pref_change`
/// parity: INT-020
#[test]
fn brave_starting_during_a_sync_leaves_the_profile_unchanged() {
    let fixture = Fixture::new();
    let answers = Mutex::new(VecDeque::from([false, true]));
    let brave = fixture.brave_with(move || answers.lock().expect("answers").pop_front().unwrap_or(true));

    let outcome = brave
        .sync(
            &[PROFILE_ID.to_owned()],
            &fixture.destination_text(),
            Confirmation::Confirmed,
        )
        .expect("checked");

    assert!(outcome.updated.is_empty());
    assert_eq!(outcome.failures.len(), 1);
    assert!(matches!(outcome.failures[0].error, BraveError::ChangedDuringSync));
    assert_eq!(fixture.preferences_json(), original_preferences());
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_preference_race_detected`
/// parity: INT-020
#[test]
fn preferences_changed_during_a_sync_are_not_overwritten() {
    let fixture = Fixture::new();
    let calls = AtomicUsize::new(0);
    let preferences = fixture.preferences.clone();
    let brave = fixture.brave_with(move || {
        if calls.fetch_add(1, Ordering::SeqCst) == 1 {
            fs::write(&preferences, r#"{"external":true}"#).expect("external write");
        }
        false
    });

    let outcome = brave
        .sync(
            &[PROFILE_ID.to_owned()],
            &fixture.destination_text(),
            Confirmation::Confirmed,
        )
        .expect("checked");

    assert!(outcome.updated.is_empty());
    assert_eq!(fixture.preferences_json(), json!({"external": true}));
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_invalid_preference_type_rejected`
/// parity: INT-020
#[test]
fn a_download_group_that_is_not_an_object_is_refused() {
    let fixture = Fixture::new();
    fs::write(&fixture.preferences, r#"{"download":[]}"#).expect("write");

    let refused = fixture.sync(Confirmation::Confirmed);

    assert!(refused.is_err());
    assert_eq!(
        fs::read_to_string(&fixture.preferences).expect("read"),
        r#"{"download":[]}"#
    );
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_multiple_profiles`
/// parity: INT-020
#[test]
fn several_profiles_are_updated_together() {
    let fixture = Fixture::new();
    let second = fixture
        .preferences
        .parent()
        .expect("profile")
        .with_file_name("Profile 1");
    fs::create_dir(&second).expect("second profile");
    fs::write(second.join("Preferences"), "{}").expect("second preferences");
    let profiles = [PROFILE_ID.to_owned(), "Brave-Browser:Profile 1".to_owned()];

    let outcome = fixture
        .brave()
        .sync(&profiles, &fixture.destination_text(), Confirmation::Confirmed)
        .expect("sync");

    assert_eq!(outcome.updated.len(), 2);
}

/// Ported from `v2.0.0:desktop/tests/test_v07.py::BraveTests::test_relative_destination_refused`
/// parity: INT-020
#[test]
fn a_relative_download_folder_is_refused() {
    let fixture = Fixture::new();

    let refused = fixture.sync_to(".");

    assert!(
        matches!(refused, Err(BraveError::RelativeDirectory)),
        "{refused:?}"
    );
}

/// parity: INT-020
#[test]
fn only_one_to_forty_distinct_profiles_can_be_selected() {
    let fixture = Fixture::new();
    let destination = fixture.destination_text();
    let too_many: Vec<String> = (0..41)
        .map(|number| format!("Brave-Browser:Profile {number}"))
        .collect();
    let selections = [
        Vec::new(),
        too_many,
        vec![PROFILE_ID.to_owned(), PROFILE_ID.to_owned()],
    ];

    for selection in selections {
        let refused = fixture
            .brave()
            .sync(&selection, &destination, Confirmation::Confirmed);

        assert!(
            matches!(refused, Err(BraveError::InvalidSelection)),
            "{}: {refused:?}",
            selection.len()
        );
    }
}

/// parity: INT-020
#[test]
fn a_volatile_system_folder_is_not_a_download_folder() {
    let fixture = Fixture::new();
    if !Path::new("/dev/shm").is_dir() {
        return;
    }

    let refused = fixture.sync_to("/dev/shm");

    assert!(matches!(refused, Err(BraveError::NotDedicated)), "{refused:?}");
}

/// parity: INT-020, INT-021
#[test]
fn inside_flatpak_brave_is_never_changed() {
    let fixture = Fixture::new();
    let brave = fixture.brave_in_flatpak();

    let synced = brave.sync(
        &[PROFILE_ID.to_owned()],
        &fixture.destination_text(),
        Confirmation::Confirmed,
    );
    let restored = brave.restore(PROFILE_ID, Confirmation::Confirmed);

    assert!(matches!(synced, Err(BraveError::Sandboxed)), "{synced:?}");
    assert!(matches!(restored, Err(BraveError::Sandboxed)), "{restored:?}");
    assert_eq!(fixture.preferences_json(), original_preferences());
}
