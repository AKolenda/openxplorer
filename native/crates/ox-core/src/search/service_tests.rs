// SPDX-License-Identifier: AGPL-3.0-only
//! Index service tests that need the crate's internals: lowered limits, a
//! scan a stopped app left running, and the changed-folder limit. The
//! other `LiveTests` of `v2.0.0:desktop/tests/test_v05.py` run against the public
//! API in `tests/search_live.rs`.

use std::fs;
use std::time::Instant;

use super::fixtures::{found_names, LocalRoot};
use super::limits::ServiceLimits;
use super::root::{RootStatus, UpdateMode};

/// Ported from `v2.0.0:desktop/tests/test_v05.py::LiveTests::test_watch_limit_fallback_is_reported`
/// (the service half; the watch half is in `watch.rs`).
///
/// parity: SRCH-022, SRCH-028, SRCH-029
#[test]
fn a_folder_beyond_the_watch_limit_falls_back_to_timed_checks() {
    let local = LocalRoot::new();
    let limits = ServiceLimits {
        watched_folders: 1,
        ..ServiceLimits::default()
    };
    let service = local.start_service(limits);
    service.refresh(&local.root).unwrap();
    local.tick_until(&service, "the first scan is ready", |root| {
        root.status == RootStatus::Ready
    });

    fs::create_dir(local.tree.join("extra")).unwrap();

    local.tick_until(&service, "the watch limit is reported", |root| {
        root.watch_error.is_some()
    });
    let root = local.state();
    assert_eq!(root.update_mode, UpdateMode::LiveWithTimedFallback);
    assert!(root.update_mode.as_str().contains("fallback"));
    assert_eq!(
        root.watch_error.as_deref(),
        Some("Live watch limit reached (8192 directories). Unwatched directories use timed checks.")
    );
}

/// Safety rule "at most a million entries per root", reached with a limit
/// of three: the scan stores three of five files and stops with the limit
/// message.
///
/// parity: SRCH-032
#[test]
fn a_scan_stops_storing_at_the_entry_limit() {
    let local = LocalRoot::new();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"] {
        fs::write(local.tree.join(name), "x").unwrap();
    }
    let limits = ServiceLimits {
        entries_per_root: 3,
        ..ServiceLimits::default()
    };

    let service = local.start_service(limits);

    local.tick_until(&service, "the scan stops", |root| {
        root.status == RootStatus::Incomplete
    });
    let root = local.state();
    assert_eq!(root.status.as_str(), "Incomplete / offline");
    assert_eq!(
        root.error.as_deref(),
        Some(
            "Some folders could not be read. One-million-entry limit reached. \
             Select smaller roots; additional entries were not indexed."
        )
    );
    assert_eq!(root.entry_count, 3);
}

/// Safety rule "at most a million entries per folder", reached with a
/// limit of one: a live update of a folder that grew to two files stores
/// neither and reports the check as failed. A full scan counts entries
/// per root instead, so the first scan stores the first file.
///
/// parity: SRCH-030, SRCH-032
#[test]
fn a_live_update_leaves_a_folder_beyond_the_entry_limit_as_it_was() {
    let local = LocalRoot::new();
    fs::write(local.tree.join("first.txt"), "x").unwrap();
    let limits = ServiceLimits {
        entries_per_folder: 1,
        ..ServiceLimits::default()
    };
    let service = local.start_service(limits);
    local.tick_until(&service, "the first scan is ready", |root| {
        root.status == RootStatus::Ready
    });

    fs::write(local.tree.join("second.txt"), "x").unwrap();

    local.tick_until(&service, "the update fails", |root| {
        root.update_mode == UpdateMode::OfflineChecks
    });
    let root = local.state();
    assert_eq!(root.update_mode.as_str(), "Offline / incomplete checks");
    assert_eq!(
        root.watch_error.as_deref(),
        Some("Directory exceeds the one-million-entry safety limit.")
    );
    assert_eq!(root.entry_count, 1);
    assert!(found_names(&local.index, "second").is_empty());
}

/// Safety rule "at most 10,000 folders per live update", reached with a
/// limit of two: a moved-in tree three folders deep is stored down to the
/// second folder, and the update stops with the limit message.
///
/// parity: SRCH-030, SRCH-032
#[test]
fn a_live_update_stops_at_the_new_folder_limit() {
    let local = LocalRoot::new();
    let limits = ServiceLimits {
        folders_per_update: 2,
        ..ServiceLimits::default()
    };
    let service = local.start_service(limits);
    local.tick_until(&service, "the first scan is ready", |root| {
        root.status == RootStatus::Ready
    });
    let outside = local.tree.with_file_name("outside");
    let deepest = outside.join("level1/level2/level3");
    fs::create_dir_all(&deepest).unwrap();

    fs::rename(outside.join("level1"), local.tree.join("level1")).unwrap();

    local.tick_until(&service, "the update stops", |root| {
        root.update_mode == UpdateMode::OfflineChecks
    });
    let root = local.state();
    assert_eq!(
        root.watch_error.as_deref(),
        Some("Many new directories appeared; use Refresh for a complete scan.")
    );
    assert_eq!(found_names(&local.index, "level2"), ["level2"]);
    assert!(found_names(&local.index, "level3").is_empty());
}

/// parity: SRCH-025
#[test]
fn the_next_owner_marks_a_scan_left_running_as_interrupted() {
    let local = LocalRoot::new();
    // A scan an application stopped in the middle of.
    local.index.begin_scan(&local.root).unwrap();

    let service = local.start_service(ServiceLimits::default());

    assert!(service.is_owner());
    let root = local.state();
    assert_eq!(root.status, RootStatus::Interrupted);
    assert_eq!(
        root.error.as_deref(),
        Some("Refresh to finish the interrupted scan.")
    );
}

/// Safety rule "only the index owner recovers": a second window opening
/// the cache never marks the owner's running scan as interrupted.
///
/// parity: SRCH-025, SRCH-027
#[test]
fn a_second_process_leaves_the_owners_scan_alone() {
    let local = LocalRoot::new();
    let owner = local.start_service(ServiceLimits::default());
    // The owner's scan is running.
    local.index.begin_scan(&local.root).unwrap();

    let second = local.start_service(ServiceLimits::default());

    assert!(owner.is_owner());
    assert!(!second.is_owner());
    assert_eq!(local.state().status, RootStatus::Indexing);
}

/// parity: SRCH-029
#[test]
fn too_many_changed_folders_force_a_full_rescan() {
    let local = LocalRoot::new();
    let service = local.start_service(ServiceLimits::default());
    let mut state = service.shared.state();
    for number in 0..=8192 {
        let folder = format!("{}/{number}", local.root);
        state.record_dirty_folder(&local.root, &folder, Instant::now());
    }
    assert!(state.forced_rescans.is_empty());

    let last = format!("{}/last", local.root);
    state.record_dirty_folder(&local.root, &last, Instant::now());

    assert!(state.forced_rescans.contains(&local.root));
    assert_eq!(state.dirty.len(), 1);
}
