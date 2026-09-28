// SPDX-License-Identifier: AGPL-3.0-only
//! What the Brave dialog shows: native profiles, sandboxed installs and
//! whether Brave runs. Ports the detection cases of `BraveTests`.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use ox_core::integration::{
    BraveActivity, BraveError, BraveIntegration, BraveReach, ProcessTable, SandboxedBrave,
    MANUAL_SETTINGS_URL,
};

use super::{Fixture, PROFILE_ID};

/// A `/proc`-like folder with one process of this user running `cmdline`.
fn process_table_with(root: &Path, cmdline: &[u8]) -> PathBuf {
    let proc_root = root.join("proc");
    fs::create_dir_all(proc_root.join("123")).expect("process folder");
    fs::write(proc_root.join("123/cmdline"), cmdline).expect("cmdline");
    proc_root
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_detect_profiles`
/// parity: INT-019
#[test]
fn native_profiles_are_detected_with_their_names() {
    let fixture = Fixture::new();

    let status = fixture.brave().status();

    assert_eq!(status.profiles[0].name, "Test person");
    assert_eq!(status.profiles[0].id, PROFILE_ID);
    assert_eq!(status.profiles[0].download_path, "/old/downloads");
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_nonregular_pref_file_rejected`
/// parity: INT-019
#[test]
fn a_profile_whose_preferences_are_not_a_file_is_not_offered() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.preferences).expect("remove");
    fs::create_dir(&fixture.preferences).expect("folder in its place");

    assert!(fixture.brave().profiles().is_empty());
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_profile_symlink_ignored`
/// parity: INT-019
#[test]
fn a_symlinked_profile_folder_is_ignored() {
    let fixture = Fixture::new();
    let profile = fixture.preferences.parent().expect("profile folder");
    symlink(profile, profile.with_file_name("Profile 1")).expect("symlink");

    assert_eq!(fixture.brave().profiles().len(), 1);
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_sandbox_detected_manual_only`
/// parity: INT-019
#[test]
fn flatpak_and_snap_installs_are_reported_for_manual_setup() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.home().join(".var/app/com.brave.Browser")).expect("Flatpak data");
    fs::create_dir_all(fixture.home().join("snap/brave")).expect("Snap data");

    let status = fixture.brave().status();

    assert_eq!(
        status.sandboxed_installs,
        [SandboxedBrave::Flatpak, SandboxedBrave::Snap]
    );
    assert_eq!(status.profiles.len(), 1);
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_proc_brave_detected`
/// parity: INT-019
#[test]
fn a_brave_process_is_detected() {
    let root = tempfile::tempdir().expect("temporary folder");
    let proc_root = process_table_with(root.path(), b"/opt/brave.com/brave/brave\0--background\0");

    assert!(ProcessTable::at(&proc_root).is_running());
}

/// Ported from `desktop/tests/test_v07.py::BraveTests::test_proc_unrelated_ignored`
/// parity: INT-019
#[test]
fn other_processes_are_not_brave() {
    let root = tempfile::tempdir().expect("temporary folder");
    let proc_root = process_table_with(root.path(), b"/usr/bin/python3\0script.py\0");

    assert!(!ProcessTable::at(&proc_root).is_running());
}

/// parity: INT-019
#[test]
fn a_process_table_that_cannot_be_read_counts_as_brave_running() {
    let root = tempfile::tempdir().expect("temporary folder");

    assert!(ProcessTable::at(&root.path().join("missing")).is_running());
}

/// parity: INT-019
#[test]
fn inside_flatpak_the_status_says_brave_cannot_be_reached() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.home().join(".var/app/com.brave.Browser")).expect("Flatpak data");

    let on_host = fixture.brave().status();
    let in_flatpak = fixture.brave_in_flatpak().status();

    assert_eq!(on_host.reach, BraveReach::Native);
    assert_eq!(in_flatpak.reach, BraveReach::Sandboxed);
    assert!(in_flatpak.is_running, "host processes cannot be seen");
    assert!(in_flatpak.profiles.is_empty());
    assert!(in_flatpak.sandboxed_installs.is_empty());
    assert!(BraveError::Sandboxed.to_string().contains(MANUAL_SETTINGS_URL));
}

/// parity: INT-019
#[test]
fn the_status_can_be_read_on_a_worker_thread() {
    let fixture = Fixture::new();

    let reading = fixture.brave().run_in_background(BraveIntegration::status);
    let status = glib::MainContext::new().block_on(reading);

    assert_eq!(status.profiles.len(), 1);
    assert!(!status.is_running);
}
