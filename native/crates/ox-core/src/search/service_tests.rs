// SPDX-License-Identifier: AGPL-3.0-only
//! Index service tests that need the crate's internals: a small watch
//! limit, a scan a stopped app left running, and the changed-folder
//! limit. The other `LiveTests` of `desktop/tests/test_v05.py` run against
//! the public API in `tests/search_live.rs`.

use std::fs;
use std::time::Instant;

use super::fixtures::LocalRoot;
use super::root::{RootStatus, UpdateMode};
use super::watch::WATCH_LIMIT;

/// Ported from `desktop/tests/test_v05.py::LiveTests::test_watch_limit_fallback_is_reported`
/// (the service half; the watch half is in `watch.rs`).
///
/// parity: SRCH-022, SRCH-028, SRCH-029
#[test]
fn a_folder_beyond_the_watch_limit_falls_back_to_timed_checks() {
    let local = LocalRoot::new();
    let service = local.start_service(1);
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

/// parity: SRCH-025
#[test]
fn the_next_owner_marks_a_scan_left_running_as_interrupted() {
    let local = LocalRoot::new();
    // A scan an application stopped in the middle of.
    local.index.begin_scan(&local.root).unwrap();

    let service = local.start_service(WATCH_LIMIT);

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
    let owner = local.start_service(WATCH_LIMIT);
    // The owner's scan is running.
    local.index.begin_scan(&local.root).unwrap();

    let second = local.start_service(WATCH_LIMIT);

    assert!(owner.is_owner());
    assert!(!second.is_owner());
    assert_eq!(local.state().status, RootStatus::Indexing);
}

/// parity: SRCH-029
#[test]
fn too_many_changed_folders_force_a_full_rescan() {
    let local = LocalRoot::new();
    let service = local.start_service(WATCH_LIMIT);
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
